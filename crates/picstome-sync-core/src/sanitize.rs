use std::path::{Component, Path, PathBuf};

pub fn sanitize_file_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 255 {
        return None;
    }
    if trimmed == "." || trimmed == ".." {
        return None;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return None;
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn sanitize_file_names<S: AsRef<str>>(names: &[S]) -> Option<Vec<String>> {
    if names.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        out.push(sanitize_file_name(name.as_ref())?);
    }
    Some(out)
}

pub fn sanitize_gallery_id(id: &str) -> Option<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn is_inside(parent: &Path, candidate: &Path) -> bool {
    let parent = lexical_absolute(parent);
    let candidate = lexical_absolute(candidate);
    candidate.starts_with(&parent)
}

pub fn safe_join(parent: &Path, filename: &str) -> Option<PathBuf> {
    let name = sanitize_file_name(filename)?;
    let target = parent.join(name);
    if is_inside(parent, &target) {
        Some(target)
    } else {
        None
    }
}

pub fn lexical_absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };

    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn escape_header_value(value: &str) -> String {
    value.replace(['"', '\r', '\n'], "_")
}

pub fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_names() {
        assert_eq!(
            sanitize_file_name("1Z8A4331.jpg").as_deref(),
            Some("1Z8A4331.jpg")
        );
        assert_eq!(
            sanitize_file_name("  foto final .JPG ").as_deref(),
            Some("foto final .JPG")
        );
    }

    #[test]
    fn rejects_traversal_and_controls() {
        assert!(sanitize_file_name("../x.jpg").is_none());
        assert!(sanitize_file_name("a/b.jpg").is_none());
        assert!(sanitize_file_name("a\\b.jpg").is_none());
        assert!(sanitize_file_name("..").is_none());
        assert!(sanitize_file_name("bad\u{0}name.jpg").is_none());
        assert!(sanitize_file_name("").is_none());
        assert!(sanitize_file_name(&"x".repeat(256)).is_none());
    }

    #[test]
    fn validates_gallery_ids() {
        assert_eq!(sanitize_gallery_id("G123").as_deref(), Some("G123"));
        assert!(sanitize_gallery_id("../../etc").is_none());
        assert!(sanitize_gallery_id("G 123").is_none());
    }

    #[test]
    fn joins_stay_inside() {
        let base = Path::new("/tmp/picstome-test");
        assert!(is_inside(base, &base.join("a.jpg")));
        assert!(!is_inside(base, &base.join("../a.jpg")));
        assert_eq!(safe_join(base, "a.jpg"), Some(base.join("a.jpg")));
        assert!(safe_join(base, "../a.jpg").is_none());
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn lexical_absolute_normalizes_relative_paths() {
        let path = lexical_absolute(Path::new("a/./b/../c"));
        assert!(path.is_absolute());
        assert!(path.ends_with("a/c"));
    }

    #[test]
    fn escapes_header_values() {
        assert_eq!(escape_header_value("a\"b\r\nc"), "a_b__c");
    }

    #[test]
    fn urlencode_escapes_reserved_characters() {
        assert_eq!(urlencode("a b/c"), "a+b%2Fc");
    }
}
