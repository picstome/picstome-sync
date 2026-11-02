use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use picstome_sync_core::api::ApiClient;
use picstome_sync_core::CoreError;

async fn server() -> String {
    let app = Router::new()
        .route(
            "/case1/api/v1/me",
            get(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"message": "nope", "code": "custom_unauth"})),
                )
                    .into_response()
            }),
        )
        .route(
            "/case2/api/v1/me",
            get(|| async { StatusCode::UNAUTHORIZED.into_response() }),
        )
        .route(
            "/case3/api/v1/me",
            get(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"message": "boom"})),
                )
                    .into_response()
            }),
        )
        .route(
            "/case4/api/v1/me",
            get(|| async { (StatusCode::OK, "not json").into_response() }),
        )
        .route(
            "/case5/api/v1/me",
            get(|| async { (StatusCode::OK, "").into_response() }),
        );

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn preserves_api_error_code_and_message() {
    let base = server().await;
    let api = ApiClient::new(Some(&format!("{base}/case1")), None).unwrap();
    let err = api.me().await.unwrap_err();
    match err {
        CoreError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 401);
            assert_eq!(code, "custom_unauth");
            assert_eq!(message, "nope");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn falls_back_to_unauthorized_and_http_message() {
    let base = server().await;
    let api = ApiClient::new(Some(&format!("{base}/case2")), None).unwrap();
    match api.me().await.unwrap_err() {
        CoreError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 401);
            assert_eq!(code, "unauthorized");
            assert_eq!(message, "HTTP 401");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn falls_back_to_api_error_code() {
    let base = server().await;
    let api = ApiClient::new(Some(&format!("{base}/case3")), None).unwrap();
    match api.me().await.unwrap_err() {
        CoreError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 500);
            assert_eq!(code, "api_error");
            assert_eq!(message, "boom");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn rejects_invalid_json_and_handles_empty_body() {
    let base = server().await;
    let bad = ApiClient::new(Some(&format!("{base}/case4")), None).unwrap();
    assert!(matches!(
        bad.me().await.unwrap_err(),
        CoreError::InvalidData(_)
    ));

    let empty = ApiClient::new(Some(&format!("{base}/case5")), None).unwrap();
    assert_eq!(empty.me().await.unwrap().email, "");
}
