//! Persistent local fingerprint cache.
//!
//! The sync snapshot stores one entry per logical photo identity, which means a
//! RAW next to its JPEG never had a cached hash and was re-read on every sync.
//! This cache is keyed by relative path instead, so every file the walk sees is
//! covered, and a repeat sync can resolve hashes from metadata alone.
//!
//! Safety rules (the cache only ever accelerates, never decides):
//! * an entry is only used when size, mtime and (when both known) inode/device
//!   still match the file on disk;
//! * a missing, unreadable, corrupt or older-schema cache behaves exactly like
//!   no cache: the files are hashed again;
//! * entries only ever describe what was seen during a scan, and are pruned to
//!   the current walk.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::atomic::write_json_compact_atomic;
use crate::error::CoreResult;

pub const FINGERPRINT_SCHEMA: u32 = 1;
pub const HASH_ALGO: &str = "sha256";

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// What we know about one file on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub size: u64,
    pub mtime_ms: i64,
    /// Inode and device on Unix. `None` on platforms without them, or for
    /// entries created from a transfer instead of a directory walk.
    #[serde(default)]
    pub inode: Option<u64>,
    #[serde(default)]
    pub device: Option<u64>,
    pub hash: String,
    #[serde(default = "default_algo")]
    pub algo: String,
    #[serde(default)]
    pub checked_at: i64,
}

fn default_algo() -> String {
    HASH_ALGO.to_string()
}

impl Fingerprint {
    /// Whether this entry still describes the file on disk.
    ///
    /// Size and mtime must always match. Inode/device are compared only when
    /// both sides know them, so a fingerprint created from a finished download
    /// (no inode yet) is still usable, but an entry recorded during a walk is
    /// invalidated as soon as the path points at a different file.
    pub fn matches(
        &self,
        size: u64,
        mtime_ms: i64,
        inode: Option<u64>,
        device: Option<u64>,
    ) -> bool {
        if self.algo != HASH_ALGO || self.size != size || self.mtime_ms != mtime_ms {
            return false;
        }
        if let (Some(cached), Some(current)) = (self.inode, inode) {
            if cached != current {
                return false;
            }
        }
        if let (Some(cached), Some(current)) = (self.device, device) {
            if cached != current {
                return false;
            }
        }
        true
    }
}

/// `relative path -> fingerprint` for one gallery folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintCache {
    #[serde(default)]
    pub schema: u32,
    #[serde(default)]
    pub gallery_id: String,
    #[serde(default)]
    pub local_path: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub entries: BTreeMap<String, Fingerprint>,
}

impl FingerprintCache {
    pub fn new(gallery_id: impl Into<String>, local_path: impl Into<String>) -> Self {
        Self {
            schema: FINGERPRINT_SCHEMA,
            gallery_id: gallery_id.into(),
            local_path: local_path.into(),
            updated_at: now_millis(),
            entries: BTreeMap::new(),
        }
    }

    /// A cache is only usable for the same folder and the schema this build
    /// understands. Anything else means "start from scratch".
    pub fn usable_for(&self, gallery_id: &str, local_path: &str) -> bool {
        self.schema == FINGERPRINT_SCHEMA
            && self.gallery_id == gallery_id
            && self.local_path == local_path
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, relative_path: &str) -> Option<&Fingerprint> {
        self.entries.get(relative_path)
    }

    /// Cached hash for a file, only when the metadata still matches.
    pub fn lookup(
        &self,
        relative_path: &str,
        size: u64,
        mtime_ms: i64,
        inode: Option<u64>,
        device: Option<u64>,
    ) -> Option<&str> {
        self.entries
            .get(relative_path)
            .filter(|entry| entry.matches(size, mtime_ms, inode, device))
            .map(|entry| entry.hash.as_str())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &mut self,
        relative_path: impl Into<String>,
        size: u64,
        mtime_ms: i64,
        inode: Option<u64>,
        device: Option<u64>,
        hash: impl Into<String>,
    ) {
        self.entries.insert(
            relative_path.into(),
            Fingerprint {
                size,
                mtime_ms,
                inode,
                device,
                hash: hash.into(),
                algo: HASH_ALGO.to_string(),
                checked_at: now_millis(),
            },
        );
    }

    /// Keep only entries whose path is still part of the scan.
    pub fn retain_paths(&mut self, keep: &std::collections::HashSet<String>) {
        self.entries.retain(|path, _| keep.contains(path));
    }

    pub fn load(path: &Path) -> Option<Self> {
        let contents = fs::read_to_string(path).ok()?;
        let mut cache: Self = serde_json::from_str(&contents).ok()?;
        if cache.schema != FINGERPRINT_SCHEMA {
            return None;
        }
        // Drop entries written by another hash algorithm instead of trusting
        // them.
        cache.entries.retain(|_, entry| entry.algo == HASH_ALGO);
        Some(cache)
    }

    pub fn save(&self, path: &Path) -> CoreResult<()> {
        let mut cache = self.clone();
        cache.schema = FINGERPRINT_SCHEMA;
        cache.updated_at = now_millis();
        write_json_compact_atomic(path, &cache)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(size: u64, mtime: i64, hash: &str) -> Fingerprint {
        Fingerprint {
            size,
            mtime_ms: mtime,
            inode: Some(7),
            device: Some(1),
            hash: hash.into(),
            algo: HASH_ALGO.into(),
            checked_at: 0,
        }
    }

    #[test]
    fn matches_only_identical_metadata() {
        let cached = entry(10, 100, "h1");
        assert!(cached.matches(10, 100, Some(7), Some(1)));
        assert!(!cached.matches(11, 100, Some(7), Some(1)));
        assert!(!cached.matches(10, 101, Some(7), Some(1)));
        assert!(!cached.matches(10, 100, Some(8), Some(1)));
        assert!(!cached.matches(10, 100, Some(7), Some(2)));
        // Unknown inode on one side falls back to size + mtime.
        assert!(cached.matches(10, 100, None, None));
        let incomplete = Fingerprint {
            inode: None,
            device: None,
            ..cached.clone()
        };
        assert!(incomplete.matches(10, 100, Some(9), Some(5)));
    }

    #[test]
    fn rejects_unknown_algorithms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = FingerprintCache::new("G1", "/tmp/g");
        cache.insert("a.jpg", 10, 100, None, None, "h1");
        cache.entries.get_mut("a.jpg").unwrap().algo = "md5".into();
        cache.save(&path).unwrap();

        let loaded = FingerprintCache::load(&path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn roundtrips_and_rejects_other_schemata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = FingerprintCache::new("G1", "/tmp/g");
        cache.insert("a.jpg", 10, 100, Some(3), Some(4), "h1");
        cache.save(&path).unwrap();

        let loaded = FingerprintCache::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded.lookup("a.jpg", 10, 100, Some(3), Some(4)),
            Some("h1")
        );
        assert!(loaded.usable_for("G1", "/tmp/g"));
        assert!(!loaded.usable_for("G1", "/tmp/other"));
        assert!(!loaded.usable_for("G2", "/tmp/g"));

        let text = fs::read_to_string(&path).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["schema"] = serde_json::json!(FINGERPRINT_SCHEMA + 1);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(FingerprintCache::load(&path).is_none());
    }

    #[test]
    fn corrupt_cache_is_ignored_and_pruning_drops_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(FingerprintCache::load(&path).is_none());
        assert!(FingerprintCache::load(&dir.path().join("absent.json")).is_none());

        let mut cache = FingerprintCache::new("G1", "/tmp/g");
        cache.insert("a.jpg", 1, 1, None, None, "h");
        cache.insert("b.jpg", 1, 1, None, None, "h");
        cache.retain_paths(&["b.jpg".to_string()].into_iter().collect());
        assert_eq!(cache.len(), 1);
        assert!(cache.get("b.jpg").is_some());
    }
}
