use std::path::{Path, PathBuf};
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
    pub api: ApiClient,
    pub mock: bool,
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
            api,
            mock,
            lang: strings::language(),
            app_version,
            build_profile,
            exiftool,
            auth_nonce: std::sync::Mutex::new(None),
            _mock_server: std::sync::Mutex::new(mock_server),
        }
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
    let connected = state.api.has_token().await;
    let email = if state.mock && connected {
        Some("desktop@picstome.test".to_string())
    } else {
        config.account_email.clone()
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
            api_base: state.api.base_url().to_string(),
            api_kind: picstome_sync_core::model::api_kind(state.api.base_url(), state.mock)
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
        mock: state.mock,
        auth: AuthState {
            connected,
            email,
            account_email: config.account_email.clone(),
        },
        galleries,
    }
}

async fn emit_state(app: &AppHandle, state: &AppState) {
    let view = build_state(state).await;
    let _ = app.emit("app-state", view);
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
    serde_json::to_value(state.store.load_config()).map_err(|err| ErrorPayload {
        message: err.to_string(),
        code: "invalid_data".into(),
        status: 0,
    })
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
    if state.mock {
        state.api.dev_login().await?;
        let _ = state.store.update_config(|config| {
            config.account_email = Some("desktop@picstome.test".into());
        });
        emit_state(&app, &state).await;
        return Ok(json!({"ok": true, "mock": true}));
    }

    let nonce = auth::new_nonce();
    *state.auth_nonce.lock().unwrap() = Some(nonce.clone());
    let url = state.api.authorize_url(&nonce)?;
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
    if state.api.has_token().await {
        let _ = state.api.revoke().await;
    }
    state.api.set_token(None).await;
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
    Ok(state.api.galleries().await?)
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
        return Err(ErrorPayload {
            message: "Invalid gallery".into(),
            code: "invalid_gallery".into(),
            status: 0,
        });
    };

    let gallery: GalleryEntry = match state.store.get_gallery(&id) {
        Some(gallery) => gallery,
        None => {
            let Some(folder) = pick_folder(&app) else {
                return Ok(StartSyncResult {
                    canceled: true,
                    report: None,
                });
            };
            let Some(gallery) = state.store.upsert_gallery(GalleryEntry {
                gallery_id: id.clone(),
                name: name.unwrap_or_else(|| "Gallery".into()),
                local_path: folder,
                last_sync_at: None,
                remote_rev: None,
                parent_gallery_id: None,
            })?
            else {
                return Err(ErrorPayload {
                    message: "Could not link gallery".into(),
                    code: "invalid_gallery".into(),
                    status: 0,
                });
            };
            gallery
        }
    };

    if !folder_exists(&gallery.local_path) {
        return Err(ErrorPayload {
            message: "Folder not found".into(),
            code: "folder_missing".into(),
            status: 0,
        });
    }

    show_main(&app);
    let report = run_gallery_sync(
        &state.api,
        &state.store,
        &gallery,
        SyncOptions::default(),
        progress_emitter(app.clone()),
    )
    .await?;

    emit_state(&app, &state).await;
    Ok(StartSyncResult {
        canceled: false,
        report: Some(report),
    })
}

#[tauri::command]
pub async fn sync_gallery(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<SyncReport, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(ErrorPayload {
            message: "Invalid gallery".into(),
            code: "invalid_gallery".into(),
            status: 0,
        });
    };
    let Some(gallery) = state.store.get_gallery(&id) else {
        return Err(ErrorPayload {
            message: "Gallery is not linked".into(),
            code: "not_linked".into(),
            status: 0,
        });
    };
    if !folder_exists(&gallery.local_path) {
        return Err(ErrorPayload {
            message: "Folder not found".into(),
            code: "folder_missing".into(),
            status: 0,
        });
    }

    show_main(&app);
    let report = run_gallery_sync(
        &state.api,
        &state.store,
        &gallery,
        SyncOptions::default(),
        progress_emitter(app.clone()),
    )
    .await?;

    emit_state(&app, &state).await;
    Ok(report)
}

#[tauri::command]
pub fn locate_gallery(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(ErrorPayload {
            message: "Invalid gallery".into(),
            code: "invalid_gallery".into(),
            status: 0,
        });
    };
    let Some(folder) = pick_folder(&app) else {
        return Ok(json!({"ok": false, "canceled": true}));
    };
    state.store.patch_gallery(&id, |entry| {
        entry.local_path = folder.clone();
    })?;
    Ok(json!({"ok": true, "localPath": folder}))
}

#[tauri::command]
pub fn change_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(ErrorPayload {
            message: "Invalid gallery".into(),
            code: "invalid_gallery".into(),
            status: 0,
        });
    };
    let Some(folder) = pick_folder(&app) else {
        return Ok(json!({"ok": false, "canceled": true}));
    };
    state.store.patch_gallery(&id, |entry| {
        entry.local_path = folder.clone();
    })?;
    let _ = state.store.delete_snapshot(&id);
    Ok(json!({"ok": true, "localPath": folder}))
}

#[tauri::command]
pub fn unlink_gallery(
    state: State<'_, AppState>,
    gallery_id: String,
) -> Result<Value, ErrorPayload> {
    let Some(id) = sanitize_gallery_id(&gallery_id) else {
        return Err(ErrorPayload {
            message: "Invalid gallery".into(),
            code: "invalid_gallery".into(),
            status: 0,
        });
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
        return Err(ErrorPayload {
            message: "Invalid gallery".into(),
            code: "invalid_gallery".into(),
            status: 0,
        });
    };
    let Some(gallery) = state.store.get_gallery(&id) else {
        return Err(ErrorPayload {
            message: "Gallery is not linked".into(),
            code: "not_linked".into(),
            status: 0,
        });
    };
    if !folder_exists(&gallery.local_path) {
        return Err(ErrorPayload {
            message: "Folder not found".into(),
            code: "folder_missing".into(),
            status: 0,
        });
    }
    app.opener()
        .open_path(gallery.local_path, None::<&str>)
        .map_err(|err| ErrorPayload {
            message: err.to_string(),
            code: "open_folder_failed".into(),
            status: 0,
        })?;
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
        return Err(ErrorPayload {
            message: "Invalid gallery".into(),
            code: "invalid_gallery".into(),
            status: 0,
        });
    };
    let result = apply_remote_deletions(&state.api, &state.store, &id, &photo_ids).await?;
    emit_state(&app, &state).await;
    Ok(result)
}

#[tauri::command]
pub async fn rate_files(
    state: State<'_, AppState>,
    files: Vec<String>,
    rating: String,
    photos_dir: Option<String>,
) -> Result<RateResult, ErrorPayload> {
    let config = state.store.load_config();
    let Some(trusted) = config.photos_dir else {
        return Ok(rate::rate_files(None, &files, &rating, Path::new(".")).await);
    };

    let Some(requested) = photos_dir else {
        return Err(ErrorPayload {
            message: "No folder selected".into(),
            code: "no_folder".into(),
            status: 0,
        });
    };

    let trusted_path = std::fs::canonicalize(&trusted).map_err(|err| ErrorPayload {
        message: err.to_string(),
        code: "folder_error".into(),
        status: 0,
    })?;
    let requested_path = std::fs::canonicalize(&requested).map_err(|err| ErrorPayload {
        message: err.to_string(),
        code: "folder_error".into(),
        status: 0,
    })?;
    if trusted_path != requested_path {
        return Err(ErrorPayload {
            message: "Folder not accessible".into(),
            code: "folder_error".into(),
            status: 0,
        });
    }

    if sanitize_file_names(&files).is_none() {
        return Err(ErrorPayload {
            message: "Invalid request".into(),
            code: "invalid_request".into(),
            status: 0,
        });
    }

    let _ = state.store.update_config(|config| {
        config.last_rating = Some(rating.clone());
    });

    Ok(rate::rate_files(state.exiftool.as_deref(), &files, &rating, &trusted_path).await)
}

#[tauri::command]
pub async fn dev_handle_deep_link(app: AppHandle, url: String) -> Result<Value, ErrorPayload> {
    if !cfg!(debug_assertions) {
        return Err(ErrorPayload {
            message: "Deep-link injection is disabled in release builds".into(),
            code: "not_allowed".into(),
            status: 0,
        });
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
