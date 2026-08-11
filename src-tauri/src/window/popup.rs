use objc2_app_kit::{NSScreen, NSWindow};
use objc2_foundation::MainThreadMarker;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager, TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

pub const SPACE_PIN_DELAY_MS: u64 = 350;

const POPUP_WIDTH: f64 = 420.0;
const POPUP_HEIGHT: f64 = 320.0;
const POPUP_SCREEN_MARGIN: f64 = 16.0;
const POPUP_STACK_GAP: f64 = 12.0;

#[derive(Default)]
pub struct PopupCounter(AtomicU32);

#[derive(Default)]
pub struct PopupStack(pub Mutex<Vec<(String, f64)>>);

pub enum PopupMode {
    Capture,
    Replay,
}

impl PopupMode {
    fn as_query(&self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Replay => "replay",
        }
    }
}

fn next_label(app: &AppHandle) -> String {
    let counter = app.state::<PopupCounter>();
    let n = counter.0.fetch_add(1, Ordering::SeqCst);
    format!("popup-{n}")
}

fn cursor_screen_bottom_right(cursor_x: f64, cursor_y: f64) -> Result<(f64, f64), String> {
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
    let pos_x = frame.origin.x + frame.size.width - POPUP_SCREEN_MARGIN - POPUP_WIDTH;
    let top_appkit_y = frame.origin.y + POPUP_SCREEN_MARGIN + POPUP_HEIGHT;
    let pos_y = main_screen_height - top_appkit_y;

    Ok((pos_x, pos_y))
}

fn stacked_position(app: &AppHandle, cursor_x: f64, cursor_y: f64) -> Result<(f64, f64), String> {
    let (base_x, base_y) = cursor_screen_bottom_right(cursor_x, cursor_y)?;

    let stack = app.state::<PopupStack>();
    let mut slots = stack.0.lock().unwrap();
    slots.retain(|(label, _)| app.get_webview_window(label).is_some());

    let topmost_y = slots
        .iter()
        .map(|(_, top_y)| *top_y)
        .fold(None, |acc: Option<f64>, y| Some(acc.map_or(y, |min| min.min(y))));

    let pos_y = match topmost_y {
        Some(top_y) => top_y - POPUP_STACK_GAP - POPUP_HEIGHT,
        None => base_y,
    };

    Ok((base_x, pos_y))
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
    let (pos_x, pos_y) = stacked_position(app, cursor_x, cursor_y)?;

    let window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App(format!("popup/index.html?w={label}&mode={}", mode.as_query()).into()),
    )
    .title("TraPoP")
    .inner_size(POPUP_WIDTH, POPUP_HEIGHT)
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
        .push((label.clone(), pos_y));

    apply_collection_behavior(&window, CollectionBehaviorPreset::MoveToActiveSpace)?;
    window.show().map_err(|e| e.to_string())?;

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
    let labels: Vec<String> = stack.0.lock().unwrap().iter().map(|(l, _)| l.clone()).collect();
    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
    }
}
