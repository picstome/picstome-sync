use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::process::Command;

use picstome_sync_core::identity::{IMAGE_EXTS, RAW_EXTS};
use picstome_sync_core::localfiles::{scan_local_photos, MAX_PHOTO_BYTES};
use picstome_sync_core::sanitize::sanitize_file_names;

pub const MAX_FILES: usize = 5000;
const BATCH_SIZE: usize = 20;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateDetail {
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateResult {
    pub rated: usize,
    pub not_found: usize,
    pub errors: usize,
    pub details: Vec<RateDetail>,
}

impl RateResult {
    fn error_for(names: &[String], message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            rated: 0,
            not_found: 0,
            errors: names.len(),
            details: names
                .iter()
                .map(|name| RateDetail {
                    name: name.clone(),
                    status: "error".into(),
                    file: None,
                    msg: Some(message.clone()),
                })
                .collect(),
        }
    }
}

fn rating_percent(rating: &str) -> &'static str {
    match rating {
        "1" => "1",
        "2" => "25",
        "3" => "50",
        "4" => "75",
        "5" => "99",
        _ => "0",
    }
}

fn normalize_rating(value: &str) -> &'static str {
    match value.trim() {
        "1" => "1",
        "2" => "2",
        "3" => "3",
        "4" => "4",
        "5" => "5",
        _ => "1",
    }
}

#[derive(Debug, Clone)]
struct Invocation {
    program: PathBuf,
    prefix: Vec<PathBuf>,
}

async fn probe(invocation: &Invocation) -> bool {
    let Ok(output) = Command::new(&invocation.program)
        .args(&invocation.prefix)
        .arg("-ver")
        .output()
        .await
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .trim()
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
}

async fn resolve_exiftool(path: &Path) -> Option<Invocation> {
    let mut candidates = vec![Invocation {
        program: path.to_path_buf(),
        prefix: Vec::new(),
    }];

    if !cfg!(windows) {
        let lib = path.parent().unwrap_or_else(|| Path::new(".")).join("lib");
        if lib.is_dir() {
            candidates.push(Invocation {
                program: PathBuf::from("perl"),
                prefix: vec![PathBuf::from("-I"), lib, path.to_path_buf()],
            });
        }
        candidates.push(Invocation {
            program: PathBuf::from("exiftool"),
            prefix: Vec::new(),
        });
    }

    for candidate in candidates {
        if probe(&candidate).await {
            return Some(candidate);
        }
    }
    None
}

async fn run_exiftool(
    invocation: &Invocation,
    files: &[PathBuf],
    rating: &str,
) -> Result<(), String> {
    let rating = normalize_rating(rating);
    let output = Command::new(&invocation.program)
        .args(&invocation.prefix)
        .args([
            "-m",
            "-overwrite_original",
            "-P",
            "-quiet",
            &format!("-Rating={rating}"),
            &format!("-RatingPercent={}", rating_percent(rating)),
        ])
        .args(files)
        .output()
        .await
        .map_err(|err| err.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if message.is_empty() { stdout } else { message })
}

pub async fn rate_files(
    exiftool: Option<&Path>,
    requested_files: &[String],
    rating: &str,
    photos_dir: &Path,
) -> RateResult {
    let Some(names) = sanitize_file_names(requested_files) else {
        return RateResult::error_for(
            &requested_files
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            "Invalid request",
        );
    };

    if names.len() > MAX_FILES {
        return RateResult::error_for(&names, format!("Too many files (limit {MAX_FILES})"));
    }

    let Some(exiftool) = (match exiftool {
        Some(path) => resolve_exiftool(path).await,
        None => None,
    }) else {
        return RateResult::error_for(&names, "Photo tool unavailable");
    };

    if !photos_dir.is_dir() {
        return RateResult::error_for(&names, "Folder not accessible");
    }

    let (records, _) = scan_local_photos(photos_dir, 8, true, MAX_PHOTO_BYTES);
    let mut dir_files: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
    for record in records {
        dir_files
            .entry(record.filename.to_lowercase())
            .or_default()
            .push((record.filename, record.path));
    }

    struct Slot {
        file: Option<PathBuf>,
        detail: RateDetail,
    }

    let mut slots: Vec<Slot> = Vec::new();
    for base in &names {
        let mut matches: Vec<(String, PathBuf)> = Vec::new();
        for ext in IMAGE_EXTS.iter().chain(RAW_EXTS.iter()) {
            let key = format!("{}.{}", base.to_lowercase(), ext);
            if let Some(entries) = dir_files.get(&key) {
                matches.extend(entries.iter().cloned());
            }
        }

        if matches.is_empty() {
            slots.push(Slot {
                file: None,
                detail: RateDetail {
                    name: base.clone(),
                    status: "notfound".into(),
                    file: None,
                    msg: Some("not found".into()),
                },
            });
            continue;
        }

        for (name, path) in matches {
            let detail_name = name.clone();
            slots.push(Slot {
                file: Some(path),
                detail: RateDetail {
                    name: detail_name,
                    status: "ok".into(),
                    file: Some(name),
                    msg: None,
                },
            });
        }
    }

    let file_slots: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| slot.file.as_ref().map(|_| index))
        .collect();

    let mut rated = 0usize;
    let mut errors = 0usize;

    for batch in file_slots.chunks(BATCH_SIZE) {
        let paths: Vec<PathBuf> = batch
            .iter()
            .filter_map(|index| slots[*index].file.clone())
            .collect();

        if run_exiftool(&exiftool, &paths, rating).await.is_ok() {
            rated += paths.len();
            continue;
        }

        for index in batch {
            let Some(path) = slots[*index].file.clone() else {
                continue;
            };
            match run_exiftool(&exiftool, &[path], rating).await {
                Ok(()) => rated += 1,
                Err(err) => {
                    errors += 1;
                    slots[*index].detail.status = "error".into();
                    slots[*index].detail.msg = Some(err);
                }
            }
        }
    }

    let not_found = slots
        .iter()
        .filter(|slot| slot.detail.status == "notfound")
        .count();

    RateResult {
        rated,
        not_found,
        errors,
        details: slots.into_iter().map(|slot| slot.detail).collect(),
    }
}

#[cfg(test)]
fn bundled_test_exiftool() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("resources")
        .join(if cfg!(windows) {
            "exiftool.exe"
        } else {
            "exiftool"
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_rating_for_files_in_subfolders() {
        let exiftool = bundled_test_exiftool();
        if !exiftool.is_file() {
            eprintln!("skipping: bundled exiftool not present");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let subfolder = dir.path().join("Shoot");
        std::fs::create_dir_all(&subfolder).unwrap();
        let path = subfolder.join("photo.jpg");
        std::fs::write(&path, include_bytes!("../tests/fixtures/pixel.jpg")).unwrap();

        let result = rate_files(Some(&exiftool), &["photo".to_string()], "5", dir.path()).await;
        assert_eq!(result.errors, 0, "{:?}", result.details);
        assert_eq!(result.rated, 1);

        let invocation = resolve_exiftool(&exiftool).await.expect("exiftool");
        let output = Command::new(&invocation.program)
            .args(&invocation.prefix)
            .args(["-Rating", "-s", "-s", "-s"])
            .arg(&path)
            .output()
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "5");
    }

    #[tokio::test]
    async fn writes_rating_with_bundled_test_exiftool() {
        let exiftool = bundled_test_exiftool();
        if !exiftool.is_file() {
            eprintln!("skipping: bundled exiftool not present");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, include_bytes!("../tests/fixtures/pixel.jpg")).unwrap();

        let result = rate_files(Some(&exiftool), &["photo".to_string()], "4", dir.path()).await;
        assert_eq!(result.errors, 0, "{:?}", result.details);
        assert_eq!(result.rated, 1);

        let invocation = resolve_exiftool(&exiftool).await.expect("exiftool");
        let output = Command::new(&invocation.program)
            .args(&invocation.prefix)
            .args(["-Rating", "-RatingPercent", "-s", "-s", "-s"])
            .arg(&path)
            .output()
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let values: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(values, vec!["4", "75"], "{text:?}");
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn maps_ratings_and_percentages() {
        assert_eq!(normalize_rating("5"), "5");
        assert_eq!(normalize_rating("9"), "1");
        assert_eq!(rating_percent("1"), "1");
        assert_eq!(rating_percent("2"), "25");
        assert_eq!(rating_percent("3"), "50");
        assert_eq!(rating_percent("4"), "75");
        assert_eq!(rating_percent("5"), "99");
        assert_eq!(rating_percent("0"), "0");
    }

    #[tokio::test]
    async fn reports_missing_tool_and_invalid_requests() {
        let dir = tempfile::tempdir().unwrap();
        let no_tool = rate_files(None, &["a".into()], "1", dir.path()).await;
        assert_eq!(no_tool.errors, 1);
        assert_eq!(
            no_tool.details[0].msg.as_deref(),
            Some("Photo tool unavailable")
        );

        let invalid = rate_files(None, &["../bad".into()], "1", dir.path()).await;
        assert_eq!(invalid.errors, 1);
        assert_eq!(invalid.details[0].msg.as_deref(), Some("Invalid request"));

        let many: Vec<String> = (0..=MAX_FILES).map(|i| format!("f{i}")).collect();
        let too_many = rate_files(None, &many, "1", dir.path()).await;
        assert_eq!(too_many.errors, many.len());
        assert!(too_many.details[0]
            .msg
            .as_deref()
            .unwrap()
            .contains("Too many files"));
    }

    #[tokio::test]
    async fn reports_missing_files_and_accessible_folder_errors() {
        let exiftool = bundled_test_exiftool();
        if !exiftool.is_file() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let missing = rate_files(Some(&exiftool), &["nope".into()], "1", dir.path()).await;
        assert_eq!(missing.not_found, 1);
        assert_eq!(missing.details[0].status, "notfound");

        let inaccessible = rate_files(
            Some(&exiftool),
            &["a".into()],
            "1",
            &dir.path().join("missing-folder"),
        )
        .await;
        assert_eq!(inaccessible.errors, 1);
        assert_eq!(
            inaccessible.details[0].msg.as_deref(),
            Some("Folder not accessible")
        );
    }
}
