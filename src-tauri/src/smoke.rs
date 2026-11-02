use std::sync::Arc;

use serde_json::json;

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::config::Store;
use picstome_sync_core::mock::{start_mock_server, MockServerOptions};
use picstome_sync_core::model::GalleryEntry;
use picstome_sync_core::sync::{run_gallery_sync, SyncOptions};

pub fn run() -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("Could not create runtime: {err}");
            return 1;
        }
    };

    match runtime.block_on(run_async()) {
        Ok(summary) => {
            println!("Picstome Sync smoke test passed: {summary}");
            0
        }
        Err(err) => {
            eprintln!("Picstome Sync smoke test failed: {err}");
            1
        }
    }
}

async fn run_async() -> Result<serde_json::Value, String> {
    let mock = start_mock_server(MockServerOptions {
        photo_size: 8 * 1024,
        ..Default::default()
    })
    .await
    .map_err(|err| err.to_string())?;

    let result = async {
        let dir = tempfile::tempdir().map_err(|err| err.to_string())?;
        let local_path = dir.path().join("MiBoda");
        std::fs::create_dir_all(&local_path).map_err(|err| err.to_string())?;
        let store = Store::new(dir.path().join("state")).map_err(|err| err.to_string())?;
        let gallery = store
            .upsert_gallery(GalleryEntry {
                gallery_id: "G123".into(),
                name: "MiBoda".into(),
                local_path: local_path.to_string_lossy().to_string(),
                last_sync_at: None,
                remote_rev: None,
                parent_gallery_id: None,
            })
            .map_err(|err| err.to_string())?
            .ok_or_else(|| "could not link gallery".to_string())?;

        let api = ApiClient::new(Some(&mock.url), None).map_err(|err| err.to_string())?;
        api.dev_login().await.map_err(|err| err.to_string())?;

        let progress = Arc::new(|_: picstome_sync_core::model::Progress| {});
        let first = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), progress.clone())
            .await
            .map_err(|err| err.to_string())?;
        if first.downloaded.len() != 50 || !first.errors.is_empty() {
            return Err(format!("first sync unexpected: {first:?}"));
        }

        let files = std::fs::read_dir(&local_path)
            .map_err(|err| err.to_string())?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".jpg"))
            .count();
        if files != 50 {
            return Err(format!("expected 50 local files, got {files}"));
        }
        if store.load_snapshot("G123").is_none() {
            return Err("snapshot was not written".into());
        }

        let second = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), progress.clone())
            .await
            .map_err(|err| err.to_string())?;
        if !second.skipped || !second.downloaded.is_empty() {
            return Err(format!("second sync should be a no-op: {second:?}"));
        }

        reqwest::Client::new()
            .post(format!("{}/api/v1/dev/bump", mock.url))
            .bearer_auth(&mock.token)
            .json(&json!({"gallery_id": "G123"}))
            .send()
            .await
            .map_err(|err| err.to_string())?;

        let third = run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), progress)
            .await
            .map_err(|err| err.to_string())?;
        if third.downloaded.len() != 1 {
            return Err(format!("remote change was not downloaded: {third:?}"));
        }

        Ok(json!({
            "firstDownloaded": first.downloaded.len(),
            "secondSkipped": second.skipped,
            "thirdDownloaded": third.downloaded.len(),
            "localFiles": files,
            "snapshotFiles": store.load_snapshot("G123").map(|snapshot| snapshot.files.len()).unwrap_or(0),
        }))
    }
    .await;

    mock.close().await;
    result
}
