mod config;
mod engine;
mod history;
mod hotkey;
mod sanitize;
mod translation_registry;
mod window;

use engine::{TranslationEngine, TranslationInput};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;
use tokio::sync::mpsc;
use translation_registry::TranslationRegistry;

struct EngineState {
    engine: Arc<dyn TranslationEngine>,
    unavailable_reason: Option<String>,
}

struct EngineHandle(Mutex<EngineState>);

impl EngineHandle {
    fn current(&self) -> (Arc<dyn TranslationEngine>, Option<String>) {
        let state = self.0.lock().unwrap();
        (state.engine.clone(), state.unavailable_reason.clone())
    }

    fn set(&self, state: EngineState) {
        *self.0.lock().unwrap() = state;
    }
}

fn build_engine(config: &config::AppConfig) -> EngineState {
    let resolved = engine::resolve(
        config.engine_choice.as_str(),
        config.model_override.as_deref(),
    );
    EngineState {
        engine: Arc::from(resolved.engine),
        unavailable_reason: resolved.unavailable_reason,
    }
}

#[derive(Default)]
pub(crate) struct HistoryReplayState(pub(crate) Mutex<HashMap<String, history::HistoryRecord>>);

// 起動時のホットキー登録失敗を保持し、設定画面に表示する(stderr だけだと
// 「押しても何も起きないアプリ」に見えるため)
#[derive(Default)]
struct HotkeyStatus(Mutex<Option<String>>);

#[derive(serde::Serialize)]
struct SettingsView {
    hotkey: String,
    hotkey_error: Option<String>,
    engine_choice: &'static str,
    model_override: Option<String>,
    has_openai_key: bool,
    has_gemini_key: bool,
    effective_engine_name: &'static str,
    lang_a: String,
    lang_b: String,
}

#[derive(serde::Serialize)]
struct LangPairView {
    lang_a: String,
    lang_b: String,
}

#[derive(serde::Serialize, Clone)]
struct TranslateChunkEvent {
    request_id: u64,
    text: String,
    done: bool,
    error: bool,
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

fn open_paste_popup(app: &AppHandle) {
    let app_for_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        let (x, y) = cursor_position();
        window::hide_main_window_before_popup(&app_for_main);

        if let Err(e) = window::popup::spawn(&app_for_main, x, y, window::popup::PopupMode::Paste)
        {
            eprintln!("[trapop] popup spawn failed: {e}");
        }
    });
}

// キー未設定のプレースホルダ(MockEngine)を "mock" と見せない。
// mock は設定で選べる正規の選択肢なので、未設定状態と区別できる名前を返す。
fn effective_engine_label(engine: &EngineHandle) -> &'static str {
    let (engine_handle, unavailable_reason) = engine.current();
    if unavailable_reason.is_some() {
        "未設定"
    } else {
        engine_handle.name()
    }
}

#[tauri::command]
fn engine_name(engine: tauri::State<EngineHandle>) -> &'static str {
    effective_engine_label(&engine)
}

// 長文の ammonia パースは CPU バウンドなので spawn_blocking で逃がす。
#[tauri::command]
async fn sanitize_html(html: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || sanitize::sanitize_translation_html(&html))
        .await
        .map_err(|e| e.to_string())
}

// 履歴ファイルの読み書きはブロッキングなので spawn_blocking で逃がす
#[tauri::command]
async fn list_history(app: AppHandle) -> Result<Vec<history::HistoryRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || history::load_recent(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn clear_history(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || history::clear(&app))
        .await
        .map_err(|e| e.to_string())?
}

// popup::spawn は NSScreen 参照のためメインスレッド必須。同期 command が
// メインスレッドで走るという実装詳細に依存せず、run_on_main_thread で明示する。
#[tauri::command]
async fn open_history_popup(app: AppHandle, id: String) -> Result<(), String> {
    let app_for_find = app.clone();
    let record = tauri::async_runtime::spawn_blocking(move || history::find(&app_for_find, &id))
        .await
        .map_err(|e| e.to_string())??
        .ok_or_else(|| "履歴が見つかりません".to_string())?;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            let (x, y) = cursor_position();
            let label =
                window::popup::spawn(&app_for_main, x, y, window::popup::PopupMode::Replay)?;

            app_for_main
                .state::<HistoryReplayState>()
                .0
                .lock()
                .unwrap()
                .insert(label, record);

            // 受け渡しは popup 側のポーリング(get_history_replay)で行う。
            // event 通知は webview の JS ロード前に emit されて必ず失われるため使わない。
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    rx.await.map_err(|e| e.to_string())?
}

// remove ではなく clone を返す。消してしまうと履歴 popup の「再試行」が
// 二度と record を取れず必ずタイムアウトする。掃除は popup Destroyed 側が担う。
// label は引数で受けず呼び出し元ウィンドウから取る(他 popup の record を覗けない)。
#[tauri::command]
fn get_history_replay(
    window: tauri::WebviewWindow,
    state: tauri::State<HistoryReplayState>,
) -> Option<history::HistoryRecord> {
    state.0.lock().unwrap().get(window.label()).cloned()
}

// Keychain 存在確認(security プロセス2回)とファイル読みはブロッキングなので
// spawn_blocking で逃がす。
#[tauri::command]
async fn get_settings(
    app: AppHandle,
    engine: tauri::State<'_, EngineHandle>,
    hotkey_status: tauri::State<'_, HotkeyStatus>,
) -> Result<SettingsView, String> {
    let effective_engine_name = effective_engine_label(&engine);
    let hotkey_error = hotkey_status.0.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = config::load(&app);
        SettingsView {
            hotkey: cfg.hotkey,
            hotkey_error,
            engine_choice: cfg.engine_choice.as_str(),
            model_override: cfg.model_override,
            has_openai_key: engine::openai::has_stored_key(),
            has_gemini_key: engine::gemini::has_stored_key(),
            effective_engine_name,
            lang_a: cfg.lang_a,
            lang_b: cfg.lang_b,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_lang_pair(app: AppHandle) -> Result<LangPairView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = config::load(&app);
        LangPairView {
            lang_a: cfg.lang_a,
            lang_b: cfg.lang_b,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_lang_pair(app: AppHandle, lang_a: String, lang_b: String) -> Result<(), String> {
    let lang_a = lang_a.trim().to_string();
    let lang_b = lang_b.trim().to_string();
    if lang_a.is_empty() || lang_b.is_empty() {
        return Err("言語名が空です".to_string());
    }
    if lang_a == lang_b {
        return Err("同じ言語同士は指定できません".to_string());
    }
    // 言語名はシステムプロンプトに埋まるため、長さと制御文字だけ弾く
    if [&lang_a, &lang_b]
        .iter()
        .any(|l| l.chars().count() > 20 || l.chars().any(|c| c.is_control()))
    {
        return Err("言語名が不正です(20文字以内・制御文字不可)".to_string());
    }

    tauri::async_runtime::spawn_blocking(move || {
        config::update(&app, |cfg| {
            cfg.lang_a = lang_a;
            cfg.lang_b = lang_b;
        })
        .map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_popup_size(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("不正なポップアップサイズです".to_string());
    }

    tauri::async_runtime::spawn_blocking(move || {
        config::update(&app, |cfg| {
            cfg.popup_width = width;
            cfg.popup_height = height;
        })
        .map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
}

// 設定ファイル読み書きは spawn_blocking へ、ホットキー登録(OS API)は
// メインスレッドへ、それぞれ明示的に振り分ける。
#[tauri::command]
async fn set_hotkey(app: AppHandle, accelerator: String) -> Result<(), String> {
    let spec = hotkey::HotkeySpec::from_accelerator(&accelerator)?;

    let app_for_load = app.clone();
    let old_hotkey =
        tauri::async_runtime::spawn_blocking(move || config::load(&app_for_load).hotkey)
            .await
            .map_err(|e| e.to_string())?;
    let old_spec = hotkey::HotkeySpec::from_accelerator(&old_hotkey).ok();

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(hotkey::set_hotkey(&app_for_main, &spec, old_spec.as_ref()));
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())??;

    let app_for_save = app.clone();
    let save_result = tauri::async_runtime::spawn_blocking(move || {
        config::update(&app_for_save, |cfg| cfg.hotkey = spec.to_accelerator()).map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?;

    // 保存に失敗したら OS 側の登録を旧キーへ戻し、「今は効くが再起動で戻る」乖離を防ぐ。
    // 起動時の失敗表示のクリアは、登録・保存の両方が成功したときだけ行う。
    if save_result.is_ok() {
        *app.state::<HotkeyStatus>().0.lock().unwrap() = None;
    } else if let Some(old) = old_spec {
        let app_for_revert = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = hotkey::set_hotkey(&app_for_revert, &old, Some(&spec));
        });
    }
    save_result
}

// エンジン再構築は Keychain 読み(security プロセス)を含むため spawn_blocking で逃がす。
#[tauri::command]
async fn set_engine_choice(
    app: AppHandle,
    engine: tauri::State<'_, EngineHandle>,
    choice: String,
) -> Result<(), String> {
    let engine_choice = config::EngineChoice::parse(&choice)?;

    let state = tauri::async_runtime::spawn_blocking(move || -> Result<EngineState, String> {
        let cfg = config::update(&app, |cfg| cfg.engine_choice = engine_choice)?;
        Ok(build_engine(&cfg))
    })
    .await
    .map_err(|e| e.to_string())??;
    engine.set(state);
    Ok(())
}

// 検証は保存の入口で行う。エンジン側で黙って既定に落とすと、設定画面の表示と
// 実際に使うモデルが食い違うため、ここで Err にしてユーザーに見せる。
#[tauri::command]
async fn set_model_override(
    app: AppHandle,
    engine: tauri::State<'_, EngineHandle>,
    model: Option<String>,
) -> Result<(), String> {
    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    if let Some(m) = &model {
        let safe = m
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if !safe {
            return Err(
                "モデル名に使用できない文字が含まれています(英数字と . - _ のみ)".to_string(),
            );
        }
    }

    let state = tauri::async_runtime::spawn_blocking(move || -> Result<EngineState, String> {
        let cfg = config::update(&app, |cfg| cfg.model_override = model)?;
        Ok(build_engine(&cfg))
    })
    .await
    .map_err(|e| e.to_string())??;
    engine.set(state);
    Ok(())
}

// Keychain 書き込み(security プロセス・許可ダイアログの可能性)を含むため
// spawn_blocking で逃がす。
#[tauri::command]
async fn save_api_key(
    app: AppHandle,
    engine: tauri::State<'_, EngineHandle>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let trimmed = key.trim().to_string();
    if trimmed.is_empty() {
        return Err("APIキーが空です".to_string());
    }

    let state = tauri::async_runtime::spawn_blocking(move || -> Result<EngineState, String> {
        match provider.as_str() {
            "openai" => engine::openai::store_api_key(&trimmed)?,
            "gemini" => engine::gemini::store_api_key(&trimmed)?,
            other => return Err(format!("未知のエンジン指定です: {other}")),
        }
        let cfg = config::load(&app);
        Ok(build_engine(&cfg))
    })
    .await
    .map_err(|e| e.to_string())??;
    engine.set(state);
    Ok(())
}

#[tauri::command]
fn cancel_translation(
    window: tauri::WebviewWindow,
    translation_registry: tauri::State<TranslationRegistry>,
    request_id: u64,
) {
    translation_registry.cancel(window.label(), Some(request_id));
}

// label は引数で受けず呼び出し元ウィンドウから取る(別 popup へのなりすまし不可)。
#[tauri::command]
async fn start_translation(
    app: AppHandle,
    window: tauri::WebviewWindow,
    engine: tauri::State<'_, EngineHandle>,
    translation_registry: tauri::State<'_, TranslationRegistry>,
    input: String,
    html: Option<String>,
    request_id: u64,
) -> Result<(), String> {
    let label = window.label().to_string();
    let Some(translation_input) = TranslationInput::from_paste(Some(input), html) else {
        let _ = app.emit_to(
            &label,
            "translate-chunk",
            &TranslateChunkEvent {
                request_id,
                text: String::new(),
                done: true,
                error: false,
            },
        );
        return Ok(());
    };

    // begin は最初の await より前に置く。後ろだと config 読みの間に押された
    // 「停止」が inflight を見つけられず no-op になり、翻訳が走り切ってしまう。
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    translation_registry.begin(&label, request_id, cancel_tx);

    let app_for_cfg = app.clone();
    let cfg = match tauri::async_runtime::spawn_blocking(move || config::load(&app_for_cfg)).await
    {
        Ok(cfg) => cfg,
        Err(e) => {
            // begin 済みなので、どの早期 return でも finish と対にする
            translation_registry.finish(&label, request_id);
            return Err(e.to_string());
        }
    };
    let (engine_handle, unavailable_reason) = engine.current();
    if let Some(reason) = unavailable_reason {
        translation_registry.finish(&label, request_id);
        let _ = app.emit_to(
            &label,
            "translate-chunk",
            &TranslateChunkEvent {
                request_id,
                text: reason,
                done: true,
                error: true,
            },
        );
        return Ok(());
    }
    let engine_name = engine_handle.name();
    let is_html = translation_input.is_html();
    let source_body = translation_input.body().to_string();

    let lang_a = cfg.lang_a.clone();
    let lang_b = cfg.lang_b.clone();
    let engine_task = tokio::spawn(async move {
        engine_handle
            .translate(&translation_input, &lang_a, &lang_b, tx)
            .await;
    });

    let mut full_text = String::new();
    let mut completed = false;
    let mut errored = false;
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => {
                drop(rx);
                // rx の drop だけだと engine は次のチャンク送信まで気づかず
                // HTTP 接続とトークン消費が続くため、task ごと打ち切る
                engine_task.abort();
                break;
            }
            chunk = rx.recv() => {
                match chunk {
                    Some(chunk) => {
                        let done = chunk.done;
                        let error = chunk.error;
                        if !error {
                            full_text.push_str(&chunk.text);
                        }
                        let _ = app.emit_to(
                            &label,
                            "translate-chunk",
                            &TranslateChunkEvent {
                                request_id,
                                text: chunk.text,
                                done,
                                error,
                            },
                        );
                        if error {
                            errored = true;
                        }
                        if done {
                            completed = true;
                            break;
                        }
                    }
                    // 全 sender drop(エンジンが done/error を送らず終了)は異常終了。
                    // 「✓ 完了」に見せず、エラーとして popup に理由を出す。
                    None => {
                        let _ = app.emit_to(
                            &label,
                            "translate-chunk",
                            &TranslateChunkEvent {
                                request_id,
                                text: "翻訳が中断されました。再試行してください。".to_string(),
                                done: true,
                                error: true,
                            },
                        );
                        break;
                    }
                }
            }
        }
    }

    let _ = engine_task.await;
    translation_registry.finish(&label, request_id);

    if completed && !errored && !full_text.trim().is_empty() {
        let record = history::HistoryRecord::new(&source_body, full_text, is_html, engine_name);
        let app_for_history = app.clone();
        let appended = tauri::async_runtime::spawn_blocking(move || {
            history::append(&app_for_history, &record)
        })
        .await;
        if matches!(appended, Ok(Ok(()))) {
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
                        open_paste_popup(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .manage(HistoryReplayState::default())
        .manage(HotkeyStatus::default())
        .manage(window::popup::PopupCounter::default())
        .manage(window::popup::PopupStack::default())
        .manage(TranslationRegistry::default())
        .invoke_handler(tauri::generate_handler![
            engine_name,
            sanitize_html,
            list_history,
            clear_history,
            open_history_popup,
            get_history_replay,
            get_settings,
            get_lang_pair,
            set_lang_pair,
            save_popup_size,
            set_hotkey,
            set_engine_choice,
            set_model_override,
            save_api_key,
            start_translation,
            cancel_translation
        ])
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            let cfg = config::load(handle);
            handle.manage(EngineHandle(Mutex::new(build_engine(&cfg))));

            let hotkey_spec =
                hotkey::HotkeySpec::from_accelerator(&cfg.hotkey).unwrap_or_default();
            if let Err(e) = hotkey::register(handle, &hotkey_spec) {
                eprintln!("[trapop] hotkey registration failed (app continues without hotkey): {e}");
                *handle.state::<HotkeyStatus>().0.lock().unwrap() = Some(e);
            }

            window::setup_main_window(handle)?;
            window::setup_tray(handle)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
