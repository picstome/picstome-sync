use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use walkdir::WalkDir;

use crate::error::{CoreError, CoreResult};
use crate::identity::{identity_key, is_photo_file};
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
    let root = crate::sanitize::lexical_absolute(dir);
    let mut files = Vec::new();
    let mut errors = Vec::new();

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

    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    (files, errors)
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
        if file.hash.is_some() {
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
    let cache = build_cache(snapshot_files);
    let stream = futures::stream::iter(records.into_iter().map(|record| {
        let cache = &cache;
        async move {
            let cached = cache.get(&record.relative_path);
            let cached_hash = cached
                .filter(|file| {
                    file.hash.is_some()
                        && file.size == Some(record.size)
                        && file.mtime == Some(record.mtime_ms)
                })
                .and_then(|file| file.hash.clone());

            let hash = match cached_hash {
                Some(hash) => hash,
                None => match hash_file(&record.path).await {
                    Ok(hash) => hash,
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
    let metadata = tokio::fs::metadata(dir).await?;
    if !metadata.is_dir() {
        return Err(CoreError::InvalidData(format!(
            "not a directory: {}",
            dir.display()
        )));
    }

    let (records, mut errors) = scan_local_photos(dir, 8, true, MAX_PHOTO_BYTES);
    let results = hash_local_records(records, snapshot_files, concurrency).await;

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
            version_count: Some(1),
            version_number: Some(1),
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
    async fn scans_files_in_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z.jpg"), b"z").unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"a").unwrap();
        let (files, errors) = scan_and_hash(dir.path(), &[], 2).await.unwrap();
        assert!(errors.is_empty());
        let names: Vec<&str> = files.iter().map(|file| file.filename.as_str()).collect();
        assert_eq!(names, vec!["a.jpg", "z.jpg"]);
    }
}
