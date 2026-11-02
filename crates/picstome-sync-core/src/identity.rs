use std::path::Path;

pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "dng"];
pub const RAW_EXTS: &[&str] = &["cr3", "cr2", "arw", "nef", "raf", "orf", "rw2"];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "m4v"];
pub const DOC_EXTS: &[&str] = &["pdf"];
pub const PHOTO_EXTS: &[&str] = &[
    "jpg", "jpeg", "dng", "cr3", "cr2", "arw", "nef", "raf", "orf", "rw2", "mp4", "mov", "avi",
    "mkv", "webm", "m4v", "pdf",
];

pub fn extension_of(filename: &str) -> String {
    Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn stem_of(filename: &str) -> String {
    Path::new(filename)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn family_of(filename: &str) -> &'static str {
    let ext = extension_of(filename);
    if VIDEO_EXTS.contains(&ext.as_str()) {
        "video"
    } else if DOC_EXTS.contains(&ext.as_str()) {
        "pdf"
    } else {
        "image"
    }
}

pub fn identity_key(filename: &str) -> Option<String> {
    let stem = stem_of(filename);
    if stem.is_empty() {
        return None;
    }
    Some(format!("{}:{}", family_of(filename), stem))
}

pub fn is_photo_file(filename: &str) -> bool {
    PHOTO_EXTS.contains(&extension_of(filename).as_str())
}

fn file_rank(filename: &str) -> u8 {
    match extension_of(filename).as_str() {
        "jpg" | "jpeg" | "png" => 0,
        "dng" => 1,
        "mp4" | "mov" => 2,
        _ => 3,
    }
}

pub fn preferred_file<T, F>(files: &[T], filename: F) -> Option<&T>
where
    F: Fn(&T) -> &str,
{
    files.iter().min_by(|a, b| {
        let ra = file_rank(filename(a));
        let rb = file_rank(filename(b));
        ra.cmp(&rb).then_with(|| filename(a).cmp(filename(b)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_image_extensions() {
        assert_eq!(
            identity_key("1Z8A4331.JPG").as_deref(),
            Some("image:1z8a4331")
        );
        assert_eq!(
            identity_key("1Z8A4331.jpeg").as_deref(),
            Some("image:1z8a4331")
        );
        assert_eq!(
            identity_key("1Z8A4331.DNG").as_deref(),
            Some("image:1z8a4331")
        );
    }

    #[test]
    fn separates_video_and_pdf() {
        assert_eq!(identity_key("clip.MP4").as_deref(), Some("video:clip"));
        assert_eq!(identity_key("invoice.PDF").as_deref(), Some("pdf:invoice"));
    }

    #[test]
    fn picks_jpeg_over_raw() {
        let files = ["a.DNG", "a.jpg"];
        let chosen = preferred_file(&files, |f| f).unwrap();
        assert_eq!(chosen.to_ascii_lowercase(), "a.jpg");
    }
}
