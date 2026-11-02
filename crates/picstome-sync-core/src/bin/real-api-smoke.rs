//! End-to-end smoke against a real Picstome API (local Laravel or devel).
//!
//! Usage:
//!   PICSTOME_REAL_TOKEN=... PICSTOME_REAL_GALLERY=... \
//!     cargo run -p picstome-sync-core --bin real-api-smoke
//!
//! Optional: PICSTOME_REAL_API (default http://127.0.0.1:8000)
//! Set PICSTOME_REAL_WRITE=1 to exercise upload + delete round-trip.

use std::sync::Arc;

use serde_json::json;

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::config::Store;
use picstome_sync_core::model::GalleryEntry;
use picstome_sync_core::sync::{apply_remote_deletions, run_gallery_sync, SyncOptions};

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("real API smoke failed: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let base =
        std::env::var("PICSTOME_REAL_API").unwrap_or_else(|_| "http://127.0.0.1:8000".into());
    let token = std::env::var("PICSTOME_REAL_TOKEN")
        .map_err(|_| "PICSTOME_REAL_TOKEN is required".to_string())?;
    let gallery_id = std::env::var("PICSTOME_REAL_GALLERY")
        .map_err(|_| "PICSTOME_REAL_GALLERY is required".to_string())?;
    let write = std::env::var("PICSTOME_REAL_WRITE")
        .map(|value| value == "1")
        .unwrap_or(false);

    let api = ApiClient::new(Some(&base), Some(token)).map_err(|err| err.to_string())?;
    let me = api.me().await.map_err(|err| err.to_string())?;
    let manifest = api
        .manifest(&gallery_id)
        .await
        .map_err(|err| err.to_string())?;
    let remote_count = manifest.photos.len();

    let dir = std::env::temp_dir().join(format!(
        "picstome-real-smoke-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let local_path = dir.join("gallery");
    std::fs::create_dir_all(&local_path).map_err(|err| err.to_string())?;
    let store = Store::new(dir.join("state")).map_err(|err| err.to_string())?;
    let gallery = store
        .upsert_gallery(GalleryEntry {
            gallery_id: gallery_id.clone(),
            name: manifest.name.clone(),
            local_path: local_path.to_string_lossy().to_string(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
        })
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "could not link gallery".to_string())?;

    let progress = Arc::new(|_: picstome_sync_core::model::Progress| {});
    let first = run_gallery_sync(
        &api,
        &store,
        &gallery,
        SyncOptions::default(),
        progress.clone(),
    )
    .await
    .map_err(|err| err.to_string())?;
    if !first.errors.is_empty() {
        return Err(format!("download errors: {:?}", first.errors));
    }
    if first.downloaded.len() != remote_count {
        return Err(format!(
            "expected {remote_count} downloads, got {}",
            first.downloaded.len()
        ));
    }

    let second = run_gallery_sync(
        &api,
        &store,
        &gallery,
        SyncOptions::default(),
        progress.clone(),
    )
    .await
    .map_err(|err| err.to_string())?;
    if !second.skipped {
        return Err(format!(
            "second sync should be a no-op: {:?}",
            second.counts
        ));
    }

    let mut write_summary = json!(null);
    if write {
        let source = std::fs::read_dir(&local_path)
            .map_err(|err| err.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jpg"))
            .ok_or_else(|| "no downloaded file to copy".to_string())?;
        let name = format!("sync-smoke-{}.jpg", std::process::id());
        let upload_path = local_path.join(&name);
        std::fs::copy(&source, &upload_path).map_err(|err| err.to_string())?;

        let after_upload = run_gallery_sync(
            &api,
            &store,
            &gallery,
            SyncOptions::default(),
            progress.clone(),
        )
        .await
        .map_err(|err| err.to_string())?;
        if after_upload.uploaded.len() != 1 {
            let local_count = std::fs::read_dir(&local_path)
                .map(|entries| entries.count())
                .unwrap_or(0);
            return Err(format!(
                "expected 1 upload, got {:?} errors {:?} counts {:?} localCount {} name {}",
                after_upload.uploaded, after_upload.errors, after_upload.counts, local_count, name
            ));
        }

        let remote_after_upload = api
            .manifest(&gallery_id)
            .await
            .map_err(|err| err.to_string())?;
        let uploaded = remote_after_upload
            .photos
            .iter()
            .find(|photo| photo.filename == name)
            .ok_or_else(|| "uploaded photo not present in manifest".to_string())?;

        std::fs::remove_file(&upload_path).map_err(|err| err.to_string())?;
        let after_delete_proposal =
            run_gallery_sync(&api, &store, &gallery, SyncOptions::default(), progress)
                .await
                .map_err(|err| err.to_string())?;
        if after_delete_proposal.proposals.delete_remote.len() != 1 {
            return Err("expected one delete proposal".into());
        }
        let ids: Vec<String> = after_delete_proposal
            .proposals
            .delete_remote
            .iter()
            .map(|proposal| proposal.id.clone())
            .collect();
        apply_remote_deletions(&api, &store, &gallery_id, &ids)
            .await
            .map_err(|err| err.to_string())?;

        let remote_after_delete = api
            .manifest(&gallery_id)
            .await
            .map_err(|err| err.to_string())?;
        if remote_after_delete.photos.len() != remote_count {
            return Err(format!(
                "gallery should be back to {remote_count} photos, got {}",
                remote_after_delete.photos.len()
            ));
        }

        write_summary = json!({
            "uploaded": upload_path.file_name().and_then(|name| name.to_str()).unwrap_or_default(),
            "uploadedPhotoId": uploaded.id,
            "remoteAfterUpload": remote_after_upload.photos.len(),
        });
    }

    let _ = std::fs::remove_dir_all(&dir);

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "api": base,
            "user": me.email,
            "gallery": gallery_id,
            "remotePhotos": remote_count,
            "downloaded": first.downloaded.len(),
            "secondSkipped": second.skipped,
            "write": write_summary,
        }))
        .unwrap_or_default()
    );

    Ok(())
}
