#![recursion_limit = "512"]

pub mod auth;
pub mod commands;
pub mod deep_link;
pub mod rate;
pub mod smoke;
pub mod strings;

use commands::AppState;
use picstome_sync_core::mock::{start_mock_server, MockServerOptions};
use picstome_sync_core::{ApiClient, Store};
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let demo_requested = std::env::args().any(|arg| arg == "--demo")
        || std::env::var("PICSTOME_MOCK")
            .map(|value| value == "1")
            .unwrap_or(false);

    let mut mock_server = None;
    if demo_requested
        && std::env::var("PICSTOME_API_BASE")
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
    {
        match tauri::async_runtime::block_on(start_mock_server(MockServerOptions::default())) {
            Ok(server) => {
                std::env::set_var("PICSTOME_API_BASE", &server.url);
                std::env::set_var("PICSTOME_MOCK", "1");
                println!("Picstome Sync demo API: {}", server.url);
                mock_server = Some(server);
            }
            Err(err) => {
                eprintln!("Could not start the demo API: {err}");
            }
        }
    }

    let mock = std::env::var("PICSTOME_MOCK")
        .map(|value| value == "1")
        .unwrap_or(false);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(url) = argv.iter().find(|arg| arg.starts_with("picstome://")) {
                let app = app.clone();
                let url = url.clone();
                tauri::async_runtime::spawn(async move {
                    deep_link::handle_url(app, url).await;
                });
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            let handle = app.handle().clone();

            let data_dir = match std::env::var("PICSTOME_USER_DATA_DIR") {
                Ok(dir) if !dir.trim().is_empty() => std::path::PathBuf::from(dir),
                _ => app.path().app_data_dir()?,
            };
            let store = Store::new(data_dir)?;
            let token_store = auth::TokenStore::new(store.root().join("auth.json"));
            let base_url = commands::api_base_from_env_or_config(&store);
            let token = std::env::var("PICSTOME_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .filter(|_| cfg!(debug_assertions) || mock)
                .or_else(|| if mock { None } else { token_store.load() });
            let api = ApiClient::new(Some(&base_url), token)?;
            let exiftool = commands::exiftool_path(&handle);

            app.manage(AppState::new(
                store,
                api,
                mock,
                token_store,
                app.package_info().version.to_string(),
                if cfg!(debug_assertions) {
                    "debug".to_string()
                } else {
                    "release".to_string()
                },
                exiftool,
                mock_server,
            ));
            if let Some(window) = handle.get_webview_window("main") {
                let app_for_events = handle.clone();
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Destroyed) {
                        if let Some(state) = app_for_events.try_state::<AppState>() {
                            state.mark_frontend_closed();
                        }
                    }
                });
            }

            deep_link::setup(&handle);

            if !mock {
                let app_for_account = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let api = app_for_account.state::<AppState>().api().await;
                    if api.has_token().await {
                        if let Ok(me) = api.me().await {
                            let store = app_for_account.state::<AppState>().store.clone();
                            let _ = store.update_config(|config| {
                                config.account_email = Some(me.email.clone());
                                config.account_name = me.display_name.clone();
                            });
                            let view = {
                                let state = app_for_account.state::<AppState>();
                                commands::build_state(&state).await
                            };
                            let _ = app_for_account.emit("app-state", view);
                        }
                    }
                });
            }

            if mock {
                let app_for_login = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let api = app_for_login.state::<AppState>().api().await;
                    if api.dev_login().await.is_ok() {
                        let view = {
                            let state = app_for_login.state::<AppState>();
                            commands::build_state(&state).await
                        };
                        let _ = app_for_login.emit("app-state", view);
                        let _ = app_for_login.emit(
                            "auth-status",
                            serde_json::json!({"connected": true, "email": "desktop@picstome.test", "mock": true}),
                        );
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::get_config,
            commands::choose_folder,
            commands::connect,
            commands::logout,
            commands::list_remote_galleries,
            commands::get_gallery_favorites,
            commands::start_sync,
            commands::sync_gallery,
            commands::locate_gallery,
            commands::change_folder,
            commands::unlink_gallery,
            commands::open_folder,
            commands::apply_deletions,
            commands::resolve_remote_deletions,
            commands::rate_files,
            commands::open_account_profile,
            commands::open_galleries_page,
            commands::set_api_base,
            commands::get_sync_log,
            commands::clear_sync_log,
            commands::frontend_ready,
            commands::dev_handle_deep_link,
            commands::install_update,
            commands::check_updates,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Picstome Sync");
}
