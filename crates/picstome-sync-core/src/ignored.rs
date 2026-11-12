use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic::write_json_atomic;
use crate::error::CoreResult;
use crate::identity::identity_key;
use crate::model::{LocalFile, Photo};

pub const IGNORE_FILE_NAME: &str = ".picstome-ignore.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredLocal {
    pub relative_path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredRemote {
    pub id: String,
    #[serde(default)]
    pub version_count: Option<u64>,
}

/// Per-gallery choices made during the first-sync preview. Entries are only
/// honoured while the local bytes or the remote version stay unchanged, so a
/// later edit automatically puts the item back into the sync candidate list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryIgnores {
    #[serde(default)]
    pub local: Vec<IgnoredLocal>,
    #[serde(default)]
    pub remote: Vec<IgnoredRemote>,
}

impl GalleryIgnores {
    pub fn path(root: &Path) -> PathBuf {
        root.join(IGNORE_FILE_NAME)
    }

    pub fn load(root: &Path) -> Self {
        fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> CoreResult<()> {
        if self.local.is_empty() && self.remote.is_empty() {
            return Self::clear(root);
        }
        write_json_atomic(&Self::path(root), self)
    }

    pub fn clear(root: &Path) -> CoreResult<()> {
        match fs::remove_file(Self::path(root)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.local.is_empty() && self.remote.is_empty()
    }

    pub fn ignores_local(&self, file: &LocalFile) -> bool {
        self.local.iter().any(|entry| {
            entry.relative_path == file.relative_path
                && entry.size == file.size
                && entry.mtime_ms == file.mtime_ms
        })
    }

    pub fn ignores_remote(&self, photo: &Photo) -> bool {
        self.remote
            .iter()
            .any(|entry| entry.id == photo.id && entry.version_count == photo.version_count)
    }

    pub fn add_local(&mut self, file: &LocalFile) {
        let entry = IgnoredLocal {
            relative_path: file.relative_path.clone(),
            size: file.size,
            mtime_ms: file.mtime_ms,
        };
        if !self.local.contains(&entry) {
            self.local.push(entry);
        }
    }

    pub fn add_remote(&mut self, photo: &Photo) {
        let entry = IgnoredRemote {
            id: photo.id.clone(),
            version_count: photo.version_count,
        };
        if !self.remote.contains(&entry) {
            self.remote.push(entry);
        }
    }

    /// If an ignored local file and a remote photo now share an identity, stop
    /// ignoring the local side so a download cannot overwrite the local copy.
    /// The pair is adopted as in-sync instead.
    pub fn release_cross_side(&mut self, local: &[LocalFile], remote: &[Photo]) -> bool {
        let remote_keys: HashSet<String> =
            remote.iter().filter_map(|photo| photo.identity()).collect();
        let local_keys: HashSet<String> = local.iter().filter_map(|file| file.identity()).collect();
        let before = (self.local.len(), self.remote.len());
        self.local.retain(|entry| {
            let name = Path::new(&entry.relative_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&entry.relative_path);
            identity_key(name)
                .map(|key| !remote_keys.contains(&key))
                .unwrap_or(true)
        });
        self.remote.retain(|entry| {
            remote
                .iter()
                .find(|photo| photo.id == entry.id)
                .and_then(|photo| photo.identity())
                .map(|key| !local_keys.contains(&key))
                .unwrap_or(true)
        });
        before != (self.local.len(), self.remote.len())
    }

    /// Drop entries whose file or remote photo changed or disappeared. This is
    /// what makes "ignored" temporary: editing the file or receiving a new
    /// remote version makes the item syncable again.
    pub fn retain_current(&mut self, local: &[LocalFile], remote: &[Photo]) -> bool {
        let before = (self.local.len(), self.remote.len());
        self.local.retain(|entry| {
            local.iter().any(|file| {
                entry.relative_path == file.relative_path
                    && entry.size == file.size
                    && entry.mtime_ms == file.mtime_ms
            })
        });
        self.remote.retain(|entry| {
            remote
                .iter()
                .any(|photo| entry.id == photo.id && entry.version_count == photo.version_count)
        });
        before != (self.local.len(), self.remote.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LocalFile;

    fn local(path: &str, size: u64, mtime_ms: i64) -> LocalFile {
        LocalFile {
            filename: path.rsplit('/').next().unwrap_or(path).to_string(),
            relative_path: path.to_string(),
            path: PathBuf::from(format!("/tmp/gallery/{path}")),
            size,
            mtime_ms,
            hash: Some("hash".into()),
            identity_key: "image:test".into(),
        }
    }

    fn remote(id: &str, version_count: u64) -> Photo {
        Photo {
            id: id.into(),
            filename: "a.jpg".into(),
            size: Some(10),
            hash: None,
            download_hash: None,
            hash_algo: None,
            url: String::new(),
            updated_at: None,
            version_count: Some(version_count),
            version_number: Some(version_count),
            identity_key: None,
            favorited_at: None,
            rating: None,
            is_favorite: None,
            archived_at: None,
        }
    }

    #[test]
    fn saves_and_loads_ignores() {
        let dir = tempfile::tempdir().unwrap();
        let mut ignores = GalleryIgnores::default();
        ignores.add_local(&local("Shoot/a.jpg", 10, 100));
        ignores.add_remote(&remote("p1", 2));
        ignores.save(dir.path()).unwrap();

        let loaded = GalleryIgnores::load(dir.path());
        assert!(loaded.ignores_local(&local("Shoot/a.jpg", 10, 100)));
        assert!(loaded.ignores_remote(&remote("p1", 2)));
        assert!(!loaded.ignores_local(&local("Shoot/a.jpg", 11, 100)));
        assert!(!loaded.ignores_remote(&remote("p1", 3)));
    }

    #[test]
    fn changed_items_are_released_and_missing_ones_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let mut ignores = GalleryIgnores::default();
        ignores.add_local(&local("a.jpg", 10, 100));
        ignores.add_local(&local("gone.jpg", 10, 100));
        ignores.add_remote(&remote("p1", 1));
        ignores.add_remote(&remote("gone", 1));
        ignores.save(dir.path()).unwrap();

        let changed = ignores.retain_current(
            &[local("a.jpg", 20, 200), local("new.jpg", 10, 100)],
            &[remote("p1", 1)],
        );
        assert!(changed);
        assert!(ignores.ignores_remote(&remote("p1", 1)));
        assert!(!ignores.ignores_local(&local("a.jpg", 20, 200)));
        assert!(ignores
            .local
            .iter()
            .all(|entry| entry.relative_path != "gone.jpg"));
        assert!(ignores.remote.iter().all(|entry| entry.id != "gone"));
    }

    #[test]
    fn cross_side_matches_release_the_ignore_to_protect_local_copies() {
        let dir = tempfile::tempdir().unwrap();
        let mut ignores = GalleryIgnores::default();
        ignores.add_local(&local("Shoot/a.jpg", 10, 100));
        ignores.add_remote(&remote("p2", 1));
        let remote_a = Photo {
            filename: "a.jpg".into(),
            ..remote("p1", 1)
        };
        let local_b = local("Shoot/b.jpg", 10, 100);
        assert!(ignores.release_cross_side(&[local_b], &[remote_a]));
        assert!(ignores.local.is_empty());
        assert_eq!(ignores.remote.len(), 1);
        assert!(ignores.save(dir.path()).is_ok());
    }

    #[test]
    fn clearing_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut ignores = GalleryIgnores::default();
        ignores.add_local(&local("a.jpg", 10, 100));
        ignores.save(dir.path()).unwrap();
        assert!(GalleryIgnores::path(dir.path()).is_file());
        GalleryIgnores::clear(dir.path()).unwrap();
        assert!(!GalleryIgnores::path(dir.path()).exists());
        assert!(GalleryIgnores::load(dir.path()).is_empty());
    }
}
