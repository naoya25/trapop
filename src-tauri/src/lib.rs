mod config;
mod engine;
mod history;
mod sanitize;
mod translation_registry;
mod window;

use engine::{TranslationEngine, TranslationInput};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
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
        config.custom_prompt.as_deref(),
    );
    EngineState {
        engine: Arc::from(resolved.engine),
        unavailable_reason: resolved.unavailable_reason,
    }
}

#[derive(serde::Serialize)]
struct SettingsView {
    engine_choice: &'static str,
    model_override: Option<String>,
    custom_prompt: Option<String>,
    default_prompt: &'static str,
    has_openai_key: bool,
    has_gemini_key: bool,
    effective_engine_name: &'static str,
    lang_a: String,
    lang_b: String,
    translation_target: &'static str,
}

#[derive(serde::Serialize, Clone)]
struct TranslateChunkEvent {
    request_id: u64,
    text: String,
    done: bool,
    error: bool,
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

// Keychain 存在確認(security プロセス2回)とファイル読みはブロッキングなので
// spawn_blocking で逃がす。
#[tauri::command]
async fn get_settings(
    app: AppHandle,
    engine: tauri::State<'_, EngineHandle>,
) -> Result<SettingsView, String> {
    let effective_engine_name = effective_engine_label(&engine);
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = config::load(&app);
        SettingsView {
            engine_choice: cfg.engine_choice.as_str(),
            model_override: cfg.model_override,
            custom_prompt: cfg.custom_prompt,
            default_prompt: engine::DEFAULT_PROMPT_TEMPLATE,
            has_openai_key: engine::openai::has_stored_key(),
            has_gemini_key: engine::gemini::has_stored_key(),
            effective_engine_name,
            lang_a: cfg.lang_a,
            lang_b: cfg.lang_b,
            translation_target: cfg.translation_target.as_str(),
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

// プロンプトは翻訳実行時に組むため、エンジン再構築は不要(config 保存のみ)
#[tauri::command]
async fn set_translation_target(app: AppHandle, target: String) -> Result<(), String> {
    let translation_target = config::TranslationTarget::parse(&target)?;

    tauri::async_runtime::spawn_blocking(move || {
        config::update(&app, |cfg| cfg.translation_target = translation_target).map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_window_size(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    if !config::is_valid_window_size(width, height) {
        return Err("不正なウィンドウサイズです".to_string());
    }

    tauri::async_runtime::spawn_blocking(move || {
        config::update(&app, |cfg| {
            cfg.window_width = width;
            cfg.window_height = height;
        })
        .map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
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

// 空・空白のみは None(=組み込みプロンプト)に正規化する。エンジンが
// 生成時にプロンプトを保持するため、保存後にエンジンを組み直す。
#[tauri::command]
async fn set_custom_prompt(
    app: AppHandle,
    engine: tauri::State<'_, EngineHandle>,
    prompt: Option<String>,
) -> Result<(), String> {
    let prompt = prompt
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());

    let state = tauri::async_runtime::spawn_blocking(move || -> Result<EngineState, String> {
        let cfg = config::update(&app, |cfg| cfg.custom_prompt = prompt)?;
        Ok(build_engine(&cfg))
    })
    .await
    .map_err(|e| e.to_string())??;
    engine.set(state);
    Ok(())
}

// 各プロバイダの ListModels API から翻訳に使えるモデル名を取得する。
// キー未設定・通信失敗は Err で返し、フロント側は静的リストへフォールバックする。
#[tauri::command]
async fn list_available_models(provider: String) -> Result<Vec<String>, String> {
    match provider.as_str() {
        "openai" => engine::openai::list_models().await,
        "gemini" => engine::gemini::list_models().await,
        other => Err(format!("不明なプロバイダです: {other}")),
    }
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

// label は引数で受けず呼び出し元ウィンドウから取る(他パネルを閉じさせない)
#[tauri::command]
fn close_panel(app: AppHandle, window: tauri::WebviewWindow) {
    window::panel::hide_panel(&app, window.label());
}

#[tauri::command]
fn cancel_translation(
    window: tauri::WebviewWindow,
    translation_registry: tauri::State<TranslationRegistry>,
    request_id: u64,
) {
    translation_registry.cancel(window.label(), Some(request_id));
}

// label は引数で受けず呼び出し元ウィンドウから取る(なりすまし不可)。
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
    let target = match cfg.translation_target {
        config::TranslationTarget::Auto => None,
        config::TranslationTarget::LangA => Some(cfg.lang_a.clone()),
        config::TranslationTarget::LangB => Some(cfg.lang_b.clone()),
    };
    let engine_task = tokio::spawn(async move {
        engine_handle
            .translate(&translation_input, &lang_a, &lang_b, target.as_deref(), tx)
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
                    // 「✓ 完了」に見せず、エラーとして理由を出す。
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_nspanel::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(TranslationRegistry::default())
        .manage(window::DeepLinkSeen::default())
        .invoke_handler(tauri::generate_handler![
            sanitize_html,
            list_history,
            clear_history,
            get_settings,
            set_lang_pair,
            set_translation_target,
            save_window_size,
            set_engine_choice,
            set_model_override,
            set_custom_prompt,
            list_available_models,
            save_api_key,
            start_translation,
            cancel_translation,
            close_panel
        ])
        .setup(|app| {
            let handle = app.handle();
            let cfg = config::load(handle);
            handle.manage(EngineHandle(Mutex::new(build_engine(&cfg))));

            // メイン窓はここでは表示しない。RunEvent::Ready 後、deep link 起動で
            // なかったと分かってから window::show_main_window で表示する
            window::setup_main_window(handle, false)?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            // 閉じるボタン(CloseRequested→exit)も ⌘Q(terminate→LoopDestroyed)も
            // 最終的に RunEvent::Exit を通る。ExitRequested は ⌘Q 経路で発火しない
            tauri::RunEvent::Exit => window::flush_window_size(app_handle),
            tauri::RunEvent::Ready => {
                window::schedule_main_window_show_unless_deep_link(app_handle);
            }
            tauri::RunEvent::Opened { urls } => {
                if urls.iter().any(window::deep_link::is_new_panel_url) {
                    app_handle.state::<window::DeepLinkSeen>().mark();
                    let _ = window::panel::show_or_create_panel(app_handle);
                }
            }
            tauri::RunEvent::Reopen { has_visible_windows, .. } => {
                if !has_visible_windows {
                    window::show_main_window(app_handle);
                }
            }
            _ => {}
        });
}
