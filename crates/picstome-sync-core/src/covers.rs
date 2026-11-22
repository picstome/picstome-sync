//! Gallery cover thumbnails for linked galleries.
//!
//! Picstome exposes `cover_url` in the gallery list: the gallery cover, or its
//! first photo when no cover was chosen. The URL is signed and expires, so the
//! image itself is cached in `cache/covers/<gallery-id>.img` next to a small
//! JSON file describing where it came from. The sidebar and the home page then
//! show covers instantly and offline; the cache is refreshed in the background
//! every few days, and only re-downloaded when the cover actually changed.
//!
//! When Picstome has no image for the gallery, or cannot be reached and
//! nothing is cached yet, the first JPEG/PNG in the linked folder is reduced to
//! a thumbnail instead, so a synced gallery is recognisable even offline.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::atomic::{write_bytes, write_json_compact_atomic};
use crate::error::{CoreError, CoreResult};

/// A cached cover older than this is checked against Picstome again.
pub const COVER_REFRESH_MS: i64 = 3 * 24 * 60 * 60 * 1000;
/// Covers are small thumbnails; anything bigger is not what we asked for.
pub const MAX_COVER_BYTES: usize = 2 * 1024 * 1024;
/// Longest side of a thumbnail generated from a local photo.
pub const LOCAL_THUMB_PX: u32 = 240;
/// Local photos larger than this are not decoded for a thumbnail.
const MAX_LOCAL_SOURCE_BYTES: u64 = 60 * 1024 * 1024;
/// Gallery list pages scanned when a name search does not find the gallery
/// (for example after it was renamed in Picstome).
const FALLBACK_PAGES: u32 = 3;

/// An image ready to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cover {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Where a cached cover came from and when it was last confirmed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverMeta {
    /// Cover URL without its query string (the signature changes on every
    /// request, the image path only when the cover changes).
    pub source: String,
    pub mime: String,
    pub checked_at: i64,
}

/// On-disk cover cache, one image per gallery.
#[derive(Debug, Clone)]
pub struct CoverCache {
    dir: PathBuf,
}

impl CoverCache {
    /// Cache rooted at `<app data>/cache/covers`.
    pub fn new(app_root: &Path) -> Self {
        Self {
            dir: app_root.join("cache").join("covers"),
        }
    }

    fn image_path(&self, gallery_id: &str) -> PathBuf {
        self.dir.join(format!("{gallery_id}.img"))
    }

    fn meta_path(&self, gallery_id: &str) -> PathBuf {
        self.dir.join(format!("{gallery_id}.json"))
    }

    /// The cached cover, if both the image and its metadata are readable.
    pub fn load(&self, gallery_id: &str) -> Option<(Cover, CoverMeta)> {
        let meta: CoverMeta =
            serde_json::from_slice(&std::fs::read(self.meta_path(gallery_id)).ok()?).ok()?;
        let bytes = std::fs::read(self.image_path(gallery_id)).ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some((
            Cover {
                mime: meta.mime.clone(),
                bytes,
            },
            meta,
        ))
    }

    /// Store a cover. The image is written before its metadata, so a crash in
    /// between leaves an entry that `load` rejects rather than a wrong image.
    pub fn save(&self, gallery_id: &str, source: &str, cover: &Cover, now: i64) -> CoreResult<()> {
        write_bytes(&self.image_path(gallery_id), &cover.bytes)?;
        write_json_compact_atomic(
            &self.meta_path(gallery_id),
            &CoverMeta {
                source: source.to_string(),
                mime: cover.mime.clone(),
                checked_at: now,
            },
        )
    }

    /// Record that the cached cover is still current.
    pub fn touch(&self, gallery_id: &str, meta: &CoverMeta, now: i64) {
        let _ = write_json_compact_atomic(
            &self.meta_path(gallery_id),
            &CoverMeta {
                checked_at: now,
                ..meta.clone()
            },
        );
    }

    /// Forget a gallery's cover (unlinked, or the gallery has no photos).
    pub fn remove(&self, gallery_id: &str) {
        let _ = std::fs::remove_file(self.image_path(gallery_id));
        let _ = std::fs::remove_file(self.meta_path(gallery_id));
    }
}

/// Cover URL without query or fragment: stable across re-signed URLs.
pub fn source_key(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or(url).to_string()
}

fn is_stale(meta: &CoverMeta, now: i64) -> bool {
    now.saturating_sub(meta.checked_at) > COVER_REFRESH_MS
}

/// Ask Picstome for a gallery's current cover URL.
///
/// The gallery list has no id filter, so search by name first (one small
/// request) and fall back to the first pages of the list. `Ok(None)` means the
/// gallery exists but has no image to show.
pub async fn find_cover_url(
    api: &ApiClient,
    gallery_id: &str,
    name: &str,
) -> CoreResult<Option<String>> {
    let search = Some(name).filter(|value| !value.trim().is_empty());
    let mut queries: Vec<(Option<&str>, u32)> = Vec::new();
    if search.is_some() {
        queries.push((search, 1));
    }
    queries.extend((1..=FALLBACK_PAGES).map(|page| (None, page)));

    for (search, page) in queries {
        let result = api.galleries_page(search, page, 100).await?;
        if let Some(gallery) = result.data.iter().find(|gallery| gallery.id == gallery_id) {
            return Ok(gallery.cover_url.clone().filter(|url| !url.is_empty()));
        }
        if search.is_none() && page >= result.meta.last_page {
            break;
        }
    }
    Err(CoreError::InvalidData(format!(
        "gallery {gallery_id} not found in the gallery list"
    )))
}

/// Download a cover image, refusing anything that is not a small image.
pub async fn fetch_cover(http: &reqwest::Client, url: &str) -> CoreResult<Cover> {
    let response = http
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(CoreError::Network(format!(
            "cover download failed (HTTP {})",
            response.status().as_u16()
        )));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_lowercase()
        })
        .unwrap_or_default();
    if !mime.starts_with("image/") {
        return Err(CoreError::InvalidData(format!(
            "cover is not an image ({mime})"
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length as usize > MAX_COVER_BYTES)
    {
        return Err(CoreError::InvalidData("cover is too large".into()));
    }
    let bytes = response.bytes().await?;
    if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES {
        return Err(CoreError::InvalidData("cover has an invalid size".into()));
    }
    Ok(Cover {
        mime,
        bytes: bytes.to_vec(),
    })
}

/// Thumbnail of the first JPEG/PNG photo in `folder` (sorted by path), with
/// the cache source key that identifies that exact file version.
///
/// RAW files are skipped: decoding them would need a RAW library, and a
/// shoot almost always has a JPEG next to them.
pub fn local_cover(folder: &Path) -> Option<(String, Cover)> {
    let (files, _) = crate::localfiles::scan_local_photos(folder, 4, true, MAX_LOCAL_SOURCE_BYTES);
    files
        .iter()
        .filter(|file| {
            matches!(
                crate::identity::extension_of(&file.filename).as_str(),
                "jpg" | "jpeg" | "png"
            )
        })
        .find_map(|file| {
            let thumb = thumbnail(&std::fs::read(&file.path).ok()?)?;
            let source = format!(
                "local:{}:{}:{}",
                file.relative_path, file.size, file.mtime_ms
            );
            Some((source, thumb))
        })
}

/// Decode an image and re-encode it as a small JPEG thumbnail.
fn thumbnail(bytes: &[u8]) -> Option<Cover> {
    let image = image::load_from_memory(bytes).ok()?;
    let small = image.thumbnail(LOCAL_THUMB_PX, LOCAL_THUMB_PX).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
        .encode_image(&small)
        .ok()?;
    Some(Cover {
        mime: "image/jpeg".into(),
        bytes: out.into_inner(),
    })
}

/// Use (and cache) the local thumbnail, unless the cached one already comes
/// from the same file version.
fn local_fallback(
    cache: &CoverCache,
    gallery_id: &str,
    folder: &Path,
    cached: Option<(Cover, CoverMeta)>,
    now: i64,
) -> Option<Cover> {
    let Some((source, cover)) = local_cover(folder) else {
        return cached.map(|(cover, _)| cover);
    };
    if let Some((cached_cover, meta)) = cached {
        if meta.source == source {
            cache.touch(gallery_id, &meta, now);
            return Some(cached_cover);
        }
    }
    let _ = cache.save(gallery_id, &source, &cover, now);
    Some(cover)
}

/// The cover to show for a linked gallery.
///
/// A fresh cache entry is returned without touching the network. Otherwise
/// Picstome is asked for the current cover; the image is downloaded only when
/// its source changed. When Picstome cannot be reached, a stale cover is still
/// better than none, so it is returned as is.
pub async fn resolve_cover(
    api: &ApiClient,
    cache: &CoverCache,
    gallery_id: &str,
    name: &str,
    folder: &Path,
    now: i64,
) -> Option<Cover> {
    let cached = cache.load(gallery_id);
    if let Some((cover, meta)) = &cached {
        if !is_stale(meta, now) {
            return Some(cover.clone());
        }
    }

    let url = match find_cover_url(api, gallery_id, name).await {
        Ok(Some(url)) => url,
        // Picstome has no image for this gallery: show a local photo.
        Ok(None) => return local_thumbnail(cache, gallery_id, folder, cached, now).await,
        // Offline: a cached cover (even stale) wins; otherwise a local photo.
        Err(_) => {
            return match cached {
                Some((cover, _)) => Some(cover),
                None => local_thumbnail(cache, gallery_id, folder, None, now).await,
            }
        }
    };
    let source = source_key(&url);
    if let Some((cover, meta)) = &cached {
        if meta.source == source {
            cache.touch(gallery_id, meta, now);
            return Some(cover.clone());
        }
    }
    match fetch_cover(&api.http(), &url).await {
        Ok(cover) => {
            let _ = cache.save(gallery_id, &source, &cover, now);
            Some(cover)
        }
        Err(_) => match cached {
            Some((cover, _)) => Some(cover),
            None => local_thumbnail(cache, gallery_id, folder, None, now).await,
        },
    }
}

/// [`local_fallback`] on the blocking pool: it walks the folder and decodes
/// a photo.
async fn local_thumbnail(
    cache: &CoverCache,
    gallery_id: &str,
    folder: &Path,
    cached: Option<(Cover, CoverMeta)>,
    now: i64,
) -> Option<Cover> {
    let (cache, gallery_id, folder) = (cache.clone(), gallery_id.to_string(), folder.to_path_buf());
    tokio::task::spawn_blocking(move || local_fallback(&cache, &gallery_id, &folder, cached, now))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cover() -> Cover {
        Cover {
            mime: "image/jpeg".into(),
            bytes: vec![0xFF, 0xD8, 0xFF, 1, 2, 3],
        }
    }

    #[test]
    fn source_key_ignores_signatures() {
        assert_eq!(
            source_key("https://cdn.test/t/a.jpg?expires=1&sig=x"),
            "https://cdn.test/t/a.jpg"
        );
        assert_eq!(
            source_key("https://cdn.test/t/a.jpg"),
            "https://cdn.test/t/a.jpg"
        );
    }

    #[test]
    fn saves_loads_touches_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverCache::new(dir.path());
        assert!(cache.load("G1").is_none());

        cache
            .save("G1", "https://cdn.test/a.jpg", &cover(), 10)
            .unwrap();
        let (loaded, meta) = cache.load("G1").unwrap();
        assert_eq!(loaded, cover());
        assert_eq!(meta.checked_at, 10);
        assert!(!is_stale(&meta, 10 + COVER_REFRESH_MS));
        assert!(is_stale(&meta, 11 + COVER_REFRESH_MS));

        cache.touch("G1", &meta, 99);
        assert_eq!(cache.load("G1").unwrap().1.checked_at, 99);

        cache.remove("G1");
        assert!(cache.load("G1").is_none());
    }

    #[test]
    fn builds_a_thumbnail_from_the_first_local_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(800, 600, image::Rgb([200, 40, 40]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.path().join("B.png"), png.get_ref()).unwrap();
        std::fs::write(dir.path().join("A.CR3"), b"raw").unwrap();

        let (source, cover) = local_cover(dir.path()).expect("thumbnail");
        assert!(source.starts_with("local:B.png:"), "{source}");
        assert_eq!(cover.mime, "image/jpeg");
        let decoded = image::load_from_memory(&cover.bytes).unwrap();
        assert_eq!(decoded.width(), LOCAL_THUMB_PX);
        assert_eq!(decoded.height(), 180);
    }

    #[test]
    fn rejects_metadata_without_image() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverCache::new(dir.path());
        cache.save("G1", "s", &cover(), 1).unwrap();
        std::fs::remove_file(cache.image_path("G1")).unwrap();
        assert!(cache.load("G1").is_none());
    }
}
