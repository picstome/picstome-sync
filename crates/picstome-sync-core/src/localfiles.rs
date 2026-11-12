use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use futures::StreamExt;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::cache::FingerprintCache;
use crate::error::{CoreError, CoreResult};
use crate::identity::{identity_key, is_photo_file, preferred_file};
use crate::model::{LocalFile, SnapshotFile};

pub const MAX_PHOTO_BYTES: u64 = 500 * 1024 * 1024;
pub const EDITED_FOLDER_NAMES: &[&str] =
    &["editadas", "editada", "edits", "edited", "edit", "finales"];
/// Sequential reads benefit from a large buffer: a 64 KiB buffer turns one
/// 40 MB RAW into 640 blocking-pool round trips.
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ScanError {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct RawLocalFile {
    pub filename: String,
    pub relative_path: String,
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    /// File identity used to notice a replacement that kept size and mtime.
    pub inode: Option<u64>,
    pub device: Option<u64>,
}

#[cfg(unix)]
pub fn file_identity(metadata: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    use std::os::unix::fs::MetadataExt;
    (Some(metadata.ino()), Some(metadata.dev()))
}

#[cfg(not(unix))]
pub fn file_identity(_metadata: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    (None, None)
}

/// Counters for the hash phase, surfaced in the sync report so a repeat sync
/// can prove it did not read any file content.
#[derive(Debug, Clone, Copy, Default)]
pub struct HashStats {
    /// Files considered in total.
    pub files: u64,
    /// Files whose bytes had to be read.
    pub hashed: u64,
    /// Files resolved from the fingerprint cache or the snapshot baseline.
    pub cached: u64,
    /// Non-preferred duplicates left unhashed on purpose.
    pub skipped: u64,
    /// Bytes read from disk to compute hashes.
    pub bytes_read: u64,
}

pub fn is_edited_folder_name(name: &str) -> bool {
    EDITED_FOLDER_NAMES.contains(&name.trim().to_ascii_lowercase().as_str())
}

pub fn scan_local_photos(
    dir: &Path,
    max_depth: usize,
    exclude_edited_folders: bool,
    max_bytes: u64,
) -> (Vec<RawLocalFile>, Vec<ScanError>) {
    scan_local_photos_with(
        dir,
        max_depth,
        exclude_edited_folders,
        max_bytes,
        &mut |_, _| {},
    )
}

/// Same walk as [`scan_local_photos`], reporting `(seen, matched)` entries so
/// the UI can show that a large folder is progressing instead of appearing
/// frozen.
pub fn scan_local_photos_with<F>(
    dir: &Path,
    max_depth: usize,
    exclude_edited_folders: bool,
    max_bytes: u64,
    on_progress: &mut F,
) -> (Vec<RawLocalFile>, Vec<ScanError>)
where
    F: FnMut(u64, u64),
{
    let root = crate::sanitize::lexical_absolute(dir);
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut seen = 0u64;

    let walker = WalkDir::new(&root).max_depth(max_depth).into_iter();
    for entry in walker.filter_entry(|entry| {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name().to_string_lossy();
        if name.starts_with('.') {
            return false;
        }
        if exclude_edited_folders && entry.file_type().is_dir() && is_edited_folder_name(&name) {
            return false;
        }
        true
    }) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                errors.push(ScanError {
                    path: err
                        .path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    error: err.to_string(),
                });
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        seen += 1;
        // Report every 20 entries: enough to keep the counter alive on huge
        // folders without flooding the IPC channel. The first few are reported
        // too, so a small folder still shows movement immediately.
        if seen <= 5 || seen % 20 == 0 {
            on_progress(seen, files.len() as u64);
        }

        let filename = entry.file_name().to_string_lossy().to_string();
        if !is_photo_file(&filename) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                errors.push(ScanError {
                    path: entry.path().display().to_string(),
                    error: err.to_string(),
                });
                continue;
            }
        };

        if metadata.len() > max_bytes {
            errors.push(ScanError {
                path: entry.path().display().to_string(),
                error: "file too large".into(),
            });
            continue;
        }

        let relative_path = entry
            .path()
            .strip_prefix(&root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| filename.clone());

        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);

        let (inode, device) = file_identity(&metadata);

        files.push(RawLocalFile {
            filename,
            relative_path,
            path: entry.path().to_path_buf(),
            size: metadata.len(),
            mtime_ms,
            inode,
            device,
        });
    }

    on_progress(seen, files.len() as u64);
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    (files, errors)
}

pub fn group_local_files(local_files: &[LocalFile]) -> HashMap<String, Vec<&LocalFile>> {
    let mut groups: HashMap<String, Vec<&LocalFile>> = HashMap::new();
    for file in local_files {
        if let Some(key) = file.identity() {
            groups.entry(key).or_default().push(file);
        }
    }
    groups
}

pub fn preferred_local_files(local_files: &[LocalFile]) -> HashMap<String, &LocalFile> {
    let mut preferred = HashMap::new();
    for (key, group) in group_local_files(local_files) {
        if let Some(chosen) = preferred_file(&group, |file| file.filename.as_str()) {
            preferred.insert(key, *chosen);
        }
    }
    preferred
}

/// Result of hashing one file: the digest plus the metadata of the exact bytes
/// that were read.
///
/// The metadata is captured from the open handle, so a file edited between the
/// directory walk and the hash is recorded with its *new* size/mtime instead of
/// a stale pair that would make the cache trust the wrong hash forever.
#[derive(Debug, Clone)]
pub struct HashedFile {
    pub hash: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub inode: Option<u64>,
    pub device: Option<u64>,
}

/// Hash a file with a large sequential read buffer.
///
/// This is blocking on purpose: it is called from `spawn_blocking`, and the
/// async path (one small read per poll) was the main cost when hashing RAW
/// files, which are tens of megabytes each.
pub fn hash_file_sync(path: &Path) -> CoreResult<HashedFile> {
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let (inode, device) = file_identity(&metadata);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(HashedFile {
        hash: hex::encode(hasher.finalize()),
        size: metadata.len(),
        mtime_ms,
        inode,
        device,
    })
}

pub async fn hash_file(path: &Path) -> CoreResult<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || hash_file_sync(&path).map(|hashed| hashed.hash))
        .await
        .map_err(|err| CoreError::Other(format!("hash task failed: {err}")))?
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn snapshot_key(file: &SnapshotFile) -> String {
    file.relative_path
        .clone()
        .unwrap_or_else(|| file.filename.clone())
}

/// Hashes trusted from the last snapshot. Kept separate from the fingerprint
/// cache so an old snapshot can still accelerate the first run after an
/// upgrade, before a cache file exists.
///
/// Entries are indexed by relative path *and* by filename: older snapshots did
/// not always record a relative path, and a file in a subfolder must still be
/// recognised instead of being hashed again.
fn build_snapshot_cache(snapshot_files: &[SnapshotFile]) -> HashMap<String, SnapshotFile> {
    let mut cache = HashMap::new();
    for file in snapshot_files {
        if file.local_present && file.local_hash.is_some() {
            cache
                .entry(snapshot_key(file))
                .or_insert_with(|| file.clone());
            cache
                .entry(file.filename.clone())
                .or_insert_with(|| file.clone());
        }
    }
    cache
}

/// Pick the one file per logical photo that the engine will upload or diff.
/// Everything else is a duplicate representation (RAW next to JPEG).
fn preferred_paths(records: &[RawLocalFile]) -> HashMap<String, String> {
    let mut groups: HashMap<String, Vec<&RawLocalFile>> = HashMap::new();
    for record in records {
        if let Some(key) = identity_key(&record.filename) {
            groups.entry(key).or_default().push(record);
        }
    }
    let mut preferred = HashMap::new();
    for (key, group) in groups {
        if let Some(chosen) = preferred_file(&group, |file| file.filename.as_str()) {
            preferred.insert(key, chosen.relative_path.clone());
        }
    }
    preferred
}

/// Resolved hash for a file without touching its bytes, if any source knows it.
fn cached_hash(
    record: &RawLocalFile,
    snapshot: &HashMap<String, SnapshotFile>,
    cache: Option<&FingerprintCache>,
) -> Option<String> {
    if let Some(hash) = cache.and_then(|cache| {
        cache
            .lookup(
                &record.relative_path,
                record.size,
                record.mtime_ms,
                record.inode,
                record.device,
            )
            .map(str::to_string)
    }) {
        return Some(hash);
    }
    snapshot
        .get(&record.relative_path)
        .or_else(|| snapshot.get(&record.filename))
        .and_then(|file| {
            file.local_hash.clone().filter(|_| {
                file.local_size == Some(record.size) && file.mtime == Some(record.mtime_ms)
            })
        })
}

pub async fn hash_local_records(
    records: Vec<RawLocalFile>,
    snapshot_files: &[SnapshotFile],
    concurrency: usize,
) -> Vec<Result<LocalFile, ScanError>> {
    hash_local_records_with(records, snapshot_files, concurrency, |_, _| {}).await
}

/// Same as [`hash_local_records`], reporting `(done, total)` so the UI can show
/// hashing progress. Hashing thousands of RAW files takes minutes; without this
/// the scan phase looks frozen.
pub async fn hash_local_records_with<F>(
    records: Vec<RawLocalFile>,
    snapshot_files: &[SnapshotFile],
    concurrency: usize,
    on_progress: F,
) -> Vec<Result<LocalFile, ScanError>>
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    let (results, _) =
        hash_local_records_cached(records, snapshot_files, None, concurrency, on_progress).await;
    results
}

/// Hash the scan, reusing everything the fingerprint cache and the snapshot
/// already know.
///
/// Only the preferred representation of a logical photo is hashed when its
/// bytes are unknown: that is the file the engine uploads and diffs, while a
/// duplicate RAW simply reuses its cached hash or is left without one (the
/// diff and duplicate checks already fall back to size + mtime). This is what
/// stops a RAW+JPEG shoot from re-reading every RAW on every sync.
pub async fn hash_local_records_cached<F>(
    records: Vec<RawLocalFile>,
    snapshot_files: &[SnapshotFile],
    cache: Option<&FingerprintCache>,
    concurrency: usize,
    on_progress: F,
) -> (Vec<Result<LocalFile, ScanError>>, HashStats)
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    let snapshot = build_snapshot_cache(snapshot_files);
    let preferred = preferred_paths(&records);
    let total = records.len() as u64;
    let on_progress = std::sync::Arc::new(on_progress);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stats = std::sync::Arc::new(std::sync::Mutex::new(HashStats {
        files: total,
        ..Default::default()
    }));

    // Resolve everything the cache already knows, then decide what is left.
    let mut resolved: Vec<Option<String>> = Vec::with_capacity(records.len());
    let mut known: HashMap<String, String> = HashMap::new();
    let mut pre_cached = 0u64;
    for record in &records {
        match cached_hash(record, &snapshot, cache) {
            Some(hash) => {
                pre_cached += 1;
                known.insert(record.relative_path.clone(), hash.clone());
                resolved.push(Some(hash));
            }
            None => resolved.push(None),
        }
    }

    // A duplicate representation (RAW next to its JPEG) is only hashed when the
    // preferred file still had to be read too. That keeps the very first scan
    // behaving exactly as before, while a repeat sync never reads a RAW whose
    // preferred file is already cached: it is only used for warnings, and those
    // fall back to size when a hash is absent.
    let mut pre_skipped = 0u64;
    let mut skip_duplicates: HashSet<usize> = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        if resolved[index].is_some() {
            continue;
        }
        let Some(key) = identity_key(&record.filename) else {
            continue;
        };
        let Some(preferred_path) = preferred.get(&key) else {
            continue;
        };
        if preferred_path == &record.relative_path {
            continue;
        }
        if known.contains_key(preferred_path) {
            pre_skipped += 1;
            skip_duplicates.insert(index);
        }
    }
    {
        let mut stats = stats.lock().expect("hash stats poisoned");
        stats.cached = pre_cached;
        stats.skipped = pre_skipped;
    }
    done.fetch_add(
        pre_cached + pre_skipped,
        std::sync::atomic::Ordering::Relaxed,
    );
    on_progress(done.load(std::sync::atomic::Ordering::Relaxed), total);

    let results: Vec<Option<Result<LocalFile, ScanError>>> =
        futures::stream::iter(records.into_iter().zip(resolved).enumerate().map(
            |(index, (record, known_hash))| {
                let skip = skip_duplicates.contains(&index);
                let done = done.clone();
                let on_progress = on_progress.clone();
                let stats = stats.clone();
                async move {
                    if let Some(hash) = known_hash {
                        let identity_key = identity_key(&record.filename).unwrap_or_default();
                        return Some(Ok(LocalFile {
                            filename: record.filename,
                            relative_path: record.relative_path,
                            path: record.path,
                            size: record.size,
                            mtime_ms: record.mtime_ms,
                            hash: Some(hash),
                            identity_key,
                        }));
                    }

                    if skip {
                        // Duplicate representation whose preferred file is
                        // already cached: leave it unhashed instead of reading
                        // tens of megabytes for a file that is never uploaded.
                        let identity_key = identity_key(&record.filename).unwrap_or_default();
                        return Some(Ok(LocalFile {
                            filename: record.filename,
                            relative_path: record.relative_path,
                            path: record.path,
                            size: record.size,
                            mtime_ms: record.mtime_ms,
                            hash: None,
                            identity_key,
                        }));
                    }

                    let path = record.path.clone();
                    let hashed =
                        match tokio::task::spawn_blocking(move || hash_file_sync(&path)).await {
                            Ok(Ok(hashed)) => hashed,
                            Ok(Err(err)) => {
                                return Some(Err(ScanError {
                                    path: record.path.display().to_string(),
                                    error: err.to_string(),
                                }))
                            }
                            Err(err) => {
                                return Some(Err(ScanError {
                                    path: record.path.display().to_string(),
                                    error: format!("hash task failed: {err}"),
                                }))
                            }
                        };
                    {
                        let mut stats = stats.lock().expect("hash stats poisoned");
                        stats.hashed += 1;
                        stats.bytes_read = stats.bytes_read.saturating_add(hashed.size);
                    }
                    let finished = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    on_progress(finished, total);

                    let identity_key = identity_key(&record.filename).unwrap_or_default();
                    Some(Ok(LocalFile {
                        filename: record.filename,
                        relative_path: record.relative_path,
                        path: record.path,
                        // Report what was actually read: if the file changed
                        // between the walk and the hash, this is the truth the
                        // cache has to store.
                        size: hashed.size,
                        mtime_ms: hashed.mtime_ms,
                        hash: Some(hashed.hash),
                        identity_key,
                    }))
                }
            },
        ))
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await;

    let results = results.into_iter().flatten().collect();
    let stats = *stats.lock().expect("hash stats poisoned");
    (results, stats)
}

pub async fn scan_and_hash(
    dir: &Path,
    snapshot_files: &[SnapshotFile],
    concurrency: usize,
) -> CoreResult<(Vec<LocalFile>, Vec<ScanError>)> {
    scan_and_hash_with(dir, snapshot_files, concurrency, |_, _| {}, |_, _| {}).await
}

/// Same as [`scan_and_hash`], reporting progress through `on_scan` (entries
/// visited / photos matched) and `on_hash` (files hashed / total).
pub async fn scan_and_hash_with<Scan, Hash>(
    dir: &Path,
    snapshot_files: &[SnapshotFile],
    concurrency: usize,
    on_scan: Scan,
    on_hash: Hash,
) -> CoreResult<(Vec<LocalFile>, Vec<ScanError>)>
where
    Scan: Fn(u64, u64) + Send + Sync + 'static,
    Hash: Fn(u64, u64) + Send + Sync + 'static,
{
    let (files, errors, _) =
        scan_and_hash_cached(dir, snapshot_files, None, concurrency, on_scan, on_hash).await?;
    Ok((files, errors))
}

/// Full scan + hash using the fingerprint cache, returning what the sync needs
/// to persist the cache back and to report how much work was avoided.
pub async fn scan_and_hash_cached<Scan, Hash>(
    dir: &Path,
    snapshot_files: &[SnapshotFile],
    cache: Option<&FingerprintCache>,
    concurrency: usize,
    on_scan: Scan,
    on_hash: Hash,
) -> CoreResult<(Vec<LocalFile>, Vec<ScanError>, HashStats)>
where
    Scan: Fn(u64, u64) + Send + Sync + 'static,
    Hash: Fn(u64, u64) + Send + Sync + 'static,
{
    let metadata = tokio::fs::metadata(dir).await?;
    if !metadata.is_dir() {
        return Err(CoreError::InvalidData(format!(
            "not a directory: {}",
            dir.display()
        )));
    }

    let mut on_scan_cb = |seen: u64, matched: u64| on_scan(seen, matched);
    let (records, mut errors) =
        scan_local_photos_with(dir, 8, true, MAX_PHOTO_BYTES, &mut on_scan_cb);
    let (results, stats) =
        hash_local_records_cached(records, snapshot_files, cache, concurrency, on_hash).await;

    let mut files = Vec::with_capacity(results.len());
    for result in results {
        match result {
            Ok(file) => files.push(file),
            Err(err) => errors.push(err),
        }
    }
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok((files, errors, stats))
}

pub fn folder_exists(folder: &str) -> bool {
    Path::new(folder).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn scans_and_hashes_photos_and_skips_edited_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.jpg"), b"hello").unwrap();
        std::fs::write(root.join("notes.txt"), b"ignore").unwrap();
        std::fs::create_dir_all(root.join("Editadas")).unwrap();
        std::fs::write(root.join("Editadas").join("b.jpg"), b"edited").unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden").join("c.jpg"), b"hidden").unwrap();

        let (files, errors) = scan_and_hash(root, &[], 2).await.unwrap();
        assert!(errors.is_empty());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "a.jpg");
        assert_eq!(
            files[0].hash.as_deref(),
            Some(hash_bytes(b"hello").as_str())
        );
        assert_eq!(files[0].identity_key, "image:a");
    }

    #[test]
    fn scans_only_known_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut file = std::fs::File::create(root.join("photo.JPG")).unwrap();
        file.write_all(b"x").unwrap();
        std::fs::write(root.join("readme.md"), b"x").unwrap();

        let (files, errors) = scan_local_photos(root, 2, true, MAX_PHOTO_BYTES);
        assert!(errors.is_empty());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "photo.JPG");
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;
    use crate::model::SnapshotFile;
    use std::time::SystemTime;

    fn snapshot_for(path: &Path, mtime_ms: i64, hash: &str) -> SnapshotFile {
        SnapshotFile {
            id: "p1".into(),
            filename: path.file_name().unwrap().to_string_lossy().to_string(),
            identity_key: None,
            relative_path: Some(path.file_name().unwrap().to_string_lossy().to_string()),
            hash: Some(hash.into()),
            hash_algo: Some("sha256".into()),
            size: Some(std::fs::metadata(path).unwrap().len()),
            mtime: Some(mtime_ms),
            local_present: true,
            local_hash: Some(hash.into()),
            local_size: Some(std::fs::metadata(path).unwrap().len()),
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

    #[test]
    fn edited_folder_names_are_case_insensitive() {
        assert!(is_edited_folder_name("EDITS"));
        assert!(is_edited_folder_name(" Editadas "));
        assert!(!is_edited_folder_name("editorial"));
    }

    #[test]
    fn reports_too_large_files_and_missing_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.jpg"), vec![0u8; 32]).unwrap();
        let (files, errors) = scan_local_photos(dir.path(), 2, true, 16);
        assert!(files.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].error.contains("too large"));

        let (_, errors) = scan_local_photos(&dir.path().join("missing"), 2, true, 16);
        assert_eq!(errors.len(), 1);
        assert!(!folder_exists("/definitely/not/a/real/folder"));
    }

    #[tokio::test]
    async fn hashes_reuse_matching_snapshot_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        std::fs::write(&path, b"hello").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let mtime_ms = metadata
            .modified()
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let snapshot = snapshot_for(&path, mtime_ms, "cached-hash");

        let records = vec![RawLocalFile {
            filename: "a.jpg".into(),
            relative_path: "a.jpg".into(),
            path: path.clone(),
            size: metadata.len(),
            mtime_ms,
            inode: None,
            device: None,
        }];
        let results = hash_local_records(records, &[snapshot], 1).await;
        assert_eq!(
            results[0].as_ref().unwrap().hash.as_deref(),
            Some("cached-hash")
        );
    }

    #[tokio::test]
    async fn reports_hash_and_scan_errors() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.jpg");
        assert!(hash_file(&missing).await.is_err());

        let records = vec![RawLocalFile {
            filename: "missing.jpg".into(),
            relative_path: "missing.jpg".into(),
            path: missing,
            size: 1,
            mtime_ms: 0,
            inode: None,
            device: None,
        }];
        let results = hash_local_records(records, &[], 1).await;
        assert!(results[0].is_err());

        assert!(scan_and_hash(&dir.path().join("nope"), &[], 1)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn groups_local_identity_and_prefers_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("shot.dng"), b"raw").unwrap();
        std::fs::write(dir.path().join("shot.jpg"), b"jpeg").unwrap();
        let (files, errors) = scan_and_hash(dir.path(), &[], 2).await.unwrap();
        assert!(errors.is_empty());

        let groups = group_local_files(&files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups.values().next().unwrap().len(), 2);
        let preferred = preferred_local_files(&files);
        assert_eq!(
            preferred.values().next().unwrap().filename.as_str(),
            "shot.jpg"
        );
    }

    #[tokio::test]
    async fn scans_files_in_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z.jpg"), b"z").unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"a").unwrap();
        let (files, errors) = scan_and_hash(dir.path(), &[], 2).await.unwrap();
        assert!(errors.is_empty());
        let names: Vec<&str> = files.iter().map(|file| file.filename.as_str()).collect();
        assert_eq!(names, vec!["a.jpg", "z.jpg"]);
    }

    #[tokio::test]
    async fn scan_and_hash_reports_progress() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        for index in 0..60 {
            std::fs::write(dir.path().join(format!("p{index:02}.jpg")), b"x").unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();

        let scans = Arc::new(AtomicU64::new(0));
        let hashes = Arc::new(AtomicU64::new(0));
        let scans_cb = scans.clone();
        let hashes_cb = hashes.clone();
        let (files, errors) = scan_and_hash_with(
            dir.path(),
            &[],
            2,
            move |seen, matched| {
                assert!(seen >= matched);
                scans_cb.store(seen, Ordering::SeqCst);
            },
            move |done, total| {
                assert!(done <= total);
                hashes_cb.store(total, Ordering::SeqCst);
            },
        )
        .await
        .unwrap();

        assert!(errors.is_empty());
        assert_eq!(files.len(), 60);
        // The walk saw the 60 photos plus the text file; the hash phase knew
        // ahead of time how many files it had to fingerprint.
        assert_eq!(scans.load(Ordering::SeqCst), 61);
        assert_eq!(hashes.load(Ordering::SeqCst), 60);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::cache::FingerprintCache;

    fn write_photo(dir: &Path, name: &str, bytes: &[u8]) -> RawLocalFile {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let mtime_ms = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let (inode, device) = file_identity(&metadata);
        RawLocalFile {
            filename: name.to_string(),
            relative_path: name.to_string(),
            path,
            size: metadata.len(),
            mtime_ms,
            inode,
            device,
        }
    }

    async fn hash_with_cache(
        records: Vec<RawLocalFile>,
        cache: &FingerprintCache,
    ) -> (Vec<LocalFile>, HashStats) {
        let (results, stats) =
            hash_local_records_cached(records, &[], Some(cache), 2, |_, _| {}).await;
        (
            results.into_iter().map(|result| result.unwrap()).collect(),
            stats,
        )
    }

    #[tokio::test]
    async fn cache_hit_reads_nothing_and_miss_reads_everything() {
        let dir = tempfile::tempdir().unwrap();
        let record = write_photo(dir.path(), "a.jpg", b"first version");
        let mut cache = FingerprintCache::new("G1", dir.path().to_string_lossy());

        // Cold: the file must be read.
        let (files, stats) = hash_with_cache(vec![record.clone()], &cache).await;
        assert_eq!(stats.hashed, 1);
        assert!(stats.bytes_read > 0);
        let hash = files[0].hash.clone().unwrap();

        cache.insert(
            "a.jpg",
            record.size,
            record.mtime_ms,
            record.inode,
            record.device,
            hash.clone(),
        );
        let (files, stats) = hash_with_cache(vec![record.clone()], &cache).await;
        assert_eq!(stats.hashed, 0);
        assert_eq!(stats.bytes_read, 0);
        assert_eq!(stats.cached, 1);
        assert_eq!(files[0].hash.as_deref(), Some(hash.as_str()));
    }

    #[tokio::test]
    async fn cache_is_invalidated_by_size_mtime_and_inode() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = write_photo(dir.path(), "a.jpg", b"original");
        let mut cache = FingerprintCache::new("G1", dir.path().to_string_lossy());
        cache.insert(
            "a.jpg",
            record.size,
            record.mtime_ms,
            record.inode,
            record.device,
            "cached",
        );

        // Different size.
        let mut changed = record.clone();
        changed.size += 1;
        let (_, stats) = hash_with_cache(vec![changed], &cache).await;
        assert_eq!(stats.hashed, 1);

        // Different mtime.
        let mut changed = record.clone();
        changed.mtime_ms += 1;
        let (_, stats) = hash_with_cache(vec![changed], &cache).await;
        assert_eq!(stats.hashed, 1);

        // Same size and mtime, but a different file (replaced by a copy).
        let mut changed = record.clone();
        changed.inode = record.inode.map(|inode| inode.wrapping_add(1_000_000));
        let (_, stats) = hash_with_cache(vec![changed], &cache).await;
        assert_eq!(stats.hashed, 1);

        // Back to the untouched record: still cached.
        record.inode = cache.get("a.jpg").unwrap().inode;
        let (_, stats) = hash_with_cache(vec![record], &cache).await;
        assert_eq!(stats.hashed, 0);
    }

    #[tokio::test]
    async fn missing_file_is_an_error_not_a_silent_skip() {
        let dir = tempfile::tempdir().unwrap();
        let record = RawLocalFile {
            filename: "gone.jpg".into(),
            relative_path: "gone.jpg".into(),
            path: dir.path().join("gone.jpg"),
            size: 10,
            mtime_ms: 1,
            inode: None,
            device: None,
        };
        let cache = FingerprintCache::new("G1", dir.path().to_string_lossy());
        let (results, _) =
            hash_local_records_cached(vec![record], &[], Some(&cache), 1, |_, _| {}).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[tokio::test]
    async fn duplicate_raw_is_skipped_only_when_the_preferred_file_is_cached() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = write_photo(dir.path(), "shot.jpg", b"jpeg bytes");
        let raw = write_photo(dir.path(), "shot.dng", b"raw bytes here");
        let mut cache = FingerprintCache::new("G1", dir.path().to_string_lossy());

        // Cold run: both are read so the duplicate warning can compare content.
        let (_, stats) = hash_with_cache(vec![jpg.clone(), raw.clone()], &cache).await;
        assert_eq!(stats.hashed, 2);

        cache.insert(
            "shot.jpg",
            jpg.size,
            jpg.mtime_ms,
            jpg.inode,
            jpg.device,
            hash_bytes(b"jpeg bytes"),
        );
        // Warm run: the JPEG is cached, so the RAW is never read.
        let (files, stats) = hash_with_cache(vec![jpg, raw], &cache).await;
        assert_eq!(stats.hashed, 0);
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.bytes_read, 0);
        let raw_file = files
            .iter()
            .find(|file| file.filename == "shot.dng")
            .unwrap();
        assert!(raw_file.hash.is_none());
    }
}
