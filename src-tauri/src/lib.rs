mod classifier;
mod config;
mod context;
mod diagnostics;
mod ime;
mod logger;
mod platform;

use config::AppConfig;
use logger::EventLogger;
use platform::windows::{
    ForegroundWatcher, TrayRuntimeControl, WindowsImeController, WindowsSingleInstance,
};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tauri_plugin_opener::open_path;
use tauri_plugin_store::StoreExt;

pub struct AppState {
    watcher_control: Arc<TrayRuntimeControl>,
    event_logger: Arc<EventLogger>,
    debug_mode: Arc<AtomicBool>,
    config_store: Arc<tauri_plugin_store::Store<tauri::Wry>>,
    app_config: Arc<RwLock<AppConfig>>,
}

#[tauri::command]
fn get_watcher_status(state: State<'_, AppState>) -> serde_json::Value {
    serde_json::json!({
        "paused": state.watcher_control.is_paused(),
    })
}

#[tauri::command]
fn toggle_watcher_pause(state: State<'_, AppState>) -> bool {
    state.watcher_control.toggle_paused()
}

#[tauri::command]
fn test_classify(line: String, cursor: usize) -> Result<serde_json::Value, String> {
    let context = context::LineContext::new(line, cursor)
        .map_err(|(c, len)| format!("cursor {} is out of bounds for text length {}", c, len))?;
    let decision = classifier::classify(&context);

    Ok(serde_json::json!({
        "line": context.line(),
        "cursor": context.cursor(),
        "target_mode": format!("{}", decision.mode),
        "reason": format!("{}", decision.reason),
    }))
}

#[tauri::command]
fn get_current_ime_mode() -> Result<String, String> {
    let controller = WindowsImeController::new();
    controller
        .current_mode()
        .map(|mode| format!("{}", mode))
        .map_err(|e| e)
}

#[tauri::command]
fn get_debug_mode(state: State<'_, AppState>) -> bool {
    state.debug_mode.load(Ordering::Relaxed)
}

#[tauri::command]
fn set_debug_mode(enabled: bool, state: State<'_, AppState>) -> bool {
    state.debug_mode.store(enabled, Ordering::Relaxed);
    enabled
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> AppConfig {
    config::load_config(&state.config_store)
}

#[tauri::command]
fn set_config(config: AppConfig, state: State<'_, AppState>) -> Result<(), String> {
    config.validate()?;
    config::save_config(&state.config_store, &config)?;
    if let Ok(mut guard) = state.app_config.write() {
        *guard = config;
    }
    Ok(())
}

#[tauri::command]
async fn open_log_folder(state: State<'_, AppState>) -> Result<(), String> {
    let path = state.event_logger.log_dir().to_path_buf();
    open_path(&path, None::<&str>)
        .map_err(|e| format!("failed to open log folder: {e}"))
}

#[tauri::command]
fn reset_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    let default = AppConfig::default();
    config::save_config(&state.config_store, &default)?;
    if let Ok(mut guard) = state.app_config.write() {
        *guard = default.clone();
    }
    Ok(default)
}

#[tauri::command]
fn get_log_file_path(state: State<'_, AppState>) -> Result<String, String> {
    state
        .event_logger
        .current_log_path()
        .map(|path| path.display().to_string())
}

#[tauri::command]
fn get_recent_log_lines(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    state
        .event_logger
        .read_recent_lines(limit.unwrap_or(50).min(500))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let event_logger = Arc::new(EventLogger::new(
        EventLogger::default_log_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("smart-shift").join("logs")),
    ));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .setup(move |app| {
            if let Err(e) = WindowsSingleInstance::acquire("smart-shift-tauri") {
                eprintln!("smart-shift is already running: {e}");
                std::process::exit(1);
            }

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            let store = app.store("config.json").expect("failed to create config store");
            config::init_default_config(&store);
            let cfg = config::load_config(&store);

            let watcher_control = Arc::new(TrayRuntimeControl::new());
            if !cfg.auto_start {
                watcher_control.set_paused(true);
            }

            let debug_mode = Arc::new(AtomicBool::new(cfg.debug_mode));
            let app_config = Arc::new(RwLock::new(cfg.clone()));

            app.manage(AppState {
                watcher_control: watcher_control.clone(),
                event_logger: event_logger.clone(),
                debug_mode: debug_mode.clone(),
                config_store: store,
                app_config: app_config.clone(),
            });

            let app_handle_thread = app.handle().clone();

            // Startup checks
            let check_result = diagnostics::run_startup_checks();
            if !check_result.errors.is_empty() {
                let _ = app_handle_thread.emit("startup-check-failed", check_result.errors);
            }

            let _ = event_logger.cleanup_old_logs(7);

            let event_logger_thread = event_logger.clone();
            thread::spawn(move || {
                let mut consecutive_panics = 0u32;
                const PANIC_THRESHOLD: u32 = 5;

                loop {
                    let debug_mode_c = debug_mode.clone();
                    let app_config_c = app_config.clone();
                    let app_handle_c = app_handle_thread.clone();
                    let event_logger_c = event_logger_thread.clone();
                    let watcher_control_c = watcher_control.clone();

                    let result = panic::catch_unwind(panic::AssertUnwindSafe(move || {
                        let watcher = ForegroundWatcher::new(
                            cfg.poll_interval_ms,
                            debug_mode_c,
                            app_config_c,
                            Some(app_handle_c),
                            Some(event_logger_c),
                        );
                        watcher.run_until_controlled(&watcher_control_c)
                    }));

                    match result {
                        Ok(Ok(())) => {
                            // Normal exit (quit requested)
                            break;
                        }
                        Ok(Err(e)) => {
                            eprintln!("watcher error: {e}");
                            consecutive_panics = 0;
                            thread::sleep(Duration::from_secs(1));
                        }
                        Err(_) => {
                            consecutive_panics += 1;
                            eprintln!(
                                "watcher panic #{}/{}, restarting...",
                                consecutive_panics, PANIC_THRESHOLD
                            );
                            if consecutive_panics >= PANIC_THRESHOLD {
                                eprintln!("watcher panic threshold reached, pausing");
                                watcher_control.set_paused(true);
                                let _ = app_handle_thread.emit("watcher-panic-threshold", ());
                                break;
                            }
                            thread::sleep(Duration::from_secs(2));
                        }
                    }
                }
            });

            // Tray setup
            #[cfg(target_os = "windows")]
            {
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
                use tauri::tray::TrayIconBuilder;

                let menu = Menu::new(app)?;
                let open_i = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
                let toggle_i =
                    MenuItem::with_id(app, "toggle", "Pause / Resume", true, None::<&str>)?;
                let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

                menu.append(&open_i)?;
                menu.append(&toggle_i)?;
                menu.append(&PredefinedMenuItem::separator(app)?)?;
                menu.append(&quit_i)?;

                TrayIconBuilder::new()
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "open" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "toggle" => {
                            if let Some(state) = app.try_state::<AppState>() {
                                state.watcher_control.toggle_paused();
                            }
                        }
                        "quit" => {
                            if let Some(state) = app.try_state::<AppState>() {
                                state.watcher_control.request_stop();
                            }
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_watcher_status,
            toggle_watcher_pause,
            test_classify,
            get_current_ime_mode,
            get_log_file_path,
            get_recent_log_lines,
            get_debug_mode,
            set_debug_mode,
            get_config,
            set_config,
            reset_config,
            open_log_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
