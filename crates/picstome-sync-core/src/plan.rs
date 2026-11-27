//! Review, then apply.
//!
//! A sync happens in two steps so nothing moves without the photographer's
//! say-so:
//!
//! 1. [`plan_sync`] walks the folder, fetches the manifest and works out every
//!    change, **without transferring or deleting anything**. The result is a
//!    [`SyncPlan`] the UI shows as a review, plus private context kept by the
//!    app until the photographer applies or discards it.
//! 2. [`apply_sync`] runs only the actions ticked in a [`PlanSelection`].
//!    Before each action it checks that the file or photo is still exactly as
//!    reviewed (local size and mtime, remote version); anything that changed
//!    in between is skipped and reported, never acted on.
//!
//! The scan is not repeated on apply: the fingerprint cache already made it
//! cheap, and the review is what the photographer agreed to.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::cache::FingerprintCache;
use crate::config::Store;
use crate::diff::compute_diff;
use crate::error::{CoreError, CoreResult};
use crate::identity::identity_key;
use crate::ignored::GalleryIgnores;
use crate::localfiles::{
    detect_edited_folders, is_edited_gallery_folder_name, preferred_local_files, scan_local_photos,
    scan_local_photos_with, RawLocalFile, MAX_PHOTO_BYTES,
};
use crate::model::{
    Conflict, CreatedGallery, DeleteProposal, DeleteProposals, Diff, DownloadedItem, GalleryEntry,
    LocalFile, Manifest, Photo, Progress, RatingSyncPlan, RemoteDeletedItem, SnapshotFile,
    SyncErrorItem, SyncReport, SyncScanStats, SyncTimings, SyncWarningItem,
};
use crate::sanitize::{is_inside, lexical_absolute};
use crate::sync::{
    apply_remote_deletions, attach_companions, build_rating_plan, build_snapshot, companion_paths,
    download_proposals, download_target, duplicate_local_warnings, file_mtime_ms, now_millis,
    plan_uploads, resolve_remote_deletions, trash_targets, upload_unit, DownloadTarget,
    RemoteDeleteAction, SyncOptions, SyncProgressFn, TransferPhase, UnitOutcome, UploadUnit,
};
use crate::transfer::{download_file, DownloadOptions};

/// Deleting this many photos in Picstome, or more than a fifth of the
/// gallery (from 10 photos on), needs the count typed back. It is the
/// signature of an unplugged drive or an emptied folder, not of a cull.
pub const MASS_DELETE_MIN: usize = 10;
pub const MASS_DELETE_ABSOLUTE: usize = 50;

/// A photo Picstome would send to this computer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedDownload {
    pub id: String,
    pub filename: String,
    /// Where it would be written, relative to the gallery folder.
    pub relative_path: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// `"new"` or `"modified"` (a new version of a photo already here).
    pub reason: String,
}

/// A local photo that would go to Picstome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedUpload {
    pub relative_path: String,
    pub filename: String,
    pub size: u64,
    /// `"new"` or `"modified"`.
    pub reason: String,
    /// RAW files uploaded first, as earlier versions ("JPEG + RAW").
    #[serde(default)]
    pub raw_first: Vec<String>,
}

/// A subfolder of the gallery (`editadas`, `edit`, `exportadas`, `export`)
/// that can become its own Picstome gallery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedFolderProposal {
    /// Folder name as it is on disk.
    pub folder: String,
    /// Folder path relative to the gallery root.
    pub relative_path: String,
    /// Proposed gallery name: parent name plus the folder name.
    pub gallery_name: String,
    pub photo_count: u64,
    /// `None` means the parent gallery has no photoshoot; the new gallery is
    /// created standalone.
    pub photoshoot_id: Option<i64>,
}

/// Everything a sync would do, for the photographer to review.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub id: String,
    pub gallery_id: String,
    pub name: String,
    /// No previous sync: every difference is "only here" or "only there".
    pub first_sync: bool,
    pub downloads: Vec<PlannedDownload>,
    pub uploads: Vec<PlannedUpload>,
    /// Photos with a RAW file waiting for this gallery's RAW choice.
    pub raw_decision_needed: usize,
    /// RAW files that stay on this computer ("JPEG only").
    pub raw_kept_local: Vec<String>,
    /// Deleted in Picstome, still on this computer.
    pub remote_deleted: Vec<RemoteDeletedItem>,
    /// Deleted on this computer, still in Picstome.
    pub local_deleted: Vec<DeleteProposal>,
    /// Changed on both sides: shown, never touched.
    pub conflicts: Vec<Conflict>,
    /// Picstome stars to write into the local files (after downloads).
    pub ratings: RatingSyncPlan,
    pub warnings: Vec<SyncWarningItem>,
    pub errors: Vec<SyncErrorItem>,
    pub ignored_local: Vec<String>,
    pub ignored_remote: Vec<String>,
    /// Photos in the Picstome gallery, for the mass-delete guard.
    pub remote_total: usize,
    /// Deleting every `local_deleted` photo in Picstome needs the count typed
    /// back (see [`MASS_DELETE_MIN`]).
    pub mass_delete: bool,
    /// Edited subfolders that can become their own gallery in the same
    /// photoshoot as this one.
    #[serde(default)]
    pub edited_folders: Vec<EditedFolderProposal>,
    /// Edited subfolders that already have a linked local gallery.
    #[serde(default)]
    pub edited_folders_linked: Vec<String>,
    pub scan_stats: SyncScanStats,
    pub timings: SyncTimings,
}

impl SyncPlan {
    /// Anything for the photographer to decide or apply.
    pub fn has_changes(&self) -> bool {
        !self.downloads.is_empty()
            || !self.uploads.is_empty()
            || self.raw_decision_needed > 0
            || !self.remote_deleted.is_empty()
            || !self.local_deleted.is_empty()
            || !self.conflicts.is_empty()
            || !self.ratings.items.is_empty()
            || !self.ratings.needs_rating.is_empty()
            || !self.edited_folders.is_empty()
    }
}

/// What to do with a photo deleted in Picstome that is still here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeletedChoice {
    pub id: String,
    pub action: RemoteDeleteAction,
    /// Also trash the RAW beside it (only with `delete_local`).
    #[serde(default)]
    pub include_raw: bool,
}

/// The actions ticked in the review. Anything not listed is left for the
/// next review.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanSelection {
    /// Planned downloads to run (photo ids).
    pub downloads: Vec<String>,
    /// Planned uploads to run (relative paths of the main file).
    pub uploads: Vec<String>,
    /// Photos deleted here to download again (photo ids).
    pub restore: Vec<String>,
    /// Photos deleted here to delete in Picstome too (photo ids).
    pub delete_remote: Vec<String>,
    /// The typed-back count when the plan is a mass delete.
    pub confirm_mass_delete: Option<usize>,
    pub remote_deleted: Vec<RemoteDeletedChoice>,
    /// Local files never to propose again (relative paths).
    pub ignore_local: Vec<String>,
    /// Picstome photos never to propose again (photo ids).
    pub ignore_remote: Vec<String>,
    /// Local files to move to the trash (relative paths), with every other
    /// file of the same photo beside them.
    pub trash_local: Vec<String>,
    /// Edited subfolders to turn into new galleries (relative paths).
    pub create_edited: Vec<String>,
}

impl PlanSelection {
    /// Every download and upload, nothing destructive: what a plain
    /// "sync" used to do.
    pub fn transfers(plan: &SyncPlan) -> Self {
        Self {
            downloads: plan.downloads.iter().map(|item| item.id.clone()).collect(),
            uploads: plan
                .uploads
                .iter()
                .map(|item| item.relative_path.clone())
                .collect(),
            ..Default::default()
        }
    }
}

/// A plan plus the context needed to apply it. The app keeps it in memory
/// between the review and Apply; it is never sent to the UI.
pub struct PlannedSync {
    pub plan: SyncPlan,
    context: PlanContext,
}

struct PlanContext {
    gallery: GalleryEntry,
    /// `last_sync_at` of the snapshot the plan was computed from. Apply
    /// refuses a plan whose snapshot has moved on.
    snapshot_at: Option<i64>,
    manifest: Manifest,
    local_files: Vec<LocalFile>,
    previous_files: Vec<SnapshotFile>,
    diff: Diff,
    units: Vec<UploadUnit>,
    ignored_local: Vec<LocalFile>,
    fingerprints: FingerprintCache,
}

fn new_plan_id() -> String {
    format!("{:x}{:08x}", now_millis(), rand::random::<u32>())
}

/// `"MiBoda"` + `"editadas"` -> `"MiBoda editadas"`, capped to the API's
/// 255-character name limit.
fn child_gallery_name(base: &str, folder: &str) -> String {
    let base = base.trim();
    let folder = folder.trim();
    let name = if base.is_empty() {
        folder.to_string()
    } else {
        format!("{base} {folder}")
    };
    name.chars().take(255).collect()
}

fn same_local_folder(entry: &GalleryEntry, path: &Path) -> bool {
    lexical_absolute(Path::new(&entry.local_path)) == lexical_absolute(path)
}

fn progress_event(gallery_id: &str, phase: &str, done: u64, total: u64) -> Progress {
    Progress {
        gallery_id: gallery_id.to_string(),
        phase: phase.into(),
        done,
        total,
        ..Default::default()
    }
}

/// Work out everything a sync would do. Transfers and deletes nothing; it
/// only refreshes the fingerprint cache (pure bookkeeping).
pub async fn plan_sync(
    api: &ApiClient,
    store: &Store,
    gallery: &GalleryEntry,
    options: &SyncOptions,
    progress: SyncProgressFn,
) -> CoreResult<PlannedSync> {
    let mut timings = SyncTimings::default();
    let mut scan_stats = SyncScanStats::default();
    let gallery_id = gallery.gallery_id.as_str();

    // Fail before touching the network or the cache: a missing or unmounted
    // folder must never look like "everything was deleted locally".
    let folder = tokio::fs::metadata(&gallery.local_path).await?;
    if !folder.is_dir() {
        return Err(CoreError::InvalidData(format!(
            "not a directory: {}",
            gallery.local_path
        )));
    }

    let previous = store.load_snapshot(gallery_id);
    let snapshot_at = previous.as_ref().map(|snapshot| snapshot.last_sync_at);
    let first_sync = previous.is_none();
    let previous_files = previous.map(|snapshot| snapshot.files).unwrap_or_default();
    let mut fingerprints = store.load_fingerprints(gallery_id, &gallery.local_path);

    progress(progress_event(gallery_id, "manifest", 0, 1));
    // The manifest request and the local walk are independent; overlapping
    // them removes one full round trip from every review.
    let api_for_manifest = api.clone();
    let gallery_for_manifest = gallery.gallery_id.clone();
    let mut manifest_task = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let result = api_for_manifest.manifest(&gallery_for_manifest).await;
        (result, started.elapsed().as_millis() as u64)
    });

    let scan_started = std::time::Instant::now();
    progress(progress_event(gallery_id, "scan", 0, 1));
    let progress_scan = progress.clone();
    let gallery_for_scan = gallery.gallery_id.clone();
    let walk_dir = Path::new(&gallery.local_path).to_path_buf();
    let (records, mut scan_errors) = tokio::task::spawn_blocking(move || {
        let mut on_scan = |seen: u64, matched: u64| {
            progress_scan(Progress {
                gallery_id: gallery_for_scan.clone(),
                phase: "scan".into(),
                done: matched,
                total: 0,
                file_done: Some(seen),
                ..Default::default()
            });
        };
        scan_local_photos_with(&walk_dir, 8, true, MAX_PHOTO_BYTES, &mut on_scan)
    })
    .await
    .map_err(|err| CoreError::Other(format!("scan task failed: {err}")))?;

    // Remember every file the walk saw so the cache never keeps entries for
    // deleted paths, and keep the metadata to record fingerprints.
    let seen_paths: HashSet<String> = records
        .iter()
        .map(|record| record.relative_path.clone())
        .collect();
    let metadata_by_path: HashMap<String, RawLocalFile> = records
        .iter()
        .map(|record| (record.relative_path.clone(), record.clone()))
        .collect();

    let progress_hash = progress.clone();
    let gallery_for_hash = gallery.gallery_id.clone();
    let fingerprints_for_hash = fingerprints.clone();
    let hash_started = std::time::Instant::now();
    // Hashing overlaps the manifest request. It is a plain future (not a
    // task) so an API failure drops it at once instead of waiting for
    // minutes of disk reads before showing the error.
    let mut hash_fut = Box::pin(crate::localfiles::hash_local_records_cached(
        records,
        &previous_files,
        Some(&fingerprints_for_hash),
        options.hash_concurrency,
        move |done, total| {
            progress_hash(Progress {
                gallery_id: gallery_for_hash.clone(),
                phase: "hash".into(),
                done,
                total,
                ..Default::default()
            });
        },
    ));
    let mut hash_output = None;
    let manifest_result = tokio::select! {
        result = &mut manifest_task => result,
        output = &mut hash_fut => {
            hash_output = Some(output);
            manifest_task.await
        }
    };
    let (manifest, manifest_ms) =
        manifest_result.map_err(|err| CoreError::Other(format!("manifest task failed: {err}")))?;
    let manifest = manifest?;
    timings.manifest_ms = manifest_ms;
    // The hashing future borrows the snapshot files; end it here.
    let (results, hash_stats) = match hash_output {
        Some(output) => {
            drop(hash_fut);
            output
        }
        None => hash_fut.await,
    };

    let mut all_local_files = Vec::with_capacity(results.len());
    for result in results {
        match result {
            Ok(file) => all_local_files.push(file),
            Err(err) => scan_errors.push(err),
        }
    }
    all_local_files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    timings.scan_ms = scan_started.elapsed().as_millis() as u64;
    timings.hash_ms = hash_started.elapsed().as_millis() as u64;

    // Fold fresh hashes into the cache: this write is what makes the next
    // review a metadata-only pass.
    for file in &all_local_files {
        let Some(hash) = file.hash.as_deref() else {
            continue;
        };
        // Only keep the inode when the file did not change between the walk
        // and the read, otherwise it would describe a different file.
        let (inode, device) = metadata_by_path
            .get(&file.relative_path)
            .filter(|record| record.size == file.size && record.mtime_ms == file.mtime_ms)
            .map(|record| (record.inode, record.device))
            .unwrap_or((None, None));
        fingerprints.insert(
            file.relative_path.clone(),
            file.size,
            file.mtime_ms,
            inode,
            device,
            hash,
        );
    }
    fingerprints.retain_paths(&seen_paths);
    let _ = store.save_fingerprints(&fingerprints);

    scan_stats.files = hash_stats.files;
    scan_stats.hashed = hash_stats.hashed;
    scan_stats.cached = hash_stats.cached;
    scan_stats.skipped_duplicates = hash_stats.skipped;
    scan_stats.bytes_read = hash_stats.bytes_read;
    progress(progress_event(gallery_id, "manifest", 1, 1));

    let mut ignores = GalleryIgnores::load(Path::new(&gallery.local_path));
    let released = ignores.release_cross_side(&all_local_files, &manifest.photos);
    let retained = ignores.retain_current(&all_local_files, &manifest.photos);
    if released || retained {
        let _ = ignores.save(Path::new(&gallery.local_path));
    }
    let (ignored_local, local_files): (Vec<LocalFile>, Vec<LocalFile>) = all_local_files
        .into_iter()
        .partition(|file| ignores.ignores_local(file));
    let ignored_remote: Vec<String> = manifest
        .photos
        .iter()
        .filter(|photo| ignores.ignores_remote(photo))
        .map(|photo| photo.filename.clone())
        .collect();
    let mut active_photos: Vec<Photo> = manifest
        .photos
        .iter()
        .filter(|photo| !ignores.ignores_remote(photo))
        .cloned()
        .collect();
    progress(progress_event(gallery_id, "scan", 1, 1));

    // A file over the size limit still exists on disk; it is just not synced.
    // Leave its photo out on *both* sides: otherwise it would look deleted
    // locally, or the remote version would be downloaded over it.
    let oversized_keys: HashSet<String> = scan_errors
        .iter()
        .filter(|error| error.is_oversized())
        .filter_map(|error| {
            Path::new(&error.path)
                .file_name()
                .and_then(|name| identity_key(&name.to_string_lossy()))
        })
        .collect();
    if !oversized_keys.is_empty() {
        active_photos.retain(|photo| {
            photo
                .identity()
                .is_none_or(|key| !oversized_keys.contains(&key))
        });
    }
    let previous_files: Vec<SnapshotFile> = previous_files
        .into_iter()
        .filter(|file| {
            file.identity()
                .is_none_or(|key| !oversized_keys.contains(&key))
        })
        .collect();

    let mut diff = compute_diff(&previous_files, &active_photos, &local_files);
    let upload_plan = plan_uploads(&mut diff, &local_files, options.raw_mode);
    attach_companions(&mut diff.remote_deleted, &local_files);

    // Oversized files are a deliberate skip, reported as warnings. Only real
    // read failures are errors: they make local deletions unprovable, so
    // no deletion is proposed while there are any.
    let (oversized, scan_errors): (Vec<_>, Vec<_>) = scan_errors
        .into_iter()
        .partition(|error| error.is_oversized());
    let mut warnings = duplicate_local_warnings(&local_files);
    warnings.extend(oversized.into_iter().map(|error| {
        let filename = Path::new(&error.path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string());
        SyncWarningItem {
            kind: "file_too_large".into(),
            message: format!(
                "{} is larger than {} MB and was not synced",
                filename.as_deref().unwrap_or(&error.path),
                MAX_PHOTO_BYTES / (1024 * 1024)
            ),
            filename,
            preferred: None,
            path: Some(error.path),
        }
    }));
    let errors: Vec<SyncErrorItem> = scan_errors
        .into_iter()
        .map(|error| SyncErrorItem {
            phase: "scan".into(),
            filename: None,
            path: Some(error.path),
            message: error.error,
        })
        .collect();
    let local_deleted = if errors.is_empty() {
        download_proposals(&diff)
    } else {
        Vec::new()
    };

    let base_name = if manifest.name.is_empty() {
        gallery.name.clone()
    } else {
        manifest.name.clone()
    };

    // Edited subfolders can become sibling galleries in the same photoshoot.
    // They are excluded from the parent scan on purpose, so this is a
    // separate, direct-children-only detection.
    let mut edited_folders = Vec::new();
    let mut edited_folders_linked = Vec::new();
    let edited_candidates = detect_edited_folders(Path::new(&gallery.local_path), MAX_PHOTO_BYTES);
    if !edited_candidates.is_empty() {
        let index = store.load_index();
        let mut to_propose = Vec::new();
        for candidate in edited_candidates {
            if index
                .iter()
                .any(|entry| same_local_folder(entry, &candidate.path))
            {
                edited_folders_linked.push(candidate.name);
            } else {
                to_propose.push(candidate);
            }
        }
        if !to_propose.is_empty() {
            match api.find_gallery(gallery_id, &base_name).await {
                Ok(Some(parent)) if parent.photoshoot_known() => {
                    let photoshoot_id = parent.photoshoot();
                    for candidate in to_propose {
                        edited_folders.push(EditedFolderProposal {
                            folder: candidate.name.clone(),
                            relative_path: candidate.name.clone(),
                            gallery_name: child_gallery_name(&base_name, &candidate.name),
                            photo_count: candidate.photo_count,
                            photoshoot_id,
                        });
                    }
                }
                Ok(Some(_)) => warnings.push(SyncWarningItem {
                    kind: "edited_folder_unsupported".into(),
                    filename: None,
                    preferred: None,
                    path: None,
                    message: "This Picstome API does not expose the gallery photoshoot; update Picstome to create edited galleries in the same photoshoot.".into(),
                }),
                Ok(None) => warnings.push(SyncWarningItem {
                    kind: "edited_folder_lookup_failed".into(),
                    filename: None,
                    preferred: None,
                    path: None,
                    message: format!(
                        "Could not find \"{base_name}\" in the Picstome gallery list; \
                         edited folders will be proposed again on the next review."
                    ),
                }),
                Err(error) => warnings.push(SyncWarningItem {
                    kind: "edited_folder_lookup_failed".into(),
                    filename: None,
                    preferred: None,
                    path: None,
                    message: format!("Could not read the gallery photoshoot: {error}"),
                }),
            }
        }
    }

    // Downloads, with the place each one would be written.
    let local_by_key = preferred_local_files(&local_files);
    let mut downloads = Vec::new();
    for (photo, reason) in diff
        .remote_new
        .iter()
        .map(|photo| (photo, "new"))
        .chain(diff.remote_modified.iter().map(|photo| (photo, "modified")))
    {
        let existing = photo
            .identity()
            .and_then(|key| local_by_key.get(&key).copied());
        match download_target(Path::new(&gallery.local_path), photo, existing) {
            DownloadTarget::Path { relative_path, .. } => downloads.push(PlannedDownload {
                id: photo.id.clone(),
                filename: photo.filename.clone(),
                relative_path,
                size: photo.size,
                reason: reason.into(),
            }),
            DownloadTarget::Unsafe => warnings.push(SyncWarningItem {
                kind: "unsafe_filename".into(),
                filename: Some(photo.filename.clone()),
                preferred: None,
                path: None,
                message: format!(
                    "{} has a name that cannot be saved safely and was not downloaded",
                    photo.filename
                ),
            }),
        }
    }
    let uploads: Vec<PlannedUpload> = upload_plan
        .units
        .iter()
        .map(|unit| PlannedUpload {
            relative_path: unit.file.relative_path.clone(),
            filename: unit.file.filename.clone(),
            size: unit.file.size,
            reason: unit.reason.clone(),
            raw_first: unit
                .raw_first
                .iter()
                .map(|file| file.filename.clone())
                .collect(),
        })
        .collect();

    // Stars Picstome would write once the planned downloads are on disk.
    let simulated: Vec<DownloadedItem> = diff
        .remote_new
        .iter()
        .chain(diff.remote_modified.iter())
        .map(|photo| DownloadedItem {
            id: photo.id.clone(),
            filename: photo.filename.clone(),
            relative_path: None,
            size: photo.size.unwrap_or(0),
            skipped: false,
            identity_key: photo.identity(),
            hash: photo.content_hash().map(str::to_string),
            mtime: None,
        })
        .collect();
    let simulated_snapshot = build_snapshot(
        gallery,
        &manifest,
        &local_files,
        &previous_files,
        &diff,
        &simulated,
        &[],
        now_millis(),
    );
    let ratings = build_rating_plan(&active_photos, &simulated_snapshot.files);

    let remote_total = manifest.photos.len();
    let deletes = local_deleted.len();
    let mass_delete = deletes >= MASS_DELETE_MIN
        && (deletes > MASS_DELETE_ABSOLUTE || deletes * 5 > remote_total);

    let plan = SyncPlan {
        id: new_plan_id(),
        gallery_id: gallery.gallery_id.clone(),
        name: base_name,
        first_sync,
        downloads,
        uploads,
        raw_decision_needed: upload_plan.raw_decision_needed,
        raw_kept_local: upload_plan.raw_kept_local,
        remote_deleted: diff.remote_deleted.clone(),
        local_deleted,
        conflicts: diff.conflicts.clone(),
        ratings,
        warnings,
        errors,
        ignored_local: ignored_local
            .iter()
            .map(|file| file.filename.clone())
            .collect(),
        ignored_remote,
        remote_total,
        mass_delete,
        edited_folders,
        edited_folders_linked,
        scan_stats,
        timings,
    };
    Ok(PlannedSync {
        plan,
        context: PlanContext {
            gallery: gallery.clone(),
            snapshot_at,
            manifest,
            local_files,
            previous_files,
            diff,
            units: upload_plan.units,
            ignored_local,
            fingerprints,
        },
    })
}

/// The file at `relative_path` still has the size and mtime seen in the
/// review. Anything else means it was edited, replaced or removed since.
async fn unchanged(root: &Path, file: &LocalFile) -> bool {
    let Some(path) = safe_join_relative(root, &file.relative_path) else {
        return false;
    };
    let Ok(metadata) = tokio::fs::metadata(&path).await else {
        return false;
    };
    metadata.len() == file.size && file_mtime_ms(&path).await == Some(file.mtime_ms)
}

/// Like [`unchanged`], but also re-reads the file when the reviewed content
/// hash is known.
///
/// Used immediately before an action that destroys local bytes (overwriting a
/// photo with a new remote version, or moving it to the trash): size and mtime
/// alone can miss a same-size edit made within the filesystem timestamp
/// resolution, and that edit must never be thrown away silently.
async fn unchanged_exact(root: &Path, file: &LocalFile) -> bool {
    if !unchanged(root, file).await {
        return false;
    }
    let Some(expected) = file.hash.as_deref() else {
        return true;
    };
    let Some(path) = safe_join_relative(root, &file.relative_path) else {
        return false;
    };
    match crate::localfiles::hash_file(&path).await {
        Ok(hash) => hash == expected,
        Err(_) => false,
    }
}

/// Join a `/`-separated relative path under `root`, refusing anything that
/// would leave it.
fn safe_join_relative(root: &Path, relative: &str) -> Option<std::path::PathBuf> {
    let path = root.join(relative);
    is_inside(root, &path).then_some(path)
}

/// Create one private Picstome gallery per selected edited subfolder.
///
/// The new gallery is a sibling of the original inside the same photoshoot.
/// Before creating, the parent gallery is read again so a photoshoot removed
/// or changed in the web since the review is respected; a gallery that already
/// exists with the same name and photoshoot is linked instead of duplicated.
#[allow(clippy::too_many_arguments)]
async fn create_edited_galleries(
    api: &ApiClient,
    store: &Store,
    parent: &GalleryEntry,
    parent_name: &str,
    proposals: &[EditedFolderProposal],
    selected: &[String],
    root: &Path,
) -> (Vec<CreatedGallery>, Vec<SyncErrorItem>) {
    let mut created = Vec::new();
    let mut errors = Vec::new();
    let wanted: HashSet<&str> = selected.iter().map(String::as_str).collect();
    if wanted.is_empty() {
        return (created, errors);
    }

    let edited_error = |folder: &str, message: String| SyncErrorItem {
        phase: "edited_gallery".into(),
        filename: Some(folder.to_string()),
        path: None,
        message,
    };

    // The photoshoot is read fresh: the web may have changed it after the
    // review. Refuse to create a standalone gallery when the API is too old
    // to tell, because that would put the photos in the wrong place.
    let fresh_parent = match api.find_gallery(&parent.gallery_id, parent_name).await {
        Ok(Some(gallery)) if gallery.photoshoot_known() => gallery,
        Ok(Some(_)) => {
            errors.push(edited_error(
                "",
                "This Picstome API does not expose the gallery photoshoot; update Picstome and review again.".into(),
            ));
            return (created, errors);
        }
        Ok(None) => {
            errors.push(edited_error(
                "",
                "Could not find the original gallery in Picstome; review the changes again.".into(),
            ));
            return (created, errors);
        }
        Err(error) => {
            errors.push(edited_error(
                "",
                format!("Could not read the original gallery photoshoot: {error}"),
            ));
            return (created, errors);
        }
    };
    let photoshoot_id = fresh_parent.photoshoot();

    let index = store.load_index();
    for proposal in proposals {
        if !wanted.contains(proposal.relative_path.as_str()) {
            continue;
        }
        let folder = Path::new(&proposal.relative_path);
        if folder.components().count() != 1
            || !is_edited_gallery_folder_name(&proposal.relative_path)
        {
            errors.push(edited_error(
                &proposal.folder,
                "This folder can no longer be turned into a gallery".into(),
            ));
            continue;
        }
        let folder_path = root.join(folder);
        let Ok(metadata) = std::fs::symlink_metadata(&folder_path) else {
            errors.push(edited_error(
                &proposal.folder,
                "The edited folder no longer exists".into(),
            ));
            continue;
        };
        if !metadata.file_type().is_dir() {
            errors.push(edited_error(
                &proposal.folder,
                "The edited folder is not a regular directory anymore".into(),
            ));
            continue;
        }
        if index
            .iter()
            .any(|entry| same_local_folder(entry, &folder_path))
        {
            // Another apply or the previous review already linked it.
            continue;
        }
        let (files, _scan_errors) = scan_local_photos(&folder_path, 8, true, MAX_PHOTO_BYTES);
        if files.is_empty() {
            errors.push(edited_error(
                &proposal.folder,
                "The edited folder no longer contains photos".into(),
            ));
            continue;
        }

        // Reuse a gallery created by a previous attempt (or manually in the
        // web) instead of creating a duplicate.
        let existing = match api
            .galleries_page(Some(&proposal.gallery_name), 1, 100)
            .await
        {
            Ok(page) => page.data.into_iter().find(|gallery| {
                gallery.name == proposal.gallery_name
                    && gallery.photoshoot_known()
                    && gallery.photoshoot() == photoshoot_id
            }),
            Err(error) => {
                errors.push(edited_error(
                    &proposal.folder,
                    format!("Could not check for an existing gallery: {error}"),
                ));
                continue;
            }
        };
        let gallery = match existing {
            Some(gallery) => gallery,
            None => match api
                .create_gallery(&proposal.gallery_name, photoshoot_id)
                .await
            {
                Ok(gallery) => gallery,
                Err(error) => {
                    errors.push(edited_error(
                        &proposal.folder,
                        format!("Could not create the edited gallery: {error}"),
                    ));
                    continue;
                }
            },
        };

        let local_path = folder_path.to_string_lossy().to_string();
        let entry = GalleryEntry {
            gallery_id: gallery.id.clone(),
            name: gallery.name.clone(),
            local_path: local_path.clone(),
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
            raw_mode: parent.raw_mode,
        };
        if let Err(error) = store.upsert_gallery(entry) {
            errors.push(edited_error(
                &proposal.folder,
                format!(
                    "The gallery was created in Picstome but could not be linked locally: {error}"
                ),
            ));
            continue;
        }
        let photoshoot = gallery.photoshoot();
        created.push(CreatedGallery {
            id: gallery.id,
            name: gallery.name,
            local_path,
            photoshoot_id: photoshoot,
            photo_count: files.len() as u64,
        });
    }
    (created, errors)
}

/// Apply the ticked actions of a reviewed plan.
///
/// Order: forget/ignore choices, trash, downloads, uploads, snapshot, then
/// the Picstome-side deletions and the resolutions of photos deleted in
/// Picstome (both work on the saved snapshot). Every action is revalidated
/// first; whatever changed since the review lands in `skipped_changed`.
pub async fn apply_sync(
    api: &ApiClient,
    store: &Store,
    planned: PlannedSync,
    selection: &PlanSelection,
    options: &SyncOptions,
    progress: SyncProgressFn,
) -> CoreResult<SyncReport> {
    let PlannedSync { plan, context } = planned;
    let PlanContext {
        gallery,
        snapshot_at,
        manifest,
        local_files,
        previous_files,
        mut diff,
        units,
        ignored_local,
        mut fingerprints,
    } = context;
    let gallery_id = gallery.gallery_id.clone();
    let root = Path::new(&gallery.local_path).to_path_buf();
    let now = now_millis();
    let mut timings = plan.timings;
    // "Nothing to sync" means no file differed when the review was made and no
    // new gallery was requested.
    let mut in_sync = !diff.has_changes() && selection.create_edited.is_empty();

    if store
        .load_snapshot(&gallery_id)
        .map(|snapshot| snapshot.last_sync_at)
        != snapshot_at
    {
        return Err(CoreError::Outdated(
            "This gallery changed after the review; review the changes again".into(),
        ));
    }
    // The folder may have been relocated or relinked after the review. Applying
    // against the old path could recreate it and write photos there.
    if !store
        .get_gallery(&gallery_id)
        .is_some_and(|current| current.local_path == gallery.local_path)
    {
        return Err(CoreError::Outdated(
            "The gallery folder changed after the review; review the changes again".into(),
        ));
    }
    let deletes: HashSet<&String> = selection
        .delete_remote
        .iter()
        .filter(|id| plan.local_deleted.iter().any(|item| &item.id == *id))
        .collect();
    if plan.mass_delete
        && !deletes.is_empty()
        && selection.confirm_mass_delete != Some(deletes.len())
    {
        return Err(CoreError::InvalidData(format!(
            "Deleting {} photos in Picstome needs the number typed to confirm",
            deletes.len()
        )));
    }

    // Fresh manifest: current signed URLs, and the remote state to check
    // every action against.
    let fresh = api.manifest(&gallery_id).await?;
    let fresh_by_id: HashMap<&str, &Photo> = fresh
        .photos
        .iter()
        .map(|photo| (photo.id.as_str(), photo))
        .collect();
    let remote_unchanged = |photo: &Photo| {
        fresh_by_id
            .get(photo.id.as_str())
            .is_some_and(|current| current.version_count == photo.version_count)
    };
    let local_by_path: HashMap<&str, &LocalFile> = local_files
        .iter()
        .map(|file| (file.relative_path.as_str(), file))
        .collect();

    let mut skipped_changed: Vec<String> = Vec::new();
    let mut errors: Vec<SyncErrorItem> = Vec::new();

    // "Never propose again".
    if !selection.ignore_local.is_empty() || !selection.ignore_remote.is_empty() {
        let mut ignores = GalleryIgnores::load(&root);
        for path in &selection.ignore_local {
            if let Some(file) = local_by_path.get(path.as_str()) {
                ignores.add_local(file);
            }
        }
        for id in &selection.ignore_remote {
            if let Some(photo) = manifest.photos.iter().find(|photo| &photo.id == id) {
                ignores.add_remote(photo);
            }
        }
        ignores.save(&root)?;
    }

    // Trash local files the photographer chose to discard, with every other
    // file of the same photo beside them.
    let mut trashed_paths: HashSet<String> = HashSet::new();
    let mut trashed_local = Vec::new();
    let mut trash_targets_list = Vec::new();
    for path in &selection.trash_local {
        let Some(file) = local_by_path.get(path.as_str()) else {
            continue;
        };
        if !unchanged_exact(&root, file).await {
            skipped_changed.push(file.filename.clone());
            continue;
        }
        let Some(full) = safe_join_relative(&root, &file.relative_path) else {
            continue;
        };
        for candidate in std::iter::once(full.clone()).chain(companion_paths(&full)) {
            let relative = candidate
                .strip_prefix(&root)
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if trashed_paths.insert(relative) {
                let name = candidate
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default();
                trash_targets_list.push((String::new(), name, candidate));
            }
        }
    }
    if !trash_targets_list.is_empty() {
        let gallery_for_trash = gallery.clone();
        let names: Vec<String> = trash_targets_list
            .iter()
            .map(|(_, name, _)| name.clone())
            .collect();
        match tokio::task::spawn_blocking(move || {
            trash_targets(&gallery_for_trash, &trash_targets_list)
        })
        .await
        {
            Ok((items, failures)) => {
                trashed_local.extend(items);
                for (index, message) in failures {
                    errors.push(SyncErrorItem {
                        phase: "trash".into(),
                        filename: Some(names[index].clone()),
                        path: None,
                        message,
                    });
                }
            }
            Err(error) => errors.push(SyncErrorItem {
                phase: "trash".into(),
                filename: None,
                path: None,
                message: error.to_string(),
            }),
        }
    }

    // Downloads: the ticked ones plus photos deleted here to bring back.
    let wanted: HashSet<&String> = selection.downloads.iter().collect();
    let restore: HashSet<&String> = selection.restore.iter().collect();
    let restored: Vec<Photo> = diff
        .local_deleted
        .iter()
        .filter(|photo| restore.contains(&photo.id))
        .cloned()
        .collect();
    let previous_paths: HashMap<&str, &SnapshotFile> = previous_files
        .iter()
        .map(|file| (file.id.as_str(), file))
        .collect();
    let local_by_key = preferred_local_files(&local_files);
    let mut to_download = Vec::new();
    for photo in diff
        .remote_new
        .iter()
        .chain(diff.remote_modified.iter())
        .filter(|photo| wanted.contains(&photo.id))
        .chain(restored.iter())
    {
        if !remote_unchanged(photo) {
            skipped_changed.push(photo.filename.clone());
            continue;
        }
        let existing = photo
            .identity()
            .and_then(|key| local_by_key.get(&key).copied());
        // A new version overwrites the local file: only if that file is
        // still exactly what was reviewed, or the photographer's edits since
        // then would be lost.
        if let Some(local) = existing {
            if !unchanged_exact(&root, local).await {
                skipped_changed.push(photo.filename.clone());
                continue;
            }
        }
        let target = match previous_paths.get(photo.id.as_str()) {
            // Bring a deleted photo back where it was.
            Some(previous) if restore.contains(&photo.id) => previous
                .relative_path
                .clone()
                .and_then(|relative| {
                    safe_join_relative(&root, &relative).map(|dest| (dest, relative))
                })
                .map(|(dest, relative_path)| DownloadTarget::Path {
                    dest,
                    relative_path,
                })
                .unwrap_or_else(|| download_target(&root, photo, None)),
            _ => download_target(&root, photo, existing),
        };
        // A file that appeared at the destination after the review is not
        // overwritten.
        if let DownloadTarget::Path {
            dest,
            relative_path,
        } = &target
        {
            if existing.is_none()
                && !local_by_path.contains_key(relative_path.as_str())
                && dest.exists()
            {
                skipped_changed.push(photo.filename.clone());
                continue;
            }
        }
        let current = fresh_by_id
            .get(photo.id.as_str())
            .map(|current| (*current).clone())
            .unwrap_or_else(|| photo.clone());
        to_download.push((current, target));
    }
    let download_phase = TransferPhase::new(
        progress.clone(),
        &gallery_id,
        "download",
        to_download.len() as u64,
        to_download
            .iter()
            .map(|(photo, _)| photo.size)
            .sum::<Option<u64>>(),
    );
    let download_started = std::time::Instant::now();
    let download_results: Vec<Result<DownloadedItem, SyncErrorItem>> =
        futures::stream::iter(to_download)
            .map(|(photo, target)| {
                let http = api.http();
                let phase = download_phase.clone();
                let chunk_bytes = options.chunk_bytes;
                let parallel = options.parallel;
                async move {
                    let transfer_error = |message: String| SyncErrorItem {
                        phase: "download".into(),
                        filename: Some(photo.filename.clone()),
                        path: None,
                        message,
                    };
                    let DownloadTarget::Path {
                        dest,
                        relative_path,
                    } = target
                    else {
                        return Err(transfer_error("unsafe filename in manifest".into()));
                    };
                    let result = download_file(
                        &http,
                        &photo.url,
                        &dest,
                        DownloadOptions {
                            expected_hash: photo.content_hash().map(str::to_string),
                            expected_size: photo.size,
                            chunk_bytes,
                            parallel,
                            ..Default::default()
                        },
                        phase.file_progress(&photo.filename),
                    )
                    .await
                    .map_err(|err| transfer_error(err.to_string()))?;
                    phase.finish_file(&photo.filename);
                    Ok(DownloadedItem {
                        identity_key: photo.identity(),
                        mtime: file_mtime_ms(&dest).await,
                        id: photo.id,
                        filename: photo.filename,
                        relative_path: Some(relative_path),
                        size: result.size,
                        skipped: result.skipped,
                        hash: Some(result.hash),
                    })
                }
            })
            .buffer_unordered(options.download_concurrency.max(1))
            .collect()
            .await;
    let mut downloaded = Vec::new();
    for result in download_results {
        match result {
            Ok(item) => downloaded.push(item),
            Err(error) => errors.push(error),
        }
    }
    timings.download_ms = download_started.elapsed().as_millis() as u64;

    // Uploads: ticked, not trashed, and still exactly as reviewed.
    let wanted_uploads: HashSet<&String> = selection.uploads.iter().collect();
    let mut chosen_units = Vec::new();
    for unit in units {
        if !wanted_uploads.contains(&unit.file.relative_path)
            || trashed_paths.contains(&unit.file.relative_path)
        {
            continue;
        }
        let mut intact = unchanged(&root, &unit.file).await;
        for raw in &unit.raw_first {
            intact = intact && unchanged(&root, raw).await;
        }
        if intact {
            chosen_units.push(unit);
        } else {
            skipped_changed.push(unit.file.filename.clone());
        }
    }
    let upload_phase = TransferPhase::new(
        progress.clone(),
        &gallery_id,
        "upload",
        chosen_units
            .iter()
            .map(|unit| 1 + unit.raw_first.len() as u64)
            .sum(),
        Some(
            chosen_units
                .iter()
                .flat_map(|unit| unit.raw_first.iter().chain(std::iter::once(&unit.file)))
                .map(|file| file.size)
                .sum(),
        ),
    );
    let upload_started = std::time::Instant::now();
    let outcomes: Vec<UnitOutcome> = futures::stream::iter(chosen_units)
        .map(|unit| {
            let api = api.clone();
            let gallery_id = gallery_id.clone();
            let phase = upload_phase.clone();
            async move { upload_unit(&api, &gallery_id, &phase, unit).await }
        })
        .buffer_unordered(options.upload_concurrency.max(1))
        .collect()
        .await;
    let mut uploaded = Vec::new();
    let mut uploaded_raw = Vec::new();
    for outcome in outcomes {
        uploaded_raw.extend(outcome.raw);
        match outcome.main {
            Ok(item) => uploaded.push(item),
            Err(error) => errors.push(error),
        }
    }
    timings.upload_ms = upload_started.elapsed().as_millis() as u64;

    // Snapshot. Photos brought back count as downloads of a "new" photo;
    // undecided local deletions keep their last state (see build_snapshot).
    let snapshot_started = std::time::Instant::now();
    diff.local_deleted
        .retain(|photo| !restore.contains(&photo.id));
    diff.remote_new.extend(restored);
    // The scan happened before the trash phase: a file that was just moved to
    // the trash must not be counted (or recorded) as still present locally.
    let trashed_relative: HashSet<String> = trashed_local
        .iter()
        .filter_map(|item| {
            Path::new(&item.original_path)
                .strip_prefix(&root)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    let snapshot_local_files: Vec<LocalFile> = local_files
        .iter()
        .filter(|file| !trashed_relative.contains(&file.relative_path))
        .cloned()
        .collect();
    let mut snapshot = build_snapshot(
        &gallery,
        &manifest,
        &snapshot_local_files,
        &previous_files,
        &diff,
        &downloaded,
        &uploaded,
        now,
    );
    snapshot.local_photo_count = snapshot
        .local_photo_count
        .saturating_add(ignored_local.len() as u64);
    snapshot.local_bytes = snapshot
        .local_bytes
        .saturating_add(ignored_local.iter().map(|file| file.size).sum::<u64>());
    let rating_plan = build_rating_plan(&manifest.photos, &snapshot.files);
    store.save_snapshot(&snapshot)?;
    for item in &downloaded {
        if let (Some(hash), Some(mtime)) = (item.hash.as_deref(), item.mtime) {
            fingerprints.insert(
                item.local_relative_path(),
                item.size,
                mtime,
                None,
                None,
                hash,
            );
        }
    }
    let _ = store.save_fingerprints(&fingerprints);
    store.patch_gallery(&gallery_id, |entry| {
        entry.name = snapshot.name.clone();
        entry.last_sync_at = Some(now);
        entry.remote_rev = snapshot.remote_rev.clone();
    })?;
    timings.snapshot_ms = snapshot_started.elapsed().as_millis() as u64;

    // Edited subfolders selected in the review become sibling galleries in
    // the same photoshoot. This is a server-side create plus a local link; the
    // photos themselves are uploaded by the new gallery's own review.
    let (created_galleries, edited_errors) = create_edited_galleries(
        api,
        store,
        &gallery,
        &plan.name,
        &plan.edited_folders,
        &selection.create_edited,
        &root,
    )
    .await;
    if !created_galleries.is_empty() {
        in_sync = false;
    }
    errors.extend(edited_errors);

    // Delete in Picstome the photos deleted here, if still unchanged there
    // and still absent here.
    let mut deleted_remote = Vec::new();
    let delete_ids: Vec<String> = diff
        .local_deleted
        .iter()
        .filter(|photo| deletes.contains(&photo.id))
        .filter(|photo| {
            let still_absent = previous_paths
                .get(photo.id.as_str())
                .and_then(|file| file.relative_path.as_deref())
                .and_then(|relative| safe_join_relative(&root, relative))
                .is_none_or(|path| !path.exists());
            if remote_unchanged(photo) && still_absent {
                true
            } else {
                skipped_changed.push(photo.filename.clone());
                false
            }
        })
        .map(|photo| photo.id.clone())
        .collect();
    if !delete_ids.is_empty() {
        match apply_remote_deletions(api, store, &gallery_id, &delete_ids).await {
            Ok(response) => {
                let gone: HashSet<&String> = response
                    .deleted
                    .iter()
                    .chain(response.not_found.iter())
                    .collect();
                deleted_remote = diff
                    .local_deleted
                    .iter()
                    .filter(|photo| gone.contains(&photo.id))
                    .map(|photo| photo.filename.clone())
                    .collect();
            }
            Err(error) => errors.push(SyncErrorItem {
                phase: "remote_delete".into(),
                filename: None,
                path: None,
                message: error.to_string(),
            }),
        }
    }

    // Photos deleted in Picstome: trash, keep or upload again.
    let mut groups: HashMap<(RemoteDeleteAction, bool), Vec<String>> = HashMap::new();
    for choice in &selection.remote_deleted {
        let Some(item) = plan.remote_deleted.iter().find(|item| item.id == choice.id) else {
            continue;
        };
        // The photo is back in Picstome (restored, re-created, or re-added
        // under a new id) after the review. Trashing the local copy over a
        // stale plan would destroy the only remaining copy, so leave it alone.
        let key = item
            .identity_key
            .clone()
            .or_else(|| identity_key(&item.filename));
        let reappeared = fresh
            .photos
            .iter()
            .any(|photo| photo.id == choice.id || (key.is_some() && photo.identity() == key));
        if reappeared {
            skipped_changed.push(item.filename.clone());
            continue;
        }
        let local = item
            .relative_path
            .as_deref()
            .and_then(|path| local_by_path.get(path));
        if let Some(local) = local {
            if !unchanged_exact(&root, local).await {
                skipped_changed.push(item.filename.clone());
                continue;
            }
        }
        let include_raw = choice.include_raw && choice.action == RemoteDeleteAction::DeleteLocal;
        groups
            .entry((choice.action, include_raw))
            .or_default()
            .push(choice.id.clone());
    }
    let mut remote_deleted_resolved = 0usize;
    for ((action, include_raw), ids) in groups {
        match resolve_remote_deletions(
            api,
            store,
            &gallery,
            &ids,
            action,
            include_raw,
            now_millis(),
        )
        .await
        {
            Ok(resolution) => {
                remote_deleted_resolved += resolution.resolved.len();
                trashed_local.extend(resolution.trashed);
                errors.extend(resolution.errors);
            }
            Err(error) => errors.push(SyncErrorItem {
                phase: "remote_delete".into(),
                filename: None,
                path: None,
                message: error.to_string(),
            }),
        }
    }

    progress(progress_event(&gallery_id, "done", 1, 1));

    skipped_changed.sort();
    skipped_changed.dedup();
    Ok(SyncReport {
        gallery_id,
        name: snapshot.name,
        local_path: gallery.local_path,
        rev: snapshot.remote_rev,
        counts: diff.counts.clone(),
        downloaded,
        uploaded,
        conflicts: plan.conflicts.clone(),
        remote_deleted: plan.remote_deleted.clone(),
        proposals: DeleteProposals {
            delete_remote: plan.local_deleted.clone(),
        },
        favorites: rating_plan.needs_rating.clone(),
        rating_plan,
        created_galleries,
        errors: plan.errors.iter().cloned().chain(errors).collect(),
        warnings: plan.warnings.clone(),
        ignored_local: plan.ignored_local.clone(),
        ignored_remote: plan.ignored_remote.clone(),
        trashed_local,
        raw_kept_local: plan.raw_kept_local.clone(),
        uploaded_raw,
        raw_decision_needed: plan.raw_decision_needed,
        timings,
        scan_stats: plan.scan_stats,
        skipped: in_sync,
        plan_id: Some(plan.id.clone()),
        skipped_changed,
        deleted_remote,
        remote_deleted_resolved,
    })
}

/// The id of a plan, for matching an Apply request to the stored plan.
impl PlannedSync {
    pub fn id(&self) -> &str {
        &self.plan.id
    }

    pub fn gallery_id(&self) -> &str {
        &self.plan.gallery_id
    }
}

/// Plan and apply every download and upload, no deletions: a one-shot sync
/// without a review. Used by the smoke test, the benchmark and the engine
/// tests; the app always goes through [`plan_sync`] and [`apply_sync`].
pub async fn plan_and_apply_transfers(
    api: &ApiClient,
    store: &Store,
    gallery: &GalleryEntry,
    options: SyncOptions,
    progress: SyncProgressFn,
) -> CoreResult<SyncReport> {
    let planned = plan_sync(api, store, gallery, &options, progress.clone()).await?;
    let mut selection = PlanSelection::transfers(&planned.plan);
    selection.trash_local = options.trash_local.clone();
    // Trashed files are not uploaded.
    let trashed: HashSet<&String> = selection.trash_local.iter().collect();
    selection.uploads.retain(|path| !trashed.contains(path));
    apply_sync(api, store, planned, &selection, &options, progress).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_selection_ticks_downloads_and_uploads_only() {
        let plan = SyncPlan {
            id: "p".into(),
            gallery_id: "G".into(),
            name: "G".into(),
            first_sync: false,
            downloads: vec![PlannedDownload {
                id: "a".into(),
                filename: "a.jpg".into(),
                relative_path: "a.jpg".into(),
                size: None,
                reason: "new".into(),
            }],
            uploads: vec![PlannedUpload {
                relative_path: "sub/b.jpg".into(),
                filename: "b.jpg".into(),
                size: 1,
                reason: "new".into(),
                raw_first: vec![],
            }],
            raw_decision_needed: 0,
            raw_kept_local: vec![],
            remote_deleted: vec![],
            local_deleted: vec![DeleteProposal {
                id: "c".into(),
                filename: "c.jpg".into(),
                identity_key: None,
            }],
            conflicts: vec![],
            ratings: RatingSyncPlan::default(),
            warnings: vec![],
            errors: vec![],
            ignored_local: vec![],
            ignored_remote: vec![],
            remote_total: 3,
            mass_delete: false,
            edited_folders: vec![],
            edited_folders_linked: vec![],
            scan_stats: SyncScanStats::default(),
            timings: SyncTimings::default(),
        };
        let selection = PlanSelection::transfers(&plan);
        assert_eq!(selection.downloads, vec!["a".to_string()]);
        assert_eq!(selection.uploads, vec!["sub/b.jpg".to_string()]);
        assert!(selection.delete_remote.is_empty());
        assert!(plan.has_changes());
    }
}
