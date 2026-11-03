use std::sync::Arc;

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::config::Store;
use picstome_sync_core::localfiles::hash_file;
use picstome_sync_core::mock::{start_mock_server, MockServerOptions};
use picstome_sync_core::model::{GalleryEntry, Snapshot, SnapshotFile};
use picstome_sync_core::sync::{
    apply_remote_deletions, mark_favorites_rated, resolve_remote_deletions, run_gallery_sync,
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
    tokio::fs::write(&file, vec![1u8; 900]).await.unwrap();

    let first = api
        .upload_photo("G123", "NEW_0001.jpg", &file, 900)
        .await
        .unwrap();
    assert_eq!(first.version_count, Some(1));

    let second = api
        .upload_photo("G123", "NEW_0001.jpg", &file, 900)
        .await
        .unwrap();
    assert_eq!(second.version_count, Some(1));
    assert!(second.unchanged);

    tokio::fs::write(&file, vec![2u8; 901]).await.unwrap();
    let third = api
        .upload_photo("G123", "NEW_0001.jpg", &file, 901)
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
    assert_eq!(report.favorites.len(), 5);
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
async fn remote_only_snapshot_entries_are_downloaded_not_deleted_remotely() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let manifest = api.manifest("G123").await.unwrap();
    let snapshot = Snapshot {
        gallery_id: "G123".into(),
        name: "MiBoda".into(),
        local_path: gallery.local_path.clone(),
        last_sync_at: 1,
        remote_rev: manifest.rev.clone(),
        hash_semantics: 0,
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
                version_count: photo.version_count,
                version_number: photo.version_number,
                remote_deleted: false,
                keep_local: false,
                favorite_rated_for: None,
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
async fn legacy_source_hashes_are_repaired_without_false_conflicts() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();

    let mut snapshot = store.load_snapshot("G123").unwrap();
    snapshot.hash_semantics = 0;
    for file in &mut snapshot.files {
        file.hash = Some("legacy-source-hash".into());
    }
    store.save_snapshot(&snapshot).unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.counts.conflicts, 0);
    assert_eq!(report.counts.local_modified, 0);
    assert_eq!(report.counts.remote_modified, 0);
    assert_eq!(report.downloaded.len(), 0);
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
    std::fs::write(&files[0], vec![9u8; 1234]).unwrap();
    std::fs::write(
        format!("{}/LOCAL_NEW.jpg", gallery.local_path),
        vec![7u8; 777],
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
async fn favorite_ratings_are_not_asked_again() {
    let (mock, api, _dir, store, gallery) = setup_sync().await;
    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.favorites.len(), 5);
    let marks: Vec<(String, String)> = report
        .favorites
        .iter()
        .filter_map(|photo| Some((photo.identity_key.clone()?, photo.favorited_at.clone()?)))
        .collect();
    assert_eq!(marks.len(), 5);

    assert_eq!(
        mark_favorites_rated(&store, "G123", &marks[..1]).unwrap(),
        1
    );
    let after_one = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(after_one.favorites.len(), 4);

    assert_eq!(mark_favorites_rated(&store, "G123", &marks).unwrap(), 5);
    let after_all = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert!(after_all.favorites.is_empty());
    assert!(after_all.skipped);

    mock.close().await;
}
