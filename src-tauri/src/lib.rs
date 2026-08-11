mod capture;
mod engine;
mod hotkey;
mod sanitize;
mod window;

use engine::{TranslationEngine, TranslationInput};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;
use tokio::sync::mpsc;

struct EngineHandle(Arc<dyn TranslationEngine>);

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
fn engine_name(engine: tauri::State<EngineHandle>) -> &'static str {
    engine.0.name()
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
fn sanitize_html(html: String) -> String {
    sanitize::sanitize_translation_html(&html)
}

#[tauri::command]
async fn start_translation(
    app: AppHandle,
    engine: tauri::State<'_, EngineHandle>,
    label: String,
    input: String,
    html: Option<String>,
) -> Result<(), String> {
    let Some(translation_input) = TranslationInput::from_capture(Some(input), html) else {
        let _ = app.emit_to(
            &label,
            "translate-chunk",
            &engine::TranslationChunk {
                text: String::new(),
                done: true,
            },
        );
        return Ok(());
    };

    let engine = engine.0.clone();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();

    if let Some(window) = app.get_webview_window(&label) {
        let cancel_tx = Mutex::new(Some(cancel_tx));
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                if let Some(tx) = cancel_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
    }

    let engine_task = tokio::spawn(async move {
        engine.translate(&translation_input, tx).await;
    });

    loop {
        tokio::select! {
            chunk = rx.recv() => {
                match chunk {
                    Some(chunk) => {
                        let done = chunk.done;
                        let _ = app.emit_to(&label, "translate-chunk", &chunk);
                        if done {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = &mut cancel_rx => {
                drop(rx);
                break;
            }
        }
    }

    let _ = engine_task.await;
    Ok(())
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
        .manage(EngineHandle(Arc::from(engine::resolve())))
        .manage(window::popup::PopupCounter::default())
        .manage(window::popup::PopupStack::default())
        .invoke_handler(tauri::generate_handler![
            get_capture,
            engine_name,
            open_accessibility_settings,
            sanitize_html,
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
