//! Resumable, parallel downloads with integrity checks.
//!
//! A download goes to `<dest>.part`, is verified against the manifest size and
//! hash, and only then replaces `<dest>`. The destination therefore always
//! holds either the previous version or the complete new one.

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

/// Resume state stored next to a partial download (`<dest>.part.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PartMeta {
    /// Resume key (see [`resume_key`]). Named `url` for compatibility with the
    /// files written by older builds.
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

/// One ranged response: the body, whether the server honoured the range
/// (`206`), and the total size announced by `Content-Range`.
struct RangeResponse {
    bytes: Vec<u8>,
    ranged: bool,
    remote_total: Option<u64>,
}

/// Failure of a single HTTP attempt, tagged with whether trying again can help.
struct AttemptError {
    error: CoreError,
    retryable: bool,
}

impl From<reqwest::Error> for AttemptError {
    fn from(error: reqwest::Error) -> Self {
        // Connection resets, timeouts and truncated bodies are transient.
        Self {
            error: error.into(),
            retryable: true,
        }
    }
}

/// `5xx`, `408 Request Timeout` and `429 Too Many Requests` are worth another
/// attempt. Any other `4xx` (an expired signed URL, a missing object) will
/// fail identically, so retrying only delays the error.
fn retryable_status(status: u16) -> bool {
    status >= 500 || status == 408 || status == 429
}

fn status_error(context: &str, status: u16) -> AttemptError {
    AttemptError {
        error: CoreError::Network(format!("{context} (HTTP {status})")),
        retryable: retryable_status(status),
    }
}

/// Exponential backoff: 250 ms, 500 ms, 1 s, 2 s... capped at 4 s.
fn backoff(attempt: usize) -> Duration {
    Duration::from_millis((250u64 << attempt.min(4)).min(4_000))
}

/// Run `operation` up to `retries + 1` times, backing off between transient
/// failures. Permanent failures are returned immediately.
async fn with_retries<T, F, Fut>(retries: usize, mut operation: F) -> CoreResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, AttemptError>>,
{
    let mut attempt = 0;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(failure) if failure.retryable && attempt < retries => {
                tokio::time::sleep(backoff(attempt)).await;
                attempt += 1;
            }
            Err(failure) => return Err(failure.error),
        }
    }
}

fn content_range_total(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit('/').next())
        .and_then(|value| value.parse::<u64>().ok())
}

async fn probe(client: &Client, url: &str, retries: usize) -> CoreResult<Probe> {
    with_retries(retries, || async {
        let response = client.get(url).header(RANGE, "bytes=0-0").send().await?;
        let status = response.status().as_u16();
        if status == 206 {
            return Ok(Probe {
                ranges: true,
                total: content_range_total(&response),
            });
        }
        if response.status().is_success() {
            return Ok(Probe {
                ranges: false,
                total: response.content_length(),
            });
        }
        Err(status_error("cannot probe download", status))
    })
    .await
}

/// Fetch `bytes=start-end` (inclusive), retrying transient failures.
async fn fetch_range(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
    retries: usize,
) -> CoreResult<RangeResponse> {
    with_retries(retries, || async {
        let response = client
            .get(url)
            .header(RANGE, format!("bytes={start}-{end}"))
            .send()
            .await?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(status_error("download failed", status));
        }
        let remote_total = content_range_total(&response);
        let bytes = response.bytes().await?;
        Ok(RangeResponse {
            bytes: bytes.to_vec(),
            ranged: status == 206,
            remote_total,
        })
    })
    .await
}

/// Identity of the resource behind a download URL, used to decide whether a
/// `.part` file can be resumed. Signed URLs carry a fresh signature (query
/// string) on every manifest, so the query is ignored; the manifest hash, when
/// known, identifies the exact bytes even better.
fn resume_key(url: &str, expected_hash: Option<&str>) -> String {
    if let Some(hash) = expected_hash.filter(|hash| !hash.is_empty()) {
        return format!("sha256:{hash}");
    }
    url.split(['?', '#']).next().unwrap_or(url).to_string()
}

fn read_meta(path: &Path, key: &str, total: u64, chunk_bytes: u64, count: u64) -> Vec<u64> {
    let Ok(contents) = std::fs::read(path) else {
        return vec![];
    };
    let Ok(meta) = serde_json::from_slice::<PartMeta>(&contents) else {
        return vec![];
    };
    // Older builds stored the full URL; compare it the same way.
    let stored_key = if meta.url.starts_with("sha256:") {
        meta.url.clone()
    } else {
        resume_key(&meta.url, None)
    };
    if stored_key != key
        || meta.total != total
        || meta.chunk_bytes != chunk_bytes
        || meta.count != count
    {
        return vec![];
    }
    meta.completed
        .into_iter()
        .filter(|index| *index < count)
        .collect()
}

/// Persist which chunks are on disk. Atomic, so a crash never leaves a
/// half-written file that would discard the whole resume state.
fn write_meta(path: &Path, key: &str, total: u64, chunk_bytes: u64, count: u64, completed: &[u64]) {
    let meta = PartMeta {
        url: key.to_string(),
        total,
        chunk_bytes,
        count,
        completed: completed.to_vec(),
    };
    let _ = crate::atomic::write_json_compact_atomic(path, &meta);
}

/// Write `bytes` at `offset` of the pre-sized partial file and make them
/// durable before the chunk is recorded as complete in `.part.json`.
///
/// The explicit `flush` matters: a tokio `File` dropped with a write still in
/// flight finishes it in the background, so a later rename or hash could run
/// before the bytes land. That silently produced files full of zeros.
async fn write_chunk_at(part: &Path, offset: u64, bytes: &[u8]) -> CoreResult<()> {
    use tokio::io::AsyncSeekExt;
    let mut file = tokio::fs::OpenOptions::new().write(true).open(part).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    file.sync_data().await?;
    Ok(())
}

/// Shared bookkeeping for the chunks of one download.
struct ChunkLedger {
    completed: std::sync::Mutex<std::collections::HashSet<u64>>,
    meta_file: PathBuf,
    key: String,
    total: u64,
    chunk_bytes: u64,
    count: u64,
}

impl ChunkLedger {
    fn contains(&self, index: u64) -> bool {
        self.completed
            .lock()
            .expect("chunk set poisoned")
            .contains(&index)
    }

    /// Record a finished chunk. The meta file is written while holding the
    /// lock so an older snapshot can never overwrite a newer one.
    fn complete(&self, index: u64) {
        let mut set = self.completed.lock().expect("chunk set poisoned");
        set.insert(index);
        let mut done: Vec<u64> = set.iter().copied().collect();
        done.sort_unstable();
        write_meta(
            &self.meta_file,
            &self.key,
            self.total,
            self.chunk_bytes,
            self.count,
            &done,
        );
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
/// Writing at offsets means one write pass and one read pass (the hash), and
/// `.part.json` records finished chunks so an interrupted download resumes
/// where it stopped.
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
    let key = resume_key(url, options.expected_hash.as_deref());

    // Any chunk recorded in `.part.json` is only trusted when `.part` is
    // already the full size: a missing or truncated partial file must never be
    // "resumed" into a file full of zeros. The hash check at the end of the
    // download is the second line of defence.
    let existing_len = tokio::fs::metadata(&part)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let completed = if existing_len == total {
        read_meta(&meta_file, &key, total, options.chunk_bytes, count)
    } else {
        // Pre-size the partial file so every chunk can seek to its own offset.
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part)
            .await?;
        file.set_len(total).await?;
        Vec::new()
    };

    let already_downloaded: u64 = completed
        .iter()
        .map(|index| chunk_length(*index, total, options.chunk_bytes))
        .sum();
    let ledger = Arc::new(ChunkLedger {
        completed: std::sync::Mutex::new(completed.into_iter().collect()),
        meta_file: meta_file.clone(),
        key,
        total,
        chunk_bytes: options.chunk_bytes,
        count,
    });
    let downloaded = Arc::new(AtomicU64::new(already_downloaded));
    if already_downloaded > 0 {
        progress(already_downloaded, total);
    }

    // Ask for chunk 0 first. If the server answers `200` it ignored the range
    // and sent the whole file, so write it as one body rather than fanning out
    // requests that would each return the full file.
    if !ledger.contains(0) {
        let end = options.chunk_bytes.min(total) - 1;
        let first = fetch_range(client, url, 0, end, options.retries).await?;
        if let Some(remote_total) = first.remote_total {
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
        if !first.ranged {
            if first.bytes.len() as u64 != total {
                return Err(CoreError::Network(format!(
                    "server ignored range and returned {} bytes, expected {total}",
                    first.bytes.len()
                )));
            }
            let _ = tokio::fs::remove_file(&meta_file).await;
            write_chunk_at(&part, 0, &first.bytes).await?;
            progress(total, total);
            return Ok(());
        }
        if first.bytes.len() as u64 != end + 1 {
            return Err(CoreError::Network(format!(
                "short read at 0-{end}: got {}, expected {}",
                first.bytes.len(),
                end + 1
            )));
        }
        write_chunk_at(&part, 0, &first.bytes).await?;
        let length = first.bytes.len() as u64;
        progress(
            downloaded.fetch_add(length, Ordering::SeqCst) + length,
            total,
        );
        ledger.complete(0);
    }

    let results: Vec<CoreResult<()>> = futures::stream::iter(1..count)
        .filter(|index| futures::future::ready(!ledger.contains(*index)))
        .map(|index| {
            let part = part.clone();
            let downloaded = downloaded.clone();
            let ledger = ledger.clone();
            async move {
                let start = index * options.chunk_bytes;
                let end = (start + options.chunk_bytes).min(total) - 1;
                let expected = end - start + 1;
                let chunk = fetch_range(client, url, start, end, options.retries).await?;
                if chunk.bytes.len() as u64 != expected {
                    return Err(CoreError::Network(format!(
                        "short read at {start}-{end}: got {}, expected {expected}",
                        chunk.bytes.len()
                    )));
                }
                write_chunk_at(&part, start, &chunk.bytes).await?;
                progress(
                    downloaded.fetch_add(expected, Ordering::SeqCst) + expected,
                    total,
                );
                ledger.complete(index);
                Ok(())
            }
        })
        .buffer_unordered(options.parallel.max(1))
        .collect()
        .await;

    for result in results {
        result?;
    }

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
    file.flush().await?;
    file.sync_data().await?;
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
            let size = tokio::fs::metadata(&part)
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let _ = tokio::fs::remove_file(&part).await;
            let _ = tokio::fs::remove_file(meta_path(dest)).await;
            return Err(CoreError::HashMismatchDetail {
                path: dest.display().to_string(),
                expected,
                actual: hash,
                size,
            });
        }
    }

    // Replace atomically: removing `dest` first would leave a window in which
    // a crash loses the previous version of the photo.
    let (from, to) = (part.clone(), dest.to_path_buf());
    tokio::task::spawn_blocking(move || crate::atomic::replace_file(&from, &to))
        .await
        .map_err(|err| CoreError::Other(format!("rename task failed: {err}")))??;
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
