use std::sync::Arc;

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::config::Store;
use picstome_sync_core::ignored::{GalleryIgnores, IgnoredLocal, IgnoredRemote};
use picstome_sync_core::localfiles::hash_file;
use picstome_sync_core::mock::{start_mock_server, MockServerOptions};
use picstome_sync_core::model::{GalleryEntry, Snapshot, SnapshotFile, SNAPSHOT_SCHEMA};
use picstome_sync_core::sync::{
    apply_remote_deletions, mark_favorites_rated, mark_ratings_applied, preview_first_sync,
    rating_plan_from_snapshot, resolve_remote_deletions, run_gallery_sync, FirstSyncItem,
    RemoteDeleteAction, SyncOptions,
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
        })
        .unwrap()
        .unwrap();
    (mock, api, dir, store, gallery)
}

fn silence() -> Arc<dyn Fn(picstome_sync_core::model::Progress) + Send + Sync> {
    Arc::new(|_| {})
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
async fn scan_errors_do_not_generate_remote_deletion_proposals() {
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
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&files[0])
        .unwrap();
    file.set_len(501 * 1024 * 1024).unwrap();
    drop(file);

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(report.counts.local_deleted > 0);
    assert!(report.proposals.delete_remote.is_empty());
    assert!(report
        .errors
        .iter()
        .any(|error| error.message.contains("not proposed")));
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
async fn first_sync_preview_lists_both_sides() {
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

    let preview = preview_first_sync(&api, &store, &gallery).await.unwrap();
    assert!(preview.needed);
    assert_eq!(preview.local_total, 3);
    assert_eq!(preview.remote_total, 50);
    assert_eq!(preview.in_sync, 2);
    assert_eq!(preview.local_only.len(), 1);
    assert_eq!(preview.local_only[0].filename, "LOCAL_ONLY.jpg");
    assert_eq!(preview.remote_only.len(), 48);
    assert!(store.load_snapshot("G123").is_none());
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
    let bytes = jpeg_bytes(640, 6);
    std::fs::write(&path, &bytes).unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    let mtime_ms = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let report = run_gallery_sync(
        &api,
        &store,
        &gallery,
        SyncOptions {
            trash_local: vec![FirstSyncItem {
                filename: "TRASH_ME.jpg".into(),
                relative_path: "TRASH_ME.jpg".into(),
                size: bytes.len() as u64,
                mtime_ms,
                id: None,
                version_count: None,
            }],
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

    let preview = preview_first_sync(&api, &store, &gallery).await.unwrap();
    let names: Vec<&str> = preview
        .local_only
        .iter()
        .map(|item| item.filename.as_str())
        .collect();
    assert!(names.contains(&"PAIR.JPG"), "{names:?}");
    assert!(names.contains(&"PAIR.DNG"), "{names:?}");

    let report = run_gallery_sync(
        &api,
        &store,
        &gallery,
        SyncOptions {
            trash_local: preview.local_only,
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
