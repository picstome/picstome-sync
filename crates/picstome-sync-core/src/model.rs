use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const DEFAULT_API_BASE: &str = "https://app.picstome.com";
pub const CLIENT_NAME: &str = "picstome-sync";
/// Snapshot schema written by this engine. Snapshots from the previous schema
/// are upgraded in place so the app does not ask for a first-sync preview just
/// because new rating fields were added.
pub const SNAPSHOT_SCHEMA: u32 = 5;

/// Distinguishes a missing field from an explicit `null`: `None` means the
/// server did not send the field (older API), `Some(None)` means it sent
/// `null`. Rating writes must never clear a local value just because an older
/// server omitted the field.
fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub photos_dir: Option<String>,
    pub last_rating: Option<String>,
    pub last_update_check_at: Option<i64>,
    pub account_email: Option<String>,
    pub account_name: Option<String>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GalleryEntry {
    pub gallery_id: String,
    pub name: String,
    pub local_path: String,
    #[serde(default)]
    pub last_sync_at: Option<i64>,
    #[serde(default)]
    pub remote_rev: Option<Value>,
    #[serde(default)]
    pub parent_gallery_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default)]
    pub schema: u32,
    pub gallery_id: String,
    pub name: String,
    pub local_path: String,
    pub last_sync_at: i64,
    #[serde(default)]
    pub remote_rev: Option<Value>,
    #[serde(default)]
    pub local_photo_count: u64,
    #[serde(default)]
    pub local_bytes: u64,
    #[serde(default)]
    pub files: Vec<SnapshotFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFile {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub identity_key: Option<String>,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub hash_algo: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub mtime: Option<i64>,
    #[serde(default)]
    pub local_present: bool,
    #[serde(default)]
    pub local_hash: Option<String>,
    #[serde(default)]
    pub local_size: Option<u64>,
    #[serde(default)]
    pub version_count: Option<u64>,
    #[serde(default)]
    pub version_number: Option<u64>,
    #[serde(default)]
    pub remote_deleted: bool,
    #[serde(default)]
    pub keep_local: bool,
    /// `favorited_at` for which the photographer already answered the
    /// "favourite without a rating" prompt, so it is asked only once.
    #[serde(default)]
    pub favorite_rated_for: Option<String>,
    /// `favorited_at` reported by the last manifest, so a new heart re-opens
    /// the "apply a rating" prompt.
    #[serde(default)]
    pub favorited_at: Option<String>,
    /// Photographer rating reported by the last manifest (1-5 or none).
    #[serde(default)]
    pub rating: Option<u8>,
    /// Rating that was last written to the local file metadata. It lets the
    /// app avoid rewriting EXIF and re-prompting on every sync.
    #[serde(default)]
    pub rating_applied: Option<u8>,
    /// Favourite state reported by the last manifest (heart).
    #[serde(default)]
    pub is_favorite: bool,
}

#[derive(Debug, Clone)]
pub struct LocalFile {
    pub filename: String,
    pub relative_path: String,
    pub path: std::path::PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub hash: Option<String>,
    pub identity_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub download_hash: Option<String>,
    #[serde(default)]
    pub hash_algo: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub version_count: Option<u64>,
    #[serde(default)]
    pub version_number: Option<u64>,
    #[serde(default)]
    pub identity_key: Option<String>,
    #[serde(default)]
    pub favorited_at: Option<String>,
    /// Photographer-only stars. Double option so an older server that omits
    /// the field (`None`) is distinguishable from an explicit clear
    /// (`Some(None)`).
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub rating: Option<Option<u8>>,
    /// Heart mirror of `favorited_at`. `None` when an older server omits it.
    #[serde(default)]
    pub is_favorite: Option<bool>,
    /// Archived photos are still listed; only the prompt ignores them.
    #[serde(default)]
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub gallery_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub rev: Option<Value>,
    #[serde(default)]
    pub photos: Vec<Photo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GallerySummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub photo_count: Option<u64>,
    #[serde(default)]
    pub rev: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryPageMeta {
    #[serde(alias = "current_page")]
    pub current_page: u32,
    #[serde(alias = "per_page")]
    pub per_page: u32,
    #[serde(alias = "last_page")]
    pub last_page: u32,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryPage {
    pub data: Vec<GallerySummary>,
    pub meta: GalleryPageMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub token: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub rev: Option<Value>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub version_count: Option<u64>,
    #[serde(default)]
    pub version_number: Option<u64>,
    #[serde(default)]
    pub unchanged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResponse {
    #[serde(default)]
    pub deleted: Vec<String>,
    #[serde(default)]
    pub not_found: Vec<String>,
    #[serde(default)]
    pub rev: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffCounts {
    pub remote_new: usize,
    pub remote_modified: usize,
    pub local_new: usize,
    pub local_modified: usize,
    pub local_deleted: usize,
    pub remote_deleted: usize,
    pub conflicts: usize,
    pub in_sync: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub remote_new: Vec<Photo>,
    pub remote_modified: Vec<Photo>,
    pub local_new: Vec<LocalFile>,
    pub local_modified: Vec<LocalFile>,
    pub local_deleted: Vec<Photo>,
    pub remote_deleted: Vec<RemoteDeletedItem>,
    pub conflicts: Vec<Conflict>,
    pub in_sync: Vec<Photo>,
    pub counts: DiffCounts,
}

impl Diff {
    pub fn has_changes(&self) -> bool {
        self.counts.remote_new
            + self.counts.remote_modified
            + self.counts.local_new
            + self.counts.local_modified
            + self.counts.local_deleted
            + self.counts.remote_deleted
            + self.counts.conflicts
            > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictPhoto {
    pub filename: String,
    #[serde(default)]
    pub version_count: Option<u64>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub identity_key: String,
    pub remote: ConflictPhoto,
    pub local: ConflictPhoto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncErrorItem {
    pub phase: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncWarningItem {
    pub kind: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub preferred: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedItem {
    pub id: String,
    pub filename: String,
    pub size: u64,
    pub skipped: bool,
    #[serde(default)]
    pub identity_key: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub mtime: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedItem {
    pub id: Option<String>,
    pub filename: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub version_count: Option<u64>,
    pub version_number: Option<u64>,
    #[serde(default)]
    pub identity_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProposal {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub identity_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProposals {
    pub delete_remote: Vec<DeleteProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeletedItem {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub identity_key: Option<String>,
    #[serde(default)]
    pub relative_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoritePhoto {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub identity_key: Option<String>,
    #[serde(default)]
    pub favorited_at: Option<String>,
    /// Photographer stars, when the manifest includes them.
    #[serde(default)]
    pub rating: Option<u8>,
    #[serde(default)]
    pub archived: bool,
}

/// One photo whose Picstome rating has to be written to the local file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingSyncItem {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub identity_key: Option<String>,
    /// Stars in Picstome (1-5). `None` means unrated.
    #[serde(default)]
    pub rating: Option<u8>,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub archived: bool,
    /// Stars previously written to the local file, when known.
    #[serde(default)]
    pub applied_rating: Option<u8>,
}

/// What the desktop still has to apply after a sync.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingSyncPlan {
    /// Photos whose Picstome rating differs from the local EXIF value.
    #[serde(default)]
    pub items: Vec<RatingSyncItem>,
    /// Favourites with a heart but no rating: the photographer is asked once
    /// for a star value, then it is written to all of them.
    #[serde(default)]
    pub needs_rating: Vec<FavoritePhoto>,
}

impl RatingSyncPlan {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.needs_rating.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedFile {
    pub id: String,
    pub filename: String,
    pub original_path: String,
    pub location: String,
    #[serde(default)]
    pub trash_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub gallery_id: String,
    pub name: String,
    pub local_path: String,
    #[serde(default)]
    pub rev: Option<Value>,
    pub counts: DiffCounts,
    pub downloaded: Vec<DownloadedItem>,
    pub uploaded: Vec<UploadedItem>,
    pub conflicts: Vec<Conflict>,
    pub remote_deleted: Vec<RemoteDeletedItem>,
    pub proposals: DeleteProposals,
    pub favorites: Vec<FavoritePhoto>,
    /// Picstome ratings that still have to be written to the local files.
    #[serde(default)]
    pub rating_plan: RatingSyncPlan,
    pub errors: Vec<SyncErrorItem>,
    #[serde(default)]
    pub warnings: Vec<SyncWarningItem>,
    #[serde(default)]
    pub ignored_local: Vec<String>,
    #[serde(default)]
    pub ignored_remote: Vec<String>,
    #[serde(default)]
    pub trashed_local: Vec<TrashedFile>,
    pub skipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub gallery_id: String,
    pub phase: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub done: u64,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub file_done: Option<u64>,
    #[serde(default)]
    pub file_total: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub connected: bool,
    pub email: Option<String>,
    pub name: Option<String>,
    pub account_email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryView {
    #[serde(flatten)]
    pub entry: GalleryEntry,
    pub exists: bool,
    pub photo_count: u64,
    pub local_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub lang: String,
    pub strings: Value,
    pub config: ConfigView,
    pub mock: bool,
    pub auth: AuthState,
    pub galleries: Vec<GalleryView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub api_base: String,
    pub api_kind: String,
    pub photos_dir: Option<String>,
    pub last_rating: String,
    pub app_version: String,
    pub build_profile: String,
}

impl Manifest {
    /// Every photo that carries a heart, including legacy favourites that have
    /// no rating yet (the case the photographer has to answer).
    pub fn favorite_photos(&self) -> Vec<FavoritePhoto> {
        self.photos
            .iter()
            .filter(|photo| photo.has_heart())
            .map(|photo| FavoritePhoto {
                id: photo.id.clone(),
                filename: photo.filename.clone(),
                identity_key: photo.identity(),
                favorited_at: photo.favorited_at.clone(),
                rating: photo.rating_value(),
                archived: photo.archived_at.is_some(),
            })
            .collect()
    }
}

impl Photo {
    /// Hash of the bytes served by the download URL. The manifest `hash`
    /// field is the upload/source hash and may not match processed files.
    pub fn content_hash(&self) -> Option<&str> {
        self.download_hash.as_deref()
    }

    pub fn identity(&self) -> Option<String> {
        self.identity_key
            .clone()
            .or_else(|| crate::identity::identity_key(&self.filename))
    }

    /// Stars reported by the manifest. A field omitted by an older server is
    /// treated as "not rated" here; the sync engine keeps any stored value.
    pub fn rating_value(&self) -> Option<u8> {
        self.rating.flatten()
    }

    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }

    /// The heart, from `is_favorite` when present and `favorited_at` otherwise.
    /// Five stars implies the heart, matching the web rules.
    pub fn has_heart(&self) -> bool {
        self.is_favorite
            .unwrap_or_else(|| self.favorited_at.is_some())
            || self.rating_value() == Some(5)
    }

    /// A favourite the photographer never rated: the prompt has to ask for a
    /// star value instead of silently assuming five. Archived photos are asked
    /// too: archiving is not a rating decision.
    pub fn needs_rating(&self) -> bool {
        self.has_heart() && self.rating_value().is_none()
    }
}

impl SnapshotFile {
    pub fn identity(&self) -> Option<String> {
        self.identity_key
            .clone()
            .or_else(|| crate::identity::identity_key(&self.filename))
    }

    /// Refresh the rating/heart reported by the manifest. A manifest that
    /// omits `rating` (older server) leaves the stored value untouched, while
    /// an explicit `null` clears it. The locally applied value is carried over
    /// so a re-sync does not ask again.
    pub fn apply_remote_rating(
        &mut self,
        photo: &Photo,
        rating_applied: Option<u8>,
        favorite_rated_for: Option<String>,
    ) {
        if let Some(rating) = photo.rating {
            self.rating = rating;
        }
        // Prefer the explicit flag. When the server omits it but does speak the
        // ratings contract, `favorited_at` is authoritative (it is always
        // serialized, null included). Only a server that sends neither keeps
        // the stored heart, so an older API cannot un-favourite everything.
        let heart = match (photo.is_favorite, photo.rating) {
            (Some(flag), _) => Some(flag),
            (None, Some(_)) => Some(photo.favorited_at.is_some()),
            (None, None) => photo.favorited_at.as_ref().map(|_| true),
        };
        if let Some(heart) = heart {
            self.is_favorite = heart;
        }
        self.rating_applied = rating_applied;
        self.favorite_rated_for = favorite_rated_for;
    }
}

impl LocalFile {
    pub fn identity(&self) -> Option<String> {
        if self.identity_key.is_empty() {
            crate::identity::identity_key(&self.filename)
        } else {
            Some(self.identity_key.clone())
        }
    }
}

/// Human-readable classification of the API target shown in the in-app info
/// panel, so testers can immediately see whether they are on dev or production.
pub fn api_kind(base_url: &str, mock: bool) -> &'static str {
    if mock {
        return "demo";
    }
    let normalized = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
    if normalized == DEFAULT_API_BASE
        || normalized == "https://picstome.com"
        || normalized.contains("://app.picstome.com")
    {
        "production"
    } else if normalized.contains("devel.") || normalized.contains("staging.") {
        "development"
    } else if normalized.contains("localhost") || normalized.contains("127.0.0.1") {
        "local"
    } else {
        "custom"
    }
}

/// Validates and normalizes the API base URL used for a test target.
pub fn normalize_api_base(value: &str) -> crate::error::CoreResult<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(crate::error::CoreError::InvalidData("empty API URL".into()));
    }
    let parsed = url::Url::parse(trimmed).map_err(crate::error::CoreError::from)?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(crate::error::CoreError::InvalidData(
            "API URL must be http(s)".into(),
        ));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_api_targets() {
        assert_eq!(api_kind("https://app.picstome.com", false), "production");
        assert_eq!(api_kind("https://app.picstome.com/", false), "production");
        assert_eq!(api_kind("https://picstome.com", false), "production");
        assert_eq!(api_kind("https://devel.picstome.com", false), "development");
        assert_eq!(
            api_kind("https://staging.picstome.com", false),
            "development"
        );
        assert_eq!(api_kind("http://127.0.0.1:8000", false), "local");
        assert_eq!(api_kind("http://localhost:8787", false), "local");
        assert_eq!(api_kind("https://sync.example.test", false), "custom");
        assert_eq!(api_kind("https://anywhere", true), "demo");
    }

    #[test]
    fn validates_api_urls() {
        assert_eq!(
            normalize_api_base(" https://devel.picstome.com/ ").unwrap(),
            "https://devel.picstome.com"
        );
        assert_eq!(
            normalize_api_base("http://127.0.0.1:8787").unwrap(),
            "http://127.0.0.1:8787"
        );
        assert!(normalize_api_base("").is_err());
        assert!(normalize_api_base("ftp://example.test").is_err());
        assert!(normalize_api_base("not a url").is_err());
    }

    #[test]
    fn manifest_favorites_keep_identity_and_timestamp() {
        let manifest = Manifest {
            gallery_id: "G1".into(),
            name: "Gallery".into(),
            web_url: None,
            rev: None,
            photos: vec![
                Photo {
                    id: "p1".into(),
                    filename: "IMG_0001.JPG".into(),
                    size: None,
                    hash: None,
                    download_hash: None,
                    hash_algo: None,
                    url: String::new(),
                    updated_at: None,
                    version_count: None,
                    version_number: None,
                    identity_key: None,
                    favorited_at: Some("2026-01-01T12:00:00Z".into()),
                    rating: Some(Some(5)),
                    is_favorite: Some(true),
                    archived_at: None,
                },
                Photo {
                    id: "p2".into(),
                    filename: "IMG_0002.JPG".into(),
                    size: None,
                    hash: None,
                    download_hash: None,
                    hash_algo: None,
                    url: String::new(),
                    updated_at: None,
                    version_count: None,
                    version_number: None,
                    identity_key: None,
                    favorited_at: None,
                    rating: None,
                    is_favorite: None,
                    archived_at: None,
                },
            ],
        };

        let favorites = manifest.favorite_photos();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].id, "p1");
        assert_eq!(favorites[0].filename, "IMG_0001.JPG");
        assert_eq!(favorites[0].identity_key.as_deref(), Some("image:img_0001"));
        assert_eq!(
            favorites[0].favorited_at.as_deref(),
            Some("2026-01-01T12:00:00Z")
        );
        assert_eq!(favorites[0].rating, Some(5));
    }

    #[test]
    fn legacy_favourite_without_rating_is_flagged() {
        let photo = Photo {
            id: "p1".into(),
            filename: "a.jpg".into(),
            size: None,
            hash: None,
            download_hash: None,
            hash_algo: None,
            url: String::new(),
            updated_at: None,
            version_count: None,
            version_number: None,
            identity_key: None,
            favorited_at: Some("2026-01-01T12:00:00Z".into()),
            rating: Some(None),
            is_favorite: Some(true),
            archived_at: None,
        };
        assert!(photo.has_heart());
        assert!(photo.needs_rating());

        // Five stars implies the heart even when the server forgets the flag.
        let rated = Photo {
            rating: Some(Some(5)),
            is_favorite: None,
            favorited_at: None,
            ..photo.clone()
        };
        assert!(rated.has_heart());
        assert!(!rated.needs_rating());

        // Archived photos are still rated, and the prompt asks for them too:
        // archiving is not a rating decision.
        let archived = Photo {
            archived_at: Some("2026-02-01T10:00:00Z".into()),
            ..photo.clone()
        };
        assert!(archived.needs_rating());
    }

    #[test]
    fn missing_rating_field_keeps_the_stored_value() {
        // `rating` absent must not be confused with an explicit null: an older
        // server cannot wipe a value the app already knows.
        let photo: Photo = serde_json::from_value(serde_json::json!({
            "id": "1",
            "filename": "a.jpg",
        }))
        .unwrap();
        assert_eq!(photo.rating, None);
        assert_eq!(photo.is_favorite, None);

        let cleared: Photo = serde_json::from_value(serde_json::json!({
            "id": "1",
            "filename": "a.jpg",
            "rating": null,
            "is_favorite": false,
        }))
        .unwrap();
        assert_eq!(cleared.rating, Some(None));
        assert_eq!(cleared.is_favorite, Some(false));

        let mut file = SnapshotFile {
            id: "1".into(),
            filename: "a.jpg".into(),
            identity_key: None,
            relative_path: None,
            hash: None,
            hash_algo: None,
            size: None,
            mtime: None,
            local_present: true,
            local_hash: None,
            local_size: None,
            version_count: None,
            version_number: None,
            remote_deleted: false,
            keep_local: false,
            favorite_rated_for: None,
            favorited_at: None,
            rating: Some(4),
            rating_applied: Some(3),
            is_favorite: true,
        };
        // Absent field: nothing changes.
        file.apply_remote_rating(&photo, Some(3), None);
        assert_eq!(file.rating, Some(4));

        // Explicit clear: both the rating and the heart go away.
        file.apply_remote_rating(&cleared, Some(4), None);
        assert_eq!(file.rating, None);
        assert!(!file.is_favorite);
    }

    #[test]
    fn config_view_serializes_runtime_info_as_camel_case() {
        let view = ConfigView {
            api_base: "https://devel.picstome.com".into(),
            api_kind: "development".into(),
            photos_dir: None,
            last_rating: "1".into(),
            app_version: "0.1.0".into(),
            build_profile: "debug".into(),
        };
        let json = serde_json::to_value(view).unwrap();
        assert_eq!(json["apiBase"], "https://devel.picstome.com");
        assert_eq!(json["apiKind"], "development");
        assert_eq!(json["appVersion"], "0.1.0");
        assert_eq!(json["buildProfile"], "debug");
    }
}
