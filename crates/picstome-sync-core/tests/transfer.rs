use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::mock::{start_mock_server, MockServerOptions};
use picstome_sync_core::transfer::{download_file, DownloadOptions};

async fn mock() -> picstome_sync_core::mock::MockServer {
    start_mock_server(MockServerOptions {
        photo_size: 4096,
        ..Default::default()
    })
    .await
    .expect("mock server")
}

async fn first_photo(mock: &picstome_sync_core::mock::MockServer) -> (String, String, u64) {
    let api = ApiClient::new(Some(&mock.url), None).unwrap();
    api.dev_login().await.unwrap();
    let manifest = api.manifest("G123").await.unwrap();
    let photo = &manifest.photos[0];
    (
        photo.url.clone(),
        photo.hash.clone().unwrap(),
        photo.size.unwrap(),
    )
}

#[tokio::test]
async fn resumes_completed_chunks_from_meta() {
    let mock = mock().await;
    let (url, hash, total) = first_photo(&mock).await;
    let data = mock.state.galleries.read().await["G123"].photos[0]
        .data
        .clone();

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("resume.jpg");
    let part = dir.path().join("resume.jpg.part");
    let meta = dir.path().join("resume.jpg.part.json");
    let chunk_bytes = 512u64;
    let count = total.div_ceil(chunk_bytes);
    std::fs::write(&part, &data[..chunk_bytes as usize]).unwrap();
    std::fs::write(
        &meta,
        serde_json::to_vec(&serde_json::json!({
            "url": url,
            "total": total,
            "chunk_bytes": chunk_bytes,
            "count": count,
            "completed": [0],
        }))
        .unwrap(),
    )
    .unwrap();

    let result = download_file(
        &ApiClient::new(Some(&mock.url), None).unwrap().http(),
        &url,
        &dest,
        DownloadOptions {
            expected_hash: Some(hash.clone()),
            chunk_bytes,
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();

    assert_eq!(result.hash, hash);
    assert_eq!(std::fs::metadata(&dest).unwrap().len(), total);
    assert!(!meta.exists());
    mock.close().await;
}

#[tokio::test]
async fn rejects_hash_mismatch_and_cleans_partial_files() {
    let mock = mock().await;
    let (url, _, _) = first_photo(&mock).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("bad.jpg");

    let error = download_file(
        &ApiClient::new(Some(&mock.url), None).unwrap().http(),
        &url,
        &dest,
        DownloadOptions {
            expected_hash: Some("0".repeat(64)),
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("hash mismatch"), "{error}");
    assert!(!dest.exists());
    assert!(!dir.path().join("bad.jpg.part").exists());
    mock.close().await;
}

#[tokio::test]
async fn falls_back_to_single_stream_when_server_ignores_range() {
    let payload = b"hello picstome sync".repeat(100);
    let expected = picstome_sync_core::localfiles::hash_bytes(&payload);
    let body = payload.clone();

    let app = Router::new().route(
        "/file",
        get(move || {
            let body = body.clone();
            async move {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .header(header::CONTENT_LENGTH, body.len().to_string())
                    .body(Body::from(body))
                    .unwrap()
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("single.jpg");
    let client = reqwest::Client::new();
    let result = download_file(
        &client,
        &format!("http://{addr}/file"),
        &dest,
        DownloadOptions {
            expected_hash: Some(expected),
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();

    assert_eq!(result.size, payload.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    server.abort();
}

/// Regression for silent download corruption.
///
/// `download_chunked` pre-sizes `.part` with `set_len` and then writes each
/// chunk through a *separate* tokio file handle. A `tokio::fs::File` only
/// queues its write: dropping it without a flush loses the data and can leave
/// a file of the right size full of zeros (or with missing chunks), which the
/// hash check then reports as an intermittent "hash mismatch". Isolated at
/// ~49/50 lost writes without the explicit flush, 0/50 with it. This test
/// downloads real bytes and compares them one by one, so a lost chunk is a
/// failure even when the size still matches.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chunked_download_writes_every_byte_exactly() {
    let mock = mock().await;
    let api = ApiClient::new(Some(&mock.url), None).unwrap();
    api.dev_login().await.unwrap();
    let manifest = api.manifest("G123").await.unwrap();
    let client = api.http();

    for round in 0..15 {
        for (index, photo) in manifest.photos.iter().take(4).enumerate() {
            let expected = {
                let galleries = mock.state.galleries.read().await;
                galleries["G123"]
                    .photos
                    .iter()
                    .find(|candidate| candidate.id == photo.id)
                    .expect("photo")
                    .data
                    .clone()
            };
            let dir = tempfile::tempdir().unwrap();
            let dest = dir.path().join(format!("round{round}-{index}.jpg"));
            download_file(
                &client,
                &photo.url,
                &dest,
                DownloadOptions {
                    expected_hash: photo.content_hash().map(str::to_string),
                    expected_size: photo.size,
                    // A chunk larger than the file means the download is a
                    // single chunk, written by the "chunk 0" path and then
                    // finalised by a second handle. That is the exact shape
                    // that produced all-zero files.
                    chunk_bytes: 4 * 1024 * 1024,
                    parallel: 6,
                    retries: 0,
                },
                Arc::new(|_, _| {}),
            )
            .await
            .unwrap_or_else(|err| panic!("round {round} file {index}: {err}"));
            let got = std::fs::read(&dest).unwrap();
            assert_eq!(
                got.as_slice(),
                expected.as_slice(),
                "round {round} file {index} downloaded different bytes"
            );
        }
    }

    mock.close().await;
}
