use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use picstome_sync_core::config::Store;
use picstome_sync_core::error::ErrorPayload;
use picstome_sync_core::mock::MockServer;
use picstome_sync_core::model::{
    AppState as AppStateView, AuthState, ConfigView, DeleteResponse, GalleryEntry, GallerySummary,
    GalleryView, Progress, SyncReport, DEFAULT_API_BASE,
};
use picstome_sync_core::sanitize::{sanitize_file_names, sanitize_gallery_id};
use picstome_sync_core::sync::{
    apply_remote_deletions, folder_exists, run_gallery_sync, SyncOptions,
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
    pub app_version: String,
    pub build_profile: String,
    pub exiftool: Option<PathBuf>,
    pub auth_nonce: std::sync::Mutex<Option<String>>,
    pub _mock_server: std::sync::Mutex<Option<MockServer>>,
}

impl AppState {
    pub fn new(
        store: Store,
        api: ApiClient,
        mock: bool,
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
            app_version,
            build_profile,
            exiftool,
            auth_nonce: std::sync::Mutex::new(None),
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
        .map(|entry| GalleryView {
            exists: folder_exists(&entry.local_path),
            entry,
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
        return Err(folder_missing());
    }

    show_main(app);
    let api = state.api().await;
    let report = run_gallery_sync(
        &api,
        &state.store,
        &gallery,
        SyncOptions::default(),
        progress_emitter(app.clone()),
    )
    .await?;
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
    auth::save_token(None);
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
pub async fn apply_deletions(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
    photo_ids: Vec<String>,
) -> Result<DeleteResponse, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(invalid_gallery());
    };
    let api = state.api().await;
    let result = apply_remote_deletions(&api, &state.store, &id, &photo_ids).await?;
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
) -> Result<RateResult, ErrorPayload> {
    let config = state.store.load_config();
    let has_gallery = gallery_id.is_some();
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

    Ok(rate::rate_files(state.exiftool.as_deref(), &files, &rating, &trusted_path).await)
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
    auth::save_token(None);

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
