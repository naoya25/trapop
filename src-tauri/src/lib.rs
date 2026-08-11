mod capture;
mod engine;
mod hotkey;
mod window;

use engine::mock::MockEngine;
use engine::TranslationEngine;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;
use tokio::sync::mpsc;

#[derive(Default)]
struct CaptureState(Mutex<HashMap<String, CaptureOutcome>>);

#[derive(serde::Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CaptureOutcome {
    Ok {
        result: capture::CaptureResult,
    },
    Error {
        message: String,
        is_accessibility_error: bool,
    },
}

fn capture_error_message(code: &str) -> (String, bool) {
    match code {
        "accessibility_permission_required" => (
            "アクセシビリティ権限が必要です。下のボタンから設定を開き、TraPoP を許可してから \
             再試行してください。"
                .to_string(),
            true,
        ),
        "clipboard_empty" => (
            "コピーしてからホットキーを押してください(⌘C → ⌥⇧⌘P)。".to_string(),
            false,
        ),
        other => (format!("選択テキストの取得に失敗しました: {other}"), false),
    }
}

fn cursor_position() -> (f64, f64) {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    if let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        if let Ok(event) = CGEvent::new(source) {
            let point = event.location();
            return (point.x, point.y);
        }
    }
    (200.0, 200.0)
}

fn trigger_capture(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let (x, y) = cursor_position();
        let capture_result = capture::capture_selection();

        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            let label = match window::popup::spawn(&app_for_main, x, y) {
                Ok(label) => label,
                Err(e) => {
                    eprintln!("[trapop] popup spawn failed: {e}");
                    return;
                }
            };

            let outcome = match capture_result {
                Ok(result) => CaptureOutcome::Ok { result },
                Err(code) => {
                    let (message, is_accessibility_error) = capture_error_message(&code);
                    CaptureOutcome::Error {
                        message,
                        is_accessibility_error,
                    }
                }
            };

            app_for_main
                .state::<CaptureState>()
                .0
                .lock()
                .unwrap()
                .insert(label.clone(), outcome.clone());

            let _ = app_for_main.emit_to(&label, "capture-ready", &outcome);
        });
    });
}

#[tauri::command]
fn get_capture(state: tauri::State<CaptureState>, label: String) -> Option<CaptureOutcome> {
    state.0.lock().unwrap().get(&label).cloned()
}

#[tauri::command]
fn engine_name() -> &'static str {
    MockEngine.name()
}

#[tauri::command]
fn open_accessibility_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_translation(app: AppHandle, label: String, input: String) -> Result<(), String> {
    let engine = MockEngine;
    let (tx, mut rx) = mpsc::unbounded_channel();

    let engine_task = tokio::spawn(async move {
        engine.translate(&input, tx).await;
    });

    while let Some(chunk) = rx.recv().await {
        let _ = app.emit_to(&label, "translate-chunk", &chunk);
    }

    engine_task.await.map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        trigger_capture(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .manage(CaptureState::default())
        .manage(window::popup::PopupCounter::default())
        .manage(window::popup::PopupStack::default())
        .invoke_handler(tauri::generate_handler![
            get_capture,
            engine_name,
            open_accessibility_settings,
            start_translation
        ])
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            hotkey::register(handle, &hotkey::HotkeySpec::default())?;
            window::setup_main_window(handle)?;
            window::setup_tray(handle)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
