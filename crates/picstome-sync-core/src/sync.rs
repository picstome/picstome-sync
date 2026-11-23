use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::api::ApiClient;
use crate::config::Store;
use crate::error::{CoreError, CoreResult};
use crate::identity::{identity_key, preference_rank, preferred_file};
use crate::localfiles::{
    file_identity, folder_exists as path_is_dir, group_local_files, preferred_local_files,
};
use crate::model::RawMode;
use crate::model::{
    DeleteProposal, DeleteResponse, Diff, DownloadedItem, FavoritePhoto, GalleryEntry, Manifest,
    Photo, Progress, RatingSyncItem, RatingSyncPlan, RemoteDeletedItem, Snapshot, SnapshotFile,
    SyncErrorItem, SyncReport, SyncWarningItem, TrashedFile, UploadedItem, SNAPSHOT_SCHEMA,
};
use crate::sanitize::{is_inside, safe_join, sanitize_file_name};
use crate::transfer::ProgressFn;

pub type SyncProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub download_concurrency: usize,
    pub upload_concurrency: usize,
    pub chunk_bytes: u64,
    pub parallel: usize,
    pub hash_concurrency: usize,
    /// Local files (relative paths) to move to the trash, with the other
    /// files of the same photo, instead of uploading them.
    pub trash_local: Vec<String>,
    /// What to upload for photos that have a RAW file. `None` means the
    /// gallery has no answer yet: those photos wait and the report asks.
    pub raw_mode: Option<RawMode>,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            download_concurrency: 4,
            upload_concurrency: 3,
            chunk_bytes: crate::transfer::DOWNLOAD_CHUNK_BYTES,
            parallel: crate::transfer::DOWNLOAD_PARALLEL,
            hash_concurrency: default_hash_concurrency(),
            trash_local: Vec::new(),
            raw_mode: None,
        }
    }
}

/// Parallel hashing is disk bound; more streams than the device can serve only
/// adds seeking. Four is a safe floor, eight keeps an NVMe busy.
pub fn default_hash_concurrency() -> usize {
    std::env::var("PICSTOME_HASH_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|count| count.get().clamp(4, 8))
                .unwrap_or(4)
        })
}

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) async fn file_mtime_ms(path: &Path) -> Option<i64> {
    tokio::fs::metadata(path)
        .await
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
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
        favorited_at: None,
        rating: None,
        rating_applied: None,
        is_favorite: false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_snapshot(
    gallery: &GalleryEntry,
    manifest: &Manifest,
    local_files: &[crate::model::LocalFile],
    previous: &[SnapshotFile],
    diff: &Diff,
    downloaded: &[DownloadedItem],
    uploaded: &[UploadedItem],
    now: i64,
) -> Snapshot {
    let local_by_key = preferred_local_files(local_files);
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
                Some(item.local_relative_path()),
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
        // Deleted on this computer but not (yet) in Picstome: the photographer
        // has not decided. Keep the last common state so the next review
        // proposes the same choice again (delete in Picstome or download
        // again) instead of silently re-downloading the photo.
        if let Some(previous) = previous_by_key.get(&key).copied() {
            files.push(previous.clone());
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

    // Copy the photographer rating and heart from the manifest into the
    // snapshot. A manifest that omits `rating` keeps whatever was stored, so
    // an older server cannot wipe a value locally. A rating already written to
    // the file (`rating_applied`) is carried over so the prompt and the EXIF
    // write stay idempotent across syncs.
    for file in &mut files {
        let Some(key) = file.identity() else {
            continue;
        };
        let stored = previous_by_key.get(&key).copied();
        let applied = stored.and_then(|entry| entry.rating_applied);
        let answered = stored.and_then(|entry| entry.favorite_rated_for.clone());
        if let Some(photo) = manifest_by_key.get(&key) {
            file.apply_remote_rating(photo, applied, answered);
            file.favorited_at = photo.favorited_at.clone();
        }
    }

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

/// Work out which Picstome ratings still have to reach the local files.
///
/// Runs against the snapshot that was just built, so downloads and uploads of
/// this sync are already on disk. A photo whose rating was already written
/// (`rating_applied` matches the manifest) is skipped, which makes a re-sync a
/// no-op. Archived photos are included: archiving is not a rating decision.
pub(crate) fn build_rating_plan(photos: &[Photo], files: &[SnapshotFile]) -> RatingSyncPlan {
    let by_key: HashMap<String, &SnapshotFile> = files
        .iter()
        .filter_map(|file| file.identity().map(|key| (key, file)))
        .collect();

    let mut items = Vec::new();
    let mut needs_rating = Vec::new();
    for photo in photos {
        let Some(key) = photo.identity() else {
            continue;
        };
        let Some(file) = by_key.get(&key).copied() else {
            continue;
        };
        if !file.local_present {
            continue;
        }
        let applied = file.rating_applied;
        // `rating` absent means an older server: keep whatever is stored and
        // do not queue a clear.
        let Some(remote) = photo.rating else {
            continue;
        };

        // Legacy favourite (heart, no rating): the photographer answers once.
        // Until then it is a question, never an assumption of five stars, and
        // after the answer the local value must not be cleared just because
        // Picstome still stores `rating = null`.
        if photo.has_heart() && remote.is_none() {
            let answered = applied.is_some()
                || (file.favorite_rated_for.is_some()
                    && file.favorite_rated_for == photo.favorited_at);
            if !answered {
                needs_rating.push(FavoritePhoto {
                    id: photo.id.clone(),
                    filename: photo.filename.clone(),
                    identity_key: Some(key),
                    favorited_at: photo.favorited_at.clone(),
                    rating: None,
                    archived: photo.is_archived(),
                });
            }
            continue;
        }

        if remote == applied || (remote.is_none() && applied.is_none()) {
            continue;
        }
        items.push(RatingSyncItem {
            id: photo.id.clone(),
            filename: photo.filename.clone(),
            identity_key: Some(key),
            rating: remote,
            is_favorite: photo.has_heart(),
            archived: photo.is_archived(),
            applied_rating: applied,
        });
    }
    items.sort_by(|a, b| a.filename.cmp(&b.filename));
    needs_rating.sort_by(|a, b| a.filename.cmp(&b.filename));
    RatingSyncPlan {
        items,
        needs_rating,
    }
}

/// Rebuild the rating work from a stored snapshot, without another API call.
///
/// `favourites_without_rating` is the star value the photographer chose for
/// photos that have a heart but no rating (the legacy case from issue #321).
/// Until they answer, those photos stay in `needs_rating`.
pub fn rating_plan_from_snapshot(
    snapshot: &Snapshot,
    favourites_without_rating: Option<u8>,
) -> RatingSyncPlan {
    let mut items = Vec::new();
    let mut needs_rating = Vec::new();
    for file in snapshot.files.iter().filter(|file| file.local_present) {
        let Some(key) = file.identity() else {
            continue;
        };
        let heart_without_rating = file.is_favorite && file.rating.is_none();
        // Already answered (or already written to disk): never ask again, and
        // never clear the local stars just because Picstome stores no rating.
        let answered = file.rating_applied.is_some()
            || (file.favorite_rated_for.is_some() && file.favorite_rated_for == file.favorited_at);
        if heart_without_rating && answered {
            continue;
        }
        if heart_without_rating && favourites_without_rating.is_none() {
            needs_rating.push(FavoritePhoto {
                id: file.id.clone(),
                filename: file.filename.clone(),
                identity_key: Some(key),
                favorited_at: file.favorited_at.clone(),
                rating: None,
                archived: false,
            });
            continue;
        }
        let target = if heart_without_rating {
            favourites_without_rating
        } else {
            file.rating
        };
        if target == file.rating_applied || (target.is_none() && file.rating_applied.is_none()) {
            continue;
        }
        items.push(RatingSyncItem {
            id: file.id.clone(),
            filename: file.filename.clone(),
            identity_key: Some(key),
            rating: target,
            is_favorite: file.is_favorite,
            archived: false,
            applied_rating: file.rating_applied,
        });
    }
    items.sort_by(|a, b| a.filename.cmp(&b.filename));
    needs_rating.sort_by(|a, b| a.filename.cmp(&b.filename));
    RatingSyncPlan {
        items,
        needs_rating,
    }
}

pub(crate) fn download_proposals(diff: &Diff) -> Vec<DeleteProposal> {
    diff.local_deleted
        .iter()
        .map(|photo| DeleteProposal {
            id: photo.id.clone(),
            filename: photo.filename.clone(),
            identity_key: photo.identity(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

pub(crate) fn local_path_for(gallery: &GalleryEntry, file: &SnapshotFile) -> Option<PathBuf> {
    let root = Path::new(&gallery.local_path);
    let relative = file.relative_path.as_deref().unwrap_or(&file.filename);
    let candidate = root.join(relative);
    if is_inside(root, &candidate) && candidate.is_file() {
        return Some(candidate);
    }
    safe_join(root, &file.filename).filter(|path| path.is_file())
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

/// Move a group of files to the system trash in one operation when possible.
/// The `trash` crate exposes a batch API; using it avoids one Finder sound per
/// file on macOS. If the batch fails, fall back to per-file handling so a single
/// problematic path does not block the rest.
pub(crate) fn trash_targets(
    gallery: &GalleryEntry,
    targets: &[(String, String, PathBuf)],
) -> (Vec<TrashedFile>, Vec<(usize, String)>) {
    if targets.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let paths: Vec<&Path> = targets.iter().map(|(_, _, path)| path.as_path()).collect();
    if trash::delete_all(&paths).is_ok() {
        let items = targets
            .iter()
            .map(|(id, filename, path)| TrashedFile {
                id: id.clone(),
                filename: filename.clone(),
                original_path: path.display().to_string(),
                location: "system".into(),
                trash_path: None,
            })
            .collect();
        return (items, Vec::new());
    }

    let mut trashed = Vec::new();
    let mut errors = Vec::new();
    for (index, (id, filename, path)) in targets.iter().enumerate() {
        if !path.exists() {
            trashed.push(TrashedFile {
                id: id.clone(),
                filename: filename.clone(),
                original_path: path.display().to_string(),
                location: "system".into(),
                trash_path: None,
            });
            continue;
        }
        match trash_local_file(gallery, id, filename, path) {
            Ok(item) => trashed.push(item),
            Err(error) => errors.push((index, error)),
        }
    }
    (trashed, errors)
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

/// Record the stars that were just written to the local files (and, for
/// favourites without a rating, the answer the photographer gave).
///
/// `applied` is `(identity_key, rating)`. Photos are also marked as answered
/// for the current `favorited_at`, so the prompt is not shown again on the
/// next sync even when the photographer chose fewer than five stars.
pub fn mark_ratings_applied(
    store: &Store,
    gallery_id: &str,
    applied: &[(String, Option<u8>)],
    answered_for: &[(String, String)],
) -> CoreResult<usize> {
    if applied.is_empty() && answered_for.is_empty() {
        return Ok(0);
    }
    let mut snapshot = store
        .load_snapshot(gallery_id)
        .ok_or_else(|| CoreError::InvalidData("gallery snapshot not found".into()))?;
    let ratings: HashMap<&str, Option<u8>> = applied
        .iter()
        .map(|(key, rating)| (key.as_str(), *rating))
        .collect();
    let answers: HashMap<&str, &str> = answered_for
        .iter()
        .map(|(key, favorited_at)| (key.as_str(), favorited_at.as_str()))
        .collect();
    let mut updated = 0;
    for file in &mut snapshot.files {
        let Some(key) = file.identity() else {
            continue;
        };
        let Some(rating) = ratings.get(key.as_str()) else {
            continue;
        };
        file.rating_applied = *rating;
        if let Some(favorited_at) = answers.get(key.as_str()) {
            file.favorite_rated_for = Some((*favorited_at).to_string());
        }
        updated += 1;
    }
    store.save_snapshot(&snapshot)?;
    Ok(updated)
}

/// Adopt local files that the app itself rewrote (for example after writing
/// EXIF ratings) as the new baseline. Without this, the next sync mistakes the
/// metadata update for a user edit and uploads a spurious new version.
pub async fn adopt_local_changes(
    store: &Store,
    gallery_id: &str,
    filenames: &[String],
) -> CoreResult<usize> {
    if filenames.is_empty() {
        return Ok(0);
    }
    let Some(gallery) = store.get_gallery(gallery_id) else {
        return Ok(0);
    };
    let Some(mut snapshot) = store.load_snapshot(gallery_id) else {
        return Ok(0);
    };
    let wanted: HashSet<String> = filenames
        .iter()
        .filter_map(|filename| identity_key(filename))
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }

    let mut updated = 0;
    let mut fingerprints = store.load_fingerprints(&gallery.gallery_id, &gallery.local_path);
    for file in &mut snapshot.files {
        let Some(key) = file.identity() else {
            continue;
        };
        if !wanted.contains(&key) {
            continue;
        }
        let Some(path) = local_path_for(&gallery, file) else {
            continue;
        };
        let Ok(metadata) = tokio::fs::metadata(&path).await else {
            continue;
        };
        let Some(mtime_ms) = file_mtime_ms(&path).await else {
            continue;
        };
        let Ok(hash) = crate::localfiles::hash_file(&path).await else {
            continue;
        };

        if !file.local_present {
            snapshot.local_photo_count = snapshot.local_photo_count.saturating_add(1);
        }
        if let Some(previous) = file.local_size {
            snapshot.local_bytes = snapshot.local_bytes.saturating_sub(previous);
        }
        snapshot.local_bytes = snapshot.local_bytes.saturating_add(metadata.len());

        // The rewrite happened outside a sync (exiftool). Record the new bytes
        // in the fingerprint cache as well, otherwise the next sync reads the
        // whole file again to recompute a hash we just computed here.
        let relative = file
            .relative_path
            .clone()
            .unwrap_or_else(|| file.filename.clone());
        fingerprints.insert(
            relative,
            metadata.len(),
            mtime_ms,
            file_identity(&metadata).0,
            file_identity(&metadata).1,
            hash.clone(),
        );

        file.local_present = true;
        file.local_hash = Some(hash);
        file.local_size = Some(metadata.len());
        file.mtime = Some(mtime_ms);
        updated += 1;
    }

    if updated > 0 {
        store.save_snapshot(&snapshot)?;
        let _ = store.save_fingerprints(&fingerprints);
    }
    Ok(updated)
}

/// Other photo files next to `path` that belong to the same photo (same
/// stem and family, e.g. `IMG_1.CR3` beside `IMG_1.JPG`). Only the same
/// folder is searched: that is where cameras and editors put them.
pub(crate) fn companion_paths(path: &Path) -> Vec<PathBuf> {
    let Some(key) = path
        .file_name()
        .and_then(|name| identity_key(&name.to_string_lossy()))
    else {
        return Vec::new();
    };
    let Some(folder) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut companions: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| candidate != path && candidate.is_file())
        .filter(|candidate| {
            candidate.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                crate::identity::is_photo_file(&name)
                    && identity_key(&name).as_deref() == Some(&key)
            })
        })
        .collect();
    companions.sort();
    companions
}

/// Resolve photos deleted in Picstome: trash the local copy, keep it, or
/// upload it again. With `include_companions`, trashing a photo also trashes
/// the other files of the same photo beside it (its RAW).
pub async fn resolve_remote_deletions(
    api: &ApiClient,
    store: &Store,
    gallery: &GalleryEntry,
    ids: &[String],
    action: RemoteDeleteAction,
    include_companions: bool,
    now: i64,
) -> CoreResult<RemoteDeleteResolution> {
    let mut snapshot = store
        .load_snapshot(&gallery.gallery_id)
        .ok_or_else(|| CoreError::InvalidData("gallery snapshot not found".into()))?;
    let mut resolved = Vec::new();
    let mut trashed = Vec::new();
    let mut errors = Vec::new();

    if action == RemoteDeleteAction::DeleteLocal {
        let mut targets = Vec::new();
        let mut companion_targets = Vec::new();
        let mut target_ids = Vec::new();
        let mut target_names = Vec::new();
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
            if let Some(path) = local_path_for(gallery, &snapshot.files[index]) {
                if include_companions {
                    for companion in companion_paths(&path) {
                        let name = companion
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default();
                        companion_targets.push((id.clone(), name, companion));
                    }
                }
                targets.push((id.clone(), snapshot.files[index].filename.clone(), path));
                target_ids.push(id.clone());
                target_names.push(snapshot.files[index].filename.clone());
            } else {
                resolved.push(id.clone());
            }
        }

        let mut failed = HashSet::new();
        if !targets.is_empty() {
            let gallery_for_trash = gallery.clone();
            match tokio::task::spawn_blocking(move || trash_targets(&gallery_for_trash, &targets))
                .await
            {
                Ok((items, failures)) => {
                    trashed.extend(items);
                    for (index, message) in failures {
                        failed.insert(index);
                        errors.push(SyncErrorItem {
                            phase: "remote_delete".into(),
                            filename: Some(target_names[index].clone()),
                            path: None,
                            message,
                        });
                    }
                }
                Err(error) => {
                    for name in &target_names {
                        errors.push(SyncErrorItem {
                            phase: "remote_delete".into(),
                            filename: Some(name.clone()),
                            path: None,
                            message: error.to_string(),
                        });
                    }
                    failed.extend(0..target_ids.len());
                }
            }
        }
        for (index, id) in target_ids.iter().enumerate() {
            if !failed.contains(&index) {
                resolved.push(id.clone());
            }
        }

        // Companions (RAW files) follow only photos whose main file went to
        // the trash. A failure here is reported but does not undo the
        // resolution: the photo itself is already gone.
        companion_targets.retain(|(id, _, _)| resolved.contains(id));
        if !companion_targets.is_empty() {
            let gallery_for_trash = gallery.clone();
            let names: Vec<String> = companion_targets
                .iter()
                .map(|(_, name, _)| name.clone())
                .collect();
            match tokio::task::spawn_blocking(move || {
                trash_targets(&gallery_for_trash, &companion_targets)
            })
            .await
            {
                Ok((items, failures)) => {
                    trashed.extend(items);
                    for (index, message) in failures {
                        errors.push(SyncErrorItem {
                            phase: "remote_delete".into(),
                            filename: Some(names[index].clone()),
                            path: None,
                            message,
                        });
                    }
                }
                Err(error) => errors.push(SyncErrorItem {
                    phase: "remote_delete".into(),
                    filename: None,
                    path: None,
                    message: error.to_string(),
                }),
            }
        }

        let resolved_set: HashSet<&String> = resolved.iter().collect();
        snapshot
            .files
            .retain(|file| !resolved_set.contains(&file.id));
        // Recompute the counters from the snapshot instead of walking the
        // whole folder again: the files that were just trashed are the only
        // change.
        snapshot.local_photo_count = snapshot
            .files
            .iter()
            .filter(|file| file.local_present)
            .count() as u64;
        snapshot.local_bytes = snapshot
            .files
            .iter()
            .filter(|file| file.local_present)
            .filter_map(|file| file.local_size.or(file.size))
            .sum();
        snapshot.last_sync_at = now;
        store.save_snapshot(&snapshot)?;
        store.patch_gallery(&gallery.gallery_id, |entry| {
            entry.last_sync_at = Some(now);
        })?;
        return Ok(RemoteDeleteResolution {
            resolved,
            trashed,
            errors,
        });
    }

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
                unreachable!("DeleteLocal is handled before the per-photo loop")
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
                        snapshot.files[index].local_hash.as_deref(),
                    )
                    .await
                {
                    Ok(response) => {
                        if let (Some(server_hash), Some(local_hash)) = (
                            response.hash.as_deref(),
                            snapshot.files[index].local_hash.as_deref(),
                        ) {
                            if local_hash != server_hash {
                                errors.push(SyncErrorItem {
                                    phase: "remote_upload".into(),
                                    filename: Some(snapshot.files[index].filename.clone()),
                                    path: Some(path.display().to_string()),
                                    message: "the server stored different bytes; sync again".into(),
                                });
                                continue;
                            }
                        }
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

pub(crate) fn duplicate_local_warnings(
    local_files: &[crate::model::LocalFile],
) -> Vec<SyncWarningItem> {
    let groups = group_local_files(local_files);

    let mut warnings = Vec::new();
    for group in groups.values().filter(|group| group.len() > 1) {
        let Some(preferred) = preferred_file(group, |file| file.filename.as_str()).copied() else {
            continue;
        };
        let preferred_rank = preference_rank(&preferred.filename);
        for file in group {
            if file.relative_path == preferred.relative_path
                || preference_rank(&file.filename) != preferred_rank
            {
                continue;
            }
            let same_content = match (file.hash.as_deref(), preferred.hash.as_deref()) {
                (Some(left), Some(right)) => left == right,
                _ => file.size == preferred.size,
            };
            if same_content {
                continue;
            }
            warnings.push(SyncWarningItem {
                kind: "duplicate_local_ignored".into(),
                filename: Some(file.filename.clone()),
                preferred: Some(preferred.filename.clone()),
                path: Some(file.relative_path.clone()),
                message: format!(
                    "{} won't be uploaded; {} is used for this photo",
                    file.filename, preferred.filename
                ),
            });
        }
    }
    warnings.sort_by(|a, b| a.filename.cmp(&b.filename));
    warnings
}

/// List, for every photo deleted in Picstome, the other local files of the
/// same photo (typically the RAW beside the JPEG), so the UI can ask whether
/// to trash them too.
pub(crate) fn attach_companions(
    items: &mut [RemoteDeletedItem],
    local_files: &[crate::model::LocalFile],
) {
    let groups = group_local_files(local_files);
    for item in items {
        let Some(key) = item.identity_key.as_deref() else {
            continue;
        };
        let Some(group) = groups.get(key) else {
            continue;
        };
        item.companions = group
            .iter()
            .filter(|file| Some(&file.relative_path) != item.relative_path.as_ref())
            .map(|file| file.filename.clone())
            .collect();
        item.companions.sort();
    }
}

/// Where a remote photo is written locally.
pub(crate) enum DownloadTarget {
    Path {
        dest: PathBuf,
        /// `dest` relative to the gallery root, with `/` separators.
        relative_path: String,
    },
    /// The manifest filename cannot be written safely (traversal, controls).
    Unsafe,
}

/// Choose the local path for a download.
///
/// A new version of a photo that already exists locally replaces it *where it
/// is*: writing it to the gallery root would leave the old copy in its
/// subfolder and turn one photo into two local duplicates. When the existing
/// file has the same name (ignoring case) it is overwritten; otherwise the
/// remote file is placed next to it. New photos go to the gallery root.
pub(crate) fn download_target(
    root: &Path,
    photo: &Photo,
    existing: Option<&crate::model::LocalFile>,
) -> DownloadTarget {
    let Some(remote_name) = sanitize_file_name(&photo.filename) else {
        return DownloadTarget::Unsafe;
    };
    let relative_path = match existing {
        Some(local) if local.filename.eq_ignore_ascii_case(&remote_name) => {
            local.relative_path.clone()
        }
        Some(local) => match local.relative_path.rsplit_once('/') {
            Some((folder, _)) => format!("{folder}/{remote_name}"),
            None => remote_name,
        },
        None => remote_name,
    };
    let dest = root.join(&relative_path);
    if is_inside(root, &dest) {
        DownloadTarget::Path {
            dest,
            relative_path,
        }
    } else {
        DownloadTarget::Unsafe
    }
}

/// Progress of one transfer phase (downloads or uploads) with several files
/// in flight at once.
///
/// Every file reports its own `(done, total)` bytes; the phase folds those
/// into one counter so the UI shows a single, monotonic bar instead of one
/// that jumps between whichever concurrent file reported last.
#[derive(Clone)]
pub(crate) struct TransferPhase {
    progress: SyncProgressFn,
    gallery_id: String,
    phase: &'static str,
    files_total: u64,
    files_done: Arc<AtomicU64>,
    bytes_total: Option<u64>,
    bytes_done: Arc<AtomicU64>,
}

/// Minimum bytes between two progress events of the same file. Keeps the IPC
/// channel quiet on fast connections while still moving the bar smoothly.
const PROGRESS_STEP_BYTES: u64 = 512 * 1024;

impl TransferPhase {
    pub(crate) fn new(
        progress: SyncProgressFn,
        gallery_id: &str,
        phase: &'static str,
        files_total: u64,
        bytes_total: Option<u64>,
    ) -> Self {
        Self {
            progress,
            gallery_id: gallery_id.to_string(),
            phase,
            files_total,
            files_done: Arc::new(AtomicU64::new(0)),
            bytes_total: bytes_total.filter(|total| *total > 0),
            bytes_done: Arc::new(AtomicU64::new(0)),
        }
    }

    fn emit(&self, file: &str, file_done: Option<u64>, file_total: Option<u64>) {
        (self.progress)(Progress {
            gallery_id: self.gallery_id.clone(),
            phase: self.phase.into(),
            file: Some(file.to_string()),
            done: self.files_done.load(Ordering::SeqCst),
            total: self.files_total,
            file_done,
            file_total,
            bytes_done: Some(self.bytes_done.load(Ordering::SeqCst)),
            bytes_total: self.bytes_total,
        });
    }

    /// Byte callback for one file. Converts its cumulative counter into
    /// deltas for the phase total and throttles the events it emits.
    pub(crate) fn file_progress(&self, file: &str) -> ProgressFn {
        let phase = self.clone();
        let file = file.to_string();
        let seen = AtomicU64::new(0);
        let last_emit = AtomicU64::new(0);
        Arc::new(move |done, total| {
            let previous = seen.swap(done, Ordering::SeqCst);
            phase
                .bytes_done
                .fetch_add(done.saturating_sub(previous), Ordering::SeqCst);
            let emitted = last_emit.load(Ordering::SeqCst);
            if done >= total || done.saturating_sub(emitted) >= PROGRESS_STEP_BYTES {
                last_emit.store(done, Ordering::SeqCst);
                phase.emit(&file, Some(done), Some(total));
            }
        })
    }

    /// Count a finished file and report it.
    pub(crate) fn finish_file(&self, file: &str) {
        self.files_done.fetch_add(1, Ordering::SeqCst);
        self.emit(file, None, None);
    }
}

/// One photo to upload: its main file, preceded by the RAW files that go up
/// first in "JPEG + RAW" mode (so the JPEG ends up as the current version).
pub(crate) struct UploadUnit {
    pub(crate) file: crate::model::LocalFile,
    /// `"new"` or `"modified"`; a `String` because a borrowed `&'static str`
    /// inside the stream closure trips a rustc higher-ranked lifetime
    /// limitation once the future must be `Send` (Tauri commands).
    pub(crate) reason: String,
    pub(crate) raw_first: Vec<crate::model::LocalFile>,
}

/// What a sync uploads, and what it leaves alone because of the RAW choice.
pub(crate) struct UploadPlan {
    pub(crate) units: Vec<UploadUnit>,
    pub(crate) raw_kept_local: Vec<String>,
    pub(crate) raw_decision_needed: usize,
}

/// Decide what to upload according to the gallery's RAW choice.
///
/// * No choice yet: photos that have a RAW wait (JPEG included), so the
///   answer can still be "JPEG + RAW" without re-uploading anything. The
///   report asks and the UI syncs again.
/// * JPEG only: a JPEG is uploaded without its RAW; a RAW-only photo stays on
///   this computer.
/// * JPEG + RAW: new photos upload their RAW files first and the JPEG last.
///   A modified photo only re-uploads the edited file.
///
/// Held photos are removed from the diff, so they are not pending changes,
/// never deletion proposals and not recorded in the snapshot.
pub(crate) fn plan_uploads(
    diff: &mut Diff,
    local_files: &[crate::model::LocalFile],
    raw_mode: Option<RawMode>,
) -> UploadPlan {
    use crate::identity::is_raw;
    let groups = group_local_files(local_files);
    let mut plan = UploadPlan {
        units: Vec::new(),
        raw_kept_local: Vec::new(),
        raw_decision_needed: 0,
    };
    let mut keep_new = Vec::new();
    let mut keep_modified = Vec::new();

    let pending = std::mem::take(&mut diff.local_new)
        .into_iter()
        .map(|file| (file, true))
        .chain(
            std::mem::take(&mut diff.local_modified)
                .into_iter()
                .map(|file| (file, false)),
        );
    for (file, is_new) in pending {
        let raws: Vec<crate::model::LocalFile> = groups
            .get(&file.identity_key)
            .map(|group| {
                group
                    .iter()
                    .filter(|other| other.relative_path != file.relative_path)
                    .filter(|other| is_raw(&other.filename))
                    .map(|other| (*other).clone())
                    .collect()
            })
            .unwrap_or_default();
        let has_raw = is_raw(&file.filename) || !raws.is_empty();

        let raw_first = match (raw_mode, has_raw) {
            (_, false) => Vec::new(),
            (None, true) => {
                plan.raw_decision_needed += 1;
                continue;
            }
            (Some(RawMode::JpegOnly), true) if is_raw(&file.filename) => {
                plan.raw_kept_local.push(file.filename.clone());
                continue;
            }
            (Some(RawMode::JpegOnly), true) => Vec::new(),
            (Some(RawMode::JpegAndRaw), true) if is_new => {
                let mut raws = raws;
                raws.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
                raws
            }
            (Some(RawMode::JpegAndRaw), true) => Vec::new(),
        };
        plan.units.push(UploadUnit {
            file: file.clone(),
            reason: if is_new { "new" } else { "modified" }.to_string(),
            raw_first,
        });
        if is_new {
            keep_new.push(file);
        } else {
            keep_modified.push(file);
        }
    }

    diff.local_new = keep_new;
    diff.local_modified = keep_modified;
    diff.counts.local_new = diff.local_new.len();
    diff.counts.local_modified = diff.local_modified.len();
    plan.raw_kept_local.sort();
    plan
}

/// Result of uploading one photo: its RAW files and the main file.
pub(crate) struct UnitOutcome {
    pub(crate) raw: Vec<UploadedItem>,
    pub(crate) main: Result<UploadedItem, SyncErrorItem>,
}

/// Upload one file and check that the server stored the same bytes.
async fn upload_one(
    api: &ApiClient,
    gallery_id: &str,
    phase: &TransferPhase,
    file: &crate::model::LocalFile,
    reason: &str,
) -> Result<UploadedItem, SyncErrorItem> {
    let transfer_error = |message: String| SyncErrorItem {
        phase: "upload".into(),
        filename: Some(file.filename.clone()),
        path: None,
        message,
    };
    let response = api
        .upload_photo_with_progress(
            gallery_id,
            &file.filename,
            &file.path,
            file.size,
            file.hash.as_deref(),
            Some(phase.file_progress(&file.filename)),
        )
        .await
        .map_err(|err| transfer_error(err.to_string()))?;
    if let (Some(server_hash), Some(local_hash)) = (response.hash.as_deref(), file.hash.as_deref())
    {
        if local_hash != server_hash {
            return Err(transfer_error(
                "the server stored different bytes; sync again".into(),
            ));
        }
    }
    phase.finish_file(&file.filename);
    Ok(UploadedItem {
        id: response.id,
        filename: response.filename.unwrap_or_else(|| file.filename.clone()),
        reason: Some(reason.to_string()),
        size: Some(file.size),
        version_count: response.version_count,
        version_number: response.version_number,
        identity_key: Some(file.identity_key.clone()),
    })
}

/// Upload a photo: RAW files first (in order), then the main file.
///
/// If a RAW fails, the JPEG is not uploaded either, so the next sync retries
/// the whole photo. Once a RAW is in Picstome the JPEG must follow, or the RAW
/// would stay the version clients see: that last upload is retried a few
/// times before giving up.
pub(crate) async fn upload_unit(
    api: &ApiClient,
    gallery_id: &str,
    phase: &TransferPhase,
    unit: UploadUnit,
) -> UnitOutcome {
    let mut raw = Vec::new();
    for file in &unit.raw_first {
        match upload_one(api, gallery_id, phase, file, &unit.reason).await {
            Ok(item) => raw.push(item),
            Err(error) => {
                return UnitOutcome {
                    raw,
                    main: Err(error),
                };
            }
        }
    }
    let attempts = if raw.is_empty() { 1 } else { 3 };
    let mut main = upload_one(api, gallery_id, phase, &unit.file, &unit.reason).await;
    for attempt in 1..attempts {
        if main.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
        main = upload_one(api, gallery_id, phase, &unit.file, &unit.reason).await;
    }
    UnitOutcome { raw, main }
}

/// Plan and apply every download and upload, without deletions or a
/// review. The app uses [`crate::plan::plan_sync`] and
/// [`crate::plan::apply_sync`]; this one-shot form serves the smoke test,
/// the benchmark and the engine tests.
pub async fn run_gallery_sync(
    api: &ApiClient,
    store: &Store,
    gallery: &GalleryEntry,
    options: SyncOptions,
    progress: SyncProgressFn,
) -> CoreResult<SyncReport> {
    crate::plan::plan_and_apply_transfers(api, store, gallery, options, progress).await
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
        // Forget the photos the server confirmed as gone (deleted now, or
        // already missing). An older server that reports neither list is
        // trusted for the whole request, as before. Anything else stays in the
        // snapshot so a partial failure is proposed again on the next sync.
        let removed: HashSet<&String> = if result.deleted.is_empty() && result.not_found.is_empty()
        {
            photo_ids.iter().collect()
        } else {
            result
                .deleted
                .iter()
                .chain(result.not_found.iter())
                .collect()
        };
        snapshot.files.retain(|file| !removed.contains(&file.id));
        snapshot.last_sync_at = now_millis();
        snapshot.remote_rev = result.rev.clone().or(snapshot.remote_rev);
        store.save_snapshot(&snapshot)?;
        if let Some(gallery) = store.get_gallery(gallery_id) {
            let mut fingerprints = store.load_fingerprints(gallery_id, &gallery.local_path);
            let keep: HashSet<String> = snapshot
                .files
                .iter()
                .filter(|file| file.local_present)
                .map(|file| {
                    file.relative_path
                        .clone()
                        .unwrap_or_else(|| file.filename.clone())
                })
                .collect();
            fingerprints.retain_paths(&keep);
            let _ = store.save_fingerprints(&fingerprints);
        }
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
    use crate::diff::compute_diff;
    use crate::model::LocalFile;

    fn gallery() -> GalleryEntry {
        GalleryEntry {
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            local_path: "/tmp/gallery".into(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
            raw_mode: None,
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
            rating: None,
            is_favorite: None,
            archived_at: None,
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
            favorited_at: None,
            rating: None,
            rating_applied: None,
            is_favorite: false,
        }
    }

    fn downloaded(hash: &str, mtime: i64) -> DownloadedItem {
        DownloadedItem {
            id: "p1".into(),
            filename: "a.jpg".into(),
            relative_path: None,
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
            web_url: None,
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

    #[tokio::test]
    async fn adopts_app_rewrites_as_the_local_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let gallery_dir = dir.path().join("gallery");
        std::fs::create_dir_all(&gallery_dir).unwrap();
        let photo_path = gallery_dir.join("a.jpg");
        std::fs::write(&photo_path, b"original").unwrap();

        let store = Store::new(dir.path().join("state")).unwrap();
        store
            .upsert_gallery(GalleryEntry {
                gallery_id: "G1".into(),
                name: "Gallery".into(),
                local_path: gallery_dir.to_string_lossy().to_string(),
                last_sync_at: None,
                remote_rev: None,
                parent_gallery_id: None,
                raw_mode: None,
            })
            .unwrap();

        let mut base = previous(Some(123));
        base.local_hash = Some(crate::localfiles::hash_bytes(b"original"));
        base.local_size = Some(8);
        let snapshot = Snapshot {
            schema: SNAPSHOT_SCHEMA,
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            local_path: gallery_dir.to_string_lossy().to_string(),
            last_sync_at: 1,
            remote_rev: None,
            local_photo_count: 1,
            local_bytes: 8,
            files: vec![base],
        };
        store.save_snapshot(&snapshot).unwrap();

        // Simulate exiftool rewriting the rating metadata.
        std::fs::write(&photo_path, b"rated-metadata").unwrap();
        let rewritten = crate::localfiles::hash_bytes(b"rated-metadata");
        let local_file = LocalFile {
            filename: "a.jpg".into(),
            relative_path: "a.jpg".into(),
            path: photo_path.clone(),
            size: 14,
            mtime_ms: 456,
            hash: Some(rewritten.clone()),
            identity_key: "image:a".into(),
        };
        let before = compute_diff(
            &snapshot.files,
            &[remote()],
            std::slice::from_ref(&local_file),
        );
        assert_eq!(before.counts.local_modified, 1);

        let updated = adopt_local_changes(&store, "G1", &["a.jpg".to_string()])
            .await
            .unwrap();
        assert_eq!(updated, 1);

        let saved = store.load_snapshot("G1").unwrap();
        assert_eq!(
            saved.files[0].local_hash.as_deref(),
            Some(rewritten.as_str())
        );
        assert_eq!(saved.files[0].local_size, Some(14));
        assert_eq!(saved.local_bytes, 14);

        let after = compute_diff(&saved.files, &[remote()], &[local_file]);
        assert_eq!(after.counts.local_modified, 0);
        assert_eq!(after.counts.in_sync, 1);
    }

    #[test]
    fn rating_plan_only_lists_ratings_that_are_not_on_disk() {
        let mut file = previous(Some(123));
        file.identity_key = Some("image:a".into());
        file.rating = Some(4);
        file.rating_applied = Some(4);
        let mut photo = remote();
        photo.rating = Some(Some(4));
        photo.is_favorite = Some(false);
        photo.favorited_at = None;
        let plan = build_rating_plan(&[photo.clone()], std::slice::from_ref(&file));
        assert!(plan.items.is_empty());
        assert!(plan.needs_rating.is_empty());

        // A rating-only change (four to three stars) is planned without any
        // download: the file diff never enters this function.
        let mut changed = photo.clone();
        changed.rating = Some(Some(3));
        let plan = build_rating_plan(&[changed], std::slice::from_ref(&file));
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].rating, Some(3));
        assert_eq!(plan.items[0].applied_rating, Some(4));
        assert!(!plan.items[0].is_favorite);
    }

    #[test]
    fn rating_plan_asks_for_legacy_favourites_including_archived() {
        let mut file = previous(Some(123));
        file.identity_key = Some("image:a".into());
        file.rating = None;
        file.is_favorite = true;
        file.favorited_at = Some("2026-01-01T12:00:00Z".into());

        let mut photo = remote();
        photo.rating = Some(None);
        photo.is_favorite = Some(true);
        photo.favorited_at = Some("2026-01-01T12:00:00Z".into());
        let plan = build_rating_plan(&[photo.clone()], std::slice::from_ref(&file));
        assert_eq!(plan.needs_rating.len(), 1);
        assert_eq!(
            plan.needs_rating[0].identity_key.as_deref(),
            Some("image:a")
        );
        // Nothing is queued until the photographer answers.
        assert!(plan.items.is_empty());

        // Archived photos are still rated on disk; the prompt asks for them
        // too because archiving is not a rating decision.
        let mut archived = photo;
        archived.archived_at = Some("2026-02-01T10:00:00Z".into());
        let plan = build_rating_plan(&[archived], std::slice::from_ref(&file));
        assert_eq!(plan.needs_rating.len(), 1);
    }

    #[test]
    fn snapshot_keeps_the_rating_when_the_manifest_omits_it() {
        let mut file = previous(Some(123));
        file.identity_key = Some("image:a".into());
        file.rating = Some(4);
        file.is_favorite = true;
        let mut photo = remote();
        // Old server: no rating/is_favorite field at all.
        photo.rating = None;
        photo.is_favorite = None;
        file.apply_remote_rating(&photo, Some(3), None);
        assert_eq!(file.rating, Some(4));
        assert!(file.is_favorite);
        assert_eq!(file.rating_applied, Some(3));

        // Explicit null clears the stars and the heart.
        photo.rating = Some(None);
        photo.is_favorite = Some(false);
        file.apply_remote_rating(&photo, Some(4), None);
        assert_eq!(file.rating, None);
        assert!(!file.is_favorite);
    }
}
