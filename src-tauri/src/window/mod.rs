pub mod popup;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

pub fn setup_main_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(main) = app.get_webview_window("main") {
        main.hide()?;
    }
    Ok(())
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_settings = MenuItem::with_id(app, "open_settings", "設定...", true, None::<&str>)?;
    let close_all_popups =
        MenuItem::with_id(app, "close_all_popups", "すべての popup を閉じる", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_settings, &close_all_popups, &quit])?;

    TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open_settings" => {
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.show();
                    let _ = main.set_focus();
                }
            }
            "close_all_popups" => popup::close_all(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
