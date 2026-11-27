use std::collections::{HashMap, HashSet};

use crate::localfiles::preferred_local_files;
use crate::model::{
    Conflict, ConflictPhoto, Diff, DiffCounts, LocalFile, Photo, RemoteDeletedItem, SnapshotFile,
};

fn snapshot_had_local(base: &SnapshotFile) -> bool {
    base.local_present
}

fn remote_changed(photo: &Photo, base: &SnapshotFile) -> bool {
    match (photo.version_count, base.version_count) {
        (Some(current), Some(previous)) => current != previous,
        _ => false,
    }
}

fn local_changed(file: &LocalFile, base: &SnapshotFile) -> bool {
    if !base.local_present {
        return true;
    }
    match (file.hash.as_deref(), base.local_hash.as_deref()) {
        (Some(current), Some(previous)) => current != previous,
        _ => base.local_size != Some(file.size) || base.mtime != Some(file.mtime_ms),
    }
}

/// The local bytes already are the remote photo: either the served copy
/// (`download_hash`) or the uploaded source (`hash`).
///
/// This is how the engine recovers from a crash between a transfer and the
/// snapshot write, or from an upload whose response was lost: the next review
/// sees "both sides changed", but they hold identical content, so it must
/// adopt the pair instead of reporting a conflict the photographer cannot
/// resolve from the app.
fn local_matches_remote(file: &LocalFile, photo: &Photo) -> bool {
    let Some(local) = file.hash.as_deref() else {
        return false;
    };
    photo.content_hash() == Some(local) || photo.hash.as_deref() == Some(local)
}

pub fn compute_diff(
    snapshot_files: &[SnapshotFile],
    remote_photos: &[Photo],
    local_files: &[LocalFile],
) -> Diff {
    let mut previous: HashMap<String, &SnapshotFile> = HashMap::new();
    for file in snapshot_files {
        if let Some(key) = file.identity() {
            previous.entry(key).or_insert(file);
        }
    }

    let mut remote: HashMap<String, &Photo> = HashMap::new();
    for photo in remote_photos {
        if let Some(key) = photo.identity() {
            remote.entry(key).or_insert(photo);
        }
    }

    let local = preferred_local_files(local_files);

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
                // A remote photo only counts as locally deleted if the last snapshot
                // proves it was actually present on disk. Remote-only entries from an
                // interrupted download must be downloaded, never proposed for deletion.
                if base.is_some_and(snapshot_had_local) {
                    diff.local_deleted.push(photo.clone());
                } else {
                    diff.remote_new.push(photo.clone());
                }
            }
            (None, Some(file)) => match base {
                Some(base) if base.keep_local => {}
                Some(base) if snapshot_had_local(base) => {
                    diff.remote_deleted.push(RemoteDeletedItem {
                        id: base.id.clone(),
                        filename: file.filename.clone(),
                        identity_key: Some(key.clone()),
                        relative_path: Some(file.relative_path.clone()),
                        companions: Vec::new(),
                    });
                }
                _ => diff.local_new.push(file.clone()),
            },
            (Some(photo), Some(file)) => {
                let local_matches = local_matches_remote(file, photo);
                let Some(base) = base else {
                    // First time this identity is seen on both sides. Adopt it as
                    // the common baseline; the remote may be a resized version of
                    // the original local file, so sizes/hashes must not conflict.
                    //
                    // Exception: Picstome is currently showing a RAW while this
                    // folder has the JPEG for the same photo. That happens when
                    // the RAW upload succeeded but the JPEG upload failed, and
                    // adopting the pair would leave the RAW as the version
                    // clients see forever. Upload the local JPEG again.
                    if local_matches {
                        diff.in_sync.push(photo.clone());
                    } else if crate::identity::is_raw(&photo.filename)
                        && !crate::identity::is_raw(&file.filename)
                    {
                        diff.local_modified.push(file.clone());
                    } else {
                        diff.in_sync.push(photo.clone());
                    }
                    continue;
                };

                let remote_was_changed = remote_changed(photo, base);
                let local_was_changed = local_changed(file, base);

                // The local bytes already are the remote content and they moved
                // since the review: a transfer landed on disk but the snapshot
                // write did not (crash, full disk, lost response). Adopting the
                // pair avoids an unresolvable conflict.
                if local_was_changed && local_matches {
                    diff.in_sync.push(photo.clone());
                } else if remote_was_changed && local_was_changed {
                    diff.conflicts.push(Conflict {
                        identity_key: key.clone(),
                        remote: ConflictPhoto {
                            filename: photo.filename.clone(),
                            version_count: photo.version_count,
                            size: photo.size,
                        },
                        local: ConflictPhoto {
                            filename: file.filename.clone(),
                            version_count: base.version_count,
                            size: Some(file.size),
                        },
                    });
                } else if remote_was_changed {
                    diff.remote_modified.push(photo.clone());
                } else if local_was_changed {
                    diff.local_modified.push(file.clone());
                } else {
                    diff.in_sync.push(photo.clone());
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
    use crate::identity::identity_key;

    fn remote(id: &str, filename: &str, hash: &str, version: u64) -> Photo {
        Photo {
            id: id.into(),
            filename: filename.into(),
            size: Some(100),
            hash: Some(hash.into()),
            download_hash: Some(hash.into()),
            hash_algo: Some("sha256".into()),
            url: String::new(),
            updated_at: None,
            version_count: Some(version),
            version_number: Some(version),
            identity_key: None,
            favorited_at: None,
            rating: None,
            is_favorite: None,
            archived_at: None,
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
            mtime: Some(1),
            local_present: true,
            local_hash: Some(hash.into()),
            local_size: Some(100),
            version_count: Some(version),
            version_number: Some(version),
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
            &[remote("p1", "a.jpg", "h2", 2)],
            &[local("a.jpg", "h1")],
        );
        assert_eq!(remote_mod.counts.remote_modified, 1);
    }

    #[test]
    fn both_sides_changed_is_conflict() {
        let diff = compute_diff(
            &[snapshot("p1", "a.jpg", "h1", 1)],
            &[remote("p1", "a.jpg", "h2", 2)],
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
        assert_eq!(remote_deleted.remote_deleted[0].id, "p1");
    }

    #[test]
    fn legacy_manifest_hash_is_not_used_for_download_validation() {
        let mut photo = remote("p1", "a.jpg", "source-hash", 1);
        photo.download_hash = None;
        let diff = compute_diff(&[], &[photo], &[local("a.jpg", "served-hash")]);
        assert_eq!(diff.counts.in_sync, 1);
        assert_eq!(diff.counts.conflicts, 0);
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

#[cfg(test)]
mod extra_tests {
    use super::*;
    use crate::identity::identity_key;

    fn photo_with(
        id: &str,
        filename: &str,
        hash: Option<&str>,
        size: Option<u64>,
        version: Option<u64>,
    ) -> Photo {
        Photo {
            id: id.into(),
            filename: filename.into(),
            size,
            hash: hash.map(str::to_string),
            download_hash: hash.map(str::to_string),
            hash_algo: Some("sha256".into()),
            url: String::new(),
            updated_at: None,
            version_count: version,
            version_number: version,
            identity_key: None,
            favorited_at: None,
            rating: None,
            is_favorite: None,
            archived_at: None,
        }
    }

    fn local_with(filename: &str, hash: Option<&str>, size: u64) -> LocalFile {
        LocalFile {
            filename: filename.into(),
            relative_path: filename.into(),
            path: std::path::PathBuf::from(filename),
            size,
            mtime_ms: 1,
            hash: hash.map(str::to_string),
            identity_key: identity_key(filename).unwrap_or_default(),
        }
    }

    fn snapshot_with(
        id: &str,
        filename: &str,
        hash: Option<&str>,
        size: Option<u64>,
        version: Option<u64>,
    ) -> SnapshotFile {
        SnapshotFile {
            id: id.into(),
            filename: filename.into(),
            identity_key: None,
            relative_path: None,
            hash: hash.map(str::to_string),
            hash_algo: Some("sha256".into()),
            size,
            mtime: None,
            local_present: false,
            local_hash: hash.map(str::to_string),
            local_size: size,
            version_count: version,
            version_number: version,
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
    fn compares_by_size_when_hashes_are_missing() {
        let mut base = snapshot_with("p1", "a.jpg", None, Some(10), Some(1));
        base.local_present = true;
        base.local_size = Some(10);
        base.mtime = Some(1);
        let diff = compute_diff(
            &[base.clone()],
            &[photo_with("p1", "a.jpg", None, Some(10), Some(1))],
            &[local_with("a.jpg", None, 10)],
        );
        assert_eq!(diff.counts.in_sync, 1);

        let changed = compute_diff(
            &[base],
            &[photo_with("p1", "a.jpg", None, Some(10), Some(1))],
            &[local_with("a.jpg", None, 11)],
        );
        assert_eq!(changed.counts.local_modified, 1);
    }

    #[test]
    fn resized_remote_copy_does_not_conflict_with_the_local_original() {
        let diff = compute_diff(
            &[],
            &[photo_with("p1", "a.jpg", None, Some(500), Some(1))],
            &[local_with("a.jpg", Some("original"), 6_000_000)],
        );
        assert_eq!(diff.counts.in_sync, 1);
        assert_eq!(diff.counts.conflicts, 0);

        let mut base = snapshot_with("p1", "a.jpg", Some("remote"), Some(500), Some(1));
        base.local_present = true;
        base.local_hash = Some("original".into());
        base.local_size = Some(6_000_000);
        base.mtime = Some(1);
        let local = local_with("a.jpg", Some("original"), 6_000_000);
        let diff = compute_diff(
            &[base.clone()],
            &[photo_with(
                "p1",
                "a.jpg",
                Some("remote"),
                Some(500),
                Some(1),
            )],
            std::slice::from_ref(&local),
        );
        assert_eq!(diff.counts.in_sync, 1);

        let remote_updated = compute_diff(
            &[base],
            &[photo_with(
                "p1",
                "a.jpg",
                Some("remote-v2"),
                Some(500),
                Some(2),
            )],
            &[local],
        );
        assert_eq!(remote_updated.counts.remote_modified, 1);
        assert_eq!(remote_updated.counts.conflicts, 0);
    }

    #[test]
    fn kept_local_files_are_not_reported_as_remote_deleted() {
        let mut base = snapshot_with("p1", "a.jpg", Some("h1"), Some(10), Some(1));
        base.keep_local = true;
        let diff = compute_diff(&[base], &[], &[local_with("a.jpg", Some("h1"), 10)]);
        assert_eq!(diff.counts.remote_deleted, 0);
        assert!(!diff.has_changes());
    }

    #[test]
    fn unknown_baseline_is_adopted_as_in_sync() {
        let diff = compute_diff(
            &[],
            &[photo_with("p1", "a.jpg", Some("remote"), Some(10), None)],
            &[local_with("a.jpg", Some("local"), 10)],
        );
        assert_eq!(diff.counts.in_sync, 1);
        assert_eq!(diff.counts.conflicts, 0);
    }

    #[test]
    fn version_metadata_without_a_base_does_not_force_a_conflict() {
        let diff = compute_diff(
            &[],
            &[photo_with("p1", "a.jpg", Some("same"), Some(10), Some(2))],
            &[local_with("a.jpg", Some("same"), 10)],
        );
        assert_eq!(diff.counts.in_sync, 1);
    }

    #[test]
    fn uses_explicit_identity_keys_and_ignores_unkeyed_items() {
        let mut remote = photo_with("p1", "a.jpg", Some("h1"), Some(10), Some(1));
        remote.identity_key = Some("image:explicit".into());
        let mut local = local_with("b.jpg", Some("h1"), 10);
        local.identity_key = "image:explicit".into();
        let diff = compute_diff(&[], &[remote], &[local]);
        assert_eq!(diff.counts.in_sync, 1);

        let mut no_key = local_with("c.jpg", Some("h"), 10);
        no_key.identity_key.clear();
        let diff = compute_diff(&[], &[], &[no_key]);
        assert_eq!(diff.counts.remote_new + diff.counts.local_new, 1);
    }

    #[test]
    fn remote_only_snapshot_entry_is_downloaded_not_deleted() {
        let base = snapshot_with("p1", "a.jpg", Some("h1"), Some(10), Some(1));
        let diff = compute_diff(
            &[base],
            &[photo_with("p1", "a.jpg", Some("h1"), Some(10), Some(1))],
            &[],
        );
        assert_eq!(diff.counts.local_deleted, 0);
        assert_eq!(diff.counts.remote_new, 1);
    }

    #[test]
    fn missing_previously_local_photo_is_still_reported_as_deleted() {
        let mut base = snapshot_with("p1", "a.jpg", Some("h1"), Some(10), Some(1));
        base.mtime = Some(1);
        base.local_present = true;
        let diff = compute_diff(
            &[base.clone()],
            &[photo_with("p1", "a.jpg", Some("h1"), Some(10), Some(1))],
            &[],
        );
        assert_eq!(diff.counts.local_deleted, 1);
        assert_eq!(diff.counts.remote_new, 0);
    }

    #[test]
    fn remote_deleted_photo_not_previously_local_is_a_new_local_file() {
        let diff = compute_diff(
            &[snapshot_with("p1", "a.jpg", Some("h1"), Some(10), Some(1))],
            &[],
            &[local_with("a.jpg", Some("h1"), 10)],
        );
        assert_eq!(diff.counts.remote_deleted, 0);
        assert_eq!(diff.counts.local_new, 1);
    }

    #[test]
    fn identical_content_recovers_from_an_interrupted_snapshot_write() {
        // A download reached the disk but the snapshot write did not.
        let mut base = snapshot_with("p1", "a.jpg", Some("old"), Some(10), Some(1));
        base.local_present = true;
        base.local_hash = Some("old".into());
        base.local_size = Some(10);
        base.mtime = Some(1);
        let mut remote = photo_with("p1", "a.jpg", Some("new-source"), Some(10), Some(2));
        remote.download_hash = Some("new-download".into());
        let diff = compute_diff(
            &[base.clone()],
            &[remote.clone()],
            &[local_with("a.jpg", Some("new-download"), 10)],
        );
        assert_eq!(diff.counts.conflicts, 0);
        assert_eq!(diff.counts.in_sync, 1);

        // An upload succeeded but the snapshot write did not.
        remote.download_hash = Some("served-elsewhere".into());
        let diff = compute_diff(
            &[base],
            &[remote],
            &[local_with("a.jpg", Some("new-source"), 10)],
        );
        assert_eq!(diff.counts.conflicts, 0);
        assert_eq!(diff.counts.in_sync, 1);
    }

    #[test]
    fn remote_raw_with_a_local_jpeg_uploads_the_jpeg_again() {
        // The RAW upload succeeded and the JPEG upload failed: Picstome shows
        // the RAW, the folder has the JPEG. The JPEG must become current again
        // instead of the pair being adopted as "in sync".
        let remote = photo_with("p1", "PAIR.CR3", Some("raw"), Some(10), Some(1));
        let diff = compute_diff(&[], &[remote], &[local_with("PAIR.jpg", Some("jpeg"), 10)]);
        assert_eq!(diff.counts.local_modified, 1);
        assert_eq!(diff.counts.in_sync, 0);
    }
}
