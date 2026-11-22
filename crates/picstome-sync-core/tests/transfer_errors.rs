use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use picstome_sync_core::localfiles::hash_bytes;
use picstome_sync_core::transfer::{download_file, DownloadOptions};

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn reports_probe_failures() {
    let base = serve(Router::new().route(
        "/fail",
        get(|| async { StatusCode::INTERNAL_SERVER_ERROR.into_response() }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let err = download_file(
        &reqwest::Client::new(),
        &format!("{base}/fail"),
        &dir.path().join("x.jpg"),
        DownloadOptions::default(),
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("cannot probe download"), "{err}");
}

#[tokio::test]
async fn reports_short_chunk_reads() {
    async fn short(headers: HeaderMap) -> Response {
        let range = headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if range == "bytes=0-0" {
            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, "bytes 0-0/4096")
                .header(header::CONTENT_LENGTH, "1")
                .body(Body::from(vec![0u8]))
                .unwrap();
        }
        Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, "bytes 0-511/4096")
            .header(header::CONTENT_LENGTH, "5")
            .body(Body::from(vec![1u8; 5]))
            .unwrap()
    }

    let base = serve(Router::new().route("/short", get(short))).await;
    let dir = tempfile::tempdir().unwrap();
    let err = download_file(
        &reqwest::Client::new(),
        &format!("{base}/short"),
        &dir.path().join("x.jpg"),
        DownloadOptions {
            chunk_bytes: 512,
            retries: 0,
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("short read"), "{err}");
}

#[tokio::test]
async fn downloads_without_content_length_or_range_support() {
    let payload = b"unknown total payload".repeat(50);
    let expected = hash_bytes(&payload);
    let body = payload.clone();
    let base = serve(Router::new().route(
        "/stream",
        get(move || {
            let body = body.clone();
            async move {
                let stream = futures::stream::iter(vec![Ok::<_, std::io::Error>(body)]);
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from_stream(stream))
                    .unwrap()
            }
        }),
    ))
    .await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("stream.jpg");
    let result = download_file(
        &reqwest::Client::new(),
        &format!("{base}/stream"),
        &dest,
        DownloadOptions {
            expected_hash: Some(expected),
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert_eq!(result.size, payload.len() as u64);
}

#[tokio::test]
async fn restarts_when_server_ignores_resume_range() {
    let payload = b"resume me".repeat(100);
    let body = payload.clone();
    let base = serve(Router::new().route(
        "/plain",
        get(move || {
            let body = body.clone();
            async move {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_LENGTH, body.len().to_string())
                    .body(Body::from(body))
                    .unwrap()
            }
        }),
    ))
    .await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("resume.jpg");
    let part = dir.path().join("resume.jpg.part");
    std::fs::write(&part, b"stale partial").unwrap();

    let result = download_file(
        &reqwest::Client::new(),
        &format!("{base}/plain"),
        &dest,
        DownloadOptions {
            expected_hash: Some(hash_bytes(&payload)),
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert_eq!(result.size, payload.len() as u64);
}

#[tokio::test]
async fn retries_transient_probe_failures() {
    #[derive(Clone)]
    struct Counter(Arc<AtomicUsize>);

    async fn flaky(State(counter): State<Counter>, headers: HeaderMap) -> Response {
        let attempt = counter.0.fetch_add(1, Ordering::SeqCst);
        if attempt < 2 {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let range = headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if range == "bytes=0-0" {
            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, "bytes 0-0/4")
                .header(header::CONTENT_LENGTH, "1")
                .body(Body::from(vec![1u8]))
                .unwrap();
        }
        Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, "bytes 0-3/4")
            .header(header::CONTENT_LENGTH, "4")
            .body(Body::from(vec![1u8, 2, 3, 4]))
            .unwrap()
    }

    let counter = Counter(Arc::new(AtomicUsize::new(0)));
    let app = Router::new()
        .route("/flaky", get(flaky))
        .with_state(counter);
    let base = serve(app).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("flaky.jpg");
    let result = download_file(
        &reqwest::Client::new(),
        &format!("{base}/flaky"),
        &dest,
        DownloadOptions {
            chunk_bytes: 4,
            retries: 3,
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();
    assert_eq!(result.size, 4);
    assert_eq!(std::fs::read(&dest).unwrap(), vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn does_not_retry_permanent_http_errors() {
    #[derive(Clone)]
    struct Counter(Arc<AtomicUsize>);

    async fn forbidden(State(counter): State<Counter>) -> Response {
        counter.0.fetch_add(1, Ordering::SeqCst);
        StatusCode::FORBIDDEN.into_response()
    }

    let counter = Counter(Arc::new(AtomicUsize::new(0)));
    let hits = counter.0.clone();
    let base = serve(
        Router::new()
            .route("/expired", get(forbidden))
            .with_state(counter),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let err = download_file(
        &reqwest::Client::new(),
        &format!("{base}/expired"),
        &dir.path().join("x.jpg"),
        DownloadOptions {
            expected_size: Some(8),
            retries: 3,
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("HTTP 403"), "{err}");
    // An expired signed URL fails the same way every time: one request only.
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resumes_a_partial_download_when_the_signed_url_changes() {
    #[derive(Clone)]
    struct Requests(Arc<std::sync::Mutex<Vec<String>>>);

    const DATA: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

    async fn ranged(State(requests): State<Requests>, headers: HeaderMap) -> Response {
        let range = headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        requests.0.lock().unwrap().push(range.clone());
        let (start, end) = range
            .trim_start_matches("bytes=")
            .split_once('-')
            .map(|(start, end)| {
                (
                    start.parse::<usize>().unwrap(),
                    end.parse::<usize>().unwrap(),
                )
            })
            .unwrap();
        Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/8"))
            .body(Body::from(DATA[start..=end].to_vec()))
            .unwrap()
    }

    let requests = Requests(Arc::new(std::sync::Mutex::new(Vec::new())));
    let seen = requests.0.clone();
    let base = serve(
        Router::new()
            .route("/photo.jpg", get(ranged))
            .with_state(requests),
    )
    .await;

    // A previous attempt finished chunk 0 with an older signature.
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("photo.jpg");
    let mut partial = DATA[..4].to_vec();
    partial.extend_from_slice(&[0; 4]);
    std::fs::write(dir.path().join("photo.jpg.part"), &partial).unwrap();
    std::fs::write(
        dir.path().join("photo.jpg.part.json"),
        serde_json::to_vec(&serde_json::json!({
            "url": format!("{base}/photo.jpg?signature=old"),
            "total": 8,
            "chunk_bytes": 4,
            "count": 2,
            "completed": [0],
        }))
        .unwrap(),
    )
    .unwrap();

    let result = download_file(
        &reqwest::Client::new(),
        &format!("{base}/photo.jpg?signature=new"),
        &dest,
        DownloadOptions {
            expected_size: Some(8),
            expected_hash: None,
            chunk_bytes: 4,
            ..Default::default()
        },
        Arc::new(|_, _| {}),
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), DATA);
    assert_eq!(result.hash, hash_bytes(&DATA));
    assert_eq!(*seen.lock().unwrap(), vec!["bytes=4-7".to_string()]);
}
