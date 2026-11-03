use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::model::{Config, GalleryEntry, Snapshot};
use crate::sanitize::sanitize_gallery_id;

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let contents = fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> CoreResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.{}.tmp",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("json"),
        std::process::id()
    ));
    let data = serde_json::to_vec_pretty(value)?;
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&data)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn normalize_entry(entry: GalleryEntry) -> Option<GalleryEntry> {
    let gallery_id = sanitize_gallery_id(&entry.gallery_id)?;
    let local_path = entry.local_path.trim();
    if local_path.is_empty() {
        return None;
    }
    let name = if entry.name.trim().is_empty() {
        "Gallery".to_string()
    } else {
        entry.name.trim().to_string()
    };
    Some(GalleryEntry {
        gallery_id,
        name,
        local_path: local_path.to_string(),
        last_sync_at: entry.last_sync_at,
        remote_rev: entry.remote_rev,
        parent_gallery_id: entry
            .parent_gallery_id
            .and_then(|id| sanitize_gallery_id(&id)),
    })
}

impl Store {
    pub fn new(root: impl AsRef<Path>) -> CoreResult<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    pub fn snapshot_path(&self, gallery_id: &str) -> CoreResult<PathBuf> {
        let id = sanitize_gallery_id(gallery_id)
            .ok_or_else(|| CoreError::InvalidData("invalid gallery id".into()))?;
        Ok(self.root.join(format!("{id}.json")))
    }

    pub fn load_config(&self) -> Config {
        read_json(&self.config_path()).unwrap_or_default()
    }

    pub fn save_config(&self, config: &Config) -> CoreResult<()> {
        write_json_atomic(&self.config_path(), config)
    }

    pub fn update_config<F>(&self, update: F) -> CoreResult<Config>
    where
        F: FnOnce(&mut Config),
    {
        let mut config = self.load_config();
        update(&mut config);
        self.save_config(&config)?;
        Ok(config)
    }

    pub fn load_index(&self) -> Vec<GalleryEntry> {
        let raw: Vec<GalleryEntry> = read_json(&self.index_path()).unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        raw.into_iter()
            .filter_map(normalize_entry)
            .filter(|entry| seen.insert(entry.gallery_id.clone()))
            .collect()
    }

    pub fn save_index(&self, entries: &[GalleryEntry]) -> CoreResult<Vec<GalleryEntry>> {
        let entries: Vec<GalleryEntry> = entries
            .iter()
            .cloned()
            .filter_map(normalize_entry)
            .collect();
        write_json_atomic(&self.index_path(), &entries)?;
        Ok(entries)
    }

    pub fn get_gallery(&self, gallery_id: &str) -> Option<GalleryEntry> {
        let id = sanitize_gallery_id(gallery_id)?;
        self.load_index()
            .into_iter()
            .find(|entry| entry.gallery_id == id)
    }

    pub fn upsert_gallery(&self, entry: GalleryEntry) -> CoreResult<Option<GalleryEntry>> {
        let Some(normalized) = normalize_entry(entry) else {
            return Ok(None);
        };
        let mut entries = self.load_index();
        entries.retain(|existing| existing.gallery_id != normalized.gallery_id);
        entries.push(normalized.clone());
        self.save_index(&entries)?;
        Ok(Some(normalized))
    }

    pub fn patch_gallery<F>(&self, gallery_id: &str, patch: F) -> CoreResult<Option<GalleryEntry>>
    where
        F: FnOnce(&mut GalleryEntry),
    {
        let Some(id) = sanitize_gallery_id(gallery_id) else {
            return Ok(None);
        };
        let mut entries = self.load_index();
        let Some(index) = entries.iter().position(|entry| entry.gallery_id == id) else {
            return Ok(None);
        };
        patch(&mut entries[index]);
        let Some(normalized) = normalize_entry(entries[index].clone()) else {
            return Ok(None);
        };
        entries[index] = normalized.clone();
        self.save_index(&entries)?;
        Ok(Some(normalized))
    }

    pub fn remove_gallery(&self, gallery_id: &str) -> CoreResult<bool> {
        let Some(id) = sanitize_gallery_id(gallery_id) else {
            return Ok(false);
        };
        let mut entries = self.load_index();
        let before = entries.len();
        entries.retain(|entry| entry.gallery_id != id);
        if entries.len() == before {
            return Ok(false);
        }
        self.save_index(&entries)?;
        Ok(true)
    }

    pub fn load_snapshot(&self, gallery_id: &str) -> Option<Snapshot> {
        let path = self.snapshot_path(gallery_id).ok()?;
        read_json(&path)
    }

    pub fn save_snapshot(&self, snapshot: &Snapshot) -> CoreResult<()> {
        let path = self.snapshot_path(&snapshot.gallery_id)?;
        write_json_atomic(&path, snapshot)
    }

    pub fn delete_snapshot(&self, gallery_id: &str) -> CoreResult<bool> {
        let path = self.snapshot_path(gallery_id)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn saves_and_loads_config() {
        let (_dir, store) = temp_store();
        store
            .save_config(&Config {
                last_rating: Some("3".into()),
                api_base: Some("http://localhost:1".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(store.load_config().last_rating.as_deref(), Some("3"));
    }

    #[test]
    fn upserts_patches_and_removes_galleries() {
        let (_dir, store) = temp_store();
        store
            .upsert_gallery(GalleryEntry {
                gallery_id: "G123".into(),
                name: "MiBoda".into(),
                local_path: "/tmp/a".into(),
                last_sync_at: None,
                remote_rev: None,
                parent_gallery_id: None,
            })
            .unwrap();
        store
            .upsert_gallery(GalleryEntry {
                gallery_id: "G123".into(),
                name: "MiBoda 2".into(),
                local_path: "/tmp/a".into(),
                last_sync_at: None,
                remote_rev: None,
                parent_gallery_id: None,
            })
            .unwrap();
        assert_eq!(store.load_index().len(), 1);
        assert_eq!(store.get_gallery("G123").unwrap().name, "MiBoda 2");

        store
            .patch_gallery("G123", |entry| entry.last_sync_at = Some(123))
            .unwrap();
        assert_eq!(store.get_gallery("G123").unwrap().last_sync_at, Some(123));
        assert!(store.remove_gallery("G123").unwrap());
        assert!(store.load_index().is_empty());
    }

    #[test]
    fn rejects_unsafe_ids() {
        let (_dir, store) = temp_store();
        assert!(store
            .upsert_gallery(GalleryEntry {
                gallery_id: "../x".into(),
                name: "x".into(),
                local_path: "/tmp/a".into(),
                last_sync_at: None,
                remote_rev: None,
                parent_gallery_id: None,
            })
            .unwrap()
            .is_none());
        assert!(store.snapshot_path("../x").is_err());
        assert!(store.get_gallery("../x").is_none());
    }

    #[test]
    fn snapshots_roundtrip() {
        let (_dir, store) = temp_store();
        let snapshot = Snapshot {
            gallery_id: "G123".into(),
            name: "MiBoda".into(),
            local_path: "/tmp/a".into(),
            last_sync_at: 1,
            remote_rev: None,
            local_photo_count: 0,
            local_bytes: 0,
            files: vec![],
        };
        store.save_snapshot(&snapshot).unwrap();
        assert_eq!(store.load_snapshot("G123").unwrap().last_sync_at, 1);
        assert!(store.delete_snapshot("G123").unwrap());
        assert!(store.load_snapshot("G123").is_none());
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    fn entry(id: &str, name: &str, path: &str) -> GalleryEntry {
        GalleryEntry {
            gallery_id: id.into(),
            name: name.into(),
            local_path: path.into(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
        }
    }

    #[test]
    fn exposes_paths_and_updates_config() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        assert_eq!(store.root(), dir.path());
        assert_eq!(store.config_path(), dir.path().join("config.json"));
        assert_eq!(store.index_path(), dir.path().join("index.json"));
        let cfg = store
            .update_config(|config| config.last_rating = Some("5".into()))
            .unwrap();
        assert_eq!(cfg.last_rating.as_deref(), Some("5"));
    }

    #[test]
    fn normalizes_entries_and_rejects_invalid_ones() {
        let normalized = normalize_entry(entry("G1", "  ", "/tmp/a")).unwrap();
        assert_eq!(normalized.name, "Gallery");
        assert!(normalize_entry(entry("../bad", "x", "/tmp/a")).is_none());
        assert!(normalize_entry(entry("G1", "x", "   ")).is_none());
        let mut parent = entry("G1", "x", "/tmp/a");
        parent.parent_gallery_id = Some("../bad".into());
        assert!(normalize_entry(parent).unwrap().parent_gallery_id.is_none());
    }

    #[test]
    fn invalid_json_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        std::fs::write(store.config_path(), "{ broken").unwrap();
        std::fs::write(store.index_path(), "[] trailing").unwrap();
        assert_eq!(store.load_config().last_rating, None);
        assert!(store.load_index().is_empty());
        std::fs::write(store.index_path(), "not an array").unwrap();
        assert!(store.load_index().is_empty());
    }

    #[test]
    fn filters_duplicate_and_invalid_index_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let raw = vec![
            entry("G1", "one", "/tmp/a"),
            entry("G1", "duplicate", "/tmp/b"),
            entry("../bad", "bad", "/tmp/c"),
        ];
        std::fs::write(store.index_path(), serde_json::to_vec(&raw).unwrap()).unwrap();
        let entries = store.load_index();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "one");

        let saved = store
            .save_index(&[
                entry("G2", "two", "/tmp/x"),
                entry("../bad", "bad", "/tmp/y"),
            ])
            .unwrap();
        assert_eq!(saved.len(), 1);
    }

    #[test]
    fn patch_and_remove_handle_missing_or_invalid_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        assert!(store.patch_gallery("../bad", |_| {}).unwrap().is_none());
        assert!(store.patch_gallery("missing", |_| {}).unwrap().is_none());
        assert!(!store.remove_gallery("../bad").unwrap());
        assert!(!store.remove_gallery("missing").unwrap());

        store.upsert_gallery(entry("G1", "one", "/tmp/a")).unwrap();
        assert!(store
            .patch_gallery("G1", |entry| entry.local_path.clear())
            .unwrap()
            .is_none());
    }

    #[test]
    fn snapshot_helpers_validate_ids_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        assert!(store.load_snapshot("../bad").is_none());
        assert!(store.delete_snapshot("../bad").is_err());
        assert!(!store.delete_snapshot("G1").unwrap());
        let bad = Snapshot {
            gallery_id: "../bad".into(),
            name: "x".into(),
            local_path: "/tmp".into(),
            last_sync_at: 0,
            remote_rev: None,
            local_photo_count: 0,
            local_bytes: 0,
            files: vec![],
        };
        assert!(store.save_snapshot(&bad).is_err());
    }

    #[test]
    fn store_creation_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        let store = Store::new(&nested).unwrap();
        assert!(nested.is_dir());
        Store::new(&nested).unwrap();
        assert_eq!(store.root(), nested);
    }
}
