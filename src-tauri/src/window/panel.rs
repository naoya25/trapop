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

pub const PANEL_LABEL: &str = "panel";

const PANEL_WIDTH: f64 = 480.0;
const PANEL_HEIGHT: f64 = 400.0;
const SPACE_PIN_DELAY_MS: u64 = 350;

// 隠れていれば再表示、無ければ生成する。既存パネルの再表示時も現在の Space に
// 固定し直す(spike P4: 出現→350ms 後 managed 切替で開いた Space に固定)。
pub fn show_or_create_panel(app: &AppHandle) -> tauri::Result<()> {
    if let Ok(panel) = app.get_webview_panel(PANEL_LABEL) {
        panel.show();
        schedule_space_pin(app);
        return Ok(());
    }
    spawn_panel(app)
}

pub fn hide_panel(app: &AppHandle) {
    if let Ok(panel) = app.get_webview_panel(PANEL_LABEL) {
        panel.hide();
    }
}

fn spawn_panel(app: &AppHandle) -> tauri::Result<()> {
    let (x, y) = panel_center_position(app);

    let window = WebviewWindowBuilder::new(app, PANEL_LABEL, WebviewUrl::App("panel/index.html".into()))
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
    // 差し替える。destroy はしない(2回目以降の trapop://new を再表示で速くする)。
    let handler = TrapopPanelEventHandler::new();
    let handle_for_close = app.clone();
    handler.window_should_close(move |_window| {
        let handle = handle_for_close.clone();
        let _ = handle_for_close.run_on_main_thread(move || hide_panel(&handle));
        tauri_nspanel::objc2::runtime::Bool::new(false)
    });
    panel.set_event_handler(Some(handler.as_ref()));

    panel.show();

    schedule_space_pin(app);

    Ok(())
}

fn schedule_space_pin(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(SPACE_PIN_DELAY_MS));
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(panel) = app_for_main.get_webview_panel(PANEL_LABEL) {
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
