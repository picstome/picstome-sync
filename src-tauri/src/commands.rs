use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use picstome_sync_core::config::Store;
use picstome_sync_core::error::ErrorPayload;
use picstome_sync_core::mock::MockServer;
use picstome_sync_core::model::{
    AppState as AppStateView, AuthState, ConfigView, DeleteResponse, FavoritePhoto, GalleryEntry,
    GallerySummary, GalleryView, Progress, SyncReport, DEFAULT_API_BASE,
};
use picstome_sync_core::sanitize::{sanitize_file_names, sanitize_gallery_id};
use picstome_sync_core::sync::{
    apply_remote_deletions, folder_exists, mark_favorites_rated,
    resolve_remote_deletions as resolve_remote_deletions_core, run_gallery_sync,
    RemoteDeleteAction, SyncOptions,
};
use picstome_sync_core::ApiClient;

use crate::auth;
use crate::rate::{self, RateResult};
use crate::strings;

pub struct AppState {
    pub store: Store,
    pub api: tokio::sync::RwLock<ApiClient>,
    pub mock: AtomicBool,
    pub lang: String,
    pub token_store: auth::TokenStore,
    pub app_version: String,
    pub build_profile: String,
    pub exiftool: Option<PathBuf>,
    pub auth_nonce: std::sync::Mutex<Option<String>>,
    pending_link: std::sync::Mutex<Option<String>>,
    last_link: std::sync::Mutex<Option<(String, Instant)>>,
    frontend_ready: AtomicBool,
    pub _mock_server: std::sync::Mutex<Option<MockServer>>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Store,
        api: ApiClient,
        mock: bool,
        token_store: auth::TokenStore,
        app_version: String,
        build_profile: String,
        exiftool: Option<PathBuf>,
        mock_server: Option<MockServer>,
    ) -> Self {
        Self {
            store,
            api: tokio::sync::RwLock::new(api),
            mock: AtomicBool::new(mock),
            lang: strings::language(),
            token_store,
            app_version,
            build_profile,
            exiftool,
            auth_nonce: std::sync::Mutex::new(None),
            pending_link: std::sync::Mutex::new(None),
            last_link: std::sync::Mutex::new(None),
            frontend_ready: AtomicBool::new(false),
            _mock_server: std::sync::Mutex::new(mock_server),
        }
    }

    pub async fn api(&self) -> ApiClient {
        self.api.read().await.clone()
    }

    pub async fn replace_api(&self, api: ApiClient) {
        *self.api.write().await = api;
    }

    pub fn is_mock(&self) -> bool {
        self.mock.load(Ordering::Relaxed)
    }

    pub fn set_mock(&self, value: bool) {
        self.mock.store(value, Ordering::Relaxed);
    }

    pub fn frontend_is_ready(&self) -> bool {
        self.frontend_ready.load(Ordering::SeqCst)
    }

    pub fn defer_link(&self, raw: &str) {
        *self.pending_link.lock().unwrap() = Some(raw.to_string());
    }

    pub fn mark_frontend_ready(&self) -> Option<String> {
        self.frontend_ready.store(true, Ordering::SeqCst);
        self.pending_link.lock().unwrap().take()
    }

    pub fn mark_frontend_closed(&self) {
        self.frontend_ready.store(false, Ordering::SeqCst);
    }

    pub fn should_process_link(&self, raw: &str) -> bool {
        let now = Instant::now();
        let mut last = self.last_link.lock().unwrap();
        if let Some((previous, at)) = last.as_ref() {
            if previous == raw && now.duration_since(*at) < Duration::from_secs(2) {
                return false;
            }
        }
        *last = Some((raw.to_string(), now));
        true
    }
}

pub fn api_base_from_env_or_config(store: &Store) -> String {
    if let Ok(value) = std::env::var("PICSTOME_API_BASE") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    store
        .load_config()
        .api_base
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

pub fn exiftool_path(app: &AppHandle) -> Option<PathBuf> {
    let binary = if cfg!(windows) {
        "exiftool.exe"
    } else {
        "exiftool"
    };

    let mut candidates = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("resources").join(binary));
        candidates.push(dir.join(binary));
    }
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(
            Path::new(manifest_dir)
                .join("..")
                .join("resources")
                .join(binary),
        );
    }

    candidates.into_iter().find(|path| path.is_file())
}

pub async fn build_state(state: &AppState) -> AppStateView {
    let config = state.store.load_config();
    let api = state.api().await;
    let connected = api.has_token().await;
    let email = if state.is_mock() && connected {
        Some("desktop@picstome.test".to_string())
    } else {
        config.account_email.clone()
    };
    let name = if state.is_mock() && connected {
        Some("Picstome Sync Demo".to_string())
    } else {
        config
            .account_name
            .clone()
            .or_else(|| config.account_email.clone())
    };

    let galleries = state
        .store
        .load_index()
        .into_iter()
        .map(|entry| {
            let (photo_count, local_bytes) = state
                .store
                .load_snapshot(&entry.gallery_id)
                .map(|snapshot| {
                    if snapshot.local_photo_count == 0 && snapshot.local_bytes == 0 {
                        let local: Vec<_> = snapshot
                            .files
                            .iter()
                            .filter(|file| file.mtime.is_some())
                            .collect();
                        (
                            local.len() as u64,
                            local.iter().filter_map(|file| file.size).sum(),
                        )
                    } else {
                        (snapshot.local_photo_count, snapshot.local_bytes)
                    }
                })
                .unwrap_or((0, 0));
            GalleryView {
                exists: folder_exists(&entry.local_path),
                entry,
                photo_count,
                local_bytes,
            }
        })
        .collect();

    AppStateView {
        lang: state.lang.clone(),
        strings: strings::strings(&state.lang),
        config: ConfigView {
            api_base: api.base_url().to_string(),
            api_kind: picstome_sync_core::model::api_kind(api.base_url(), state.is_mock())
                .to_string(),
            photos_dir: config.photos_dir.filter(|dir| folder_exists(dir)),
            last_rating: match config.last_rating.as_deref() {
                Some("2") => "2",
                Some("3") => "3",
                Some("4") => "4",
                Some("5") => "5",
                _ => "1",
            }
            .to_string(),
            app_version: state.app_version.clone(),
            build_profile: state.build_profile.clone(),
        },
        mock: state.is_mock(),
        auth: AuthState {
            connected,
            email,
            name,
            account_email: config.account_email.clone(),
        },
        galleries,
    }
}

async fn emit_state(app: &AppHandle, state: &AppState) {
    let view = build_state(state).await;
    let _ = app.emit("app-state", view);
}

fn command_error(message: impl Into<String>, code: &str) -> ErrorPayload {
    ErrorPayload {
        message: message.into(),
        code: code.to_string(),
        status: 0,
    }
}

fn invalid_gallery() -> ErrorPayload {
    command_error("Invalid gallery", "invalid_gallery")
}

fn folder_missing() -> ErrorPayload {
    command_error("Folder not found", "folder_missing")
}

fn not_linked() -> ErrorPayload {
    command_error("Gallery is not linked", "not_linked")
}

const LOG_MAX_BYTES: u64 = 512 * 1024;
const LOG_KEEP_LINES: usize = 500;

fn logs_dir(store: &Store) -> std::path::PathBuf {
    store.root().join("logs")
}

fn gallery_log_path(store: &Store, gallery_id: &str) -> std::path::PathBuf {
    logs_dir(store).join(format!("{gallery_id}.log"))
}

fn legacy_log_path(store: &Store) -> std::path::PathBuf {
    store.root().join("sync.log")
}

fn write_log_file(path: &std::path::Path, lines: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("log.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    for line in lines {
        writeln!(file, "{line}")?;
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)
}

fn rotate_log_if_needed(path: &std::path::Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() <= LOG_MAX_BYTES {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(LOG_KEEP_LINES);
    let lines: Vec<String> = all[start..]
        .iter()
        .map(|line| (*line).to_string())
        .collect();
    let _ = write_log_file(path, &lines);
}

fn append_log_line(path: std::path::PathBuf, entry: serde_json::Value) {
    use std::io::Write;
    rotate_log_if_needed(&path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(path) {
        let _ = writeln!(file, "{entry}");
    }
}

fn append_rating_log(
    store: &Store,
    gallery_id: Option<&str>,
    rating: &str,
    result: &RateResult,
    source: &str,
) {
    let entry = json!({
        "kind": "rating",
        "at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0),
        "galleryId": gallery_id,
        "rating": rating,
        "source": source,
        "rated": result.rated,
        "notFound": result.not_found,
        "errors": result.errors,
        "files": result.details.iter().map(|detail| json!({
            "name": detail.name,
            "file": detail.file,
            "status": detail.status,
            "message": detail.msg,
        })).collect::<Vec<_>>(),
    });
    let path = match gallery_id {
        Some(id) => gallery_log_path(store, id),
        None => logs_dir(store).join("_unlinked.log"),
    };
    append_log_line(path, entry);
}

fn append_sync_log(store: &Store, report: &SyncReport) {
    let entry = json!({
        "kind": "sync",
        "at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0),
        "galleryId": report.gallery_id,
        "counts": report.counts,
        "uploaded": report.uploaded.iter().map(|item| json!({
            "filename": item.filename,
            "versionNumber": item.version_number,
            "reason": item.reason,
            "size": item.size,
        })).collect::<Vec<_>>(),
        "downloaded": report.downloaded.iter().map(|item| item.filename.clone()).collect::<Vec<_>>(),
        "localDeletedProposed": report.proposals.delete_remote.iter().map(|item| json!({
            "id": item.id,
            "filename": item.filename,
        })).collect::<Vec<_>>(),
        "remoteDeleted": report.remote_deleted.iter().map(|item| json!({
            "id": item.id,
            "filename": item.filename,
            "relativePath": item.relative_path,
        })).collect::<Vec<_>>(),
        "conflicts": report.conflicts.iter().map(|item| json!({
            "filename": item.local.filename,
            "remoteFilename": item.remote.filename,
        })).collect::<Vec<_>>(),
        "errors": report.errors.iter().map(|item| item.message.clone()).collect::<Vec<_>>(),
        "warnings": report.warnings.iter().map(|item| json!({
            "kind": item.kind,
            "filename": item.filename,
            "preferred": item.preferred,
            "path": item.path,
            "message": item.message,
        })).collect::<Vec<_>>(),
    });
    append_log_line(gallery_log_path(store, &report.gallery_id), entry);
}

fn append_sync_error_log(store: &Store, gallery_id: &str, message: &str) {
    let entry = json!({
        "kind": "sync_error",
        "at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0),
        "galleryId": gallery_id,
        "message": message,
    });
    append_log_line(gallery_log_path(store, gallery_id), entry);
}

#[allow(clippy::too_many_arguments)]
fn append_deletion_log(
    store: &Store,
    gallery_id: &str,
    action: &str,
    files: Vec<Value>,
    resolved: usize,
    trashed: Vec<Value>,
    errors: Vec<String>,
) {
    let entry = json!({
        "kind": "deletion",
        "at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0),
        "galleryId": gallery_id,
        "action": action,
        "resolved": resolved,
        "files": files,
        "trashed": trashed,
        "errors": errors,
    });
    append_log_line(gallery_log_path(store, gallery_id), entry);
}

fn read_log_entries(path: &std::path::Path, max_lines: usize) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..]
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect()
}

fn append_raw_log_lines(path: &std::path::Path, lines: &[String]) {
    use std::io::Write;
    if lines.is_empty() {
        return;
    }
    rotate_log_if_needed(path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(path) {
        for line in lines {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Moves the old single sync.log into the per-gallery logs once, then removes it.
/// Existing lines are deduplicated, so an interrupted migration can be retried safely.
pub fn migrate_legacy_log(store: &Store) {
    let legacy = legacy_log_path(store);
    let Ok(text) = std::fs::read_to_string(&legacy) else {
        return;
    };
    let mut groups: std::collections::HashMap<std::path::PathBuf, Vec<String>> =
        std::collections::HashMap::new();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let target = match entry.get("galleryId").and_then(serde_json::Value::as_str) {
            Some(id) if sanitize_gallery_id(id).is_some() => gallery_log_path(store, id),
            _ => logs_dir(store).join("_unlinked.log"),
        };
        groups.entry(target).or_default().push(line.to_string());
    }
    for (path, mut lines) in groups {
        let existing: std::collections::HashSet<String> = std::fs::read_to_string(&path)
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default();
        lines.retain(|line| !existing.contains(line));
        append_raw_log_lines(&path, &lines);
    }
    let _ = std::fs::remove_file(legacy);
}

async fn sync_linked(
    app: &AppHandle,
    state: &AppState,
    gallery_id: &str,
) -> Result<SyncReport, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(gallery_id) else {
        return Err(invalid_gallery());
    };
    let Some(gallery) = state.store.get_gallery(&id) else {
        return Err(not_linked());
    };
    if !folder_exists(&gallery.local_path) {
        append_sync_error_log(&state.store, &id, "Folder not found");
        return Err(folder_missing());
    }

    show_main(app);
    let api = state.api().await;
    let report = match run_gallery_sync(
        &api,
        &state.store,
        &gallery,
        SyncOptions::default(),
        progress_emitter(app.clone()),
    )
    .await
    {
        Ok(report) => report,
        Err(error) => {
            append_sync_error_log(&state.store, &id, &error.to_string());
            return Err(error.into());
        }
    };
    append_sync_log(&state.store, &report);
    emit_state(app, state).await;
    Ok(report)
}

fn patch_linked_folder(
    app: &AppHandle,
    state: &AppState,
    gallery_id: &str,
    reset_snapshot: bool,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(gallery_id) else {
        return Err(invalid_gallery());
    };
    let Some(folder) = pick_folder(app) else {
        return Ok(json!({"ok": false, "canceled": true}));
    };
    state.store.patch_gallery(&id, |entry| {
        entry.local_path = folder.clone();
    })?;
    if reset_snapshot {
        let _ = state.store.delete_snapshot(&id);
    }
    Ok(json!({"ok": true, "localPath": folder}))
}

pub fn pick_folder(app: &AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().to_string())
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> Result<AppStateView, ErrorPayload> {
    Ok(build_state(&state).await)
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<Value, ErrorPayload> {
    serde_json::to_value(state.store.load_config())
        .map_err(|err| command_error(err.to_string(), "invalid_data"))
}

#[tauri::command]
pub fn choose_folder(app: AppHandle, state: State<'_, AppState>) -> Option<String> {
    let dir = pick_folder(&app)?;
    let _ = state.store.update_config(|config| {
        config.photos_dir = Some(dir.clone());
    });
    show_main(&app);
    Some(dir)
}

#[tauri::command]
pub async fn connect(app: AppHandle, state: State<'_, AppState>) -> Result<Value, ErrorPayload> {
    let api = state.api().await;
    if state.is_mock() {
        api.dev_login().await?;
        let _ = state.store.update_config(|config| {
            config.account_email = Some("desktop@picstome.test".into());
            config.account_name = Some("Picstome Sync Demo".into());
        });
        emit_state(&app, &state).await;
        return Ok(json!({"ok": true, "mock": true}));
    }

    let nonce = auth::new_nonce();
    *state.auth_nonce.lock().unwrap() = Some(nonce.clone());
    let url = api.authorize_url(&nonce)?;
    if std::env::var("PICSTOME_DISABLE_BROWSER_OPEN")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        return Ok(json!({"ok": true, "url": url, "opened": false, "disabled": true}));
    }
    match app.opener().open_url(url.clone(), None::<&str>) {
        Ok(()) => Ok(json!({"ok": true, "url": url, "opened": true})),
        Err(err) => {
            // Headless/test environments may not have a browser handler. The
            // URL is still returned so the UI or a test can open it manually.
            eprintln!("Could not open the authorization URL: {err}");
            Ok(json!({
                "ok": true,
                "url": url,
                "opened": false,
                "warning": err.to_string()
            }))
        }
    }
}

#[tauri::command]
pub async fn logout(app: AppHandle, state: State<'_, AppState>) -> Result<Value, ErrorPayload> {
    let api = state.api().await;
    if api.has_token().await {
        let _ = api.revoke().await;
    }
    api.set_token(None).await;
    let _ = state.token_store.save(None);
    let _ = state.store.update_config(|config| {
        config.account_email = None;
    });
    emit_state(&app, &state).await;
    Ok(json!({"ok": true}))
}

#[tauri::command]
pub async fn list_remote_galleries(
    state: State<'_, AppState>,
) -> Result<Vec<GallerySummary>, ErrorPayload> {
    Ok(state.api().await.galleries().await?)
}

#[tauri::command]
pub async fn get_gallery_favorites(
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Vec<FavoritePhoto>, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    let manifest = state.api().await.manifest(&id).await?;
    Ok(manifest.favorite_photos())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSyncResult {
    pub canceled: bool,
    pub report: Option<SyncReport>,
}

fn progress_emitter(app: AppHandle) -> Arc<dyn Fn(Progress) + Send + Sync> {
    Arc::new(move |progress| {
        let _ = app.emit("sync-progress", progress);
    })
}

#[tauri::command]
pub async fn start_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
    name: Option<String>,
) -> Result<StartSyncResult, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };

    if state.store.get_gallery(&id).is_none() {
        let Some(folder) = pick_folder(&app) else {
            return Ok(StartSyncResult {
                canceled: true,
                report: None,
            });
        };
        let Some(_) = state.store.upsert_gallery(GalleryEntry {
            gallery_id: id.clone(),
            name: name.unwrap_or_else(|| "Gallery".into()),
            local_path: folder,
            last_sync_at: None,
            remote_rev: None,
            parent_gallery_id: None,
        })?
        else {
            return Err(invalid_gallery());
        };
    }

    Ok(StartSyncResult {
        canceled: false,
        report: Some(sync_linked(&app, &state, &id).await?),
    })
}

#[tauri::command]
pub async fn sync_gallery(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<SyncReport, ErrorPayload> {
    sync_linked(&app, &state, &gallery_id).await
}

#[tauri::command]
pub fn locate_gallery(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    patch_linked_folder(&app, &state, &gallery_id, false)
}

#[tauri::command]
pub fn change_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    patch_linked_folder(&app, &state, &gallery_id, true)
}

#[tauri::command]
pub fn unlink_gallery(
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    state.store.remove_gallery(&id)?;
    let _ = state.store.delete_snapshot(&id);
    Ok(json!({"ok": true}))
}

#[tauri::command]
pub fn open_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    let Some(gallery) = state.store.get_gallery(&id) else {
        return Err(not_linked());
    };
    if !folder_exists(&gallery.local_path) {
        return Err(folder_missing());
    }
    app.opener()
        .open_path(gallery.local_path, None::<&str>)
        .map_err(|err| command_error(err.to_string(), "open_folder_failed"))?;
    Ok(json!({"ok": true}))
}

#[tauri::command]
pub async fn resolve_remote_deletions(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
    photo_ids: Vec<String>,
    action: String,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    let Some(gallery) = state.store.get_gallery(&id) else {
        return Err(not_linked());
    };
    let action_name = action.clone();
    let action = match action.as_str() {
        "delete_local" => RemoteDeleteAction::DeleteLocal,
        "keep_local" => RemoteDeleteAction::KeepLocal,
        "upload" => RemoteDeleteAction::Upload,
        _ => {
            return Err(command_error(
                "Unknown remote delete action",
                "invalid_action",
            ))
        }
    };
    let api = state.api().await;
    let snapshot = state.store.load_snapshot(&id);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let files: Vec<Value> = photo_ids
        .iter()
        .map(|photo_id| {
            let file = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.files.iter().find(|file| &file.id == photo_id));
            json!({
                "id": photo_id,
                "filename": file.map(|file| file.filename.clone()).unwrap_or_default(),
                "path": file.and_then(|file| file.relative_path.clone()),
            })
        })
        .collect();
    let result =
        match resolve_remote_deletions_core(&api, &state.store, &gallery, &photo_ids, action, now)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                append_deletion_log(
                    &state.store,
                    &id,
                    &action_name,
                    files,
                    0,
                    Vec::new(),
                    vec![error.to_string()],
                );
                return Err(error.into());
            }
        };
    append_deletion_log(
        &state.store,
        &id,
        &action_name,
        files,
        result.resolved.len(),
        result
            .trashed
            .iter()
            .map(|item| {
                json!({
                    "id": item.id,
                    "filename": item.filename,
                    "originalPath": item.original_path,
                    "location": item.location,
                    "trashPath": item.trash_path,
                })
            })
            .collect(),
        result
            .errors
            .iter()
            .map(|error| error.message.clone())
            .collect(),
    );
    emit_state(&app, &state).await;
    serde_json::to_value(result).map_err(|err| command_error(err.to_string(), "invalid_data"))
}

#[tauri::command]
pub async fn apply_deletions(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
    photo_ids: Vec<String>,
) -> Result<DeleteResponse, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    let snapshot = state.store.load_snapshot(&id);
    let api = state.api().await;
    let result = match apply_remote_deletions(&api, &state.store, &id, &photo_ids).await {
        Ok(result) => result,
        Err(error) => {
            append_deletion_log(
                &state.store,
                &id,
                "remote_delete",
                photo_ids
                    .iter()
                    .map(|photo_id| json!({"id": photo_id, "status": "failed"}))
                    .collect(),
                0,
                Vec::new(),
                vec![error.to_string()],
            );
            return Err(error.into());
        }
    };
    let deleted: std::collections::HashSet<&String> = result.deleted.iter().collect();
    let not_found: std::collections::HashSet<&String> = result.not_found.iter().collect();
    let files: Vec<Value> = photo_ids
        .iter()
        .map(|photo_id| {
            let filename = snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .files
                    .iter()
                    .find(|file| &file.id == photo_id)
                    .map(|file| file.filename.clone())
            });
            let status = if deleted.contains(photo_id) {
                "deleted"
            } else if not_found.contains(photo_id) {
                "not_found"
            } else {
                "unknown"
            };
            json!({
                "id": photo_id,
                "filename": filename,
                "status": status,
            })
        })
        .collect();
    append_deletion_log(
        &state.store,
        &id,
        "remote_delete",
        files,
        result.deleted.len(),
        Vec::new(),
        Vec::new(),
    );
    emit_state(&app, &state).await;
    Ok(result)
}

#[tauri::command]
pub async fn rate_files(
    state: State<'_, AppState>,
    files: Vec<String>,
    rating: String,
    photos_dir: Option<String>,
    gallery_id: Option<String>,
    favorites: Option<Vec<FavoritePhoto>>,
) -> Result<RateResult, ErrorPayload> {
    let config = state.store.load_config();
    let has_gallery = gallery_id.is_some();
    let gallery_for_marks = gallery_id.clone();
    let trusted = match gallery_id {
        Some(gallery_id) => {
            let Some(id) = sanitize_gallery_id(&gallery_id) else {
                return Err(invalid_gallery());
            };
            let Some(gallery) = state.store.get_gallery(&id) else {
                return Err(not_linked());
            };
            Some(gallery.local_path)
        }
        None => config.photos_dir.clone(),
    };
    let Some(trusted) = trusted else {
        return Ok(rate::rate_files(None, &files, &rating, Path::new(".")).await);
    };

    let trusted_path = std::fs::canonicalize(&trusted)
        .map_err(|err| command_error(err.to_string(), "folder_error"))?;
    let requested_path = match photos_dir {
        Some(requested) if !has_gallery => std::fs::canonicalize(&requested)
            .map_err(|err| command_error(err.to_string(), "folder_error"))?,
        _ => trusted_path.clone(),
    };
    if trusted_path != requested_path {
        return Err(command_error("Folder not accessible", "folder_error"));
    }

    if sanitize_file_names(&files).is_none() {
        return Err(command_error("Invalid request", "invalid_request"));
    }

    let _ = state.store.update_config(|config| {
        config.last_rating = Some(rating.clone());
    });

    let result = rate::rate_files(state.exiftool.as_deref(), &files, &rating, &trusted_path).await;

    let has_favorites = favorites.is_some();
    if let (Some(gallery_id), Some(favorites)) = (gallery_for_marks.clone(), favorites) {
        let successful: std::collections::HashSet<String> = result
            .details
            .iter()
            .filter(|detail| detail.status == "ok")
            .filter_map(|detail| detail.file.as_deref())
            .map(|filename| {
                Path::new(filename)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("")
                    .to_lowercase()
            })
            .collect();
        let marks: Vec<(String, String)> = favorites
            .into_iter()
            .filter_map(|favorite| {
                let key = favorite.identity_key?;
                let favorited_at = favorite.favorited_at?;
                let stem = Path::new(&favorite.filename)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                successful.contains(&stem).then_some((key, favorited_at))
            })
            .collect();
        let _ = mark_favorites_rated(&state.store, &gallery_id, &marks);
    }

    let source = if has_favorites { "favorites" } else { "link" };
    append_rating_log(
        &state.store,
        gallery_for_marks.as_deref(),
        &rating,
        &result,
        source,
    );

    Ok(result)
}
#[tauri::command]
pub async fn open_galleries_page(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, ErrorPayload> {
    let base = state.api().await.base_url().to_string();
    let url = format!("{base}/galleries");
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|err| command_error(err.to_string(), "open_url_failed"))?;
    Ok(json!({"ok": true}))
}

#[tauri::command]
pub async fn open_account_profile(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, ErrorPayload> {
    let base = state.api().await.base_url().to_string();
    let url = format!("{base}/settings/profile");
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|err| command_error(err.to_string(), "open_url_failed"))?;
    Ok(json!({"ok": true}))
}

#[tauri::command]
pub async fn set_api_base(
    app: AppHandle,
    state: State<'_, AppState>,
    base_url: String,
) -> Result<AppStateView, ErrorPayload> {
    let base = picstome_sync_core::model::normalize_api_base(&base_url)?;
    let old_api = state.api().await;
    if old_api.has_token().await {
        let _ = old_api.revoke().await;
    }
    let _ = state.token_store.save(None);

    let new_api = ApiClient::new(Some(&base), None)?;
    state.replace_api(new_api).await;
    state.set_mock(false);
    let _ = state.store.update_config(|config| {
        config.api_base = Some(base.clone());
        config.account_email = None;
        config.account_name = None;
    });

    let view = build_state(&state).await;
    let _ = app.emit("app-state", view.clone());
    let _ = app.emit(
        "auth-status",
        json!({"connected": false, "reason": "api_changed"}),
    );
    Ok(view)
}

#[tauri::command]
pub async fn get_sync_log(
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Vec<serde_json::Value>, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    let mut entries = read_log_entries(&gallery_log_path(&state.store, &id), 50);
    // Backwards compatibility if migration could not remove the old file yet.
    entries.extend(
        read_log_entries(&legacy_log_path(&state.store), 200)
            .into_iter()
            .filter(|entry| {
                entry.get("galleryId").and_then(serde_json::Value::as_str) == Some(id.as_str())
            }),
    );
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .get("at")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
        )
    });
    entries.truncate(50);
    Ok(entries)
}

#[tauri::command]
pub async fn clear_sync_log(
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    let _ = std::fs::remove_file(gallery_log_path(&state.store, &id));
    let legacy = legacy_log_path(&state.store);
    if legacy.exists() {
        let kept: Vec<String> = read_log_entries(&legacy, usize::MAX)
            .into_iter()
            .filter(|entry| {
                entry.get("galleryId").and_then(serde_json::Value::as_str) != Some(id.as_str())
            })
            .map(|entry| entry.to_string())
            .collect();
        let _ = write_log_file(&legacy, &kept);
    }
    Ok(json!({"ok": true}))
}

#[tauri::command]
pub fn frontend_ready(app: AppHandle, state: State<'_, AppState>) -> bool {
    if let Some(raw) = state.mark_frontend_ready() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::deep_link::handle_url(app, raw).await;
        });
    }
    true
}

#[tauri::command]
pub async fn dev_handle_deep_link(app: AppHandle, url: String) -> Result<Value, ErrorPayload> {
    if !cfg!(debug_assertions) {
        return Err(command_error(
            "Deep-link injection is disabled in release builds",
            "not_allowed",
        ));
    }
    crate::deep_link::handle_url(app, url).await;
    Ok(json!({"ok": true}))
}

#[tauri::command]
pub fn install_update() -> Value {
    json!({"ok": false, "reason": "updater_not_configured"})
}

#[tauri::command]
pub async fn check_updates() -> Value {
    json!({"ok": false, "reason": "updater_not_configured"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defers_links_until_frontend_ready_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let api = ApiClient::new(Some("http://127.0.0.1:1"), None).unwrap();
        let token_store = auth::TokenStore::new(dir.path().join("auth.json"));
        let state = AppState::new(
            store,
            api,
            false,
            token_store,
            "test".into(),
            "debug".into(),
            None,
            None,
        );

        assert!(!state.frontend_is_ready());
        state.defer_link("picstome://sync?gallery=G1&name=Test");
        assert_eq!(
            state.mark_frontend_ready().as_deref(),
            Some("picstome://sync?gallery=G1&name=Test")
        );
        assert!(state.frontend_is_ready());
        assert!(state.should_process_link("picstome://sync?gallery=G1&name=Test"));
        assert!(!state.should_process_link("picstome://sync?gallery=G1&name=Test"));
        state.mark_frontend_closed();
        assert!(!state.frontend_is_ready());
    }

    fn legacy_entry(at: i64, gallery: Option<&str>) -> String {
        json!({
            "kind": "sync",
            "at": at,
            "galleryId": gallery,
            "counts": {},
            "uploaded": [],
            "downloaded": [],
            "errors": [],
        })
        .to_string()
    }

    #[test]
    fn migrates_legacy_log_into_per_gallery_logs_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let legacy = store.root().join("sync.log");
        let g1 = legacy_entry(1, Some("G1"));
        let g2 = legacy_entry(2, Some("G2"));
        let unlinked = legacy_entry(3, None);
        let unsafe_gallery = legacy_entry(4, Some("../escape"));
        std::fs::write(
            &legacy,
            [
                g1.as_str(),
                g2.as_str(),
                unlinked.as_str(),
                unsafe_gallery.as_str(),
            ]
            .join("\n"),
        )
        .unwrap();

        migrate_legacy_log(&store);
        assert!(!legacy.exists());
        let logs = store.root().join("logs");
        let g1_log = std::fs::read_to_string(logs.join("G1.log")).unwrap();
        assert_eq!(g1_log.lines().collect::<Vec<_>>(), vec![g1.as_str()]);
        assert!(std::fs::read_to_string(logs.join("G2.log"))
            .unwrap()
            .contains(&g2));
        assert!(std::fs::read_to_string(logs.join("_unlinked.log"))
            .unwrap()
            .contains(&unlinked));
        assert!(!store.root().join("escape.log").exists());

        // Recreating the legacy file must not duplicate imported entries.
        std::fs::write(&legacy, g1.as_str()).unwrap();
        migrate_legacy_log(&store);
        let g1_log = std::fs::read_to_string(logs.join("G1.log")).unwrap();
        assert_eq!(g1_log.lines().count(), 1);
        assert!(!legacy.exists());
    }

    #[test]
    fn reads_only_the_requested_log_tail() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path()).unwrap();
        let logs = store.root().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let path = logs.join("G1.log");
        let lines: Vec<String> = (0..60).map(|at| legacy_entry(at, Some("G1"))).collect();
        std::fs::write(&path, lines.join("\n")).unwrap();

        let entries = read_log_entries(&path, 50);
        assert_eq!(entries.len(), 50);
        assert_eq!(entries[0].get("at").unwrap().as_i64(), Some(10));
        assert_eq!(entries[49].get("at").unwrap().as_i64(), Some(59));
    }
}
