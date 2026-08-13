use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt,
};

tauri_panel! {
    panel!(TrapopPanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true
        }
    })

    panel_event!(TrapopPanelEventHandler {
        window_should_close(window: &NSWindow) -> Bool
    })
}

pub const PANEL_LABEL_PREFIX: &str = "panel-";

const PANEL_WIDTH: f64 = 480.0;
const PANEL_HEIGHT: f64 = 400.0;
const SPACE_PIN_DELAY_MS: u64 = 350;

static PANEL_COUNTER: AtomicU32 = AtomicU32::new(0);

// trapop://new のたびに「今いる Space に1枚出す」。表示中のパネルには触らない
// (各パネルは開いた Space に固定されたまま残る)。パネルは destroy できない
// (spike で実証: tao/wry と衝突してクラッシュ)ため、閉じた=隠れたパネルを
// 再利用するプール方式。無ければ新規生成する。
pub fn show_or_create_panel(app: &AppHandle) -> tauri::Result<()> {
    if let Some(label) = find_hidden_panel(app) {
        if let Ok(panel) = app.get_webview_panel(&label) {
            // 前回表示時の固定(managed)が残ったまま show すると元の Space に
            // 出てしまう。全 Space 参加へ戻して現在の Space に出し、固定し直す
            panel.set_collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary()
                    .into(),
            );
            activate_app();
            panel.show_and_make_key();
            schedule_space_pin(app, label);
            return Ok(());
        }
    }
    spawn_panel(app)
}

pub fn hide_panel(app: &AppHandle, label: &str) {
    if !label.starts_with(PANEL_LABEL_PREFIX) {
        return;
    }
    if let Ok(panel) = app.get_webview_panel(label) {
        panel.hide();
    }
}

fn find_hidden_panel(app: &AppHandle) -> Option<String> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with(PANEL_LABEL_PREFIX))
        .find(|(_, window)| !window.is_visible().unwrap_or(true))
        .map(|(label, _)| label)
}

fn spawn_panel(app: &AppHandle) -> tauri::Result<()> {
    let label = format!(
        "{PANEL_LABEL_PREFIX}{}",
        PANEL_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let (x, y) = panel_center_position(app);

    let window =
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App("panel/index.html".into()))
            .title("TraPoP")
            .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
            .position(x, y)
            .resizable(true)
            .minimizable(false)
            .skip_taskbar(true)
            .visible(false)
            .build()?;

    let panel = window.to_panel::<TrapopPanel>()?;

    panel.set_level(PanelLevel::Floating.value());
    panel.set_style_mask(
        StyleMask::empty()
            .titled()
            .closable()
            .resizable()
            .nonactivating_panel()
            .into(),
    );
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .into(),
    );

    // ネイティブ close(赤ボタン/Esc 経由の performClose)は tao/wry の破棄経路と
    // 衝突してクラッシュするため、window_should_close で横取りして hide() に
    // 差し替える。destroy はしない(隠れたパネルは次の trapop://new で再利用)。
    let handler = TrapopPanelEventHandler::new();
    let handle_for_close = app.clone();
    let label_for_close = label.clone();
    handler.window_should_close(move |_window| {
        let handle = handle_for_close.clone();
        let label = label_for_close.clone();
        let _ = handle_for_close.run_on_main_thread(move || hide_panel(&handle, &label));
        tauri_nspanel::objc2::runtime::Bool::new(false)
    });
    panel.set_event_handler(Some(handler.as_ref()));

    activate_app();
    panel.show_and_make_key();

    schedule_space_pin(app, label);

    Ok(())
}

// パネルにキーボード入力を通すには2条件を両方満たす必要がある。
// (1) パネル自身が key window であること → show() は orderFrontRegardless しか
//     呼ばないため key にならない。show_and_make_key() で makeKeyWindow まで行う。
// (2) アプリ自体がアクティブであること → trapop://new は Raycast など他アプリから
//     open されるので TraPoP は非アクティブのまま。key window でも前面アプリに
//     キーストロークを取られるため、ここで明示的にアクティブ化する。
//
// activate()(macOS 14+)は cooperative activation でシステムに無視されうるので、
// 確実に前面へ出す activateIgnoringOtherApps を使う(deprecated だが挙動が確実)。
fn activate_app() {
    // RunEvent ハンドラ = メインスレッドから呼ばれる想定。取れないときは
    // 何もしない(key window 化だけは走るのでフォーカスが完全に死ぬことはない)
    // tauri_panel! マクロが同名を import 済みのため、ここはフルパスで参照する
    let Some(mtm) = tauri_nspanel::objc2::MainThreadMarker::new() else {
        return;
    };
    #[allow(deprecated)]
    tauri_nspanel::objc2_app_kit::NSApplication::sharedApplication(mtm)
        .activateIgnoringOtherApps(true);
}

fn schedule_space_pin(app: &AppHandle, label: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(SPACE_PIN_DELAY_MS));
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(panel) = app_for_main.get_webview_panel(&label) {
                panel.set_collection_behavior(CollectionBehavior::new().managed().into());
            }
        });
    });
}

fn panel_center_position(app: &AppHandle) -> (f64, f64) {
    let fallback = (0.0, 0.0);

    let Ok(Some(monitor)) = app.primary_monitor() else {
        return fallback;
    };

    let scale = monitor.scale_factor();
    let screen_pos = monitor.position().to_logical::<f64>(scale);
    let screen_size = monitor.size().to_logical::<f64>(scale);

    let x = screen_pos.x + (screen_size.width - PANEL_WIDTH) / 2.0;
    let y = screen_pos.y + (screen_size.height - PANEL_HEIGHT) / 2.0;

    (x, y)
}
