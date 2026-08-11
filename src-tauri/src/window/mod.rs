pub mod popup;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../../icons/icon-tray.png");

pub fn setup_main_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(main) = app.get_webview_window("main") {
        main.hide()?;
    }
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
        let _ = app.emit_to("main", "main-shown", ());
    }
}

/// メイン窓(通常ウィンドウ)が表示されたまま存在すると popup 生成時に Space ジャンプが
/// 再発する(spike 実測)。ホットキー起動直前にメイン窓を隠して回避する。
pub fn hide_main_window_before_popup(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        if main.is_visible().unwrap_or(false) {
            let _ = main.hide();
        }
    }
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_settings = MenuItem::with_id(app, "open_settings", "設定...", true, None::<&str>)?;
    let close_all_popups =
        MenuItem::with_id(app, "close_all_popups", "すべての popup を閉じる", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_settings, &close_all_popups, &quit])?;

    let icon = Image::from_bytes(TRAY_ICON_BYTES)?;

    TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open_settings" => show_main_window(app),
            "close_all_popups" => popup::close_all(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
