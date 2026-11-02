use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::config::Store;
use picstome_sync_core::mock::MockServerOptions;
use picstome_sync_core::model::GalleryEntry;
use picstome_sync_core::sync::{apply_remote_deletions, run_gallery_sync, SyncOptions};

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn silence() -> Arc<dyn Fn(picstome_sync_core::model::Progress) + Send + Sync> {
    Arc::new(|_| {})
}

fn gallery(store: &Store, root: &std::path::Path) -> GalleryEntry {
    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    store
        .upsert_gallery(GalleryEntry {
            gallery_id: "G1".into(),
            name: "Test".into(),
            local_path: photos.to_string_lossy().to_string(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
        })
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn manifest_failure_is_returned() {
    let base = serve(Router::new().route(
        "/api/v1/manifest",
        get(|| async { StatusCode::INTERNAL_SERVER_ERROR.into_response() }),
    ))
    .await;
    let api = ApiClient::new(Some(&base), None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("state")).unwrap();
    let gallery = gallery(&store, dir.path());

    let err = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap_err();
    assert_eq!(err.status(), 500);
}

#[tokio::test]
async fn download_failures_are_collected_in_the_report() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let manifest = json!({
        "gallery_id": "G1",
        "name": "Test",
        "rev": 1,
        "photos": [{
            "id": "p1",
            "filename": "a.jpg",
            "size": 10,
            "hash": null,
            "hash_algo": "sha256",
            "url": format!("{base}/missing"),
        }],
    });
    let app = Router::new().route(
        "/api/v1/manifest",
        get(move || {
            let manifest = manifest.clone();
            async move { Json(manifest).into_response() }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let api = ApiClient::new(Some(&base), None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("state")).unwrap();
    let gallery = gallery(&store, dir.path());

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].phase, "download");
    assert_eq!(report.errors[0].filename.as_deref(), Some("a.jpg"));
}

#[tokio::test]
async fn upload_failures_are_collected_in_the_report() {
    let base = serve(
        Router::new()
            .route(
                "/api/v1/manifest",
                get(|| async {
                    Json(json!({"gallery_id":"G1","name":"Test","rev":1,"photos":[]}))
                }),
            )
            .route(
                "/api/v1/photos",
                post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "nope").into_response() }),
            ),
    )
    .await;
    let api = ApiClient::new(Some(&base), None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("state")).unwrap();
    let gallery = gallery(&store, dir.path());
    std::fs::write(
        std::path::Path::new(&gallery.local_path).join("new.jpg"),
        b"data",
    )
    .unwrap();

    let report = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
        .await
        .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].phase, "upload");
}

#[tokio::test]
async fn missing_local_folder_is_an_error() {
    let base = serve(Router::new().route(
        "/api/v1/manifest",
        get(|| async { Json(json!({"gallery_id":"G1","name":"Test","rev":1,"photos":[]})) }),
    ))
    .await;
    let api = ApiClient::new(Some(&base), None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("state")).unwrap();
    let gallery = store
        .upsert_gallery(GalleryEntry {
            gallery_id: "G1".into(),
            name: "Test".into(),
            local_path: dir.path().join("missing").to_string_lossy().to_string(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
        })
        .unwrap()
        .unwrap();

    assert!(
        run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), silence())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn empty_delete_is_a_noop_and_api_delete_errors_propagate() {
    let base = serve(Router::new().route(
        "/api/v1/photos",
        post(|| async { (StatusCode::FORBIDDEN, "nope").into_response() }),
    ))
    .await;
    let api = ApiClient::new(Some(&base), None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("state")).unwrap();

    let empty = apply_remote_deletions(&api, &store, "G1", &[])
        .await
        .unwrap();
    assert!(empty.deleted.is_empty());
    assert!(apply_remote_deletions(&api, &store, "G1", &["p1".into()])
        .await
        .is_err());
}

#[tokio::test]
async fn fake_mock_options_are_reusable() {
    let options = MockServerOptions::default();
    assert_eq!(options.photo_size, 32 * 1024);
}
