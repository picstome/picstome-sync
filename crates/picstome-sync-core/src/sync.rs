use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::StreamExt;

use crate::api::ApiClient;
use crate::config::Store;
use crate::diff::compute_diff;
use crate::error::{CoreError, CoreResult};
use crate::identity::{identity_key, preferred_file};
use crate::localfiles::{
    folder_exists as path_is_dir, scan_and_hash, scan_local_photos, MAX_PHOTO_BYTES,
};
use crate::model::{
    DeleteProposal, DeleteProposals, DeleteResponse, Diff, DownloadedItem, FavoritePhoto,
    GalleryEntry, Manifest, Photo, Progress, Snapshot, SnapshotFile, SyncErrorItem, SyncReport,
    UploadedItem, SNAPSHOT_SCHEMA,
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

async fn file_mtime_ms(path: &Path) -> Option<i64> {
    tokio::fs::metadata(path)
        .await
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

fn local_map(local_files: &[crate::model::LocalFile]) -> HashMap<String, &crate::model::LocalFile> {
    let mut groups: HashMap<String, Vec<&crate::model::LocalFile>> = HashMap::new();
    for file in local_files {
        if let Some(key) = file.identity() {
            groups.entry(key).or_default().push(file);
        }
    }
    let mut local_by_key: HashMap<String, &crate::model::LocalFile> = HashMap::new();
    for (key, group) in &groups {
        if let Some(chosen) = preferred_file(group, |file| file.filename.as_str()) {
            local_by_key.insert(key.clone(), chosen);
        }
    }
    local_by_key
}

#[allow(clippy::too_many_arguments)]
fn snapshot_entry(
    key: &str,
    id: String,
    filename: String,
    relative_path: Option<String>,
    base_hash: Option<String>,
    base_size: Option<u64>,
    mtime: Option<i64>,
    local_present: bool,
    local_hash: Option<String>,
    local_size: Option<u64>,
    version_count: Option<u64>,
    version_number: Option<u64>,
    remote_deleted: bool,
    keep_local: bool,
    favorite_rated_for: Option<String>,
) -> SnapshotFile {
    SnapshotFile {
        id,
        filename,
        identity_key: Some(key.to_string()),
        relative_path,
        hash: base_hash,
        hash_algo: Some("sha256".to_string()),
        size: base_size,
        mtime,
        local_present,
        local_hash,
        local_size,
        version_count,
        version_number,
        remote_deleted,
        keep_local,
        favorite_rated_for,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_snapshot(
    gallery: &GalleryEntry,
    manifest: &Manifest,
    local_files: &[crate::model::LocalFile],
    previous: &[SnapshotFile],
    diff: &Diff,
    downloaded: &[DownloadedItem],
    uploaded: &[UploadedItem],
    now: i64,
) -> Snapshot {
    let local_by_key = local_map(local_files);
    let previous_by_key: HashMap<String, &SnapshotFile> = previous
        .iter()
        .filter_map(|file| file.identity().map(|key| (key, file)))
        .collect();
    let manifest_by_key: HashMap<String, &Photo> = manifest
        .photos
        .iter()
        .filter_map(|photo| photo.identity().map(|key| (key, photo)))
        .collect();
    let downloaded_by_key: HashMap<String, &DownloadedItem> = downloaded
        .iter()
        .filter_map(|item| {
            item.identity_key
                .clone()
                .or_else(|| identity_key(&item.filename))
                .map(|key| (key, item))
        })
        .collect();
    let uploaded_by_key: HashMap<String, &UploadedItem> = uploaded
        .iter()
        .filter_map(|item| {
            item.identity_key
                .clone()
                .or_else(|| identity_key(&item.filename))
                .map(|key| (key, item))
        })
        .collect();

    let mut files: Vec<SnapshotFile> = Vec::new();
    let mut processed: HashSet<String> = HashSet::new();

    for photo in &diff.in_sync {
        let Some(key) = photo.identity() else {
            continue;
        };
        processed.insert(key.clone());
        let Some(local) = local_by_key.get(&key).copied() else {
            if let Some(previous) = previous_by_key.get(&key).copied() {
                files.push(previous.clone());
            }
            continue;
        };
        let previous = previous_by_key.get(&key).copied();
        files.push(snapshot_entry(
            &key,
            photo.id.clone(),
            local.filename.clone(),
            Some(local.relative_path.clone()),
            photo
                .content_hash()
                .map(str::to_string)
                .or_else(|| local.hash.clone()),
            photo.size.or(Some(local.size)),
            Some(local.mtime_ms),
            true,
            local.hash.clone(),
            Some(local.size),
            photo.version_count,
            photo.version_number.or(photo.version_count),
            false,
            false,
            previous.and_then(|file| file.favorite_rated_for.clone()),
        ));
    }

    for photo in diff.remote_new.iter().chain(diff.remote_modified.iter()) {
        let Some(key) = photo.identity() else {
            continue;
        };
        processed.insert(key.clone());
        let previous = previous_by_key.get(&key).copied();
        if let Some(item) = downloaded_by_key.get(&key).copied() {
            files.push(snapshot_entry(
                &key,
                photo.id.clone(),
                item.filename.clone(),
                Some(item.filename.clone()),
                item.hash
                    .clone()
                    .or_else(|| photo.content_hash().map(str::to_string)),
                Some(item.size),
                item.mtime,
                true,
                item.hash.clone(),
                Some(item.size),
                photo.version_count,
                photo.version_number.or(photo.version_count),
                false,
                false,
                previous.and_then(|file| file.favorite_rated_for.clone()),
            ));
        } else if let Some(previous) = previous {
            // A failed download keeps the last common state and is retried.
            files.push(previous.clone());
        }
    }

    for file in diff.local_new.iter().chain(diff.local_modified.iter()) {
        let key = file.identity_key.clone();
        if key.is_empty() {
            continue;
        }
        processed.insert(key.clone());
        let previous = previous_by_key.get(&key).copied();
        if let Some(item) = uploaded_by_key.get(&key).copied() {
            files.push(snapshot_entry(
                &key,
                item.id
                    .clone()
                    .or_else(|| previous.map(|file| file.id.clone()))
                    .unwrap_or_default(),
                item.filename.clone(),
                Some(file.relative_path.clone()),
                file.hash.clone(),
                Some(file.size),
                Some(file.mtime_ms),
                true,
                file.hash.clone(),
                Some(file.size),
                item.version_count
                    .or_else(|| previous.and_then(|file| file.version_count)),
                item.version_number
                    .or_else(|| previous.and_then(|file| file.version_number)),
                false,
                false,
                previous.and_then(|file| file.favorite_rated_for.clone()),
            ));
        } else if let Some(previous) = previous {
            // A failed upload keeps the old base instead of marking local as
            // remotely modified on the next run.
            files.push(previous.clone());
        }
    }

    for photo in &diff.local_deleted {
        let Some(key) = photo.identity() else {
            continue;
        };
        processed.insert(key.clone());
        if let Some(previous) = previous_by_key.get(&key).copied() {
            let mut entry = previous.clone();
            entry.id = photo.id.clone();
            entry.filename = photo.filename.clone();
            entry.local_present = false;
            entry.local_hash = None;
            entry.local_size = None;
            entry.version_count = photo.version_count.or(previous.version_count);
            entry.version_number = photo
                .version_number
                .or(photo.version_count)
                .or(previous.version_number);
            entry.remote_deleted = false;
            entry.keep_local = false;
            files.push(entry);
        }
    }

    for item in &diff.remote_deleted {
        let Some(key) = item
            .identity_key
            .clone()
            .or_else(|| identity_key(&item.filename))
        else {
            continue;
        };
        processed.insert(key.clone());
        let Some(previous) = previous_by_key.get(&key).copied() else {
            continue;
        };
        let Some(local) = local_by_key.get(&key).copied() else {
            continue;
        };
        let mut entry = previous.clone();
        entry.filename = local.filename.clone();
        entry.relative_path = Some(local.relative_path.clone());
        entry.mtime = Some(local.mtime_ms);
        entry.local_present = true;
        entry.local_hash = local.hash.clone();
        entry.local_size = Some(local.size);
        entry.remote_deleted = true;
        entry.keep_local = previous.keep_local;
        files.push(entry);
    }

    for conflict in &diff.conflicts {
        let key = conflict.identity_key.clone();
        processed.insert(key.clone());
        if let Some(previous) = previous_by_key.get(&key).copied() {
            let mut entry = previous.clone();
            if let Some(local) = local_by_key.get(&key).copied() {
                entry.filename = local.filename.clone();
                entry.relative_path = Some(local.relative_path.clone());
                entry.mtime = Some(local.mtime_ms);
                entry.local_present = true;
                entry.local_hash = local.hash.clone();
                entry.local_size = Some(local.size);
            }
            files.push(entry);
        }
    }

    // Keep previous entries whose remote copy is gone but whose local copy is
    // still present, including items the user chose to keep locally.
    for previous in previous {
        let Some(key) = previous.identity() else {
            continue;
        };
        if processed.contains(&key) {
            continue;
        }
        let Some(local) = local_by_key.get(&key).copied() else {
            continue;
        };
        if manifest_by_key.contains_key(&key) {
            continue;
        }
        let mut entry = previous.clone();
        entry.filename = local.filename.clone();
        entry.relative_path = Some(local.relative_path.clone());
        entry.mtime = Some(local.mtime_ms);
        entry.local_present = true;
        entry.local_hash = local.hash.clone();
        entry.local_size = Some(local.size);
        entry.remote_deleted = true;
        entry.keep_local = previous.keep_local;
        files.push(entry);
    }

    files.sort_by(|a, b| {
        a.filename
            .cmp(&b.filename)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });

    let mut local_photo_count = local_files.len() as u64;
    let mut local_bytes: u64 = local_files.iter().map(|file| file.size).sum();
    let mut local_sizes: HashMap<String, u64> = local_files
        .iter()
        .map(|file| (file.filename.to_lowercase(), file.size))
        .collect();
    for item in downloaded {
        match local_sizes.remove(&item.filename.to_lowercase()) {
            Some(previous_size) => {
                local_bytes = local_bytes.saturating_sub(previous_size);
            }
            None => local_photo_count += 1,
        }
        local_bytes = local_bytes.saturating_add(item.size);
    }

    Snapshot {
        gallery_id: gallery.gallery_id.clone(),
        name: if manifest.name.is_empty() {
            gallery.name.clone()
        } else {
            manifest.name.clone()
        },
        local_path: gallery.local_path.clone(),
        schema: SNAPSHOT_SCHEMA,
        last_sync_at: now,
        remote_rev: manifest.rev.clone(),
        local_photo_count,
        local_bytes,
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
        let (local, _) =
            scan_local_photos(Path::new(&gallery.local_path), 8, true, MAX_PHOTO_BYTES);
        snapshot.local_photo_count = local.len() as u64;
        snapshot.local_bytes = local.iter().map(|file| file.size).sum();
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
                            expected_hash: photo.content_hash().map(str::to_string),
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
                            let identity_key = photo.identity();
                            let mtime = file_mtime_ms(&dest).await;
                            Ok(DownloadedItem {
                                id: photo.id,
                                filename: photo.filename,
                                size: result.size,
                                skipped: result.skipped,
                                identity_key,
                                hash: Some(result.hash),
                                mtime,
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
                        let filename = response.filename.unwrap_or_else(|| entry.filename.clone());
                        Ok(UploadedItem {
                            id: response.id,
                            filename,
                            reason: Some(reason),
                            size: Some(entry.size),
                            version_count: response.version_count,
                            version_number: response.version_number,
                            identity_key: Some(entry.identity_key.clone()),
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

    let previous_by_key: HashMap<String, &SnapshotFile> = previous_files
        .iter()
        .filter_map(|file| file.identity().map(|key| (key, file)))
        .collect();
    report.favorites = manifest
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

    let snapshot = build_snapshot(
        gallery,
        &manifest,
        &local_files,
        &previous_files,
        &diff,
        &report.downloaded,
        &report.uploaded,
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
            download_hash: Some("h1".into()),
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
            local_present: mtime.is_some(),
            local_hash: mtime.map(|_| "h1".to_string()),
            local_size: mtime.map(|_| 10),
            version_count: Some(1),
            version_number: Some(1),
            remote_deleted: false,
            keep_local: false,
            favorite_rated_for: None,
        }
    }

    fn downloaded(hash: &str, mtime: i64) -> DownloadedItem {
        DownloadedItem {
            id: "p1".into(),
            filename: "a.jpg".into(),
            size: 10,
            skipped: false,
            identity_key: Some("image:a".into()),
            hash: Some(hash.into()),
            mtime: Some(mtime),
        }
    }

    fn diff_with() -> Diff {
        Diff::default()
    }

    fn conflict() -> crate::model::Conflict {
        crate::model::Conflict {
            identity_key: "image:a".into(),
            remote: crate::model::ConflictPhoto {
                filename: "a.jpg".into(),
                version_count: Some(2),
                size: Some(10),
            },
            local: crate::model::ConflictPhoto {
                filename: "a.jpg".into(),
                version_count: Some(1),
                size: Some(10),
            },
        }
    }

    #[test]
    fn remote_only_photos_are_not_saved_to_the_snapshot() {
        let manifest = manifest_with(remote());
        let snapshot = build_snapshot(&gallery(), &manifest, &[], &[], &diff_with(), &[], &[], 1);
        assert!(snapshot.files.is_empty());
    }

    #[test]
    fn snapshot_keeps_local_evidence_when_a_synced_file_disappears() {
        let manifest = manifest_with(remote());
        let mut diff = diff_with();
        diff.local_deleted = vec![remote()];
        let snapshot = build_snapshot(
            &gallery(),
            &manifest,
            &[],
            &[previous(Some(123))],
            &diff,
            &[],
            &[],
            1,
        );
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].mtime, Some(123));
    }

    #[test]
    fn downloaded_photo_is_recorded_with_local_metadata() {
        let manifest = manifest_with(remote());
        let mut diff = diff_with();
        diff.remote_new = vec![remote()];
        let snapshot = build_snapshot(
            &gallery(),
            &manifest,
            &[],
            &[],
            &diff,
            &[downloaded("h1", 123)],
            &[],
            1,
        );
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].mtime, Some(123));
        assert_eq!(snapshot.files[0].relative_path.as_deref(), Some("a.jpg"));
        assert_eq!(snapshot.local_photo_count, 1);
        assert_eq!(snapshot.local_bytes, 10);
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
        photo.download_hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let mut diff = diff_with();
        diff.remote_modified = vec![photo.clone()];
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(photo),
            &[local()],
            &[previous(Some(123))],
            &diff,
            &[],
            &[],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h1"));
        assert_eq!(snapshot.files[0].version_count, Some(1));
    }

    #[test]
    fn failed_local_upload_keeps_the_previous_base() {
        let mut file = local();
        file.hash = Some("h2".into());
        let mut diff = diff_with();
        diff.local_modified = vec![file.clone()];
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(remote()),
            &[file],
            &[previous(Some(123))],
            &diff,
            &[],
            &[],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h1"));
        assert_eq!(snapshot.files[0].version_count, Some(1));
    }

    #[test]
    fn successful_transfer_records_the_new_remote_state() {
        let mut photo = remote();
        photo.hash = Some("h2".into());
        photo.download_hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let mut diff = diff_with();
        diff.remote_modified = vec![photo.clone()];
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(photo),
            &[local()],
            &[previous(Some(123))],
            &diff,
            &[downloaded("h2", 456)],
            &[],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h2"));
        assert_eq!(snapshot.files[0].version_count, Some(2));
        assert_eq!(snapshot.files[0].mtime, Some(456));
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
        photo.download_hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let mut file = local();
        file.hash = Some("h3".into());
        let mut diff = diff_with();
        diff.conflicts = vec![conflict()];
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(photo),
            &[file],
            &[],
            &diff,
            &[],
            &[],
            1,
        );
        assert!(snapshot.files.is_empty());
    }

    #[test]
    fn conflict_keeps_the_previous_base() {
        let mut photo = remote();
        photo.hash = Some("h2".into());
        photo.download_hash = Some("h2".into());
        photo.version_count = Some(2);
        photo.version_number = Some(2);
        let mut file = local();
        file.hash = Some("h3".into());
        let mut diff = diff_with();
        diff.conflicts = vec![conflict()];
        let snapshot = build_snapshot(
            &gallery(),
            &manifest_with(photo),
            &[file],
            &[previous(Some(123))],
            &diff,
            &[],
            &[],
            1,
        );
        assert_eq!(snapshot.files[0].hash.as_deref(), Some("h1"));
        assert_eq!(snapshot.files[0].version_count, Some(1));
    }
}
