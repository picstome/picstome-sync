//! Crash-safe file writes: data goes to a sibling temporary file, is synced,
//! and then renamed over the destination, so readers only ever observe the
//! old or the new contents.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::error::CoreResult;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temporary_path(path: &Path) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tmp");
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("{extension}.{}.{sequence}.tmp", std::process::id()))
}

/// Rename `from` over `to`, replacing it.
///
/// `rename` already replaces atomically on Unix and (via `MoveFileExW`) on
/// Windows; the fallback only covers Windows cases where the destination is
/// read-only or briefly locked by another process (indexers, antivirus).
///
/// The fallback moves the old destination aside instead of deleting it, and
/// puts it back if the second rename fails: a failed replacement must never
/// leave the caller without the previous version.
pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
            ) =>
        {
            let backup = temporary_path(to);
            match fs::rename(to, &backup) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            match fs::rename(from, to) {
                Ok(()) => {
                    let _ = fs::remove_file(&backup);
                    Ok(())
                }
                Err(error) => {
                    if backup.exists() {
                        let _ = fs::rename(&backup, to);
                    }
                    Err(error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

/// Make a completed rename durable by syncing its parent directory.
///
/// `fsync` on a file only guarantees the file's contents; on a crash the
/// directory entry pointing at the new name can still be lost. This is
/// best-effort: some filesystems and Windows do not allow opening a directory,
/// and a failure here must never turn a completed write into an error.
pub fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Write bytes to a file atomically on Unix and Windows. The temporary file is
/// removed by the rename on success; callers never see a partially written file.
pub fn write_bytes(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(path);
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)?;
        sync_parent_dir(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> CoreResult<()> {
    let mut data = serde_json::to_vec_pretty(value)?;
    data.push(b'\n');
    write_bytes(path, &data)?;
    Ok(())
}

/// Same atomic guarantees as [`write_json_atomic`] without pretty printing.
/// Used for caches and snapshots: they can grow to megabytes and are never
/// meant to be read by a human.
pub fn write_json_compact_atomic<T: Serialize>(path: &Path, value: &T) -> CoreResult<()> {
    let mut data = serde_json::to_vec(value)?;
    data.push(b'\n');
    write_bytes(path, &data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_existing_files_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_bytes(&path, b"one").unwrap();
        write_bytes(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn writes_json_with_a_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_json_atomic(&path, &serde_json::json!({"ok": true})).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "{\n  \"ok\": true\n}\n");
    }
}
