use std::sync::Arc;

use picstome_sync_core::model::RawMode;

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::config::Store;
use picstome_sync_core::ignored::{GalleryIgnores, IgnoredLocal, IgnoredRemote};
use picstome_sync_core::localfiles::hash_file;
use picstome_sync_core::mock::{start_mock_server, MockServerOptions};
use picstome_sync_core::model::{GalleryEntry, Snapshot, SnapshotFile, SNAPSHOT_SCHEMA};
use picstome_sync_core::plan::{apply_sync, plan_sync, PlanSelection};
use picstome_sync_core::sync::{
    apply_remote_deletions, mark_favorites_rated, mark_ratings_applied, rating_plan_from_snapshot,
    resolve_remote_deletions, run_gallery_sync, RemoteDeleteAction, SyncOptions,
};
use picstome_sync_core::transfer::{download_file, DownloadOptions};

async fn mock() -> picstome_sync_core::mock::MockServer {
    start_mock_server(MockServerOptions {
        photo_size: 4096,
        ..Default::default()
    })
    .await
    .expect("mock server")
}

async fn login(url: &str) -> ApiClient {
    let api = ApiClient::new(Some(url), None).unwrap();
    api.dev_login().await.expect("demo login");
    api
}

/// Minimal JPEG-looking bytes for upload tests. The upload preflight checks
/// the file header, so synthetic fixtures must start with a JPEG SOI marker.
fn jpeg_bytes(size: usize, fill: u8) -> Vec<u8> {
    assert!(size >= 3);
    let mut bytes = vec![0xFF, 0xD8, 0xFF];
    bytes.resize(size, fill);
    bytes
}

#[tokio::test]
async fn api_manifest_and_range_download() {
    let mock = mock().await;
    let api = login(&mock.url).await;

    let galleries = api.galleries().await.unwrap();
    assert_eq!(galleries.len(), 2);
    assert_eq!(
        galleries.iter().find(|g| g.id == "G123").unwrap().name,
        "MiBoda"
    );

    let manifest = api.manifest("G123").await.unwrap();
    assert_eq!(manifest.photos.len(), 50);
    let photo = &manifest.photos[0];

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("downloaded.jpg");
    let result = download_file(
        &api.http(),
        &photo.url,
        &dest,
        DownloadOptions {
            expected_hash: photo.hash.clone(),
            chunk_bytes: 512,
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();

    assert_eq!(result.hash, photo.hash.clone().unwrap());
    assert_eq!(hash_file(&dest).await.unwrap(), photo.hash.clone().unwrap());
    mock.close().await;
}

#[tokio::test]
async fn api_upload_versions_and_delete() {
    let mock = mock().await;
    let api = login(&mock.url).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("NEW_0001.jpg");
    tokio::fs::write(&file, jpeg_bytes(900, 1)).await.unwrap();

    let first = api
        .upload_photo("G123", "NEW_0001.jpg", &file, 900, None)
        .await
        .unwrap();
    assert_eq!(first.version_count, Some(1));

    let second = api
        .upload_photo("G123", "NEW_0001.jpg", &file, 900, None)
        .await
        .unwrap();
    assert_eq!(second.version_count, Some(1));
    assert!(second.unchanged);

    tokio::fs::write(&file, jpeg_bytes(901, 2)).await.unwrap();
    let third = api
        .upload_photo("G123", "NEW_0001.jpg", &file, 901, None)
        .await
        .unwrap();
    assert_eq!(third.version_count, Some(2));

    let manifest = api.manifest("G123").await.unwrap();
    assert_eq!(manifest.photos.len(), 51);

    let target = manifest
        .photos
        .iter()
        .find(|p| p.identity_key.as_deref() == Some("image:new_0001"))
        .unwrap();
    let deleted = api
        .delete_photos("G123", &[target.id.clone(), "missing".into()])
        .await
        .unwrap();
    assert_eq!(deleted.deleted, vec![target.id.clone()]);
    assert_eq!(deleted.not_found, vec!["missing".to_string()]);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);
    mock.close().await;
}

#[tokio::test]
async fn api_gallery_search_and_pagination() {
    let mock = mock().await;
    let api = login(&mock.url).await;

    let search = api.galleries_page(Some("boda"), 1, 25).await.unwrap();
    assert_eq!(search.data.len(), 1);
    assert_eq!(search.data[0].id, "G123");
    assert_eq!(search.meta.total, 1);
    assert_eq!(search.meta.last_page, 1);

    let first = api.galleries_page(None, 1, 1).await.unwrap();
    assert_eq!(first.data.len(), 1);
    assert_eq!(first.meta.last_page, 2);
    let second = api.galleries_page(None, 2, 1).await.unwrap();
    assert_eq!(second.data.len(), 1);
    assert_ne!(first.data[0].id, second.data[0].id);
    mock.close().await;
}

#[tokio::test]
async fn api_authorization_code_exchange() {
    let mock = mock().await;
    let page = reqwest::get(format!(
        "{}/authorize?nonce=abc123&client_name=picstome-sync",
        mock.url
    ))
    .await
    .unwrap()
    .text()
    .await
    .unwrap();
    let start = page.find("code=").expect("code in page") + 5;
    let code: String = page[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();

    let api = ApiClient::new(Some(&mock.url), None).unwrap();
    let token = api.exchange_code(&code).await.unwrap();
    assert_eq!(token.token, mock.token);
    assert_eq!(api.me().await.unwrap().email, "desktop@picstome.test");
    api.revoke().await.unwrap();
    assert!(api.me().await.is_err());
    mock.close().await;
}

async fn setup_sync() -> (
    picstome_sync_core::mock::MockServer,
    ApiClient,
    tempfile::TempDir,
    Store,
    GalleryEntry,
) {
    let mock = mock().await;
    let api = login(&mock.url).await;
    let dir = tempfile::tempdir().unwrap();
    let local_path = dir.path().join("MiBoda");
    tokio::fs::create_dir_all(&local_path).await.unwrap();
    let store = Store::new(dir.path().join("state")).unwrap();
    let gallery = store
        .upsert_gallery(GalleryEntry {
            gallery_id: "G123".into(),
            name: "MiBoda".into(),
            local_path: local_path.to_string_lossy().to_string(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
            raw_mode: None,
        })
        .unwrap()
        .unwrap();
    (mock, api, dir, store, gallery)
}

fn silence() -> Arc<dyn Fn(picstome_sync_core::model::Progress) + Send + Sync> {
    Arc::new(|_| {})
}

type ProgressSink = Arc<dyn Fn(picstome_sync_core::model::Progress) + Send + Sync>;
type ProgressLog = Arc<std::sync::Mutex<Vec<picstome_sync_core::model::Progress>>>;

/// Collects the progress phases emitted during a sync.
fn recorder() -> (ProgressLog, ProgressSink) {
    let events: ProgressLog = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = events.clone();
    let progress: ProgressSink = Arc::new(move |event| sink.lock().unwrap().push(event));
    (events, progress)
}

#[tokio::test]
async fn sync_reports_scan_and_hash_progress() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    // A local file makes the scan phase walk and fingerprint something.
    let path = std::path::Path::new(&gallery.local_path).join("LOCAL.jpg");
    std::fs::write(path, jpeg_bytes(120, 5)).unwrap();

    let (events, progress) = recorder();
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), progress)
        .await
        .unwrap();

    // Clone the events out before the final await: holding a std MutexGuard
    // across an await point is denied by clippy and would deadlock.
    let recorded = events.lock().unwrap().clone();
    let scan: Vec<_> = recorded
        .iter()
        .filter(|event| event.phase == "scan")
        .collect();
    let hash: Vec<_> = recorded
        .iter()
        .filter(|event| event.phase == "hash")
        .collect();
    assert!(!scan.is_empty(), "no scan progress emitted");
    assert!(!hash.is_empty(), "no hash progress emitted");
    // The walk reports how many entries it visited, and the hash phase knows
    // the total so the UI can draw a real bar.
    assert!(
        scan.iter().any(|event| event.file_done.unwrap_or(0) > 0),
        "scan progress never reported entries: {scan:?}"
    );
    assert!(
        hash.iter().any(|event| event.total > 0),
        "hash progress never reported a total: {hash:?}"
    );
    assert!(
        hash.iter()
            .any(|event| event.done == event.total && event.total > 0),
        "hash progress never reached its total: {hash:?}"
    );

    mock.close().await;
}

#[tokio::test]
async fn sync_downloads_then_is_a_noop() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let report = run_gallery_sync(
        &api,
        &store,
        &gallery,
        SyncOptions {
            chunk_bytes: 1024,
            ..Default::default()
        },
        silence(),
    )
    .await
    .unwrap();

    assert_eq!(report.downloaded.len(), 50);
    assert_eq!(report.errors.len(), 0);
    assert_eq!(report.counts.remote_new, 50);
    // Five favourites carry a heart. Four already have five stars in the
    // manifest; the legacy one is the only photo the prompt has to ask about.
    assert_eq!(report.favorites.len(), 1);
    assert!(!report.rating_plan.is_empty());
    let snapshot = store.load_snapshot("G123").unwrap();
    assert_eq!(snapshot.files.len(), 50);
    assert_eq!(snapshot.local_photo_count, 50);
    let local_bytes: u64 = std::fs::read_dir(&gallery.local_path)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum();
    assert_eq!(snapshot.local_bytes, local_bytes);
    assert_eq!(
        std::fs::read_dir(&gallery.local_path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".jpg"))
            .count(),
        50
    );

    let second = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(second.downloaded.len(), 0);
    assert_eq!(second.uploaded.len(), 0);
    assert!(second.skipped);
    mock.close().await;
}

#[tokio::test]
async fn existing_local_original_is_adopted_even_if_remote_copy_is_resized() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let manifest = api.manifest("G123").await.unwrap();
    let target = &manifest.photos[0];
    let local_path = std::path::Path::new(&gallery.local_path).join(&target.filename);
    tokio::fs::write(&local_path, vec![7u8; 6000])
        .await
        .unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.counts.conflicts, 0);
    assert_eq!(report.counts.local_modified, 0);
    assert_eq!(report.uploaded.len(), 0);
    assert!(std::fs::metadata(&local_path).unwrap().len() == 6000);
    assert!(store
        .load_snapshot("G123")
        .unwrap()
        .files
        .iter()
        .any(|file| file.filename == target.filename && file.local_present));
    mock.close().await;
}

#[tokio::test]
async fn remote_only_snapshot_entries_are_downloaded_not_deleted_remotely() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let manifest = api.manifest("G123").await.unwrap();
    let snapshot = Snapshot {
        schema: SNAPSHOT_SCHEMA,
        gallery_id: "G123".into(),
        name: "MiBoda".into(),
        local_path: gallery.local_path.clone(),
        last_sync_at: 1,
        remote_rev: manifest.rev.clone(),
        local_photo_count: 0,
        local_bytes: 0,
        files: manifest
            .photos
            .iter()
            .map(|photo| SnapshotFile {
                id: photo.id.clone(),
                filename: photo.filename.clone(),
                identity_key: photo.identity(),
                relative_path: Some(photo.filename.clone()),
                hash: photo.hash.clone(),
                hash_algo: photo.hash_algo.clone(),
                size: photo.size,
                mtime: None,
                local_present: false,
                local_hash: None,
                local_size: None,
                version_count: photo.version_count,
                version_number: photo.version_number,
                remote_deleted: false,
                keep_local: false,
                favorite_rated_for: None,
                favorited_at: None,
                rating: None,
                rating_applied: None,
                is_favorite: false,
            })
            .collect(),
    };
    store.save_snapshot(&snapshot).unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.counts.remote_new, 50);
    assert_eq!(report.counts.local_deleted, 0);
    assert!(report.proposals.delete_remote.is_empty());
    assert_eq!(report.downloaded.len(), 50);

    let repaired = store.load_snapshot("G123").unwrap();
    assert_eq!(repaired.files.len(), 50);
    assert!(repaired.files.iter().all(|file| file.mtime.is_some()));
    mock.close().await;
}

#[tokio::test]
async fn oversized_files_are_skipped_never_deleted_or_overwritten() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let mut files: Vec<_> = std::fs::read_dir(&gallery.local_path)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    files.sort();
    let big = files[0].clone();
    let big_size = 501 * 1024 * 1024;
    let file = std::fs::OpenOptions::new().write(true).open(&big).unwrap();
    file.set_len(big_size).unwrap();
    drop(file);

    // Two syncs: the second one proves the skip is stable (no re-download of
    // the remote version over the large local file).
    for _ in 0..2 {
        let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
            .await
            .unwrap();
        assert_eq!(report.counts.local_deleted, 0);
        assert_eq!(report.counts.remote_new, 0);
        assert!(report.proposals.delete_remote.is_empty());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.kind == "file_too_large"));
        assert_eq!(std::fs::metadata(&big).unwrap().len(), big_size);
    }
    mock.close().await;
}

#[tokio::test]
async fn remote_new_version_replaces_the_file_in_its_subfolder() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    reqwest::Client::new()
        .post(format!("{}/api/v1/dev/bump", mock.url))
        .bearer_auth(&mock.token)
        .json(&serde_json::json!({"gallery_id": "G123"}))
        .send()
        .await
        .unwrap();
    let bumped = api
        .manifest("G123")
        .await
        .unwrap()
        .photos
        .into_iter()
        .find(|photo| photo.version_count == Some(2))
        .expect("bumped photo");

    // The photographer organised the photo into a subfolder after the first
    // sync. The new version must replace it there, not appear in the root.
    let root = std::path::Path::new(&gallery.local_path);
    let folder = root.join("selects");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::rename(root.join(&bumped.filename), folder.join(&bumped.filename)).unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.downloaded.len(), 1);
    assert!(!root.join(&bumped.filename).exists());
    let replaced = std::fs::read(folder.join(&bumped.filename)).unwrap();
    assert_eq!(
        Some(picstome_sync_core::localfiles::hash_bytes(&replaced)),
        bumped.content_hash().map(str::to_string)
    );

    let again = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(again.skipped, "{:?}", again.counts);
    mock.close().await;
}

#[tokio::test]
async fn sync_handles_remote_and_local_changes() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    client
        .post(format!("{}/api/v1/dev/bump", mock.url))
        .bearer_auth(&mock.token)
        .json(&serde_json::json!({"gallery_id": "G123"}))
        .send()
        .await
        .unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.counts.remote_modified, 1);
    assert_eq!(report.downloaded.len(), 1);

    let mut files: Vec<_> = std::fs::read_dir(&gallery.local_path)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    files.sort();
    std::fs::write(&files[0], jpeg_bytes(1234, 9)).unwrap();
    std::fs::write(
        format!("{}/LOCAL_NEW.jpg", gallery.local_path),
        jpeg_bytes(777, 7),
    )
    .unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.counts.local_modified, 1);
    assert_eq!(report.counts.local_new, 1);
    assert_eq!(report.uploaded.len(), 2);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 51);

    std::fs::remove_file(format!("{}/LOCAL_NEW.jpg", gallery.local_path)).unwrap();
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.proposals.delete_remote.len(), 1);

    let ids: Vec<String> = report
        .proposals
        .delete_remote
        .iter()
        .map(|proposal| proposal.id.clone())
        .collect();
    let deleted = apply_remote_deletions(&api, &store, "G123", &ids)
        .await
        .unwrap();
    assert_eq!(deleted.deleted.len(), 1);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);
    mock.close().await;
}

#[tokio::test]
async fn sync_reports_conflicts_without_overwriting() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    reqwest::Client::new()
        .post(format!("{}/api/v1/dev/bump", mock.url))
        .bearer_auth(&mock.token)
        .json(&serde_json::json!({"gallery_id": "G123"}))
        .send()
        .await
        .unwrap();

    let mut files: Vec<_> = std::fs::read_dir(&gallery.local_path)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    files.sort();
    std::fs::write(&files[0], vec![5u8; 999]).unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.counts.conflicts, 1);
    assert_eq!(report.downloaded.len(), 0);
    assert_eq!(report.uploaded.len(), 0);
    assert_eq!(std::fs::read(&files[0]).unwrap(), vec![5u8; 999]);
    mock.close().await;
}

#[tokio::test]
async fn remote_deletions_are_reported_and_resolved_explicitly() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let remove_remote = |photo_id: String| {
        let client = client.clone();
        let url = format!("{}/api/v1/dev/remove", mock.url);
        let token = mock.token.clone();
        async move {
            client
                .post(url)
                .bearer_auth(token)
                .json(&serde_json::json!({"gallery_id": "G123", "photo_id": photo_id}))
                .send()
                .await
                .unwrap();
        }
    };

    let manifest = api.manifest("G123").await.unwrap();
    let kept = manifest.photos.last().unwrap().clone();
    remove_remote(kept.id.clone()).await;

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.remote_deleted.len(), 1);
    assert_eq!(report.remote_deleted[0].id, kept.id);
    assert_eq!(report.uploaded.len(), 0, "remote deletions must not upload");

    let again = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(
        again.remote_deleted.len(),
        1,
        "must keep asking, not upload"
    );
    assert_eq!(again.uploaded.len(), 0);

    let kept_path = std::path::Path::new(&gallery.local_path).join(&kept.filename);
    let resolution = resolve_remote_deletions(
        &api,
        &store,
        &gallery,
        std::slice::from_ref(&kept.id),
        RemoteDeleteAction::KeepLocal,
        false,
        1,
    )
    .await
    .unwrap();
    assert_eq!(resolution.resolved, vec![kept.id.clone()]);
    assert!(kept_path.exists());
    let after_keep = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(after_keep.remote_deleted.len(), 0);

    let manifest = api.manifest("G123").await.unwrap();
    let deleted = manifest.photos.last().unwrap().clone();
    remove_remote(deleted.id.clone()).await;
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.remote_deleted.len(), 1);
    let deleted_path = std::path::Path::new(&gallery.local_path).join(&deleted.filename);
    assert!(deleted_path.exists());
    let resolution = resolve_remote_deletions(
        &api,
        &store,
        &gallery,
        std::slice::from_ref(&deleted.id),
        RemoteDeleteAction::DeleteLocal,
        false,
        2,
    )
    .await
    .unwrap();
    assert_eq!(resolution.resolved, vec![deleted.id.clone()]);
    assert!(!deleted_path.exists());
    assert_eq!(resolution.trashed.len(), 1);
    assert!(
        resolution.trashed[0].location == "system" || resolution.trashed[0].location == "gallery"
    );
    if resolution.trashed[0].location == "gallery" {
        let trash_path = resolution.trashed[0].trash_path.as_deref().unwrap();
        assert!(std::path::Path::new(trash_path).exists());
    }
    let after_delete = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(after_delete.remote_deleted.len(), 0);

    let manifest = api.manifest("G123").await.unwrap();
    let restored = manifest.photos.last().unwrap().clone();
    remove_remote(restored.id.clone()).await;
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.remote_deleted.len(), 1);
    let resolution = resolve_remote_deletions(
        &api,
        &store,
        &gallery,
        std::slice::from_ref(&restored.id),
        RemoteDeleteAction::Upload,
        false,
        3,
    )
    .await
    .unwrap();
    assert_eq!(resolution.resolved, vec![restored.id.clone()]);
    let remote = api.manifest("G123").await.unwrap();
    assert!(
        remote
            .photos
            .iter()
            .any(|photo| photo.filename == restored.filename),
        "uploaded photo should be back in the manifest"
    );

    mock.close().await;
}

#[tokio::test]
async fn favourite_without_rating_is_asked_once() {
    // The mock seeds one legacy favourite (heart, no rating) and four
    // favourites rated 5. Only the legacy one has to be asked.
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.favorites.len(), 1);
    assert!(report.favorites[0].rating.is_none());
    let marks: Vec<(String, String)> = report
        .favorites
        .iter()
        .filter_map(|photo| Some((photo.identity_key.clone()?, photo.favorited_at.clone()?)))
        .collect();
    assert_eq!(marks.len(), 1);

    assert_eq!(mark_favorites_rated(&store, "G123", &marks).unwrap(), 1);
    let after = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(after.favorites.is_empty());

    mock.close().await;
}

#[tokio::test]
async fn manifest_ratings_reach_the_rating_plan() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    // Exact Picstome ratings (1-5) are planned for disk, with no guessing.
    let ratings: Vec<(String, Option<u8>)> = report
        .rating_plan
        .items
        .iter()
        .map(|item| (item.filename.clone(), item.rating))
        .collect();
    assert!(
        ratings.iter().any(|(_, rating)| *rating == Some(5)),
        "five-star favourites should be written: {ratings:?}"
    );
    assert!(
        ratings
            .iter()
            .any(|(_, rating)| matches!(rating, Some(1..=4))),
        "1-4 star ratings should be written: {ratings:?}"
    );
    // The legacy favourite has no rating yet: it is in the prompt instead.
    assert!(report
        .rating_plan
        .items
        .iter()
        .all(|item| item.rating.is_some()));

    // A rating-only change must not re-download the file.
    assert_eq!(report.downloaded.len(), 50);
    let snapshot = store.load_snapshot("G123").unwrap();
    let rated_file = snapshot
        .files
        .iter()
        .find(|file| file.rating == Some(5))
        .expect("snapshot keeps the rating");
    assert!(rated_file.is_favorite);
    assert!(rated_file.favorited_at.is_some());

    let second = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(second.downloaded.is_empty());
    assert_eq!(second.counts.remote_new, 0);
    assert_eq!(second.counts.remote_modified, 0);

    mock.close().await;
}

#[tokio::test]
async fn applying_a_rating_clears_the_plan() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(!report.rating_plan.items.is_empty());

    let applied: Vec<(String, Option<u8>)> = report
        .rating_plan
        .items
        .iter()
        .filter_map(|item| Some((item.identity_key.clone()?, item.rating)))
        .collect();
    assert_eq!(
        mark_ratings_applied(&store, "G123", &applied, &[]).unwrap(),
        applied.len()
    );

    let snapshot = store.load_snapshot("G123").unwrap();
    let plan = rating_plan_from_snapshot(&snapshot, None);
    assert!(
        plan.items.is_empty(),
        "an applied rating must not be queued again: {:?}",
        plan.items
    );

    mock.close().await;
}

#[tokio::test]
async fn legacy_favourite_uses_the_chosen_stars_from_the_snapshot() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.favorites.len(), 1);

    let snapshot = store.load_snapshot("G123").unwrap();
    let unanswered = rating_plan_from_snapshot(&snapshot, None);
    assert_eq!(unanswered.needs_rating.len(), 1);
    assert!(unanswered
        .items
        .iter()
        .all(|item| item.rating.is_some() && item.applied_rating != item.rating));

    let answered = rating_plan_from_snapshot(&snapshot, Some(4));
    assert!(answered.needs_rating.is_empty());
    let legacy = answered
        .items
        .iter()
        .find(|item| item.filename == report.favorites[0].filename)
        .expect("the chosen stars are applied to the legacy favourite");
    assert_eq!(legacy.rating, Some(4));
    assert!(legacy.is_favorite);

    // Once written, the answer is remembered: the prompt does not come back
    // and the local stars are not cleared just because Picstome has no rating.
    let applied: Vec<(String, Option<u8>)> = answered
        .items
        .iter()
        .filter_map(|item| Some((item.identity_key.clone()?, item.rating)))
        .collect();
    let answered_for: Vec<(String, String)> = report
        .favorites
        .iter()
        .filter_map(|photo| Some((photo.identity_key.clone()?, photo.favorited_at.clone()?)))
        .collect();
    mark_ratings_applied(&store, "G123", &applied, &answered_for).unwrap();

    let after = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(
        after.favorites.is_empty(),
        "asked again: {:?}",
        after.favorites
    );
    assert!(
        after
            .rating_plan
            .items
            .iter()
            .all(|item| item.filename != report.favorites[0].filename || item.rating == Some(4)),
        "local stars were cleared: {:?}",
        after.rating_plan.items
    );

    mock.close().await;
}

#[tokio::test]
async fn first_sync_plan_lists_both_sides_and_moves_nothing() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let manifest = api.manifest("G123").await.unwrap();
    for photo in manifest.photos.iter().take(2) {
        let path = std::path::Path::new(&gallery.local_path).join(&photo.filename);
        std::fs::write(path, jpeg_bytes(120, 3)).unwrap();
    }
    std::fs::write(
        std::path::Path::new(&gallery.local_path).join("LOCAL_ONLY.jpg"),
        jpeg_bytes(80, 4),
    )
    .unwrap();

    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    let plan = &planned.plan;
    assert!(plan.first_sync);
    assert_eq!(plan.remote_total, 50);
    assert_eq!(plan.uploads.len(), 1);
    assert_eq!(plan.uploads[0].filename, "LOCAL_ONLY.jpg");
    assert_eq!(plan.downloads.len(), 48);
    // A review changes nothing: no snapshot, no download, no upload.
    assert!(store.load_snapshot("G123").is_none());
    assert_eq!(std::fs::read_dir(&gallery.local_path).unwrap().count(), 3);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);
    mock.close().await;
}

#[tokio::test]
async fn ignored_local_file_is_not_uploaded_until_it_changes() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let path = std::path::Path::new(&gallery.local_path).join("IGNORED.jpg");
    let bytes = jpeg_bytes(700, 7);
    std::fs::write(&path, &bytes).unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    let mtime_ms = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut ignores = GalleryIgnores::default();
    ignores.local.push(IgnoredLocal {
        relative_path: "IGNORED.jpg".into(),
        size: bytes.len() as u64,
        mtime_ms,
    });
    ignores
        .save(std::path::Path::new(&gallery.local_path))
        .unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(report.uploaded.is_empty(), "{:?}", report.uploaded);
    assert_eq!(report.ignored_local, vec!["IGNORED.jpg".to_string()]);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);

    std::fs::write(&path, jpeg_bytes(900, 8)).unwrap();
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.uploaded.len(), 1);
    assert!(report.ignored_local.is_empty());
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 51);
    mock.close().await;
}

#[tokio::test]
async fn ignored_remote_photo_is_not_downloaded_until_it_changes() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    client
        .post(format!("{}/api/v1/dev/add", mock.url))
        .bearer_auth(&mock.token)
        .json(&serde_json::json!({
            "gallery_id": "G123",
            "filename": "REMOTE_IGNORED.jpg",
        }))
        .send()
        .await
        .unwrap();

    let manifest = api.manifest("G123").await.unwrap();
    let target = manifest
        .photos
        .iter()
        .find(|photo| photo.filename == "REMOTE_IGNORED.jpg")
        .unwrap();
    let mut ignores = GalleryIgnores::default();
    ignores.remote.push(IgnoredRemote {
        id: target.id.clone(),
        version_count: target.version_count,
    });
    ignores
        .save(std::path::Path::new(&gallery.local_path))
        .unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(report.downloaded.is_empty(), "{:?}", report.downloaded);
    assert_eq!(
        report.ignored_remote,
        vec!["REMOTE_IGNORED.jpg".to_string()]
    );
    assert!(!std::path::Path::new(&gallery.local_path)
        .join("REMOTE_IGNORED.jpg")
        .exists());

    client
        .post(format!("{}/api/v1/dev/bump", mock.url))
        .bearer_auth(&mock.token)
        .json(&serde_json::json!({
            "gallery_id": "G123",
            "photo_id": target.id,
        }))
        .send()
        .await
        .unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.downloaded.len(), 1);
    assert!(report.ignored_remote.is_empty());
    mock.close().await;
}

#[tokio::test]
async fn first_sync_can_trash_local_only_files() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let path = std::path::Path::new(&gallery.local_path).join("TRASH_ME.jpg");
    std::fs::write(&path, jpeg_bytes(640, 6)).unwrap();

    let report = run_gallery_sync(
        &api,
        &store,
        &gallery,
        SyncOptions {
            trash_local: vec!["TRASH_ME.jpg".into()],
            ..Default::default()
        },
        silence(),
    )
    .await
    .unwrap();

    assert!(!path.exists(), "local-only file should be moved to trash");
    assert_eq!(report.trashed_local.len(), 1);
    assert_eq!(report.trashed_local[0].filename, "TRASH_ME.jpg");
    assert!(report.uploaded.is_empty(), "{:?}", report.uploaded);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);
    mock.close().await;
}

#[tokio::test]
async fn first_sync_trashes_every_local_representation() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let root = std::path::Path::new(&gallery.local_path);
    let jpg = root.join("PAIR.JPG");
    let dng = root.join("PAIR.DNG");
    std::fs::write(&jpg, jpeg_bytes(640, 5)).unwrap();
    let mut dng_bytes = b"II*\0".to_vec();
    dng_bytes.resize(640, 0);
    std::fs::write(&dng, &dng_bytes).unwrap();

    // Trashing the photo takes every file of it (JPEG and RAW) to the trash.
    let report = run_gallery_sync(
        &api,
        &store,
        &gallery,
        SyncOptions {
            trash_local: vec!["PAIR.JPG".into()],
            ..Default::default()
        },
        silence(),
    )
    .await
    .unwrap();

    assert!(!jpg.exists(), "JPG should be trashed");
    assert!(!dng.exists(), "RAW should be trashed too");
    assert!(report.uploaded.is_empty(), "{:?}", report.uploaded);
    assert_eq!(report.trashed_local.len(), 2);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);
    mock.close().await;
}

/// The regression test for the "it re-reads every image on every sync" bug.
///
/// A shoot folder holds a JPEG and a RAW for the same photo. The snapshot only
/// covers the preferred JPEG, so the RAW used to be re-hashed (full content
/// read) on every single sync. A repeat sync must now read no file content at
/// all: everything resolves from the fingerprint cache.
#[tokio::test]
async fn repeat_sync_reads_no_file_content_not_even_raw_duplicates() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let root = std::path::Path::new(&gallery.local_path);
    let jpg = root.join("PAIR.JPG");
    let dng = root.join("PAIR.DNG");
    std::fs::write(&jpg, jpeg_bytes(4096, 7)).unwrap();
    let mut raw = b"II*\0".to_vec();
    raw.resize(8192, 9);
    std::fs::write(&dng, &raw).unwrap();

    // First run hashes the new files (the RAW is a duplicate of the JPEG).
    let first = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(first.scan_stats.hashed >= 1, "{:?}", first.scan_stats);
    // The RAW is never read: only the preferred JPEG is fingerprinted.
    assert!(
        first.scan_stats.bytes_read <= 4096 + 60 * 4096,
        "unexpected first-run read volume: {:?}",
        first.scan_stats
    );

    // Second run: nothing changed anywhere, so not a single byte may be read.
    let second = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(second.skipped, "second sync should be a no-op");
    assert_eq!(
        second.scan_stats.hashed, 0,
        "repeat sync re-hashed files: {:?}",
        second.scan_stats
    );
    assert_eq!(
        second.scan_stats.bytes_read, 0,
        "repeat sync re-read content: {:?}",
        second.scan_stats
    );
    // Every file is accounted for without reading: the JPEG is cached, the RAW
    // is a duplicate that is deliberately never hashed.
    assert!(
        second.scan_stats.cached + second.scan_stats.skipped_duplicates == second.scan_stats.files,
        "{:?}",
        second.scan_stats
    );

    // Now add a *new* RAW next to an already-cached JPEG. The JPEG's hash is
    // known, so the RAW is skipped instead of being read in full.
    let mut late_raw = b"II*\0".to_vec();
    late_raw.resize(8192, 8);
    std::fs::write(root.join("PAIR.CR3"), &late_raw).unwrap();
    let third = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(
        third.scan_stats.hashed, 0,
        "new duplicate RAW was read: {:?}",
        third.scan_stats
    );
    assert!(
        third.scan_stats.skipped_duplicates >= 1,
        "{:?}",
        third.scan_stats
    );

    mock.close().await;
}

/// The fingerprint cache must survive a snapshot reset (re-link or schema
/// bump) so a folder that has not changed is never rehashed from scratch.
#[tokio::test]
async fn fingerprint_cache_survives_snapshot_reset() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    store.delete_snapshot("G123").unwrap();
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    // With no snapshot the files are adopted as in sync and nothing is read.
    assert_eq!(report.scan_stats.hashed, 0, "{:?}", report.scan_stats);
    assert_eq!(report.scan_stats.bytes_read, 0, "{:?}", report.scan_stats);

    mock.close().await;
}

/// A file that changes on disk must be re-hashed, even when only its content
/// changes and the size stays the same (mtime still moves in real life, but
/// the cache must not rely on the hash alone).
#[tokio::test]
async fn changed_file_is_rehashed_and_uploaded() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let target = std::fs::read_dir(&gallery.local_path)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jpg"))
        .unwrap();
    let mut bytes = std::fs::read(&target).unwrap();
    let last = bytes.len() - 1;
    bytes[last] = bytes[last].wrapping_add(1);
    std::fs::write(&target, &bytes).unwrap();
    // Guarantee an mtime change even on coarse filesystems.
    filetime_bump(&target);

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.scan_stats.hashed, 1, "{:?}", report.scan_stats);
    assert_eq!(report.uploaded.len(), 1, "{:?}", report.uploaded);

    mock.close().await;
}

fn filetime_bump(path: &std::path::Path) {
    use std::time::{Duration, SystemTime};
    let later = SystemTime::now() + Duration::from_secs(2);
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(later).unwrap();
}

/// The download path must not depend on a range probe: the manifest size is
/// enough. This is what removes one HTTP round trip per downloaded photo.
#[tokio::test]
async fn downloads_with_known_size_skip_the_probe_request() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let payload = b"picstome-sync-probe-check".repeat(200);
    let body = payload.clone();
    let requests = Arc::new(AtomicUsize::new(0));

    let app = {
        let body = body.clone();
        let requests = requests.clone();
        axum::Router::new().route(
            "/file",
            axum::routing::get(move |headers: axum::http::HeaderMap| {
                let body = body.clone();
                let requests = requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    let range = headers
                        .get(axum::http::header::RANGE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    if let Some(spec) = range.strip_prefix("bytes=") {
                        let (start, end) = spec.split_once('-').unwrap_or(("0", ""));
                        let start: usize = start.parse().unwrap_or(0);
                        let end: usize = end
                            .parse()
                            .unwrap_or(body.len().saturating_sub(1))
                            .min(body.len().saturating_sub(1));
                        let slice = body[start..=end].to_vec();
                        return axum::response::Response::builder()
                            .status(axum::http::StatusCode::PARTIAL_CONTENT)
                            .header(
                                axum::http::header::CONTENT_RANGE,
                                format!("bytes {start}-{end}/{}", body.len()),
                            )
                            .body(axum::body::Body::from(slice))
                            .unwrap();
                    }
                    axum::response::Response::builder()
                        .status(axum::http::StatusCode::OK)
                        .body(axum::body::Body::from(body))
                        .unwrap()
                }
            }),
        )
    };

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");
    let result = download_file(
        &reqwest::Client::new(),
        &format!("http://{addr}/file"),
        &dest,
        DownloadOptions {
            expected_hash: Some(picstome_sync_core::localfiles::hash_bytes(&payload)),
            expected_size: Some(payload.len() as u64),
            chunk_bytes: 1024,
            parallel: 4,
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert_eq!(result.size, payload.len() as u64);
    // One request per chunk, and no extra `bytes=0-0` probe.
    let expected_chunks = payload.len().div_ceil(1024);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        expected_chunks,
        "unexpected request count (probe?)"
    );
}

#[tokio::test]
async fn resolves_and_caches_gallery_covers() {
    use picstome_sync_core::covers::{resolve_cover, CoverCache};

    let mock = mock().await;
    let api = login(&mock.url).await;
    let dir = tempfile::tempdir().unwrap();
    let cache = CoverCache::new(dir.path());

    let cover = resolve_cover(&api, &cache, "G123", "MiBoda", dir.path(), 1_000)
        .await
        .expect("cover");
    assert_eq!(cover.mime, "image/svg+xml");
    assert!(String::from_utf8_lossy(&cover.bytes).contains("<svg"));

    // The photographer's profile photo comes from /me and is cached the same way.
    let avatar_url = api.me().await.unwrap().avatar_url.expect("avatar url");
    let avatar = picstome_sync_core::covers::cached_or_fetch(
        &api.http(),
        &cache,
        "@account",
        &avatar_url,
        1_000,
    )
    .await
    .expect("avatar");
    assert_eq!(avatar.mime, "image/svg+xml");

    // Fresh cache: served without asking Picstome, even when it is gone.
    mock.close().await;
    let offline_avatar = picstome_sync_core::covers::cached_or_fetch(
        &api.http(),
        &cache,
        "@account",
        &avatar_url,
        2_000,
    )
    .await;
    assert_eq!(offline_avatar, Some(avatar));
    let cached = resolve_cover(&api, &cache, "G123", "MiBoda", dir.path(), 2_000)
        .await
        .expect("cached cover");
    assert_eq!(cached, cover);

    // Stale cache and Picstome unreachable: the old cover is still shown.
    let later = 2_000 + picstome_sync_core::covers::COVER_REFRESH_MS + 1;
    let stale = resolve_cover(&api, &cache, "G123", "MiBoda", dir.path(), later).await;
    assert_eq!(stale, Some(cover));
}

fn raw_bytes(fill: u8) -> Vec<u8> {
    let mut raw = b"II*\0".to_vec();
    raw.resize(2048, fill);
    raw
}

fn sync_with(raw_mode: Option<RawMode>) -> SyncOptions {
    SyncOptions {
        raw_mode,
        ..Default::default()
    }
}

#[tokio::test]
async fn photos_with_raw_wait_for_a_choice_then_follow_jpeg_only() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    let root = std::path::Path::new(&gallery.local_path);
    std::fs::write(root.join("ONLY_RAW.CR3"), raw_bytes(3)).unwrap();
    std::fs::write(root.join("WITH_JPG.jpg"), jpeg_bytes(900, 4)).unwrap();
    std::fs::write(root.join("WITH_JPG.NEF"), raw_bytes(4)).unwrap();
    std::fs::write(root.join("PLAIN.jpg"), jpeg_bytes(800, 6)).unwrap();

    // No choice yet: photos with a RAW wait (JPEG included); the rest goes up.
    let report = run_gallery_sync(&api, &store, &gallery, sync_with(None), silence())
        .await
        .unwrap();
    let uploaded: Vec<&str> = report
        .uploaded
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    assert_eq!(uploaded, vec!["PLAIN.jpg"]);
    assert_eq!(report.raw_decision_needed, 2);
    assert!(report.proposals.delete_remote.is_empty());
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    // JPEG only: the JPEG of the pair is uploaded, RAW files stay local.
    for _ in 0..2 {
        let report = run_gallery_sync(
            &api,
            &store,
            &gallery,
            sync_with(Some(RawMode::JpegOnly)),
            silence(),
        )
        .await
        .unwrap();
        assert!(report.uploaded_raw.is_empty());
        assert_eq!(report.raw_kept_local, vec!["ONLY_RAW.CR3".to_string()]);
        assert_eq!(report.raw_decision_needed, 0);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
    }
    let names: Vec<String> = api
        .manifest("G123")
        .await
        .unwrap()
        .photos
        .into_iter()
        .map(|photo| photo.filename)
        .collect();
    assert!(names.contains(&"WITH_JPG.jpg".to_string()), "{names:?}");
    assert!(!names
        .iter()
        .any(|name| name.ends_with(".CR3") || name.ends_with(".NEF")));
    let quiet = run_gallery_sync(
        &api,
        &store,
        &gallery,
        sync_with(Some(RawMode::JpegOnly)),
        silence(),
    )
    .await
    .unwrap();
    assert!(
        quiet.skipped,
        "held RAW files are not pending changes: {:?}",
        quiet.counts
    );
    mock.close().await;
}

#[tokio::test]
async fn jpeg_and_raw_uploads_the_raw_first_so_clients_see_the_jpeg() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    let root = std::path::Path::new(&gallery.local_path);
    std::fs::write(root.join("PAIR.jpg"), jpeg_bytes(900, 4)).unwrap();
    std::fs::write(root.join("PAIR.CR3"), raw_bytes(4)).unwrap();
    std::fs::write(root.join("SOLO.NEF"), raw_bytes(7)).unwrap();

    let options = || sync_with(Some(RawMode::JpegAndRaw));
    let report = run_gallery_sync(&api, &store, &gallery, options(), silence())
        .await
        .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let raw: Vec<&str> = report
        .uploaded_raw
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    assert_eq!(raw, vec!["PAIR.CR3"]);
    let mut main: Vec<&str> = report
        .uploaded
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    main.sort();
    assert_eq!(main, vec!["PAIR.jpg", "SOLO.NEF"]);

    // In Picstome the pair is one photo with two versions, the JPEG current.
    let manifest = api.manifest("G123").await.unwrap();
    let pair = manifest
        .photos
        .iter()
        .find(|photo| photo.filename.to_lowercase().starts_with("pair."))
        .expect("pair photo");
    assert_eq!(pair.filename, "PAIR.jpg");
    assert_eq!(pair.version_count, Some(2));

    // Nothing to do on the next sync: no re-download, no re-upload.
    let again = run_gallery_sync(&api, &store, &gallery, options(), silence())
        .await
        .unwrap();
    assert!(again.skipped, "{:?}", again.counts);
    assert!(again.downloaded.is_empty());
    mock.close().await;
}

#[tokio::test]
async fn plan_asks_for_the_raw_choice_and_keeps_raw_only_photos_local() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let root = std::path::Path::new(&gallery.local_path);
    std::fs::write(root.join("ONLY_RAW.CR3"), raw_bytes(3)).unwrap();
    std::fs::write(root.join("PAIR.jpg"), jpeg_bytes(700, 2)).unwrap();
    std::fs::write(root.join("PAIR.NEF"), raw_bytes(2)).unwrap();
    std::fs::write(root.join("LOCAL.jpg"), jpeg_bytes(600, 9)).unwrap();

    let ask = plan_sync(&api, &store, &gallery, &sync_with(None), silence())
        .await
        .unwrap();
    assert_eq!(ask.plan.raw_decision_needed, 2);
    let names: Vec<&str> = ask
        .plan
        .uploads
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    assert_eq!(names, vec!["LOCAL.jpg"]);

    let jpeg_only = plan_sync(
        &api,
        &store,
        &gallery,
        &sync_with(Some(RawMode::JpegOnly)),
        silence(),
    )
    .await
    .unwrap();
    assert_eq!(
        jpeg_only.plan.raw_kept_local,
        vec!["ONLY_RAW.CR3".to_string()]
    );
    let mut names: Vec<&str> = jpeg_only
        .plan
        .uploads
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    names.sort();
    assert_eq!(names, vec!["LOCAL.jpg", "PAIR.jpg"]);

    let both = plan_sync(
        &api,
        &store,
        &gallery,
        &sync_with(Some(RawMode::JpegAndRaw)),
        silence(),
    )
    .await
    .unwrap();
    let pair = both
        .plan
        .uploads
        .iter()
        .find(|item| item.filename == "PAIR.jpg")
        .unwrap();
    assert_eq!(pair.raw_first, vec!["PAIR.NEF".to_string()]);
    mock.close().await;
}

/// Review, then apply only part of it: nothing outside the selection moves.
#[tokio::test]
async fn apply_runs_only_the_ticked_actions() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    let root = std::path::Path::new(&gallery.local_path);
    std::fs::write(root.join("UP_A.jpg"), jpeg_bytes(500, 1)).unwrap();
    std::fs::write(root.join("UP_B.jpg"), jpeg_bytes(500, 2)).unwrap();
    for name in ["DOWN_A.jpg", "DOWN_B.jpg"] {
        reqwest::Client::new()
            .post(format!("{}/api/v1/dev/add", mock.url))
            .bearer_auth(&mock.token)
            .json(&serde_json::json!({"gallery_id": "G123", "filename": name}))
            .send()
            .await
            .unwrap();
    }

    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(planned.plan.uploads.len(), 2);
    assert_eq!(planned.plan.downloads.len(), 2);
    let down_a = planned
        .plan
        .downloads
        .iter()
        .find(|item| item.filename == "DOWN_A.jpg")
        .unwrap()
        .id
        .clone();
    let selection = PlanSelection {
        uploads: vec!["UP_A.jpg".into()],
        downloads: vec![down_a],
        ..Default::default()
    };
    let report = apply_sync(
        &api,
        &store,
        planned,
        &selection,
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.uploaded.len(), 1);
    assert_eq!(report.downloaded.len(), 1);
    assert!(root.join("DOWN_A.jpg").exists());
    assert!(!root.join("DOWN_B.jpg").exists());
    let names: Vec<String> = api
        .manifest("G123")
        .await
        .unwrap()
        .photos
        .into_iter()
        .map(|p| p.filename)
        .collect();
    assert!(names.contains(&"UP_A.jpg".to_string()));
    assert!(!names.contains(&"UP_B.jpg".to_string()));

    // What was not ticked is proposed again by the next review.
    let next = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    let up: Vec<&str> = next
        .plan
        .uploads
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    let down: Vec<&str> = next
        .plan
        .downloads
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    assert_eq!(up, vec!["UP_B.jpg"]);
    assert_eq!(down, vec!["DOWN_B.jpg"]);
    mock.close().await;
}

/// A first-sync file moved to the trash must not stay in the snapshot: the
/// scan happened before the trash, and counting a file that is no longer on
/// disk makes the gallery report a phantom photo until the next review.
#[tokio::test]
async fn trashed_first_sync_file_is_not_counted_in_the_snapshot() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let root = std::path::Path::new(&gallery.local_path);
    std::fs::write(root.join("LOCAL_ONLY.jpg"), jpeg_bytes(600, 3)).unwrap();

    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(planned.plan.uploads.len(), 1);
    let selection = PlanSelection {
        trash_local: vec!["LOCAL_ONLY.jpg".into()],
        ..Default::default()
    };
    let report = apply_sync(
        &api,
        &store,
        planned,
        &selection,
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!root.join("LOCAL_ONLY.jpg").exists());

    let snapshot = store.load_snapshot("G123").expect("snapshot");
    assert_eq!(snapshot.local_photo_count, 0);
    assert_eq!(snapshot.local_bytes, 0);
    assert!(snapshot.files.is_empty(), "{:?}", snapshot.files);

    // The remote gallery is untouched: its 50 photos are proposed as new
    // downloads, never as a local deletion.
    let next = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(next.plan.local_deleted.is_empty());
    assert_eq!(next.plan.downloads.len(), 50);
    mock.close().await;
}

#[tokio::test]
async fn edited_folder_creates_a_sibling_gallery_in_the_same_photoshoot() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let root = std::path::Path::new(&gallery.local_path);
    std::fs::create_dir_all(root.join("editadas")).unwrap();
    std::fs::write(root.join("editadas/EDITED_01.jpg"), jpeg_bytes(500, 7)).unwrap();

    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(planned.plan.edited_folders.len(), 1);
    let proposal = &planned.plan.edited_folders[0];
    assert_eq!(proposal.folder, "editadas");
    assert_eq!(proposal.gallery_name, "MiBoda - Editadas");
    assert_eq!(proposal.photoshoot_id, Some(123));
    assert_eq!(proposal.photo_count, 1);
    assert!(
        planned.plan.uploads.is_empty(),
        "the parent must not upload the edited folder"
    );

    let selection = PlanSelection {
        create_edited: vec!["editadas".into()],
        ..Default::default()
    };
    let report = apply_sync(
        &api,
        &store,
        planned,
        &selection,
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.created_galleries.len(), 1);
    let created = &report.created_galleries[0];
    assert_eq!(created.name, "MiBoda - Editadas");
    assert_eq!(created.photoshoot_id, Some(123));
    assert_eq!(
        created.local_path,
        root.join("editadas").to_string_lossy().to_string()
    );

    let linked = store.get_gallery(&created.id).expect("linked gallery");
    assert_eq!(linked.local_path, created.local_path);
    assert!(
        !store
            .load_snapshot("G123")
            .unwrap()
            .files
            .iter()
            .any(|file| file.filename == "EDITED_01.jpg"),
        "the parent snapshot must ignore the edited folder"
    );

    // The local link exists, so the next parent review does not create it
    // again.
    let again = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(again.plan.edited_folders.is_empty());
    assert_eq!(
        again.plan.edited_folders_linked,
        vec!["editadas".to_string()]
    );

    // The new gallery is empty in Picstome; its own review uploads the photo.
    let child = store.get_gallery(&created.id).unwrap();
    let child_plan = plan_sync(&api, &store, &child, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(child_plan.plan.uploads.len(), 1);
    assert_eq!(child_plan.plan.uploads[0].filename, "EDITED_01.jpg");
    let selection = PlanSelection::transfers(&child_plan.plan);
    let report = apply_sync(
        &api,
        &store,
        child_plan,
        &selection,
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.uploaded.len(), 1);
    let child_manifest = api.manifest(&created.id).await.unwrap();
    assert_eq!(child_manifest.photos.len(), 1);
    assert_eq!(child_manifest.photos[0].filename, "EDITED_01.jpg");
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);
    mock.close().await;
}

#[tokio::test]
async fn edited_folder_for_a_gallery_without_photoshoot_is_standalone() {
    let (mock, api, dir, store, _gallery) = setup_sync().await;
    let local_path = dir.path().join("Retrato");
    std::fs::create_dir_all(local_path.join("export")).unwrap();
    std::fs::write(local_path.join("export/EXP_01.jpg"), jpeg_bytes(400, 4)).unwrap();
    let gallery = store
        .upsert_gallery(GalleryEntry {
            gallery_id: "G456".into(),
            name: "Retrato Estudio".into(),
            local_path: local_path.to_string_lossy().to_string(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
            raw_mode: None,
        })
        .unwrap()
        .unwrap();

    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(planned.plan.edited_folders.len(), 1);
    assert_eq!(planned.plan.edited_folders[0].photoshoot_id, None);

    let selection = PlanSelection {
        create_edited: vec!["export".into()],
        ..Default::default()
    };
    let report = apply_sync(
        &api,
        &store,
        planned,
        &selection,
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let created = &report.created_galleries[0];
    assert_eq!(created.name, "Retrato Estudio - Exports");
    assert_eq!(created.photoshoot_id, None);
    mock.close().await;
}

/// A file edited between the review and Apply is left alone and reported.
#[tokio::test]
async fn apply_skips_files_changed_after_the_review() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    let root = std::path::Path::new(&gallery.local_path);
    let path = root.join("EDITED.jpg");
    std::fs::write(&path, jpeg_bytes(500, 1)).unwrap();

    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    let selection = PlanSelection::transfers(&planned.plan);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, jpeg_bytes(777, 9)).unwrap();

    let report = apply_sync(
        &api,
        &store,
        planned,
        &selection,
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    assert!(report.uploaded.is_empty(), "{:?}", report.uploaded);
    assert_eq!(report.skipped_changed, vec!["EDITED.jpg".to_string()]);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);
    mock.close().await;
}

/// Deleting a large share of the gallery in Picstome needs the count typed
/// back; local deletions left undecided are proposed again, never
/// re-downloaded behind the photographer's back.
#[tokio::test]
async fn mass_delete_needs_confirmation_and_undecided_deletions_stay_proposed() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&gallery.local_path)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    files.sort();
    for file in files.iter().take(20) {
        std::fs::remove_file(file).unwrap();
    }

    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(planned.plan.local_deleted.len(), 20);
    assert!(planned.plan.mass_delete);
    assert!(
        planned.plan.downloads.is_empty(),
        "deleted photos are not re-downloaded"
    );
    let ids: Vec<String> = planned
        .plan
        .local_deleted
        .iter()
        .map(|item| item.id.clone())
        .collect();
    let refused = apply_sync(
        &api,
        &store,
        planned,
        &PlanSelection {
            delete_remote: ids.clone(),
            ..Default::default()
        },
        &SyncOptions::default(),
        silence(),
    )
    .await;
    assert!(refused.is_err());
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 50);

    // Undecided: applying nothing keeps them proposed.
    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    apply_sync(
        &api,
        &store,
        planned,
        &PlanSelection::default(),
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    let planned = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(planned.plan.local_deleted.len(), 20);
    assert!(planned.plan.downloads.is_empty());

    let report = apply_sync(
        &api,
        &store,
        planned,
        &PlanSelection {
            delete_remote: ids,
            confirm_mass_delete: Some(20),
            ..Default::default()
        },
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    assert_eq!(report.deleted_remote.len(), 20, "{:?}", report.errors);
    assert_eq!(api.manifest("G123").await.unwrap().photos.len(), 30);
    mock.close().await;
}

/// An outdated review (another apply happened since) is refused.
#[tokio::test]
async fn apply_refuses_an_outdated_review() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let first = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    let second = plan_sync(&api, &store, &gallery, &SyncOptions::default(), silence())
        .await
        .unwrap();
    let selection = PlanSelection::transfers(&first.plan);
    apply_sync(
        &api,
        &store,
        first,
        &selection,
        &SyncOptions::default(),
        silence(),
    )
    .await
    .unwrap();
    let error = apply_sync(
        &api,
        &store,
        second,
        &PlanSelection::default(),
        &SyncOptions::default(),
        silence(),
    )
    .await
    .expect_err("outdated");
    assert_eq!(error.code(), "plan_outdated");
    mock.close().await;
}

#[tokio::test]
async fn photo_deleted_in_picstome_can_take_its_raw_to_the_trash() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    let manifest = api.manifest("G123").await.unwrap();
    let photo = manifest.photos.first().unwrap().clone();
    let root = std::path::Path::new(&gallery.local_path);
    let jpg = root.join(&photo.filename);
    let stem = std::path::Path::new(&photo.filename)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let raw_path = root.join(format!("{stem}.CR3"));
    let mut raw = b"II*\0".to_vec();
    raw.resize(2048, 5);
    std::fs::write(&raw_path, &raw).unwrap();

    reqwest::Client::new()
        .post(format!("{}/api/v1/dev/remove", mock.url))
        .bearer_auth(&mock.token)
        .json(&serde_json::json!({"gallery_id": "G123", "photo_id": photo.id}))
        .send()
        .await
        .unwrap();
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.remote_deleted.len(), 1);
    assert_eq!(
        report.remote_deleted[0].companions,
        vec![format!("{stem}.CR3")]
    );

    let resolution = resolve_remote_deletions(
        &api,
        &store,
        &gallery,
        std::slice::from_ref(&photo.id),
        RemoteDeleteAction::DeleteLocal,
        true,
        5,
    )
    .await
    .unwrap();
    assert_eq!(resolution.resolved, vec![photo.id.clone()]);
    assert!(!jpg.exists());
    assert!(
        !raw_path.exists(),
        "the RAW should follow its JPEG to the trash"
    );
    assert_eq!(resolution.trashed.len(), 2);
    mock.close().await;
}
