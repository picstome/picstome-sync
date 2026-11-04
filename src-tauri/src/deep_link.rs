use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use picstome_sync_core::links::{parse_deep_link, DeepLink};

use crate::commands::{build_state, AppState};

pub fn setup(app: &AppHandle) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        if let Err(err) = app.deep_link().register_all() {
            eprintln!("Could not register deep links: {err}");
        }
    }

    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            let app = handle.clone();
            let raw = url.to_string();
            tauri::async_runtime::spawn(async move {
                handle_url(app, raw).await;
            });
        }
    });

    // macOS delivers cold-start URLs as an Apple Event, not as a CLI argument.
    // The plugin stores them in `get_current`; check it after registering the
    // event handler so the first deep link is not lost.
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            let app = app.clone();
            let raw = url.to_string();
            tauri::async_runtime::spawn(async move {
                handle_url(app, raw).await;
            });
        }
    }

    if let Some(raw) = std::env::args().find(|arg| arg.starts_with("picstome://")) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            handle_url(app, raw).await;
        });
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub async fn handle_url(app: AppHandle, raw: String) {
    let Some(link) = parse_deep_link(&raw) else {
        return;
    };

    match link {
        DeepLink::Rate {
            files,
            rating,
            gallery,
        } => {
            show_main(&app);
            if let Some(state) = app.try_state::<AppState>() {
                if !state.frontend_is_ready() {
                    state.defer_link(&raw);
                    return;
                }
                if !state.should_process_link(&raw) {
                    return;
                }
            }
            let _ = app.emit(
                "incoming-rate",
                json!({"files": files, "rating": rating, "gallery": gallery}),
            );
        }
        DeepLink::Sync { gallery_id, name } => {
            show_main(&app);
            if let Some(state) = app.try_state::<AppState>() {
                if !state.frontend_is_ready() {
                    state.defer_link(&raw);
                    return;
                }
                if !state.should_process_link(&raw) {
                    return;
                }
            }
            let _ = app.emit(
                "incoming-sync",
                json!({"galleryId": gallery_id, "name": name}),
            );
        }
        DeepLink::Auth { code, state } => {
            let Some(app_state) = app.try_state::<AppState>() else {
                return;
            };
            let expected = app_state.auth_nonce.lock().unwrap().take();
            if expected.as_deref() != Some(state.as_str()) {
                let _ = app.emit(
                    "auth-status",
                    json!({"connected": false, "error": "state_mismatch"}),
                );
                return;
            }

            let api = app_state.api().await;
            let store = app_state.store.clone();
            match api.exchange_code(&code).await {
                Ok(response) => {
                    let _ = app_state.token_store.save(Some(&response.token));
                    let me = api.me().await.ok();
                    let email = me
                        .as_ref()
                        .map(|me| me.email.clone())
                        .or_else(|| response.email.clone());
                    let name = me.as_ref().and_then(|me| me.display_name.clone());
                    let _ = store.update_config(|config| {
                        config.account_email = email.clone();
                        config.account_name = name.clone();
                    });
                    let view = build_state(&app_state).await;
                    let _ = app.emit("app-state", view);
                    let _ = app.emit(
                        "auth-status",
                        json!({"connected": true, "email": response.email}),
                    );
                }
                Err(err) => {
                    let _ = app.emit(
                        "auth-status",
                        json!({"connected": false, "error": err.to_string(), "code": err.code()}),
                    );
                }
            }
        }
    }
}
