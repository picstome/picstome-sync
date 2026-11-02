use std::collections::{HashMap, HashSet};

use crate::identity::{identity_key, preferred_file};
use crate::model::{Conflict, ConflictPhoto, Diff, DiffCounts, LocalFile, Photo, SnapshotFile};

fn photo_key(photo: &Photo) -> Option<String> {
    photo
        .identity_key
        .clone()
        .or_else(|| identity_key(&photo.filename))
}

fn snapshot_key(file: &SnapshotFile) -> Option<String> {
    file.identity_key
        .clone()
        .or_else(|| identity_key(&file.filename))
}

fn local_key(file: &LocalFile) -> Option<String> {
    if file.identity_key.is_empty() {
        identity_key(&file.filename)
    } else {
        Some(file.identity_key.clone())
    }
}

fn content_equal(
    a_hash: Option<&str>,
    a_size: Option<u64>,
    b_hash: Option<&str>,
    b_size: Option<u64>,
) -> bool {
    match (a_hash, b_hash) {
        (Some(a), Some(b)) => a == b,
        _ => match (a_size, b_size) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
    }
}

fn remote_matches_base(photo: &Photo, base: &SnapshotFile) -> bool {
    content_equal(
        photo.hash.as_deref(),
        photo.size,
        base.hash.as_deref(),
        base.size,
    )
}

fn local_matches_base(file: &LocalFile, base: &SnapshotFile) -> bool {
    content_equal(
        file.hash.as_deref(),
        Some(file.size),
        base.hash.as_deref(),
        base.size,
    )
}

fn remote_changed(photo: &Photo, base: &SnapshotFile) -> bool {
    if !remote_matches_base(photo, base) {
        return true;
    }
    match (photo.version_count, base.version_count) {
        (Some(current), Some(previous)) => current != previous,
        _ => false,
    }
}

pub fn compute_diff(
    snapshot_files: &[SnapshotFile],
    remote_photos: &[Photo],
    local_files: &[LocalFile],
) -> Diff {
    let mut previous: HashMap<String, &SnapshotFile> = HashMap::new();
    for file in snapshot_files {
        if let Some(key) = snapshot_key(file) {
            previous.entry(key).or_insert(file);
        }
    }

    let mut remote: HashMap<String, &Photo> = HashMap::new();
    for photo in remote_photos {
        if let Some(key) = photo_key(photo) {
            remote.entry(key).or_insert(photo);
        }
    }

    let mut local_groups: HashMap<String, Vec<&LocalFile>> = HashMap::new();
    for file in local_files {
        if let Some(key) = local_key(file) {
            local_groups.entry(key).or_default().push(file);
        }
    }

    let mut local: HashMap<String, &LocalFile> = HashMap::new();
    for (key, group) in &local_groups {
        if let Some(chosen) = preferred_file(group, |file| file.filename.as_str()) {
            local.insert(key.clone(), chosen);
        }
    }

    let keys: HashSet<String> = previous
        .keys()
        .chain(remote.keys())
        .chain(local.keys())
        .cloned()
        .collect();

    let mut diff = Diff::default();

    for key in keys {
        let base = previous.get(&key).copied();
        let remote_photo = remote.get(&key).copied();
        let local_file = local.get(&key).copied();

        match (remote_photo, local_file) {
            (None, None) => continue,
            (Some(photo), None) => {
                if base.is_some() {
                    diff.local_deleted.push(photo.clone());
                } else {
                    diff.remote_new.push(photo.clone());
                }
            }
            (None, Some(file)) => {
                if base.is_some() {
                    diff.remote_deleted.push(file.clone());
                } else {
                    diff.local_new.push(file.clone());
                }
            }
            (Some(photo), Some(file)) => {
                let version_changed =
                    match (photo.version_count, base.and_then(|b| b.version_count)) {
                        (Some(current), Some(previous)) => current != previous,
                        _ => false,
                    };

                if !version_changed
                    && content_equal(
                        photo.hash.as_deref(),
                        photo.size,
                        file.hash.as_deref(),
                        Some(file.size),
                    )
                {
                    diff.in_sync.push(photo.clone());
                    continue;
                }

                let remote_was_changed =
                    base.map(|base| remote_changed(photo, base)).unwrap_or(true);
                let local_was_changed = base
                    .map(|base| !local_matches_base(file, base))
                    .unwrap_or(true);

                if remote_was_changed && local_was_changed {
                    diff.conflicts.push(Conflict {
                        identity_key: key.clone(),
                        remote: ConflictPhoto {
                            filename: photo.filename.clone(),
                            version_count: photo.version_count,
                        },
                        local: ConflictPhoto {
                            filename: file.filename.clone(),
                            version_count: base.and_then(|b| b.version_count),
                        },
                    });
                } else if remote_was_changed {
                    diff.remote_modified.push(photo.clone());
                } else {
                    diff.local_modified.push(file.clone());
                }
            }
        }
    }

    diff.remote_new.sort_by(|a, b| a.filename.cmp(&b.filename));
    diff.remote_modified
        .sort_by(|a, b| a.filename.cmp(&b.filename));
    diff.local_new
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    diff.local_modified
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    diff.local_deleted
        .sort_by(|a, b| a.filename.cmp(&b.filename));
    diff.remote_deleted
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    diff.conflicts
        .sort_by(|a, b| a.identity_key.cmp(&b.identity_key));
    diff.in_sync.sort_by(|a, b| a.filename.cmp(&b.filename));

    diff.counts = DiffCounts {
        remote_new: diff.remote_new.len(),
        remote_modified: diff.remote_modified.len(),
        local_new: diff.local_new.len(),
        local_modified: diff.local_modified.len(),
        local_deleted: diff.local_deleted.len(),
        remote_deleted: diff.remote_deleted.len(),
        conflicts: diff.conflicts.len(),
        in_sync: diff.in_sync.len(),
    };

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(id: &str, filename: &str, hash: &str, version: u64) -> Photo {
        Photo {
            id: id.into(),
            filename: filename.into(),
            size: Some(100),
            hash: Some(hash.into()),
            hash_algo: Some("sha256".into()),
            url: String::new(),
            updated_at: None,
            version_count: Some(version),
            version_number: Some(version),
            identity_key: None,
        }
    }

    fn local(filename: &str, hash: &str) -> LocalFile {
        LocalFile {
            filename: filename.into(),
            relative_path: filename.into(),
            path: std::path::PathBuf::from(filename),
            size: 100,
            mtime_ms: 1,
            hash: Some(hash.into()),
            identity_key: identity_key(filename).unwrap(),
        }
    }

    fn snapshot(id: &str, filename: &str, hash: &str, version: u64) -> SnapshotFile {
        SnapshotFile {
            id: id.into(),
            filename: filename.into(),
            identity_key: None,
            relative_path: None,
            hash: Some(hash.into()),
            hash_algo: Some("sha256".into()),
            size: Some(100),
            mtime: None,
            version_count: Some(version),
            version_number: Some(version),
        }
    }

    #[test]
    fn first_sync_reports_remote_new() {
        let diff = compute_diff(&[], &[remote("p1", "a.jpg", "h1", 1)], &[]);
        assert_eq!(diff.counts.remote_new, 1);
        assert!(diff.has_changes());
    }

    #[test]
    fn matching_hash_is_in_sync() {
        let diff = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h1", 1)],
            &[local("a.jpg", "h1")],
        );
        assert_eq!(diff.counts.in_sync, 1);
        assert!(!diff.has_changes());
    }

    #[test]
    fn modifications_are_detected() {
        let local_mod = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h1", 1)],
            &[local("a.jpg", "h2")],
        );
        assert_eq!(local_mod.counts.local_modified, 1);

        let remote_mod = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h2", 1)],
            &[local("a.jpg", "h1")],
        );
        assert_eq!(remote_mod.counts.remote_modified, 1);
    }

    #[test]
    fn both_sides_changed_is_conflict() {
        let diff = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h2", 1)],
            &[local("a.jpg", "h3")],
        );
        assert_eq!(diff.counts.conflicts, 1);
        assert_eq!(diff.conflicts[0].identity_key, "image:a");
    }

    #[test]
    fn deletions_are_reported() {
        let local_deleted = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h1", 1)],
            &[],
        );
        assert_eq!(local_deleted.counts.local_deleted, 1);

        let remote_deleted = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[],
            &[local("a.jpg", "h1")],
        );
        assert_eq!(remote_deleted.counts.remote_deleted, 1);
    }

    #[test]
    fn version_bump_is_a_remote_change() {
        let diff = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h1", 2)],
            &[local("a.jpg", "h1")],
        );
        assert_eq!(diff.counts.remote_modified, 1);
    }

    #[test]
    fn jpg_and_dng_share_identity() {
        let diff = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h1", 1)],
            &[local("a.jpg", "h1"), local("a.dng", "raw")],
        );
        assert_eq!(diff.counts.in_sync, 1);
        assert_eq!(diff.counts.local_new, 0);
    }
}
