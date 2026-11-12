use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::RANGE;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::error::{CoreError, CoreResult};
use crate::localfiles::hash_file;

pub const DOWNLOAD_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
pub const DOWNLOAD_PARALLEL: usize = 6;
pub const DOWNLOAD_RETRIES: usize = 3;

pub type ProgressFn = Arc<dyn Fn(u64, u64) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub expected_hash: Option<String>,
    /// Size reported by the manifest. When known it lets us skip the
    /// destination pre-hash (a full read) whenever the sizes already differ.
    pub expected_size: Option<u64>,
    pub chunk_bytes: u64,
    pub parallel: usize,
    pub retries: usize,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            expected_hash: None,
            expected_size: None,
            chunk_bytes: DOWNLOAD_CHUNK_BYTES,
            parallel: DOWNLOAD_PARALLEL,
            retries: DOWNLOAD_RETRIES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PartMeta {
    url: String,
    total: u64,
    chunk_bytes: u64,
    count: u64,
    completed: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub skipped: bool,
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy)]
struct Probe {
    ranges: bool,
    total: Option<u64>,
}

fn sleep_ms(ms: u64) -> tokio::time::Sleep {
    tokio::time::sleep(Duration::from_millis(ms))
}

async fn probe_once(client: &Client, url: &str) -> CoreResult<Probe> {
    let response = client.get(url).header(RANGE, "bytes=0-0").send().await?;
    let status = response.status();
    if status.as_u16() == 206 {
        let total = response
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next())
            .and_then(|value| value.parse::<u64>().ok());
        return Ok(Probe {
            ranges: true,
            total,
        });
    }
    if status.is_success() {
        let total = response.content_length();
        return Ok(Probe {
            ranges: false,
            total,
        });
    }
    Err(CoreError::Network(format!(
        "cannot probe download (HTTP {})",
        status.as_u16()
    )))
}

async fn probe(client: &Client, url: &str, retries: usize) -> CoreResult<Probe> {
    let mut last_error = None;
    for attempt in 0..=retries {
        match probe_once(client, url).await {
            Ok(probe) => return Ok(probe),
            Err(err) => {
                last_error = Some(err);
                if attempt < retries {
                    sleep_ms(150 * (attempt as u64 + 1)).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| CoreError::Network("cannot probe download".into())))
}

async fn fetch_range(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
    retries: usize,
) -> CoreResult<Vec<u8>> {
    let mut last_error = None;
    for attempt in 0..=retries {
        let result = async {
            let response = client
                .get(url)
                .header(RANGE, format!("bytes={start}-{end}"))
                .send()
                .await?;
            let status = response.status();
            if !status.is_success() {
                return Err(CoreError::Network(format!(
                    "download failed (HTTP {})",
                    status.as_u16()
                )));
            }
            let bytes = response.bytes().await?;
            Ok::<Vec<u8>, CoreError>(bytes.to_vec())
        }
        .await;

        match result {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                last_error = Some(err);
                if attempt < retries {
                    sleep_ms(150 * (attempt as u64 + 1)).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| CoreError::Network("download failed".into())))
}

/// One ranged request that also reports whether the server honoured the range
/// and, when it did, the total file size from `Content-Range`.
async fn fetch_range_checked(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
    retries: usize,
) -> CoreResult<(Vec<u8>, bool, Option<u64>)> {
    let mut last_error = None;
    for attempt in 0..=retries {
        let result = async {
            let response = client
                .get(url)
                .header(RANGE, format!("bytes={start}-{end}"))
                .send()
                .await?;
            let status = response.status();
            if !status.is_success() {
                return Err(CoreError::Network(format!(
                    "download failed (HTTP {})",
                    status.as_u16()
                )));
            }
            let ranged = status.as_u16() == 206;
            let remote_total = response
                .headers()
                .get("content-range")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.rsplit('/').next())
                .and_then(|value| value.parse::<u64>().ok());
            let bytes = response.bytes().await?;
            Ok::<(Vec<u8>, bool, Option<u64>), CoreError>((bytes.to_vec(), ranged, remote_total))
        }
        .await;

        match result {
            Ok(value) => return Ok(value),
            Err(err) => {
                last_error = Some(err);
                if attempt < retries {
                    sleep_ms(150 * (attempt as u64 + 1)).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| CoreError::Network("download failed".into())))
}

fn read_meta(path: &Path, url: &str, total: u64, chunk_bytes: u64, count: u64) -> Vec<u64> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(meta) = serde_json::from_str::<PartMeta>(&contents) else {
        return vec![];
    };
    if meta.url != url
        || meta.total != total
        || meta.chunk_bytes != chunk_bytes
        || meta.count != count
    {
        return vec![];
    }
    meta.completed
}

fn write_meta(path: &Path, url: &str, total: u64, chunk_bytes: u64, count: u64, completed: &[u64]) {
    let meta = PartMeta {
        url: url.to_string(),
        total,
        chunk_bytes,
        count,
        completed: completed.to_vec(),
    };
    if let Ok(serialized) = serde_json::to_vec_pretty(&meta) {
        let _ = std::fs::write(path, serialized);
    }
}

fn part_path(dest: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", dest.display()))
}

fn meta_path(dest: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part.json", dest.display()))
}

/// Download a file in parallel ranges, writing each chunk straight into its
/// final offset in `.part`.
///
/// The previous implementation wrote one file per chunk, read them all back to
/// concatenate, and then read the result again to hash it: three full passes
/// over the bytes. Writing at offsets means one write pass and one read pass,
/// and the resumed state (`.part.json`) keeps exactly the same meaning.
async fn download_chunked(
    client: &Client,
    url: &str,
    dest: &Path,
    total: u64,
    options: &DownloadOptions,
    progress: &ProgressFn,
) -> CoreResult<()> {
    let count = total.div_ceil(options.chunk_bytes);
    let meta_file = meta_path(dest);
    let part = part_path(dest);

    // Any chunk recorded in `.part.json` is only trusted when `.part` is
    // already the full size: a missing or truncated partial file must never be
    // "resumed" into a file full of zeros. The hash check at the end of the
    // download is the second line of defence.
    let existing_len = tokio::fs::metadata(&part)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let resumable = existing_len == total;

    // Pre-size the partial file so every chunk can seek to its own offset.
    if existing_len != total {
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part)
            .await?;
        file.set_len(total).await?;
    }

    let completed = if resumable {
        read_meta(&meta_file, url, total, options.chunk_bytes, count)
    } else {
        Vec::new()
    };
    let completed_set = Arc::new(std::sync::Mutex::new(
        completed
            .iter()
            .copied()
            .collect::<std::collections::HashSet<u64>>(),
    ));

    let already_downloaded: u64 = completed
        .iter()
        .map(|index| chunk_length(*index, total, options.chunk_bytes))
        .sum();
    let downloaded = Arc::new(AtomicU64::new(already_downloaded));
    if already_downloaded > 0 {
        progress(already_downloaded, total);
    }

    // Ask for chunk 0 first. If the server answers `200` it ignored the range
    // and sent the whole file, so fall back to writing it as one stream rather
    // than fanning out requests that would each return the full body.
    if !completed_set
        .lock()
        .expect("chunk set poisoned")
        .contains(&0)
    {
        let end = options.chunk_bytes.min(total) - 1;
        let (bytes, ranged, remote_total) =
            fetch_range_checked(client, url, 0, end, options.retries).await?;
        if let Some(remote_total) = remote_total {
            // The manifest size is what the *version* should weigh. If the CDN
            // serves something else, the bytes on disk would not match the
            // remote, so stop before writing an unusable file.
            if remote_total != total {
                let _ = tokio::fs::remove_file(&meta_file).await;
                let _ = tokio::fs::remove_file(&part).await;
                return Err(CoreError::Network(format!(
                    "server reported {remote_total} bytes, manifest says {total}"
                )));
            }
        }
        if !ranged {
            if bytes.len() as u64 != total {
                return Err(CoreError::Network(format!(
                    "server ignored range and returned {} bytes, expected {total}",
                    bytes.len()
                )));
            }
            let _ = tokio::fs::remove_file(&meta_file).await;
            tokio::fs::write(&part, &bytes).await?;
            progress(total, total);
            return Ok(());
        }
        if bytes.len() as u64 != end + 1 {
            return Err(CoreError::Network(format!(
                "short read at 0-{end}: got {}, expected {}",
                bytes.len(),
                end + 1
            )));
        }
        {
            use tokio::io::{AsyncSeekExt, AsyncWriteExt};
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&part)
                .await?;
            file.seek(std::io::SeekFrom::Start(0)).await?;
            file.write_all(&bytes).await?;
        }
        let current =
            downloaded.fetch_add(bytes.len() as u64, Ordering::SeqCst) + bytes.len() as u64;
        progress(current, total);
        let done = {
            let mut set = completed_set.lock().expect("chunk set poisoned");
            set.insert(0);
            let mut done: Vec<u64> = set.iter().copied().collect();
            done.sort_unstable();
            done
        };
        write_meta(&meta_file, url, total, options.chunk_bytes, count, &done);
    }

    let results: Vec<CoreResult<()>> = futures::stream::iter(1..count)
        .map(|index| {
            let client = client.clone();
            let url = url.to_string();
            let part = part.clone();
            let meta_file = meta_file.clone();
            let downloaded = downloaded.clone();
            let progress = progress.clone();
            let options = options.clone();
            let completed_set = completed_set.clone();
            async move {
                {
                    let set = completed_set.lock().expect("chunk set poisoned");
                    if set.contains(&index) {
                        return Ok(());
                    }
                }

                let start = index * options.chunk_bytes;
                let end = (start + options.chunk_bytes).min(total) - 1;
                let expected = (end - start + 1) as usize;
                let bytes = fetch_range(&client, &url, start, end, options.retries).await?;
                if bytes.len() != expected {
                    return Err(CoreError::Network(format!(
                        "short read at {start}-{end}: got {}, expected {expected}",
                        bytes.len()
                    )));
                }

                {
                    use tokio::io::{AsyncSeekExt, AsyncWriteExt};
                    let mut file = tokio::fs::OpenOptions::new()
                        .write(true)
                        .open(&part)
                        .await?;
                    file.seek(std::io::SeekFrom::Start(start)).await?;
                    file.write_all(&bytes).await?;
                }
                let current =
                    downloaded.fetch_add(bytes.len() as u64, Ordering::SeqCst) + bytes.len() as u64;
                progress(current, total);

                let done = {
                    let mut set = completed_set.lock().expect("chunk set poisoned");
                    set.insert(index);
                    let mut done: Vec<u64> = set.iter().copied().collect();
                    done.sort_unstable();
                    done
                };
                write_meta(&meta_file, &url, total, options.chunk_bytes, count, &done);
                Ok(())
            }
        })
        .buffer_unordered(options.parallel.max(1))
        .collect()
        .await;

    for result in results {
        result?;
    }

    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&part)
        .await?;
    output.flush().await?;
    output.sync_all().await?;
    drop(output);

    let _ = tokio::fs::remove_file(&meta_file).await;
    // Older builds kept one file per chunk in `<dest>.parts`; clean it up.
    let _ = tokio::fs::remove_dir_all(PathBuf::from(format!("{}.parts", dest.display()))).await;
    Ok(())
}

fn chunk_length(index: u64, total: u64, chunk_bytes: u64) -> u64 {
    let start = index * chunk_bytes;
    (start + chunk_bytes).min(total) - start
}

async fn download_single(
    client: &Client,
    url: &str,
    dest: &Path,
    total: Option<u64>,
    progress: &ProgressFn,
) -> CoreResult<()> {
    let part = part_path(dest);
    let mut offset = tokio::fs::metadata(&part)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    if let Some(total) = total {
        if offset > total {
            let _ = tokio::fs::remove_file(&part).await;
            offset = 0;
        }
        if offset == total && total > 0 {
            progress(total, total);
            return Ok(());
        }
    }

    let mut request = client.get(url);
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
    }

    let mut response = request.send().await?;
    if !response.status().is_success() {
        return Err(CoreError::Network(format!(
            "download failed (HTTP {})",
            response.status().as_u16()
        )));
    }

    let append = offset > 0 && response.status().as_u16() == 206;
    if !append {
        offset = 0;
    }

    let mut file = if append {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .await?
    } else {
        tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&part)
            .await?
    };

    let mut downloaded = offset;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        progress(downloaded, total.unwrap_or(downloaded));
    }
    file.sync_all().await?;
    Ok(())
}

pub async fn download_file(
    client: &Client,
    url: &str,
    dest: &Path,
    options: DownloadOptions,
    progress: ProgressFn,
) -> CoreResult<DownloadResult> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Cheaply decide whether the destination is already the wanted file. The
    // manifest size lets us rule it out with a stat instead of reading the
    // whole file, which matters when a download is retried in a loop.
    if let (Some(expected), Some(expected_size)) = (&options.expected_hash, options.expected_size) {
        if let Ok(metadata) = tokio::fs::metadata(dest).await {
            if metadata.len() == expected_size {
                if let Ok(existing) = hash_file(dest).await {
                    if &existing == expected {
                        progress(expected_size, expected_size);
                        return Ok(DownloadResult {
                            skipped: true,
                            hash: existing,
                            size: expected_size,
                        });
                    }
                }
            }
        }
    }

    // A known size means we do not need the `Range: 0-0` probe: the first
    // range response already tells us whether ranging works.
    let probe = match options.expected_size {
        Some(total) if total > 0 => Probe {
            ranges: true,
            total: Some(total),
        },
        Some(_) => {
            tokio::fs::write(dest, b"").await?;
            return Ok(DownloadResult {
                skipped: false,
                hash: crate::localfiles::hash_bytes(b""),
                size: 0,
            });
        }
        None => probe(client, url, options.retries).await?,
    };

    if probe.total == Some(0) {
        tokio::fs::write(dest, b"").await?;
        return Ok(DownloadResult {
            skipped: false,
            hash: crate::localfiles::hash_bytes(b""),
            size: 0,
        });
    }

    if probe.ranges {
        if let Some(total) = probe.total {
            download_chunked(client, url, dest, total, &options, &progress).await?;
        } else {
            let _ = tokio::fs::remove_file(meta_path(dest)).await;
            download_single(client, url, dest, probe.total, &progress).await?;
        }
    } else {
        let _ = tokio::fs::remove_file(meta_path(dest)).await;
        download_single(client, url, dest, probe.total, &progress).await?;
    }

    let part = part_path(dest);
    let size = tokio::fs::metadata(&part)
        .await
        .map(|metadata| metadata.len())?;

    if let Some(total) = probe.total {
        if size != total {
            return Err(CoreError::Network(format!(
                "downloaded {size} bytes, expected {total}"
            )));
        }
    }

    let hash = hash_file(&part).await?;
    if let Some(expected) = options.expected_hash {
        if hash != expected {
            let _ = tokio::fs::remove_file(&part).await;
            let _ = tokio::fs::remove_file(meta_path(dest)).await;
            return Err(CoreError::HashMismatch(dest.display().to_string()));
        }
    }

    let _ = tokio::fs::remove_file(dest).await;
    tokio::fs::rename(&part, dest).await?;
    let _ = tokio::fs::remove_file(meta_path(dest)).await;

    Ok(DownloadResult {
        skipped: false,
        hash,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_lengths_cover_the_file() {
        assert_eq!(chunk_length(0, 10, 4), 4);
        assert_eq!(chunk_length(2, 10, 4), 2);
        assert_eq!(chunk_length(1, 10, 4), 4);
    }
}
