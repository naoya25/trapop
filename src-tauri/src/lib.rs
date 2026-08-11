mod capture;
mod config;
mod engine;
mod history;
mod hotkey;
mod sanitize;
mod window;

use engine::{TranslationEngine, TranslationInput};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;
use tokio::sync::mpsc;

struct EngineHandle(Mutex<Arc<dyn TranslationEngine>>);

impl EngineHandle {
    fn current(&self) -> Arc<dyn TranslationEngine> {
        self.0.lock().unwrap().clone()
    }

    fn set(&self, engine: Arc<dyn TranslationEngine>) {
        *self.0.lock().unwrap() = engine;
    }
}

fn build_engine(config: &config::AppConfig) -> Arc<dyn TranslationEngine> {
    Arc::from(engine::resolve(
        config.engine_choice.as_str(),
        config.model_override.as_deref(),
    ))
}

#[derive(Default)]
struct CaptureState(Mutex<HashMap<String, CaptureOutcome>>);

#[derive(Default)]
struct HistoryReplayState(Mutex<HashMap<String, history::HistoryRecord>>);

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

#[derive(serde::Serialize)]
struct SettingsView {
    hotkey: String,
    engine_choice: &'static str,
    model_override: Option<String>,
    has_api_key: bool,
    effective_engine_name: &'static str,
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
            window::hide_main_window_before_popup(&app_for_main);

            let label = match window::popup::spawn(
                &app_for_main,
                x,
                y,
                window::popup::PopupMode::Capture,
            ) {
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
    engine.current().name()
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
fn list_history(app: AppHandle) -> Result<Vec<history::HistoryRecord>, String> {
    history::load_recent(&app)
}

#[tauri::command]
fn clear_history(app: AppHandle) -> Result<(), String> {
    history::clear(&app)
}

#[tauri::command]
fn open_history_popup(
    app: AppHandle,
    replay_state: tauri::State<HistoryReplayState>,
    id: String,
) -> Result<(), String> {
    let record = history::find(&app, &id)?.ok_or_else(|| "履歴が見つかりません".to_string())?;
    let (x, y) = cursor_position();
    let label = window::popup::spawn(&app, x, y, window::popup::PopupMode::Replay)?;

    replay_state
        .0
        .lock()
        .unwrap()
        .insert(label.clone(), record);

    let _ = app.emit_to(&label, "history-replay-ready", ());
    Ok(())
}

#[tauri::command]
fn get_history_replay(
    state: tauri::State<HistoryReplayState>,
    label: String,
) -> Option<history::HistoryRecord> {
    state.0.lock().unwrap().get(&label).cloned()
}

#[tauri::command]
fn get_settings(app: AppHandle, engine: tauri::State<EngineHandle>) -> SettingsView {
    let cfg = config::load(&app);
    SettingsView {
        hotkey: cfg.hotkey,
        engine_choice: cfg.engine_choice.as_str(),
        model_override: cfg.model_override,
        has_api_key: engine::openai::has_stored_key(),
        effective_engine_name: engine.current().name(),
    }
}

#[tauri::command]
fn set_hotkey(app: AppHandle, accelerator: String) -> Result<(), String> {
    let spec = hotkey::HotkeySpec::from_accelerator(&accelerator)?;
    hotkey::set_hotkey(&app, &spec)?;

    let mut cfg = config::load(&app);
    cfg.hotkey = spec.to_accelerator();
    config::save(&app, &cfg)
}

#[tauri::command]
fn set_engine_choice(
    app: AppHandle,
    engine: tauri::State<EngineHandle>,
    choice: String,
) -> Result<(), String> {
    let engine_choice = config::EngineChoice::parse(&choice)?;

    let mut cfg = config::load(&app);
    cfg.engine_choice = engine_choice;
    config::save(&app, &cfg)?;

    engine.set(build_engine(&cfg));
    Ok(())
}

#[tauri::command]
fn set_model_override(
    app: AppHandle,
    engine: tauri::State<EngineHandle>,
    model: Option<String>,
) -> Result<(), String> {
    let mut cfg = config::load(&app);
    cfg.model_override = model.filter(|m| !m.trim().is_empty());
    config::save(&app, &cfg)?;

    engine.set(build_engine(&cfg));
    Ok(())
}

#[tauri::command]
fn save_api_key(
    app: AppHandle,
    engine: tauri::State<EngineHandle>,
    key: String,
) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("APIキーが空です".to_string());
    }

    engine::openai::store_api_key(trimmed)?;

    let cfg = config::load(&app);
    engine.set(build_engine(&cfg));
    Ok(())
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

    let engine_handle = engine.current();
    let engine_name = engine_handle.name();
    let is_html = translation_input.is_html();
    let source_body = translation_input.body().to_string();

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
        engine_handle.translate(&translation_input, tx).await;
    });

    let mut full_text = String::new();
    let mut completed = false;
    loop {
        tokio::select! {
            chunk = rx.recv() => {
                match chunk {
                    Some(chunk) => {
                        let done = chunk.done;
                        full_text.push_str(&chunk.text);
                        let _ = app.emit_to(&label, "translate-chunk", &chunk);
                        if done {
                            completed = true;
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

    if completed && !full_text.trim().is_empty() {
        let record = history::HistoryRecord::new(&source_body, full_text, is_html, engine_name);
        if history::append(&app, &record).is_ok() {
            let _ = app.emit("history-appended", ());
        }
    }

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
        .manage(HistoryReplayState::default())
        .manage(window::popup::PopupCounter::default())
        .manage(window::popup::PopupStack::default())
        .invoke_handler(tauri::generate_handler![
            get_capture,
            engine_name,
            open_accessibility_settings,
            sanitize_html,
            list_history,
            clear_history,
            open_history_popup,
            get_history_replay,
            get_settings,
            set_hotkey,
            set_engine_choice,
            set_model_override,
            save_api_key,
            start_translation
        ])
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            let cfg = config::load(handle);
            handle.manage(EngineHandle(Mutex::new(build_engine(&cfg))));

            let hotkey_spec =
                hotkey::HotkeySpec::from_accelerator(&cfg.hotkey).unwrap_or_default();
            hotkey::register(handle, &hotkey_spec)?;

            window::setup_main_window(handle)?;
            window::setup_tray(handle)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
