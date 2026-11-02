use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, HOST, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{oneshot, RwLock};
use tokio::task::JoinHandle;

use crate::identity::identity_key;
use crate::sanitize::urlencode;

pub const DEFAULT_TOKEN: &str = "demo-token";
pub const DEMO_EMAIL: &str = "desktop@picstome.test";

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn photo_bytes(seed: &str, size: usize) -> Vec<u8> {
    let header = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46];
    let pattern = Sha256::digest(seed.as_bytes());
    let mut out = vec![0u8; size.max(header.len())];
    out[..header.len()].copy_from_slice(&header);
    let mut offset = header.len();
    while offset < out.len() {
        let remaining = out.len() - offset;
        let take = remaining.min(pattern.len());
        out[offset..offset + take].copy_from_slice(&pattern[..take]);
        offset += take;
    }
    out
}

#[derive(Debug, Clone)]
pub struct PhotoRecord {
    pub id: String,
    pub filename: String,
    pub identity_key: String,
    pub data: Arc<Vec<u8>>,
    pub hash: String,
    pub version_count: u64,
    pub version_number: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct GalleryRecord {
    pub id: String,
    pub name: String,
    pub rev: u64,
    pub photos: Vec<PhotoRecord>,
}

#[derive(Debug)]
pub struct MockState {
    pub galleries: RwLock<HashMap<String, GalleryRecord>>,
    pub token: String,
    codes: RwLock<HashMap<String, u64>>,
    rev: AtomicU64,
    next_id: AtomicU64,
    photo_size: usize,
}

impl MockState {
    fn new(photo_size: usize, token: String) -> Self {
        Self {
            galleries: RwLock::new(seed_galleries(photo_size)),
            token,
            codes: RwLock::new(HashMap::new()),
            rev: AtomicU64::new(1),
            next_id: AtomicU64::new(1000),
            photo_size,
        }
    }

    fn touch(&self, gallery: &mut GalleryRecord) {
        gallery.rev += 1;
        self.rev.fetch_add(1, Ordering::SeqCst);
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }
}

fn make_photo(id: String, filename: String, seed: &str, size: usize, version: u64) -> PhotoRecord {
    let data = Arc::new(photo_bytes(&format!("{seed}:{version}"), size));
    let hash = sha256_hex(&data);
    PhotoRecord {
        id,
        identity_key: identity_key(&filename).unwrap_or_default(),
        filename,
        data,
        hash,
        version_count: version,
        version_number: version,
        updated_at: iso_now(),
    }
}

fn iso_now() -> String {
    // RFC3339-ish timestamp without extra deps; stable enough for fixtures.
    let seconds = unix_now();
    format!("{seconds}")
}

pub fn seed_galleries(photo_size: usize) -> HashMap<String, GalleryRecord> {
    let mut galleries = HashMap::new();

    let mut weddings = Vec::new();
    for index in 1..=50 {
        let filename = format!("1Z8A43{index:02}.jpg");
        weddings.push(make_photo(
            format!("p{index:03}"),
            filename,
            &format!("wedding-{index}"),
            photo_size,
            1,
        ));
    }

    let mut portraits = Vec::new();
    for index in 1..=8 {
        let filename = format!("IMG_{index:04}.jpg");
        portraits.push(make_photo(
            format!("s{index:03}"),
            filename,
            &format!("portrait-{index}"),
            photo_size,
            1,
        ));
    }

    galleries.insert(
        "G123".to_string(),
        GalleryRecord {
            id: "G123".into(),
            name: "MiBoda".into(),
            rev: 1,
            photos: weddings,
        },
    );
    galleries.insert(
        "G456".to_string(),
        GalleryRecord {
            id: "G456".into(),
            name: "Retrato Estudio".into(),
            rev: 1,
            photos: portraits,
        },
    );

    galleries
}

#[derive(Debug, Clone)]
pub struct MockServerOptions {
    pub host: String,
    pub port: u16,
    pub photo_size: usize,
    pub token: String,
    pub latency_ms: u64,
}

impl Default for MockServerOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 0,
            photo_size: 32 * 1024,
            token: DEFAULT_TOKEN.into(),
            latency_ms: 0,
        }
    }
}

pub struct MockServer {
    pub url: String,
    pub token: String,
    pub state: Arc<MockState>,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
}

impl MockServer {
    pub async fn close(mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        let _ = self.handle.await;
    }
}

fn is_authorized(headers: &HeaderMap, state: &MockState) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == format!("Bearer {}", state.token))
        .unwrap_or(false)
}

fn unauthorized() -> Response {
    json_response(
        StatusCode::UNAUTHORIZED,
        json!({"message": "Unauthorized", "code": "unauthorized"}),
    )
}

fn json_response<T: Serialize>(status: StatusCode, value: T) -> Response {
    (status, Json(value)).into_response()
}

fn host_base(headers: &HeaderMap) -> String {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1");
    format!("http://{host}")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn gallery_summary(gallery: &GalleryRecord) -> Value {
    json!({
        "id": gallery.id,
        "name": gallery.name,
        "photo_count": gallery.photos.len(),
        "rev": gallery.rev,
        "updated_at": gallery.photos.iter().map(|photo| photo.updated_at.clone()).max(),
    })
}

fn manifest_photo(photo: &PhotoRecord, base: &str, gallery_id: &str) -> Value {
    let expires = unix_now() + 15 * 60;
    let sig = sha256_hex(format!("{gallery_id}:{}:{expires}", photo.id).as_bytes());
    json!({
        "id": photo.id,
        "filename": photo.filename,
        "size": photo.data.len(),
        "hash": photo.hash,
        "hash_algo": "sha256",
        "url": format!("{base}/files/{gallery_id}/{}?expires={expires}&sig={sig}", photo.id),
        "updated_at": photo.updated_at,
        "version_count": photo.version_count,
        "version_number": photo.version_number,
        "identity_key": photo.identity_key,
    })
}

async fn home(State(state): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    let base = host_base(&headers);
    let galleries = state.galleries.read().await;
    let mut rows = String::new();
    for gallery in galleries.values() {
        let link = format!(
            "picstome://sync?gallery={}&name={}",
            urlencode(&gallery.id),
            urlencode(&gallery.name)
        );
        rows.push_str(&format!(
            "<tr><td><strong>{}</strong><br><code>{}</code></td><td>{}</td><td><a class=\"btn\" href=\"{}\">Open in Picstome Sync</a></td><td class=\"actions\"><button data-gallery=\"{}\" data-action=\"add\">+ Add remote photo</button><button data-gallery=\"{}\" data-action=\"bump\">~ Change remote photo</button><button data-gallery=\"{}\" data-action=\"remove\">- Delete remote photo</button></td></tr>",
            escape_html(&gallery.name),
            escape_html(&gallery.id),
            gallery.photos.len(),
            escape_html(&link),
            escape_html(&gallery.id),
            escape_html(&gallery.id),
            escape_html(&gallery.id),
        ));
    }

    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Fake Picstome web — Picstome Sync demo</title>
<style>
:root{{--pine:#316d61;--graphite:#322f30;--coral:#ee6958;--tea:#c6d8af;--floral:#fff9ec}}
*{{box-sizing:border-box}}body{{margin:0;padding:32px;background:linear-gradient(180deg,#fffdf8,#fff9ec);color:var(--graphite);font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}
main{{max-width:980px;margin:0 auto;background:rgba(255,249,236,.94);border:1px solid rgba(49,109,97,.12);border-radius:26px;padding:28px;box-shadow:0 18px 56px rgba(50,47,48,.08)}}
h1{{margin:0 0 6px;color:var(--pine)}}p{{color:#5f5b5c}}table{{width:100%;border-collapse:collapse;margin-top:20px}}th,td{{text-align:left;padding:12px 10px;border-bottom:1px solid rgba(49,109,97,.14);vertical-align:middle}}th{{font-size:12px;text-transform:uppercase;letter-spacing:.12em;color:var(--pine)}}code{{background:rgba(198,216,175,.35);padding:2px 6px;border-radius:6px}}.btn,button{{display:inline-block;padding:10px 14px;border-radius:12px;border:0;background:var(--pine);color:#fff;text-decoration:none;font-weight:700;cursor:pointer;font-size:14px}}button{{background:#fff;color:var(--graphite);border:1.5px solid rgba(50,47,48,.12)}}.actions{{display:flex;gap:6px;flex-wrap:wrap}}.note{{margin-top:24px;padding:14px 16px;border-radius:14px;background:rgba(238,105,88,.08);border:1px solid rgba(238,105,88,.25);color:#8f3f34}}.ok{{color:var(--pine);font-weight:700;margin-top:14px;min-height:20px}}
</style></head><body><main><h1>Fake Picstome web</h1><p>Local mock of Picstome running at <code>{base}</code> with fake photos. Use it to test Picstome Sync without the real API.</p>
<table><thead><tr><th>Gallery</th><th>Photos</th><th>Sync</th><th>Simulate remote changes</th></tr></thead><tbody>{rows}</tbody></table>
<div class="note">Deep links work when Picstome Sync is installed (or running from a dev build). After changing a remote gallery, open the app and press <strong>Update</strong>.</div><div class="ok" id="status"></div></main>
<script>
const status=document.getElementById('status');
for(const button of document.querySelectorAll('button[data-gallery]')){{
  button.addEventListener('click',async()=>{{
    status.textContent='Working…';
    const res=await fetch('/api/v1/dev/'+button.dataset.action,{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{gallery_id:button.dataset.gallery}})}});
    const data=await res.json();
    status.textContent=res.ok?'Remote gallery updated: '+JSON.stringify(data):'Error: '+JSON.stringify(data);
    if(res.ok)setTimeout(()=>location.reload(),700);
  }});
}}
</script></body></html>"#,
        base = escape_html(&base)
    );
    Html(html).into_response()
}

async fn authorize(
    State(state): State<Arc<MockState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let nonce = params.get("nonce").cloned().unwrap_or_default();
    let client_name = params
        .get("client_name")
        .cloned()
        .unwrap_or_else(|| "unknown".into());
    let code: String = {
        let random: u128 = rand::random();
        format!("{random:032x}")
    };
    state.codes.write().await.insert(code.clone(), unix_now());
    let deep_link = format!(
        "picstome://auth?code={}&state={}",
        urlencode(&code),
        urlencode(&nonce)
    );
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>Authorize {client}</title></head><body style="font-family:sans-serif;padding:40px;background:#fff9ec;color:#322f30"><h1 style="color:#316d61">Authorize {client} on this PC?</h1><p>This is the fake Picstome authorization page.</p><p><a href="{link}" style="display:inline-block;padding:14px 18px;border-radius:12px;background:#316d61;color:#fff;text-decoration:none;font-weight:700">Authorize and open Picstome Sync</a></p><p style="color:#5f5b5c">If nothing happens, the app is not registered for the <code>picstome://</code> protocol.</p></body></html>"#,
        client = escape_html(&client_name),
        link = escape_html(&deep_link)
    );
    Html(html).into_response()
}

async fn dev_login(State(state): State<Arc<MockState>>) -> Response {
    json_response(
        StatusCode::OK,
        json!({
            "token": state.token,
            "email": DEMO_EMAIL,
            "expires_at": (unix_now() + 30 * 24 * 3600).to_string(),
        }),
    )
}

async fn exchange(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Response {
    let code = body.get("code").and_then(Value::as_str).unwrap_or_default();
    let created = state.codes.write().await.remove(code);
    let Some(created) = created else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"message": "Invalid code", "code": "invalid_code"}),
        );
    };
    if unix_now().saturating_sub(created) > 5 * 60 {
        return json_response(
            StatusCode::GONE,
            json!({"message": "Expired code", "code": "expired_code"}),
        );
    }
    json_response(
        StatusCode::OK,
        json!({
            "token": state.token,
            "email": DEMO_EMAIL,
            "expires_at": (unix_now() + 30 * 24 * 3600).to_string(),
        }),
    )
}

async fn revoke(State(state): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, &state) {
        return unauthorized();
    }
    json_response(StatusCode::OK, json!({"ok": true}))
}

async fn me(State(state): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, &state) {
        return unauthorized();
    }
    json_response(
        StatusCode::OK,
        json!({"email": DEMO_EMAIL, "display_name": "Desktop Demo"}),
    )
}

async fn galleries(State(state): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, &state) {
        return unauthorized();
    }
    let galleries = state.galleries.read().await;
    let items: Vec<Value> = galleries.values().map(gallery_summary).collect();
    json_response(StatusCode::OK, items)
}

async fn manifest(
    State(state): State<Arc<MockState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !is_authorized(&headers, &state) {
        return unauthorized();
    }
    let gallery_id = params.get("gallery").cloned().unwrap_or_default();
    let galleries = state.galleries.read().await;
    let Some(gallery) = galleries.get(&gallery_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "Unknown gallery", "code": "unknown_gallery"}),
        );
    };
    let base = host_base(&headers);
    let photos: Vec<Value> = gallery
        .photos
        .iter()
        .map(|photo| manifest_photo(photo, &base, &gallery.id))
        .collect();
    json_response(
        StatusCode::OK,
        json!({
            "gallery_id": gallery.id,
            "name": gallery.name,
            "rev": gallery.rev,
            "photos": photos,
        }),
    )
}

async fn upload_photo(
    State(state): State<Arc<MockState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if !is_authorized(&headers, &state) {
        return unauthorized();
    }

    let mut gallery_id = String::new();
    let mut filename = String::new();
    let mut data: Vec<u8> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "gallery_id" {
            gallery_id = field.text().await.unwrap_or_default();
        } else if name == "file" {
            filename = field.file_name().unwrap_or_default().to_string();
            data = field
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .unwrap_or_default();
        }
    }

    let Some(key) = identity_key(&filename) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"message": "Invalid filename", "code": "invalid_filename"}),
        );
    };

    let mut galleries = state.galleries.write().await;
    let Some(gallery) = galleries.get_mut(&gallery_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "Unknown gallery", "code": "unknown_gallery"}),
        );
    };

    let hash = sha256_hex(&data);
    let existing = gallery
        .photos
        .iter()
        .position(|photo| photo.identity_key == key);

    if let Some(index) = existing {
        if gallery.photos[index].hash == hash {
            let photo = &gallery.photos[index];
            return json_response(
                StatusCode::OK,
                json!({
                    "id": photo.id,
                    "filename": photo.filename,
                    "rev": gallery.rev,
                    "version_count": photo.version_count,
                    "version_number": photo.version_number,
                    "unchanged": true,
                }),
            );
        }
        let photo = &mut gallery.photos[index];
        photo.data = Arc::new(data);
        photo.hash = hash;
        photo.version_count += 1;
        photo.version_number = photo.version_count;
        photo.updated_at = iso_now();
        let response = json!({
            "id": photo.id,
            "filename": photo.filename,
            "rev": gallery.rev + 1,
            "version_count": photo.version_count,
            "version_number": photo.version_number,
        });
        state.touch(gallery);
        return json_response(StatusCode::OK, response);
    }

    let id = state.next_id();
    let photo = make_photo(
        format!("p{id}"),
        filename,
        &format!("upload-{id}"),
        data.len(),
        1,
    );
    let mut photo = photo;
    photo.data = Arc::new(data);
    photo.hash = hash;
    let response = json!({
        "id": photo.id,
        "filename": photo.filename,
        "rev": gallery.rev + 1,
        "version_count": 1,
        "version_number": 1,
    });
    gallery.photos.push(photo);
    state.touch(gallery);
    json_response(StatusCode::OK, response)
}

async fn delete_photos(
    State(state): State<Arc<MockState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !is_authorized(&headers, &state) {
        return unauthorized();
    }
    let gallery_id = body
        .get("gallery_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let ids: Vec<String> = body
        .get("photo_ids")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut galleries = state.galleries.write().await;
    let Some(gallery) = galleries.get_mut(&gallery_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "Unknown gallery", "code": "unknown_gallery"}),
        );
    };

    let mut deleted = Vec::new();
    let mut not_found = Vec::new();
    for id in ids {
        if let Some(index) = gallery.photos.iter().position(|photo| photo.id == id) {
            gallery.photos.remove(index);
            deleted.push(id);
        } else {
            not_found.push(id);
        }
    }
    if !deleted.is_empty() {
        state.touch(gallery);
    }
    json_response(
        StatusCode::OK,
        json!({"deleted": deleted, "not_found": not_found, "rev": gallery.rev}),
    )
}

async fn file(
    State(state): State<Arc<MockState>>,
    AxumPath((gallery_id, photo_id)): AxumPath<(String, String)>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (data, hash) = {
        let galleries = state.galleries.read().await;
        let Some(gallery) = galleries.get(&gallery_id) else {
            return (StatusCode::NOT_FOUND, "Not found").into_response();
        };
        let Some(photo) = gallery.photos.iter().find(|photo| photo.id == photo_id) else {
            return (StatusCode::NOT_FOUND, "Not found").into_response();
        };
        (photo.data.clone(), photo.hash.clone())
    };

    if let Some(expires) = params
        .get("expires")
        .and_then(|value| value.parse::<u64>().ok())
    {
        if let Some(sig) = params.get("sig") {
            let expected = sha256_hex(format!("{gallery_id}:{photo_id}:{expires}").as_bytes());
            if sig != &expected {
                return (StatusCode::FORBIDDEN, "Bad signature").into_response();
            }
        }
    }

    let total = data.len();
    let etag = format!("\"{hash}\"");

    if let Some(range) = headers.get(RANGE).and_then(|value| value.to_str().ok()) {
        let range = range.trim_start_matches("bytes=");
        let mut parts = range.split('-');
        let start = parts
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let end = parts
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(total.saturating_sub(1))
            .min(total.saturating_sub(1));
        if start > end {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
            response.headers_mut().insert(
                CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{total}")).unwrap(),
            );
            return response;
        }
        let chunk = data[start..=end].to_vec();
        let length = chunk.len();
        let mut response = Response::new(Body::from(chunk));
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        let headers = response.headers_mut();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from_str(&length.to_string()).unwrap(),
        );
        headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total}")).unwrap(),
        );
        headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        headers.insert(ETAG, HeaderValue::from_str(&etag).unwrap());
        return response;
    }

    let mut response = Response::new(Body::from((*data).clone()));
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&total.to_string()).unwrap(),
    );
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(ETAG, HeaderValue::from_str(&etag).unwrap());
    response
}

async fn dev_reset(State(state): State<Arc<MockState>>) -> Response {
    *state.galleries.write().await = seed_galleries(state.photo_size);
    json_response(StatusCode::OK, json!({"ok": true}))
}

async fn dev_add(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Response {
    let gallery_id = body
        .get("gallery_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut galleries = state.galleries.write().await;
    let Some(gallery) = galleries.get_mut(&gallery_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "Unknown gallery", "code": "unknown_gallery"}),
        );
    };
    let id = state.next_id();
    let filename = body
        .get("filename")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("NEW_{id:04}.jpg"));
    let photo = make_photo(
        format!("p{id}"),
        filename.clone(),
        &format!("remote-new-{id}"),
        state.photo_size,
        1,
    );
    gallery.photos.push(photo);
    state.touch(gallery);
    json_response(
        StatusCode::OK,
        json!({"ok": true, "photo": {"filename": filename}}),
    )
}

async fn dev_bump(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Response {
    let gallery_id = body
        .get("gallery_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let photo_id = body.get("photo_id").and_then(Value::as_str);
    let mut galleries = state.galleries.write().await;
    let Some(gallery) = galleries.get_mut(&gallery_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "Unknown gallery", "code": "unknown_gallery"}),
        );
    };
    let Some(photo) = gallery
        .photos
        .iter_mut()
        .find(|photo| photo_id.map(|id| id == photo.id).unwrap_or(true))
    else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "No photos", "code": "not_found"}),
        );
    };
    let next_version = photo.version_count + 1;
    let size = photo.data.len();
    let data = Arc::new(photo_bytes(&format!("{}:{next_version}", photo.id), size));
    photo.hash = sha256_hex(&data);
    photo.data = data;
    photo.version_count = next_version;
    photo.version_number = next_version;
    photo.updated_at = iso_now();
    let response = json!({
        "ok": true,
        "photo": {
            "id": photo.id,
            "filename": photo.filename,
            "version_count": photo.version_count,
        }
    });
    state.touch(gallery);
    json_response(StatusCode::OK, response)
}

async fn dev_remove(State(state): State<Arc<MockState>>, Json(body): Json<Value>) -> Response {
    let gallery_id = body
        .get("gallery_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let photo_id = body.get("photo_id").and_then(Value::as_str);
    let mut galleries = state.galleries.write().await;
    let Some(gallery) = galleries.get_mut(&gallery_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "Unknown gallery", "code": "unknown_gallery"}),
        );
    };
    let index = match photo_id {
        Some(id) => gallery.photos.iter().position(|photo| photo.id == id),
        None => gallery.photos.len().checked_sub(1),
    };
    let Some(index) = index else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"message": "No photos", "code": "not_found"}),
        );
    };
    let removed = gallery.photos.remove(index);
    state.touch(gallery);
    json_response(
        StatusCode::OK,
        json!({"ok": true, "photo": {"id": removed.id, "filename": removed.filename}}),
    )
}

pub async fn start_mock_server(options: MockServerOptions) -> std::io::Result<MockServer> {
    let state = Arc::new(MockState::new(options.photo_size, options.token.clone()));

    let app = Router::new()
        .route("/", get(home))
        .route("/authorize", get(authorize))
        .route("/api/v1/dev/login", post(dev_login))
        .route("/api/v1/dev/reset", post(dev_reset))
        .route("/api/v1/dev/add", post(dev_add))
        .route("/api/v1/dev/bump", post(dev_bump))
        .route("/api/v1/dev/remove", post(dev_remove))
        .route("/api/v1/tokens/exchange", post(exchange))
        .route("/api/v1/tokens/revoke", post(revoke))
        .route("/api/v1/me", get(me))
        .route("/api/v1/galleries", get(galleries))
        .route("/api/v1/manifest", get(manifest))
        .route("/api/v1/photos", post(upload_photo).delete(delete_photos))
        .route("/files/:gallery/:photo", get(file))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind((options.host.as_str(), options.port)).await?;
    let address = listener.local_addr()?;
    let url = format!("http://{}", address);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    Ok(MockServer {
        url,
        token: options.token,
        state,
        shutdown: Some(shutdown_tx),
        handle,
    })
}
