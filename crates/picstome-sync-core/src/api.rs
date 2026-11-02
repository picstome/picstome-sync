use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use reqwest::multipart::{Form, Part};
use reqwest::{Client, Method, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tokio_util::io::ReaderStream;
use url::Url;

use crate::error::{CoreError, CoreResult};
use crate::model::{
    DeleteResponse, GallerySummary, Manifest, TokenResponse, UploadResponse, CLIENT_NAME,
    DEFAULT_API_BASE,
};

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Me {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    token: Arc<RwLock<Option<String>>>,
    client: Client,
    client_name: String,
}

fn mime_for(filename: &str) -> &'static str {
    match crate::identity::extension_of(filename).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "dng" => "image/x-adobe-dng",
        "cr3" => "image/x-canon-cr3",
        "cr2" => "image/x-canon-cr2",
        "arw" => "image/x-sony-arw",
        "nef" => "image/x-nikon-nef",
        "raf" => "image/x-fuji-raf",
        "orf" => "image/x-olympus-orf",
        "rw2" => "image/x-panasonic-rw2",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

impl ApiClient {
    pub fn new(base_url: Option<&str>, token: Option<String>) -> CoreResult<Self> {
        let base_url = base_url
            .unwrap_or(DEFAULT_API_BASE)
            .trim_end_matches('/')
            .to_string();
        let mut default_headers = HeaderMap::new();
        default_headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let client = Client::builder()
            .user_agent("Picstome Sync")
            .default_headers(default_headers)
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            base_url,
            token: Arc::new(RwLock::new(token)),
            client,
            client_name: CLIENT_NAME.to_string(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn http(&self) -> Client {
        self.client.clone()
    }

    pub async fn token(&self) -> Option<String> {
        self.token.read().await.clone()
    }

    pub async fn set_token(&self, token: Option<String>) {
        *self.token.write().await = token;
    }

    pub async fn has_token(&self) -> bool {
        self.token
            .read()
            .await
            .as_ref()
            .map(|token| !token.is_empty())
            .unwrap_or(false)
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}{}",
            self.base_url,
            if path.starts_with('/') {
                path.to_string()
            } else {
                format!("/{path}")
            }
        )
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> CoreResult<Response> {
        let mut request = self.client.request(method, self.url(path));
        if let Some(token) = self.token().await {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().await.map_err(Into::into)
    }

    async fn decode<T: DeserializeOwned>(response: Response) -> CoreResult<T> {
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            let parsed: Option<ErrorBody> = serde_json::from_str(&text).ok();
            let code = parsed
                .as_ref()
                .and_then(|body| body.code.clone())
                .unwrap_or_else(|| {
                    if status.as_u16() == 401 {
                        "unauthorized".to_string()
                    } else {
                        "api_error".to_string()
                    }
                });
            let message = parsed
                .and_then(|body| body.message)
                .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
            return Err(CoreError::Api {
                status: status.as_u16(),
                code,
                message,
            });
        }
        if text.is_empty() {
            return serde_json::from_str("null").map_err(Into::into);
        }
        serde_json::from_str(&text).map_err(Into::into)
    }

    pub fn authorize_url(&self, nonce: &str) -> CoreResult<String> {
        let mut url = Url::parse(&format!("{}/authorize", self.base_url))?;
        url.query_pairs_mut()
            .append_pair("nonce", nonce)
            .append_pair("client_name", &self.client_name);
        Ok(url.to_string())
    }

    pub async fn exchange_code(&self, code: &str) -> CoreResult<TokenResponse> {
        let response = self
            .send(
                Method::POST,
                "/api/v1/tokens/exchange",
                Some(serde_json::json!({
                    "code": code,
                    "client_name": self.client_name,
                })),
            )
            .await?;
        let token: TokenResponse = Self::decode(response).await?;
        self.set_token(Some(token.token.clone())).await;
        Ok(token)
    }

    pub async fn revoke(&self) -> CoreResult<Value> {
        let response = self
            .send(
                Method::POST,
                "/api/v1/tokens/revoke",
                Some(serde_json::json!({})),
            )
            .await?;
        let value: Value = Self::decode(response).await?;
        self.set_token(None).await;
        Ok(value)
    }

    pub async fn me(&self) -> CoreResult<Me> {
        let response = self.send(Method::GET, "/api/v1/me", None).await?;
        let value: Value = Self::decode(response).await?;
        Ok(Me {
            email: value
                .get("email")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            display_name: value
                .get("display_name")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    pub async fn galleries(&self) -> CoreResult<Vec<GallerySummary>> {
        let response = self.send(Method::GET, "/api/v1/galleries", None).await?;
        Self::decode(response).await
    }

    pub async fn manifest(&self, gallery_id: &str) -> CoreResult<Manifest> {
        let response = self
            .send(
                Method::GET,
                &format!("/api/v1/manifest?gallery={}", urlencoding(gallery_id)),
                None,
            )
            .await?;
        Self::decode(response).await
    }

    pub async fn upload_photo(
        &self,
        gallery_id: &str,
        filename: &str,
        file_path: &Path,
        size: u64,
    ) -> CoreResult<UploadResponse> {
        let file = tokio::fs::File::open(file_path).await?;
        let stream = ReaderStream::new(file);
        let part = Part::stream_with_length(reqwest::Body::wrap_stream(stream), size)
            .file_name(crate::sanitize::escape_header_value(filename))
            .mime_str(mime_for(filename))?;
        let form = Form::new()
            .text("gallery_id", gallery_id.to_string())
            .text("filename", filename.to_string())
            .part("file", part);

        let mut request = self.client.post(self.url("/api/v1/photos")).multipart(form);
        if let Some(token) = self.token().await {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        Self::decode(response).await
    }

    pub async fn delete_photos(
        &self,
        gallery_id: &str,
        photo_ids: &[String],
    ) -> CoreResult<DeleteResponse> {
        let response = self
            .send(
                Method::DELETE,
                "/api/v1/photos",
                Some(serde_json::json!({
                    "gallery_id": gallery_id,
                    "photo_ids": photo_ids,
                })),
            )
            .await?;
        Self::decode(response).await
    }

    pub async fn dev_login(&self) -> CoreResult<TokenResponse> {
        let response = self
            .send(
                Method::POST,
                "/api/v1/dev/login",
                Some(serde_json::json!({})),
            )
            .await?;
        let token: TokenResponse = Self::decode(response).await?;
        self.set_token(Some(token.token.clone())).await;
        Ok(token)
    }
}

fn urlencoding(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[derive(Debug, Serialize, Deserialize)]
struct _Unused;
