mod classifier;
mod context;
mod ime;
mod platform;

use platform::windows::{ForegroundWatcher, TrayRuntimeControl, WindowsImeController, WindowsSingleInstance};
use std::sync::Arc;
use std::thread;
use tauri::{Manager, State};

pub struct AppState {
    watcher_control: Arc<TrayRuntimeControl>,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            watcher_control: Arc::new(TrayRuntimeControl::new()),
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .setup(|app| {
            if let Err(e) = WindowsSingleInstance::acquire("smart-shift-tauri") {
                eprintln!("smart-shift is already running: {e}");
                std::process::exit(1);
            }

            let control = app.state::<AppState>().watcher_control.clone();
            let app_handle = app.handle().clone();
            thread::spawn(move || {
                let watcher = ForegroundWatcher::new(250, false, Some(app_handle));
                let _ = watcher.run_until_controlled(&control);
            });

            // Tray setup
            #[cfg(target_os = "windows")]
            {
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
                use tauri::tray::TrayIconBuilder;

                let menu = Menu::new(app)?;
                let open_i = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
                let toggle_i = MenuItem::with_id(app, "toggle", "Pause / Resume", true, None::<&str>)?;
                let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

                menu.append(&open_i)?;
                menu.append(&toggle_i)?;
                menu.append(&PredefinedMenuItem::separator(app)?)?;
                menu.append(&quit_i)?;

                TrayIconBuilder::new()
                    .menu(&menu)
                    .on_menu_event(|app, event| {
                        match event.id.as_ref() {
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
                        }
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
