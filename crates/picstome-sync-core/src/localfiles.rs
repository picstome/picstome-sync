use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use walkdir::WalkDir;

use crate::error::{CoreError, CoreResult};
use crate::identity::{identity_key, is_photo_file, preferred_file};
use crate::model::{LocalFile, SnapshotFile};

pub const MAX_PHOTO_BYTES: u64 = 500 * 1024 * 1024;
pub const EDITED_FOLDER_NAMES: &[&str] =
    &["editadas", "editada", "edits", "edited", "edit", "finales"];

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

        files.push(RawLocalFile {
            filename,
            relative_path,
            path: entry.path().to_path_buf(),
            size: metadata.len(),
            mtime_ms,
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

pub async fn hash_file(path: &Path) -> CoreResult<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn cache_key(file: &SnapshotFile) -> String {
    file.relative_path
        .clone()
        .unwrap_or_else(|| file.filename.clone())
}

fn build_cache(snapshot_files: &[SnapshotFile]) -> HashMap<String, SnapshotFile> {
    let mut cache = HashMap::new();
    for file in snapshot_files {
        if file.local_present && file.local_hash.is_some() {
            cache.entry(cache_key(file)).or_insert_with(|| file.clone());
        }
    }
    cache
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
    let cache = build_cache(snapshot_files);
    let total = records.len() as u64;
    let on_progress = std::sync::Arc::new(on_progress);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // Cached hashes need no I/O, so count them before the stream starts.
    let cached = records
        .iter()
        .filter(|record| {
            cache
                .get(&record.relative_path)
                .filter(|file| {
                    file.local_hash.is_some()
                        && file.local_size == Some(record.size)
                        && file.mtime == Some(record.mtime_ms)
                })
                .is_some()
        })
        .count() as u64;
    done.fetch_add(cached, std::sync::atomic::Ordering::Relaxed);
    on_progress(done.load(std::sync::atomic::Ordering::Relaxed), total);

    let stream = futures::stream::iter(records.into_iter().map(|record| {
        let cache = &cache;
        let done = done.clone();
        let on_progress = on_progress.clone();
        async move {
            let cached = cache.get(&record.relative_path);
            let cached_hash = cached
                .filter(|file| {
                    file.local_hash.is_some()
                        && file.local_size == Some(record.size)
                        && file.mtime == Some(record.mtime_ms)
                })
                .and_then(|file| file.local_hash.clone());

            let hash = match cached_hash {
                Some(hash) => hash,
                None => match hash_file(&record.path).await {
                    Ok(hash) => {
                        let finished = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        on_progress(finished, total);
                        hash
                    }
                    Err(err) => {
                        return Err(ScanError {
                            path: record.path.display().to_string(),
                            error: err.to_string(),
                        });
                    }
                },
            };

            let identity_key = identity_key(&record.filename).unwrap_or_default();
            Ok(LocalFile {
                filename: record.filename,
                relative_path: record.relative_path,
                path: record.path,
                size: record.size,
                mtime_ms: record.mtime_ms,
                hash: Some(hash),
                identity_key,
            })
        }
    }))
    .buffer_unordered(concurrency.max(1));

    stream.collect().await
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
    let results = hash_local_records_with(records, snapshot_files, concurrency, on_hash).await;

    let mut files = Vec::with_capacity(results.len());
    for result in results {
        match result {
            Ok(file) => files.push(file),
            Err(err) => errors.push(err),
        }
    }
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok((files, errors))
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
