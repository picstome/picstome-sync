//! Local benchmark for the sync engine's scan/hash pipeline.
//!
//! It does not need the API or the UI: it creates a folder with JPEG+RAW pairs,
//! runs the scan+hash twice and prints how much work each pass did. The second
//! pass is what matters: it must report zero hashed files and zero bytes read.
//!
//! Usage:
//!   cargo run -p picstome-sync-core --bin sync-bench -- [pairs] [raw_mb]
//!
//! Defaults: 1000 pairs, 1 MB RAW stand-ins (override RAW_MB to simulate real
//! 40 MB files; the timing is dominated by I/O so a smaller size is enough to
//! compare passes).

use std::path::Path;
use std::time::Instant;

use picstome_sync_core::cache::FingerprintCache;
use picstome_sync_core::localfiles::{
    hash_local_records_cached, scan_local_photos, MAX_PHOTO_BYTES,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let pairs: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1000);
    let raw_mb: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);

    let dir = std::env::temp_dir().join(format!(
        "picstome-sync-bench-{}-{pairs}-{raw_mb}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create bench folder");

    println!("Preparing {pairs} JPEG+RAW pairs in {}…", dir.display());
    let jpeg = vec![0xFFu8; 256 * 1024];
    let raw = vec![0x11u8; raw_mb * 1024 * 1024];
    for index in 0..pairs {
        std::fs::write(dir.join(format!("SHOT_{index:05}.JPG")), &jpeg).expect("write jpeg");
        std::fs::write(dir.join(format!("SHOT_{index:05}.DNG")), &raw).expect("write raw");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");

    let mut cache = FingerprintCache::new("BENCH", dir.to_string_lossy().to_string());

    for pass in 1..=2 {
        let (records, _) = scan_local_photos(&dir, 8, true, MAX_PHOTO_BYTES);
        let started = Instant::now();
        let (files, stats) = runtime.block_on(hash_local_records_cached(
            records.clone(),
            &[],
            Some(&cache),
            8,
            |_, _| {},
        ));
        let elapsed = started.elapsed();

        println!(
            "pass {pass}: {} files, hashed={}, cached={}, skipped_duplicates={}, read={:.1} MB in {:.2?}",
            stats.files,
            stats.hashed,
            stats.cached,
            stats.skipped,
            stats.bytes_read as f64 / (1024.0 * 1024.0),
            elapsed
        );
        assert_eq!(files.len(), records.len());

        // Persist what this pass learned, as a real sync does.
        for file in files.iter().filter_map(|file| file.as_ref().ok()) {
            if let Some(hash) = file.hash.as_deref() {
                cache.insert(
                    file.relative_path.clone(),
                    file.size,
                    file.mtime_ms,
                    None,
                    None,
                    hash,
                );
            }
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
    println!("done");
    let _ = Path::new(".");
}
