use std::collections::HashMap;

use url::Url;

use crate::sanitize::{sanitize_file_names, sanitize_gallery_id};

pub const MAX_FILES: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    Rate {
        files: Vec<String>,
        rating: String,
        gallery: Option<String>,
    },
    Sync {
        gallery_id: String,
        name: String,
    },
    Auth {
        code: String,
        state: String,
    },
}

pub fn normalize_rating(value: Option<&str>) -> String {
    match value.map(str::trim) {
        Some("1") => "1",
        Some("2") => "2",
        Some("3") => "3",
        Some("4") => "4",
        Some("5") => "5",
        _ => "1",
    }
    .to_string()
}

fn clean_text(value: Option<&String>, max_len: usize) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() || trimmed.len() > max_len {
        return None;
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(trimmed.to_string())
}

fn clean_token(value: Option<&String>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn parse_deep_link(raw: &str) -> Option<DeepLink> {
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "picstome" {
        return None;
    }

    let host = url.host_str()?.to_ascii_lowercase();
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();

    match host.as_str() {
        "rate" => {
            let files: Vec<String> = query
                .get("files")
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();

            if files.is_empty() || files.len() > MAX_FILES {
                return None;
            }
            let files = sanitize_file_names(&files)?;
            let rating = normalize_rating(query.get("rating").map(String::as_str));
            let gallery = query
                .get("gallery")
                .and_then(|value| sanitize_gallery_id(value));

            Some(DeepLink::Rate {
                files,
                rating,
                gallery,
            })
        }
        "sync" => {
            let gallery_id = sanitize_gallery_id(query.get("gallery")?)?;
            let name = clean_text(query.get("name"), 200).unwrap_or_else(|| "Gallery".to_string());
            Some(DeepLink::Sync { gallery_id, name })
        }
        "auth" => {
            let code = clean_token(query.get("code"))?;
            let state = clean_token(query.get("state"))?;
            Some(DeepLink::Auth { code, state })
        }
        _ => None,
    }
}

pub fn build_deep_link(kind: &str, params: &[(&str, &str)]) -> String {
    let mut url = Url::parse(&format!("picstome://{kind}")).expect("static scheme");
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in params {
            if !value.is_empty() {
                pairs.append_pair(key, value);
            }
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rate_link() {
        let link = parse_deep_link("picstome://rate?files=1Z8A4331,1Z8A4373&rating=4");
        assert_eq!(
            link,
            Some(DeepLink::Rate {
                files: vec!["1Z8A4331".into(), "1Z8A4373".into()],
                rating: "4".into(),
                gallery: None,
            })
        );
    }

    #[test]
    fn rate_defaults_and_rejects_bad_files() {
        assert_eq!(
            normalize_rating(None),
            "1",
            "rating should default to one star"
        );
        assert!(parse_deep_link("picstome://rate?files=../secret").is_none());
        assert!(parse_deep_link("picstome://rate?files=a%2Fb").is_none());
        assert!(parse_deep_link("picstome://rate?files=.").is_none());
    }

    #[test]
    fn parses_sync_and_auth_links() {
        assert_eq!(
            parse_deep_link("picstome://sync?gallery=G123&name=MiBoda"),
            Some(DeepLink::Sync {
                gallery_id: "G123".into(),
                name: "MiBoda".into(),
            })
        );
        assert!(parse_deep_link("picstome://sync?name=NoGallery").is_none());
        assert_eq!(
            parse_deep_link("picstome://auth?code=abc123&state=nonce_1"),
            Some(DeepLink::Auth {
                code: "abc123".into(),
                state: "nonce_1".into(),
            })
        );
        assert!(parse_deep_link("picstome://auth?code=abc").is_none());
    }

    #[test]
    fn ignores_other_schemes() {
        assert!(parse_deep_link("https://example.com").is_none());
        assert!(parse_deep_link("picstome://unknown?x=1").is_none());
        assert!(parse_deep_link("not a url").is_none());
    }

    #[test]
    fn builds_links() {
        let link = build_deep_link("sync", &[("gallery", "G123"), ("name", "Mi Boda")]);
        assert_eq!(
            parse_deep_link(&link),
            Some(DeepLink::Sync {
                gallery_id: "G123".into(),
                name: "Mi Boda".into(),
            })
        );
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn rate_accepts_gallery_hint_and_rejects_oversized_lists() {
        let link = parse_deep_link("picstome://rate?files=a,b&gallery=G1").unwrap();
        assert_eq!(
            link,
            DeepLink::Rate {
                files: vec!["a".into(), "b".into()],
                rating: "1".into(),
                gallery: Some("G1".into()),
            }
        );

        let files = (0..=MAX_FILES)
            .map(|index| format!("f{index}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_deep_link(&format!("picstome://rate?files={files}")).is_none());
    }

    #[test]
    fn rejects_control_characters_and_bad_tokens() {
        assert_eq!(
            parse_deep_link("picstome://sync?gallery=G1&name=bad%00name"),
            Some(DeepLink::Sync {
                gallery_id: "G1".into(),
                name: "Gallery".into(),
            })
        );
        assert!(parse_deep_link("picstome://auth?code=bad%20code&state=ok").is_none());
        assert!(parse_deep_link("picstome://auth?code=ok&state=bad%20state").is_none());
    }

    #[test]
    fn build_deep_link_skips_empty_values() {
        assert_eq!(
            build_deep_link("sync", &[("gallery", "G1"), ("name", "")]),
            "picstome://sync?gallery=G1"
        );
    }
}
