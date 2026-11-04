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

fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
            ) =>
        {
            match fs::remove_file(to) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::rename(from, to)
        }
        Err(error) => Err(error),
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
        replace_file(&temporary, path)
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
