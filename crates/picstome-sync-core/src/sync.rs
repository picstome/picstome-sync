use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::StreamExt;

use crate::api::ApiClient;
use crate::config::Store;
use crate::diff::compute_diff;
use crate::error::CoreResult;
use crate::identity::preferred_file;
use crate::localfiles::{folder_exists as path_is_dir, scan_and_hash};
use crate::model::{
    DeleteProposal, DeleteProposals, DeleteResponse, Diff, DownloadedItem, GalleryEntry, Manifest,
    Photo, Progress, Snapshot, SnapshotFile, SyncErrorItem, SyncReport, UploadedItem,
};
use crate::sanitize::safe_join;
use crate::transfer::{download_file, DownloadOptions, ProgressFn};

pub type SyncProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub download_concurrency: usize,
    pub upload_concurrency: usize,
    pub chunk_bytes: u64,
    pub parallel: usize,
    pub hash_concurrency: usize,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            download_concurrency: 4,
            upload_concurrency: 3,
            chunk_bytes: crate::transfer::DOWNLOAD_CHUNK_BYTES,
            parallel: crate::transfer::DOWNLOAD_PARALLEL,
            hash_concurrency: 4,
        }
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn build_snapshot(
    gallery: &GalleryEntry,
    manifest: &Manifest,
    local_files: &[crate::model::LocalFile],
    now: i64,
) -> Snapshot {
    let mut groups: HashMap<String, Vec<&crate::model::LocalFile>> = HashMap::new();
    for file in local_files {
        let key = file.identity();
        if let Some(key) = key {
            groups.entry(key).or_default().push(file);
        }
    }

    let mut local_by_key: HashMap<String, &crate::model::LocalFile> = HashMap::new();
    for (key, group) in &groups {
        if let Some(chosen) = preferred_file(group, |file| file.filename.as_str()) {
            local_by_key.insert(key.clone(), chosen);
        }
    }

    let files = manifest
        .photos
        .iter()
        .map(|photo| {
            let key = photo.identity();
            let local = key.as_ref().and_then(|key| local_by_key.get(key)).copied();
            SnapshotFile {
                id: photo.id.clone(),
                filename: local
                    .map(|file| file.filename.clone())
                    .unwrap_or_else(|| photo.filename.clone()),
                identity_key: key,
                relative_path: local
                    .map(|file| file.relative_path.clone())
                    .or_else(|| Some(photo.filename.clone())),
                hash: photo
                    .hash
                    .clone()
                    .or_else(|| local.and_then(|file| file.hash.clone())),
                hash_algo: photo
                    .hash_algo
                    .clone()
                    .or_else(|| Some("sha256".to_string())),
                size: photo.size.or_else(|| local.map(|file| file.size)),
                mtime: local.map(|file| file.mtime_ms),
                version_count: photo.version_count,
                version_number: photo.version_number.or(photo.version_count),
            }
        })
        .collect();

    Snapshot {
        gallery_id: gallery.gallery_id.clone(),
        name: if manifest.name.is_empty() {
            gallery.name.clone()
        } else {
            manifest.name.clone()
        },
        local_path: gallery.local_path.clone(),
        last_sync_at: now,
        remote_rev: manifest.rev.clone(),
        files,
    }
}

fn download_proposals(diff: &Diff) -> Vec<DeleteProposal> {
    diff.local_deleted
        .iter()
        .map(|photo| DeleteProposal {
            id: photo.id.clone(),
            filename: photo.filename.clone(),
            identity_key: photo.identity(),
        })
        .collect()
}

fn remote_deleted_proposals(diff: &Diff) -> Vec<DeleteProposal> {
    diff.remote_deleted
        .iter()
        .map(|file| DeleteProposal {
            id: String::new(),
            filename: file.filename.clone(),
            identity_key: Some(file.identity_key.clone()),
        })
        .collect()
}

pub async fn run_gallery_sync(
    api: &ApiClient,
    store: &Store,
    gallery: &GalleryEntry,
    options: SyncOptions,
    progress: SyncProgressFn,
) -> CoreResult<SyncReport> {
    let now = now_millis();

    progress(Progress {
        gallery_id: gallery.gallery_id.clone(),
        phase: "manifest".into(),
        done: 0,
        total: 1,
        ..Default::default()
    });
    let manifest = api.manifest(&gallery.gallery_id).await?;
    progress(Progress {
        gallery_id: gallery.gallery_id.clone(),
        phase: "manifest".into(),
        done: 1,
        total: 1,
        ..Default::default()
    });

    let previous = store.load_snapshot(&gallery.gallery_id);
    let previous_files = previous.map(|snapshot| snapshot.files).unwrap_or_default();

    progress(Progress {
        gallery_id: gallery.gallery_id.clone(),
        phase: "scan".into(),
        done: 0,
        total: 1,
        ..Default::default()
    });
    let (local_files, scan_errors) = scan_and_hash(
        Path::new(&gallery.local_path),
        &previous_files,
        options.hash_concurrency,
    )
    .await?;
    progress(Progress {
        gallery_id: gallery.gallery_id.clone(),
        phase: "scan".into(),
        done: 1,
        total: 1,
        ..Default::default()
    });

    let diff = compute_diff(&previous_files, &manifest.photos, &local_files);

    let mut report = SyncReport {
        gallery_id: gallery.gallery_id.clone(),
        name: manifest.name.clone(),
        local_path: gallery.local_path.clone(),
        rev: manifest.rev.clone(),
        counts: diff.counts.clone(),
        downloaded: Vec::new(),
        uploaded: Vec::new(),
        conflicts: diff.conflicts.clone(),
        remote_deleted: remote_deleted_proposals(&diff),
        proposals: DeleteProposals {
            delete_remote: download_proposals(&diff),
        },
        errors: scan_errors
            .into_iter()
            .map(|error| SyncErrorItem {
                phase: "scan".into(),
                filename: None,
                path: Some(error.path),
                message: error.error,
            })
            .collect(),
        skipped: false,
    };

    let to_download: Vec<Photo> = diff
        .remote_new
        .iter()
        .chain(diff.remote_modified.iter())
        .cloned()
        .collect();
    let total_downloads = to_download.len() as u64;
    let downloaded_count = Arc::new(AtomicU64::new(0));

    let download_results: Vec<Result<DownloadedItem, SyncErrorItem>> =
        futures::stream::iter(to_download)
            .map(|photo| {
                let api = api.clone();
                let gallery_id = gallery.gallery_id.clone();
                let local_path = gallery.local_path.clone();
                let progress = progress.clone();
                let downloaded_count = downloaded_count.clone();
                let chunk_bytes = options.chunk_bytes;
                let parallel = options.parallel;
                async move {
                    let Some(dest) = safe_join(Path::new(&local_path), &photo.filename) else {
                        return Err(SyncErrorItem {
                            phase: "download".into(),
                            filename: Some(photo.filename.clone()),
                            path: None,
                            message: "unsafe filename in manifest".into(),
                        });
                    };

                    let file_name = photo.filename.clone();
                    let gallery_for_progress = gallery_id.clone();
                    let progress_for_file = progress.clone();
                    let downloaded_count_for_progress = downloaded_count.clone();
                    let inner: ProgressFn = Arc::new(move |done, total| {
                        progress_for_file(Progress {
                            gallery_id: gallery_for_progress.clone(),
                            phase: "download".into(),
                            file: Some(file_name.clone()),
                            done: downloaded_count_for_progress.load(Ordering::SeqCst),
                            total: total_downloads,
                            file_done: Some(done),
                            file_total: Some(total),
                        });
                    });

                    match download_file(
                        &api.http(),
                        &photo.url,
                        &dest,
                        DownloadOptions {
                            expected_hash: photo.hash.clone(),
                            chunk_bytes,
                            parallel,
                            ..Default::default()
                        },
                        inner,
                    )
                    .await
                    {
                        Ok(result) => {
                            let done = downloaded_count.fetch_add(1, Ordering::SeqCst) + 1;
                            progress(Progress {
                                gallery_id,
                                phase: "download".into(),
                                file: Some(photo.filename.clone()),
                                done,
                                total: total_downloads,
                                ..Default::default()
                            });
                            Ok(DownloadedItem {
                                id: photo.id,
                                filename: photo.filename,
                                size: result.size,
                                skipped: result.skipped,
                            })
                        }
                        Err(err) => Err(SyncErrorItem {
                            phase: "download".into(),
                            filename: Some(photo.filename),
                            path: None,
                            message: err.to_string(),
                        }),
                    }
                }
            })
            .buffer_unordered(options.download_concurrency.max(1))
            .collect()
            .await;

    for result in download_results {
        match result {
            Ok(item) => report.downloaded.push(item),
            Err(error) => report.errors.push(error),
        }
    }

    let to_upload: Vec<crate::model::LocalFile> = diff
        .local_new
        .iter()
        .chain(diff.local_modified.iter())
        .cloned()
        .collect();
    let total_uploads = to_upload.len() as u64;
    let uploaded_count = Arc::new(AtomicU64::new(0));

    let upload_results: Vec<Result<UploadedItem, SyncErrorItem>> = futures::stream::iter(to_upload)
        .map(|entry| {
            let api = api.clone();
            let gallery_id = gallery.gallery_id.clone();
            let progress = progress.clone();
            let uploaded_count = uploaded_count.clone();
            async move {
                match api
                    .upload_photo(&gallery_id, &entry.filename, &entry.path, entry.size)
                    .await
                {
                    Ok(response) => {
                        let done = uploaded_count.fetch_add(1, Ordering::SeqCst) + 1;
                        progress(Progress {
                            gallery_id,
                            phase: "upload".into(),
                            file: Some(entry.filename.clone()),
                            done,
                            total: total_uploads,
                            ..Default::default()
                        });
                        Ok(UploadedItem {
                            id: response.id,
                            filename: entry.filename,
                            version_count: response.version_count,
                            version_number: response.version_number,
                        })
                    }
                    Err(err) => Err(SyncErrorItem {
                        phase: "upload".into(),
                        filename: Some(entry.filename),
                        path: None,
                        message: err.to_string(),
                    }),
                }
            }
        })
        .buffer_unordered(options.upload_concurrency.max(1))
        .collect()
        .await;

    for result in upload_results {
        match result {
            Ok(item) => report.uploaded.push(item),
            Err(error) => report.errors.push(error),
        }
    }

    let changed = total_downloads + total_uploads > 0;
    let final_manifest = if changed {
        api.manifest(&gallery.gallery_id).await?
    } else {
        manifest
    };

    let snapshot = build_snapshot(gallery, &final_manifest, &local_files, now);
    store.save_snapshot(&snapshot)?;

    if store
        .patch_gallery(&gallery.gallery_id, |entry| {
            entry.name = snapshot.name.clone();
            entry.last_sync_at = Some(now);
            entry.remote_rev = snapshot.remote_rev.clone();
        })?
        .is_none()
    {
        store.upsert_gallery(GalleryEntry {
            gallery_id: gallery.gallery_id.clone(),
            name: snapshot.name.clone(),
            local_path: gallery.local_path.clone(),
            last_sync_at: Some(now),
            remote_rev: snapshot.remote_rev.clone(),
            parent_gallery_id: gallery.parent_gallery_id.clone(),
        })?;
    }

    report.name = snapshot.name;
    report.rev = snapshot.remote_rev;
    report.skipped = !diff.has_changes();

    progress(Progress {
        gallery_id: gallery.gallery_id.clone(),
        phase: "done".into(),
        done: 1,
        total: 1,
        ..Default::default()
    });

    Ok(report)
}

pub async fn apply_remote_deletions(
    api: &ApiClient,
    store: &Store,
    gallery_id: &str,
    photo_ids: &[String],
) -> CoreResult<DeleteResponse> {
    if photo_ids.is_empty() {
        return Ok(DeleteResponse {
            deleted: vec![],
            not_found: vec![],
            rev: None,
        });
    }

    let result = api.delete_photos(gallery_id, photo_ids).await?;
    if let Some(mut snapshot) = store.load_snapshot(gallery_id) {
        let removed: std::collections::HashSet<&String> = if result.deleted.is_empty() {
            photo_ids.iter().collect()
        } else {
            result.deleted.iter().collect()
        };
        snapshot.files.retain(|file| !removed.contains(&file.id));
        snapshot.last_sync_at = now_millis();
        snapshot.remote_rev = result.rev.clone().or(snapshot.remote_rev);
        store.save_snapshot(&snapshot)?;
        store.patch_gallery(gallery_id, |entry| {
            entry.last_sync_at = Some(snapshot.last_sync_at);
        })?;
    }
    Ok(result)
}

pub fn folder_exists(folder: &str) -> bool {
    path_is_dir(folder)
}
