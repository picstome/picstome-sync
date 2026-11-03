use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::StreamExt;

use crate::api::ApiClient;
use crate::config::Store;
use crate::diff::compute_diff;
use crate::error::{CoreError, CoreResult};
use crate::identity::preferred_file;
use crate::localfiles::{folder_exists as path_is_dir, scan_and_hash};
use crate::model::{
    DeleteProposal, DeleteProposals, DeleteResponse, Diff, DownloadedItem, FavoritePhoto,
    GalleryEntry, Manifest, Photo, Progress, Snapshot, SnapshotFile, SyncErrorItem, SyncReport,
    UploadedItem,
};
use crate::sanitize::{is_inside, safe_join, sanitize_file_name};
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
    previous: &[SnapshotFile],
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

    let previous_by_key: HashMap<String, &SnapshotFile> = previous
        .iter()
        .filter_map(|file| file.identity().map(|key| (key, file)))
        .collect();

    let mut files: Vec<SnapshotFile> = Vec::new();
    for photo in &manifest.photos {
        let key = photo.identity();
        let local = key.as_ref().and_then(|key| local_by_key.get(key)).copied();
        let previous = key
            .as_ref()
            .and_then(|key| previous_by_key.get(key))
            .copied();
        if let Some(file) = local {
            let sides_match = match (photo.hash.as_deref(), file.hash.as_deref()) {
                (Some(remote), Some(local)) => remote == local,
                _ => photo.size == Some(file.size),
            };
            // When local and remote differ, a transfer failed or there is a
            // conflict. Keep the last common base so the next sync retries the
            // correct direction instead of treating one side as authoritative.
            let previous_local = previous.filter(|file| file.mtime.is_some());
            if !sides_match && previous_local.is_none() {
                // No common base and the contents differ: this is a conflict.
                // Do not invent a baseline that would silently resolve it later.
                continue;
            }
            let base = if sides_match { None } else { previous_local };
            files.push(SnapshotFile {
                id: photo.id.clone(),
                filename: file.filename.clone(),
                identity_key: key.clone(),
                relative_path: Some(file.relative_path.clone()),
                hash: base
                    .and_then(|file| file.hash.clone())
                    .or_else(|| photo.hash.clone())
                    .or_else(|| file.hash.clone()),
                hash_algo: base
                    .and_then(|file| file.hash_algo.clone())
                    .or_else(|| photo.hash_algo.clone())
                    .or_else(|| Some("sha256".to_string())),
                size: base
                    .and_then(|file| file.size)
                    .or(photo.size)
                    .or(Some(file.size)),
                mtime: Some(file.mtime_ms),
                version_count: base
                    .and_then(|file| file.version_count)
                    .or(photo.version_count),
                version_number: base
                    .and_then(|file| file.version_number)
                    .or(photo.version_number)
                    .or(photo.version_count),
                remote_deleted: false,
                keep_local: false,
                favorite_rated_for: previous.and_then(|file| file.favorite_rated_for.clone()),
            });
        } else if let Some(previous) = previous.filter(|file| file.mtime.is_some()) {
            // Keep the last known local state so a genuinely deleted local photo
            // is still surfaced, but never persist a remote-only entry: otherwise
            // a failed download would look like a local deletion on the next run.
            files.push(SnapshotFile {
                id: photo.id.clone(),
                filename: previous.filename.clone(),
                identity_key: key.clone(),
                relative_path: previous
                    .relative_path
                    .clone()
                    .or_else(|| Some(previous.filename.clone())),
                hash: photo.hash.clone().or_else(|| previous.hash.clone()),
                hash_algo: photo
                    .hash_algo
                    .clone()
                    .or_else(|| previous.hash_algo.clone()),
                size: photo.size.or(previous.size),
                mtime: previous.mtime,
                version_count: photo.version_count.or(previous.version_count),
                version_number: photo
                    .version_number
                    .or(photo.version_count)
                    .or(previous.version_number),
                remote_deleted: false,
                keep_local: false,
                favorite_rated_for: previous.favorite_rated_for.clone(),
            });
        }
    }

    let remote_keys: std::collections::HashSet<String> = manifest
        .photos
        .iter()
        .filter_map(|photo| photo.identity())
        .collect();
    let known_keys: std::collections::HashSet<String> =
        files.iter().filter_map(|file| file.identity()).collect();

    for previous_file in previous {
        let Some(key) = previous_file.identity() else {
            continue;
        };
        if remote_keys.contains(&key) || known_keys.contains(&key) {
            continue;
        }
        let Some(local) = local_by_key.get(&key).copied() else {
            continue;
        };
        files.push(SnapshotFile {
            id: previous_file.id.clone(),
            filename: local.filename.clone(),
            identity_key: Some(key),
            relative_path: Some(local.relative_path.clone()),
            hash: local.hash.clone(),
            hash_algo: Some("sha256".to_string()),
            size: Some(local.size),
            mtime: Some(local.mtime_ms),
            version_count: previous_file.version_count,
            version_number: previous_file.version_number,
            remote_deleted: true,
            keep_local: previous_file.keep_local,
            favorite_rated_for: previous_file.favorite_rated_for.clone(),
        });
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDeleteAction {
    DeleteLocal,
    KeepLocal,
    Upload,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeleteResolution {
    pub resolved: Vec<String>,
    #[serde(default)]
    pub trashed: Vec<TrashedFile>,
    pub errors: Vec<SyncErrorItem>,
}

fn local_path_for(gallery: &GalleryEntry, file: &SnapshotFile) -> Option<PathBuf> {
    let root = Path::new(&gallery.local_path);
    let relative = file.relative_path.as_deref().unwrap_or(&file.filename);
    let candidate = root.join(relative);
    if is_inside(root, &candidate) && candidate.is_file() {
        return Some(candidate);
    }
    safe_join(root, &file.filename).filter(|path| path.is_file())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedFile {
    pub id: String,
    pub filename: String,
    pub original_path: String,
    pub location: String,
    #[serde(default)]
    pub trash_path: Option<String>,
}

fn gallery_trash_destination(root: &Path, filename: &str) -> PathBuf {
    let dir = root.join(".picstome-trash");
    let stamp = now_millis();
    let filename = sanitize_file_name(filename).unwrap_or_else(|| "photo".to_string());
    let candidate = dir.join(format!("{stamp}-{filename}"));
    if !candidate.exists() {
        return candidate;
    }
    for suffix in 1..1000 {
        let candidate = dir.join(format!("{stamp}-{suffix}-{filename}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stamp}-{filename}"))
}

fn move_to_gallery_trash(
    root: &Path,
    filename: &str,
    path: &Path,
    system_error: &str,
) -> Result<PathBuf, String> {
    let destination = gallery_trash_destination(root, filename);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!("system trash failed ({system_error}); gallery trash folder failed ({error})")
        })?;
    }
    std::fs::rename(path, &destination).map_err(|error| {
        format!("system trash failed ({system_error}); moving to gallery trash failed ({error})")
    })?;
    Ok(destination)
}

fn trash_local_file(
    gallery: &GalleryEntry,
    id: &str,
    filename: &str,
    path: &Path,
) -> Result<TrashedFile, String> {
    let original_path = path.display().to_string();
    match trash::delete(path) {
        Ok(()) => Ok(TrashedFile {
            id: id.to_string(),
            filename: filename.to_string(),
            original_path,
            location: "system".into(),
            trash_path: None,
        }),
        Err(system_error) => {
            let destination = move_to_gallery_trash(
                Path::new(&gallery.local_path),
                filename,
                path,
                &system_error.to_string(),
            )?;
            Ok(TrashedFile {
                id: id.to_string(),
                filename: filename.to_string(),
                original_path,
                location: "gallery".into(),
                trash_path: Some(destination.display().to_string()),
            })
        }
    }
}

pub fn mark_favorites_rated(
    store: &Store,
    gallery_id: &str,
    marks: &[(String, String)],
) -> CoreResult<usize> {
    if marks.is_empty() {
        return Ok(0);
    }
    let mut snapshot = store
        .load_snapshot(gallery_id)
        .ok_or_else(|| CoreError::InvalidData("gallery snapshot not found".into()))?;
    let by_key: HashMap<&str, &str> = marks
        .iter()
        .map(|(key, favorited_at)| (key.as_str(), favorited_at.as_str()))
        .collect();
    let mut updated = 0;
    for file in &mut snapshot.files {
        let Some(key) = file.identity() else {
            continue;
        };
        if let Some(favorited_at) = by_key.get(key.as_str()) {
            file.favorite_rated_for = Some((*favorited_at).to_string());
            updated += 1;
        }
    }
    store.save_snapshot(&snapshot)?;
    Ok(updated)
}

pub async fn resolve_remote_deletions(
    api: &ApiClient,
    store: &Store,
    gallery: &GalleryEntry,
    ids: &[String],
    action: RemoteDeleteAction,
    now: i64,
) -> CoreResult<RemoteDeleteResolution> {
    let mut snapshot = store
        .load_snapshot(&gallery.gallery_id)
        .ok_or_else(|| CoreError::InvalidData("gallery snapshot not found".into()))?;
    let mut resolved = Vec::new();
    let mut trashed = Vec::new();
    let mut errors = Vec::new();

    for id in ids {
        let Some(index) = snapshot
            .files
            .iter()
            .position(|file| &file.id == id && file.remote_deleted)
        else {
            errors.push(SyncErrorItem {
                phase: "remote_delete".into(),
                filename: None,
                path: None,
                message: "Photo is not pending remote deletion".into(),
            });
            continue;
        };

        match action {
            RemoteDeleteAction::DeleteLocal => {
                if let Some(path) = local_path_for(gallery, &snapshot.files[index]) {
                    let gallery = gallery.clone();
                    let id = id.clone();
                    let filename = snapshot.files[index].filename.clone();
                    let path_for_error = path.display().to_string();
                    let result = tokio::task::spawn_blocking(move || {
                        trash_local_file(&gallery, &id, &filename, &path)
                    })
                    .await;
                    match result {
                        Ok(Ok(item)) => trashed.push(item),
                        Ok(Err(err)) => {
                            errors.push(SyncErrorItem {
                                phase: "remote_delete".into(),
                                filename: Some(snapshot.files[index].filename.clone()),
                                path: Some(path_for_error),
                                message: err,
                            });
                            continue;
                        }
                        Err(err) => {
                            errors.push(SyncErrorItem {
                                phase: "remote_delete".into(),
                                filename: Some(snapshot.files[index].filename.clone()),
                                path: Some(path_for_error),
                                message: err.to_string(),
                            });
                            continue;
                        }
                    }
                }
                resolved.push(id.clone());
            }
            RemoteDeleteAction::KeepLocal => {
                snapshot.files[index].keep_local = true;
                snapshot.files[index].remote_deleted = true;
                resolved.push(id.clone());
            }
            RemoteDeleteAction::Upload => {
                let Some(path) = local_path_for(gallery, &snapshot.files[index]) else {
                    errors.push(SyncErrorItem {
                        phase: "remote_upload".into(),
                        filename: Some(snapshot.files[index].filename.clone()),
                        path: None,
                        message: "Local file not found".into(),
                    });
                    continue;
                };
                let size = tokio::fs::metadata(&path)
                    .await
                    .map(|metadata| metadata.len())
                    .unwrap_or(snapshot.files[index].size.unwrap_or(0));
                match api
                    .upload_photo(
                        &gallery.gallery_id,
                        &snapshot.files[index].filename,
                        &path,
                        size,
                    )
                    .await
                {
                    Ok(response) => {
                        let entry = &mut snapshot.files[index];
                        if let Some(new_id) = response.id {
                            entry.id = new_id;
                        }
                        if let Some(filename) = response.filename {
                            entry.filename = filename;
                        }
                        entry.version_count = response.version_count;
                        entry.version_number = response.version_number;
                        entry.remote_deleted = false;
                        entry.keep_local = false;
                        resolved.push(id.clone());
                    }
                    Err(err) => errors.push(SyncErrorItem {
                        phase: "remote_upload".into(),
                        filename: Some(snapshot.files[index].filename.clone()),
                        path: Some(path.display().to_string()),
                        message: err.to_string(),
                    }),
                }
            }
        }
    }

    if action == RemoteDeleteAction::DeleteLocal {
        let resolved_set: std::collections::HashSet<&String> = resolved.iter().collect();
        snapshot
            .files
            .retain(|file| !resolved_set.contains(&file.id));
    }

    snapshot.last_sync_at = now;
    store.save_snapshot(&snapshot)?;
    store.patch_gallery(&gallery.gallery_id, |entry| {
        entry.last_sync_at = Some(now);
    })?;

    Ok(RemoteDeleteResolution {
        resolved,
        trashed,
        errors,
    })
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

    let scan_error_items: Vec<SyncErrorItem> = scan_errors
        .into_iter()
        .map(|error| SyncErrorItem {
            phase: "scan".into(),
            filename: None,
            path: Some(error.path),
            message: error.error,
        })
        .collect();
    let scan_had_errors = !scan_error_items.is_empty();
    let mut report = SyncReport {
        gallery_id: gallery.gallery_id.clone(),
        name: manifest.name.clone(),
        local_path: gallery.local_path.clone(),
        rev: manifest.rev.clone(),
        counts: diff.counts.clone(),
        downloaded: Vec::new(),
        uploaded: Vec::new(),
        conflicts: diff.conflicts.clone(),
        remote_deleted: diff.remote_deleted.clone(),
        proposals: if scan_had_errors {
            DeleteProposals::default()
        } else {
            DeleteProposals {
                delete_remote: download_proposals(&diff),
            }
        },
        favorites: Vec::new(),
        errors: scan_error_items,
        skipped: false,
    };
    if scan_had_errors && diff.counts.local_deleted > 0 {
        report.errors.push(SyncErrorItem {
            phase: "scan".into(),
            filename: None,
            path: None,
            message: format!(
                "{} local deletion(s) were not proposed because the local scan had errors; fix the scan errors and sync again",
                diff.counts.local_deleted
            ),
        });
    }

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

    let to_upload: Vec<(crate::model::LocalFile, String)> = diff
        .local_new
        .iter()
        .map(|file| (file.clone(), "new".to_string()))
        .chain(
            diff.local_modified
                .iter()
                .map(|file| (file.clone(), "modified".to_string())),
        )
        .collect();
    let total_uploads = to_upload.len() as u64;
    let uploaded_count = Arc::new(AtomicU64::new(0));

    let upload_results: Vec<Result<UploadedItem, SyncErrorItem>> = futures::stream::iter(to_upload)
        .map(|(entry, reason)| {
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
                            reason: Some(reason),
                            size: Some(entry.size),
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

    let previous_by_key: HashMap<String, &SnapshotFile> = previous_files
        .iter()
        .filter_map(|file| file.identity().map(|key| (key, file)))
        .collect();
    report.favorites = final_manifest
        .photos
        .iter()
        .filter(|photo| photo.favorited_at.is_some())
        .filter(|photo| {
            let already_rated = photo
                .identity()
                .and_then(|key| previous_by_key.get(&key))
                .map(|file| {
                    file.favorite_rated_for.is_some()
                        && file.favorite_rated_for == photo.favorited_at
                })
                .unwrap_or(false);
            !already_rated
        })
        .map(|photo| FavoritePhoto {
            id: photo.id.clone(),
            filename: photo.filename.clone(),
            identity_key: photo.identity(),
            favorited_at: photo.favorited_at.clone(),
        })
        .collect();

    let snapshot_local_files = if changed {
        match scan_and_hash(
            Path::new(&gallery.local_path),
            &previous_files,
            options.hash_concurrency,
        )
        .await
        {
            Ok((files, errors)) => {
                report
                    .errors
                    .extend(errors.into_iter().map(|error| SyncErrorItem {
                        phase: "scan".into(),
                        filename: None,
                        path: Some(error.path),
                        message: error.error,
                    }));
                files
            }
            Err(error) => {
                report.errors.push(SyncErrorItem {
                    phase: "scan".into(),
                    filename: None,
                    path: Some(gallery.local_path.clone()),
                    message: error.to_string(),
                });
                local_files.clone()
            }
        }
    } else {
        local_files.clone()
    };
    let snapshot = build_snapshot(
        gallery,
        &final_manifest,
        &snapshot_local_files,
        &previous_files,
        now,
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LocalFile;

    fn gallery() -> GalleryEntry {
        GalleryEntry {
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            local_path: "/tmp/gallery".into(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
        }
    }

    fn remote() -> Photo {
        Photo {
            id: "p1".into(),
            filename: "a.jpg".into(),
            size: Some(10),
            hash: Some("h1".into()),
            hash_algo: Some("sha256".into()),
            url: String::new(),
            updated_at: None,
            version_count: Some(1),
            version_number: Some(1),
            identity_key: None,
            favorited_at: None,
        }
    }

    fn local() -> LocalFile {
        LocalFile {
            filename: "a.jpg".into(),
            relative_path: "a.jpg".into(),
            path: PathBuf::from("/tmp/gallery/a.jpg"),
            size: 10,
            mtime_ms: 123,
            hash: Some("h1".into()),
            identity_key: "image:a".into(),
        }
    }

    fn previous(mtime: Option<i64>) -> SnapshotFile {
        SnapshotFile {
            id: "p1".into(),
            filename: "a.jpg".into(),
            identity_key: None,
            relative_path: Some("a.jpg".into()),
            hash: Some("h1".into()),
            hash_algo: Some("sha256".into()),
            size: Some(10),
            mtime,
            version_count: Some(1),
            version_number: Some(1),
            remote_deleted: false,
            keep_local: false,
            favorite_rated_for: None,
        }
    }

    #[test]
    fn remote_only_photos_are_not_saved_to_the_snapshot() {
        let manifest = Manifest {
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            rev: None,
            photos: vec![remote()],
        };
        let snapshot = build_snapshot(&gallery(), &manifest, &[], &[], 1);
        assert!(snapshot.files.is_empty());
    }

    #[test]
    fn snapshot_keeps_local_evidence_when_a_synced_file_disappears() {
        let manifest = Manifest {
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            rev: None,
            photos: vec![remote()],
        };
        let snapshot = build_snapshot(&gallery(), &manifest, &[], &[previous(Some(123))], 1);
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].mtime, Some(123));
    }

    #[test]
    fn downloaded_photo_is_recorded_with_local_metadata() {
        let manifest = Manifest {
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            rev: None,
            photos: vec![remote()],
        };
        let snapshot = build_snapshot(&gallery(), &manifest, &[local()], &[], 1);
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].mtime, Some(123));
        assert_eq!(snapshot.files[0].relative_path.as_deref(), Some("a.jpg"));
    }

    fn manifest_with(photo: Photo) -> Manifest {
        Manifest {
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            rev: None,
            photos: vec![photo],
        }
    }

    #[test]
    fn failed_remote_download_keeps_the_previous_base() {
        let mut photo = remote();
        photo.hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(photo),
            &[local()],
            &[previous(Some(123))],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h1"));
        assert_eq!(snapshot.files[0].version_count, Some(1));
    }

    #[test]
    fn failed_local_upload_keeps_the_previous_base() {
        let mut file = local();
        file.hash = Some("h2".into());
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(remote()),
            &[file],
            &[previous(Some(123))],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h1"));
        assert_eq!(snapshot.files[0].version_count, Some(1));
    }

    #[test]
    fn successful_transfer_records_the_new_remote_state() {
        let mut photo = remote();
        photo.hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let mut file = local();
        file.hash = Some("h2".into());
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(photo),
            &[file],
            &[previous(Some(123))],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h2"));
        assert_eq!(snapshot.files[0].version_count, Some(2));
    }

    #[test]
    fn local_files_can_be_moved_to_the_gallery_trash_folder() {
        let dir = tempfile::tempdir().unwrap();
        let gallery_dir = dir.path().join("gallery");
        std::fs::create_dir_all(&gallery_dir).unwrap();
        let source = gallery_dir.join("photo.jpg");
        std::fs::write(&source, b"photo-bytes").unwrap();

        let destination = move_to_gallery_trash(
            &gallery_dir,
            "photo.jpg",
            &source,
            "system trash unavailable",
        )
        .unwrap();
        assert!(!source.exists());
        assert!(destination.starts_with(gallery_dir.join(".picstome-trash")));
        assert_eq!(std::fs::read(&destination).unwrap(), b"photo-bytes");
    }

    #[test]
    fn conflict_without_a_common_base_is_not_saved_to_the_snapshot() {
        let mut photo = remote();
        photo.hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let mut file = local();
        file.hash = Some("h3".into());
        let snapshot = build_snapshot(&gallery(), &manifest_with(photo), &[file], &[], 1);
        assert!(snapshot.files.is_empty());
    }

    #[test]
    fn conflict_keeps_the_previous_base() {
        let mut photo = remote();
        photo.hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let mut file = local();
        file.hash = Some("h3".into());
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(photo),
            &[file],
            &[previous(Some(123))],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h1"));
        assert_eq!(snapshot.files[0].version_count, Some(1));
    }
}
