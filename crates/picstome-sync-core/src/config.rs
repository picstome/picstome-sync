use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::de::DeserializeOwned;

use crate::atomic::{write_json_atomic, write_json_compact_atomic};
use crate::cache::FingerprintCache;
use crate::error::{CoreError, CoreResult};
use crate::model::{Config, GalleryEntry, Snapshot, SNAPSHOT_SCHEMA};
use crate::sanitize::sanitize_gallery_id;

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let contents = fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Parsed snapshots kept in memory so `get_state`, favourites and rating plans
/// do not re-parse megabytes of JSON on every UI refresh.
///
/// Entries are validated against the file's modification time and size on
/// every read, so a snapshot written by another process (or by an older build)
/// is never served stale.
struct CachedSnapshot {
    modified_nanos: u128,
    len: u64,
    snapshot: Snapshot,
}

fn snapshot_cache() -> &'static Mutex<std::collections::HashMap<PathBuf, CachedSnapshot>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, CachedSnapshot>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn file_signature(metadata: &fs::Metadata) -> (u128, u64) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    (modified, metadata.len())
}

fn cached_snapshot(path: &Path) -> Option<Snapshot> {
    let metadata = fs::metadata(path).ok()?;
    let signature = file_signature(&metadata);
    let cache = snapshot_cache().lock().ok()?;
    let entry = cache.get(path)?;
    if entry.modified_nanos == signature.0 && entry.len == signature.1 {
        // Only a snapshot written by exactly this schema is served straight
        // from memory; anything else goes through the normal read path, which
        // applies the schema rules.
        if entry.snapshot.schema != SNAPSHOT_SCHEMA {
            return None;
        }
        return Some(entry.snapshot.clone());
    }
    None
}

fn cache_snapshot(path: &Path, snapshot: &Snapshot) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if snapshot.schema != SNAPSHOT_SCHEMA {
        forget_snapshot(path);
        return;
    }
    let signature = file_signature(&metadata);
    if let Ok(mut cache) = snapshot_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            CachedSnapshot {
                modified_nanos: signature.0,
                len: signature.1,
                snapshot: snapshot.clone(),
            },
        );
    }
}

fn forget_snapshot(path: &Path) {
    if let Ok(mut cache) = snapshot_cache().lock() {
        cache.remove(path);
    }
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

    /// Location of the per-gallery fingerprint cache. Kept apart from the
    /// snapshot so it also survives a snapshot reset or a re-link.
    pub fn fingerprint_path(&self, gallery_id: &str) -> CoreResult<PathBuf> {
        let id = sanitize_gallery_id(gallery_id)
            .ok_or_else(|| CoreError::InvalidData("invalid gallery id".into()))?;
        Ok(self
            .root
            .join("cache")
            .join("fingerprints")
            .join(format!("{id}.json")))
    }

    pub fn load_fingerprints(&self, gallery_id: &str, local_path: &str) -> FingerprintCache {
        let Some(path) = self.fingerprint_path(gallery_id).ok() else {
            return FingerprintCache::new(gallery_id, local_path);
        };
        FingerprintCache::load(&path)
            .filter(|cache| cache.usable_for(gallery_id, local_path))
            .unwrap_or_else(|| FingerprintCache::new(gallery_id, local_path))
    }

    pub fn save_fingerprints(&self, cache: &FingerprintCache) -> CoreResult<()> {
        let path = self.fingerprint_path(&cache.gallery_id)?;
        cache.save(&path)
    }

    pub fn delete_fingerprints(&self, gallery_id: &str) -> CoreResult<bool> {
        let path = self.fingerprint_path(gallery_id)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err.into()),
        }
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
        if let Some(cached) = cached_snapshot(&path) {
            return Some(cached);
        }
        let mut snapshot: Snapshot = read_json(&path)?;
        if snapshot.schema == 0 {
            return None;
        }
        if snapshot.schema > SNAPSHOT_SCHEMA {
            return None;
        }
        if snapshot.schema < SNAPSHOT_SCHEMA {
            // Upgrading in place keeps the diff baseline, so adding rating
            // fields never makes the app ask for a first-sync preview again.
            snapshot.schema = SNAPSHOT_SCHEMA;
            let _ = self.save_snapshot(&snapshot);
        }
        cache_snapshot(&path, &snapshot);
        Some(snapshot)
    }
    pub fn save_snapshot(&self, snapshot: &Snapshot) -> CoreResult<()> {
        let path = self.snapshot_path(&snapshot.gallery_id)?;
        write_json_compact_atomic(&path, snapshot)?;
        cache_snapshot(&path, snapshot);
        Ok(())
    }

    pub fn delete_snapshot(&self, gallery_id: &str) -> CoreResult<bool> {
        let path = self.snapshot_path(gallery_id)?;
        forget_snapshot(&path);
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
            schema: SNAPSHOT_SCHEMA,
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
    fn older_snapshot_schema_is_upgraded_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let snapshot = Snapshot {
            schema: SNAPSHOT_SCHEMA - 1,
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
        let loaded = store.load_snapshot("G123").expect("upgraded snapshot");
        assert_eq!(loaded.schema, SNAPSHOT_SCHEMA);
        assert_eq!(loaded.name, "MiBoda");
    }

    #[test]
    fn newer_snapshot_schema_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let snapshot = Snapshot {
            schema: SNAPSHOT_SCHEMA + 1,
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
        assert!(store.load_snapshot("G123").is_none());
    }

    #[test]
    fn snapshot_helpers_validate_ids_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        assert!(store.load_snapshot("../bad").is_none());
        assert!(store.delete_snapshot("../bad").is_err());
        assert!(!store.delete_snapshot("G1").unwrap());
        let bad = Snapshot {
            schema: SNAPSHOT_SCHEMA,
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
