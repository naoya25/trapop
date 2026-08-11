use crate::config;
use objc2_app_kit::{NSScreen, NSWindow};
use objc2_foundation::MainThreadMarker;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager, TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

pub const SPACE_PIN_DELAY_MS: u64 = 350;

const POPUP_MIN_DIMENSION: f64 = 100.0;
const POPUP_SCREEN_MARGIN: f64 = 16.0;
const POPUP_STACK_GAP: f64 = 12.0;

fn clamp_popup_dimension(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value >= POPUP_MIN_DIMENSION {
        value
    } else {
        fallback
    }
}

fn popup_size(app: &AppHandle) -> (f64, f64) {
    let cfg = config::load(app);
    let width = clamp_popup_dimension(cfg.popup_width, config::DEFAULT_POPUP_WIDTH);
    let height = clamp_popup_dimension(cfg.popup_height, config::DEFAULT_POPUP_HEIGHT);
    (width, height)
}

#[derive(Default)]
pub struct PopupCounter(AtomicU32);

#[derive(Default)]
pub struct PopupStack(pub Mutex<Vec<String>>);

pub enum PopupMode {
    Paste,
    Replay,
}

impl PopupMode {
    fn as_query(&self) -> &'static str {
        match self {
            Self::Paste => "paste",
            Self::Replay => "replay",
        }
    }
}

fn next_label(app: &AppHandle) -> String {
    let counter = app.state::<PopupCounter>();
    let n = counter.0.fetch_add(1, Ordering::SeqCst);
    format!("popup-{n}")
}

struct ScreenAnchor {
    base_x: f64,
    base_y: f64,
    min_y: f64,
    // 対象スクリーンの水平範囲(グローバル論理座標)。スタック計算で
    // 他ディスプレイ上の popup を混ぜないためのフィルタに使う。
    left_x: f64,
    right_x: f64,
}

fn cursor_screen_bottom_right(
    cursor_x: f64,
    cursor_y: f64,
    width: f64,
    height: f64,
) -> Result<ScreenAnchor, String> {
    let mtm = MainThreadMarker::new().ok_or_else(|| "not on main thread".to_string())?;
    let screens = NSScreen::screens(mtm);
    let main_screen_height = screens
        .firstObject()
        .ok_or_else(|| "no screens found".to_string())?
        .frame()
        .size
        .height;

    let appkit_x = cursor_x;
    let appkit_y = main_screen_height - cursor_y;

    let target = screens
        .iter()
        .find(|screen| {
            let frame = screen.frame();
            appkit_x >= frame.origin.x
                && appkit_x < frame.origin.x + frame.size.width
                && appkit_y >= frame.origin.y
                && appkit_y < frame.origin.y + frame.size.height
        })
        .or_else(|| screens.firstObject())
        .ok_or_else(|| "no screen contains cursor".to_string())?;

    let frame = target.frame();
    let pos_x = frame.origin.x + frame.size.width - POPUP_SCREEN_MARGIN - width;
    let top_appkit_y = frame.origin.y + POPUP_SCREEN_MARGIN + height;
    let pos_y = main_screen_height - top_appkit_y;

    let screen_top_appkit = frame.origin.y + frame.size.height;
    let min_y = main_screen_height - screen_top_appkit + POPUP_SCREEN_MARGIN;

    Ok(ScreenAnchor {
        base_x: pos_x,
        base_y: pos_y,
        min_y,
        left_x: frame.origin.x,
        right_x: frame.origin.x + frame.size.width,
    })
}

fn stacked_position(
    app: &AppHandle,
    cursor_x: f64,
    cursor_y: f64,
    width: f64,
    height: f64,
) -> Result<(f64, f64), String> {
    let anchor = cursor_screen_bottom_right(cursor_x, cursor_y, width, height)?;

    let stack = app.state::<PopupStack>();
    let mut slots = stack.0.lock().unwrap();
    slots.retain(|label| app.get_webview_window(label).is_some());

    // spawn 時の座標はドラッグ移動で古くなるため、生きている popup の
    // 現在位置から都度計算する。対象スクリーン上の popup だけを見る。
    // outer_position(物理)÷ その window 自身の scale_factor は、tao が
    // 「window の載る monitor の scale で論理⇔物理変換する」実装のため、
    // 混在 DPI でもグローバル論理座標に正しく戻る。
    let topmost_y = slots
        .iter()
        .filter_map(|label| {
            let window = app.get_webview_window(label)?;
            let scale = window.scale_factor().ok()?;
            let pos = window.outer_position().ok()?;
            let x = f64::from(pos.x) / scale;
            let y = f64::from(pos.y) / scale;
            (x >= anchor.left_x && x < anchor.right_x).then_some(y)
        })
        .fold(None, |acc: Option<f64>, y| Some(acc.map_or(y, |min| min.min(y))));

    let pos_y = match topmost_y {
        Some(top_y) => {
            let candidate = top_y - POPUP_STACK_GAP - height;
            // 画面上端を越えたら base_y に戻すと最古の popup を完全に隠すため、
            // 上端に張り付ける(重なりは部分的に留める)。
            candidate.max(anchor.min_y)
        }
        None => anchor.base_y,
    };

    Ok((anchor.base_x, pos_y))
}

fn ns_window_ref(window: &tauri::WebviewWindow) -> Result<&NSWindow, String> {
    let ptr = window.ns_window().map_err(|e| e.to_string())?;
    Ok(unsafe { &*(ptr as *const NSWindow) })
}

pub fn spawn(
    app: &AppHandle,
    cursor_x: f64,
    cursor_y: f64,
    mode: PopupMode,
) -> Result<String, String> {
    let label = next_label(app);
    let (width, height) = popup_size(app);
    let (pos_x, pos_y) = stacked_position(app, cursor_x, cursor_y, width, height)?;

    let window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App(format!("popup/index.html?w={label}&mode={}", mode.as_query()).into()),
    )
    .title("TraPoP")
    // 訳文中の外部リンクをクリックしても常時最前面の webview が
    // そのまま任意サイトへ遷移しないようアプリ内 URL だけ許可し、
    // 外部 http/https は既定ブラウザへ逃がす。localhost は dev server 用。
    .on_navigation({
        let app_for_nav = app.clone();
        move |url| {
            let scheme = url.scheme();
            // tauri scheme は popup の初期ロードだけ許可する。一律許可だと訳文中の
            // 相対リンク(ammonia 既定で素通り)を踏んだとき別パスへ遷移して訳が消える。
            let internal = (scheme == "tauri" && url.path() == "/popup/index.html")
                || (cfg!(debug_assertions)
                    && scheme == "http"
                    && url.host_str() == Some("localhost"));
            if !internal && matches!(scheme, "http" | "https") {
                use tauri_plugin_opener::OpenerExt;
                let _ = app_for_nav
                    .opener()
                    .open_url(url.to_string(), None::<String>);
            }
            internal
        }
    })
    .inner_size(width, height)
    .decorations(true)
    .title_bar_style(TitleBarStyle::Overlay)
    .hidden_title(true)
    .minimizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .visible(false)
    .focused(false)
    .position(pos_x, pos_y)
    .build()
    .map_err(|e| format!("popup window build failed: {e}"))?;

    app.state::<PopupStack>()
        .0
        .lock()
        .unwrap()
        .push(label.clone());

    let app_for_event = app.clone();
    let label_for_event = label.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            app_for_event
                .state::<crate::translation_registry::TranslationRegistry>()
                .cancel(&label_for_event, None);
            app_for_event
                .state::<crate::HistoryReplayState>()
                .0
                .lock()
                .unwrap()
                .remove(&label_for_event);
            app_for_event
                .state::<PopupStack>()
                .0
                .lock()
                .unwrap()
                .retain(|l| l != &label_for_event);
        }
    });

    // build 後に失敗すると不可視 window が残り、以降のスタック座標計算を
    // 狂わせ続けるため、失敗時は window ごと破棄して stack からも抜く。
    let post_build = apply_collection_behavior(&window, CollectionBehaviorPreset::MoveToActiveSpace)
        .and_then(|_| window.show().map_err(|e| e.to_string()));
    if let Err(e) = post_build {
        let _ = window.destroy();
        app.state::<PopupStack>()
            .0
            .lock()
            .unwrap()
            .retain(|l| l != &label);
        return Err(e);
    }

    schedule_space_pin(app, label.clone());

    Ok(label)
}

enum CollectionBehaviorPreset {
    MoveToActiveSpace,
    Managed,
}

fn apply_collection_behavior(
    window: &tauri::WebviewWindow,
    preset: CollectionBehaviorPreset,
) -> Result<(), String> {
    use objc2_app_kit::NSWindowCollectionBehavior;

    let behavior = match preset {
        CollectionBehaviorPreset::MoveToActiveSpace => {
            NSWindowCollectionBehavior::MoveToActiveSpace
        }
        CollectionBehaviorPreset::Managed => NSWindowCollectionBehavior::Managed,
    };

    ns_window_ref(window)?.setCollectionBehavior(behavior);
    Ok(())
}

fn schedule_space_pin(app: &AppHandle, label: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(SPACE_PIN_DELAY_MS));
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(window) = app_for_main.get_webview_window(&label) {
                let _ = apply_collection_behavior(&window, CollectionBehaviorPreset::Managed);
            }
        });
    });
}

pub fn close_all(app: &AppHandle) {
    let stack = app.state::<PopupStack>();
    let labels: Vec<String> = stack.0.lock().unwrap().clone();
    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
    }
}
