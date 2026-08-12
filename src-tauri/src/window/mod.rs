pub mod deep_link;
pub mod panel;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};

// 終了時にウィンドウはもう生きていない(閉じるボタン経路では drop 済み)ため、
// サイズはリサイズイベントの時点で控えておき、終了時はキャッシュを書くだけにする
static LAST_WINDOW_SIZE: Mutex<Option<(f64, f64)>> = Mutex::new(None);

const MAIN_SHOW_DELAY_MS: u64 = 100;

// RunEvent::Opened(deep link)が Ready の前後どちらで届くか保証が無いため、
// Ready 後に短い猶予を置いてから「deep link 起動ではなかった」ときだけメイン窓を出す。
pub struct DeepLinkSeen(AtomicBool);

impl Default for DeepLinkSeen {
    fn default() -> Self {
        Self(AtomicBool::new(false))
    }
}

impl DeepLinkSeen {
    pub fn mark(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn was_seen(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

// show=false のときはサイズ復元とイベント配線だけ行い、表示はしない
// (deep link 起動でメイン窓を隠したまま保つため)
pub fn setup_main_window(app: &AppHandle, show: bool) -> tauri::Result<()> {
    if let Some(main) = app.get_webview_window("main") {
        let cfg = crate::config::load(app);
        if let Err(e) = main.set_size(tauri::Size::Logical(tauri::LogicalSize {
            width: cfg.window_width,
            height: cfg.window_height,
        })) {
            // 失敗してもアプリは使える。開発時に気づけるよう stderr にだけ残す
            eprintln!("[trapop] window size restore failed: {e}");
        }

        let app_for_event = app.clone();
        let main_for_event = main.clone();
        main.on_window_event(move |event| match event {
            tauri::WindowEvent::Resized(size) => match main_for_event.scale_factor() {
                Ok(scale) => {
                    let logical = size.to_logical::<f64>(scale);
                    // 不正値(最小化の 0×0 等)はキャッシュに入れない。直前の正当な
                    // サイズを保持し続けることで終了時保存が空振りしない
                    if !crate::config::is_valid_window_size(logical.width, logical.height) {
                        return;
                    }
                    match LAST_WINDOW_SIZE.lock() {
                        Ok(mut cache) => *cache = Some((logical.width, logical.height)),
                        Err(e) => eprintln!("[trapop] window size cache failed: {e}"),
                    }
                }
                Err(e) => eprintln!("[trapop] window size cache failed: {e}"),
            },
            tauri::WindowEvent::CloseRequested { .. } => {
                app_for_event.exit(0);
            }
            _ => {}
        });

        if show {
            main.show()?;
            main.set_focus()?;
        }
    }
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

// RunEvent::Ready ハンドラから呼ぶ。deep link 起動が Ready より後に届く場合に
// 備えて短い猶予を置き、その間に DeepLinkSeen が立たなければメイン窓を出す
pub fn schedule_main_window_show_unless_deep_link(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(MAIN_SHOW_DELAY_MS));
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            let seen = app_for_main.state::<DeepLinkSeen>().was_seen();
            if !seen {
                show_main_window(&app_for_main);
            }
        });
    });
}

// フロントのリサイズ保存は debounce されているため、終了直前のサイズは
// ここで同期保存する(リサイズ直後に閉じても・⌘Q でも保存が飛ばない)
pub fn flush_window_size(app: &AppHandle) {
    // キャッシュは検証済みの値しか持たないため無条件に書ける。
    // 空(setup の set_size 由来の Resized が来る前に終了)なら保存済みの値のままでよい
    let Some((width, height)) = LAST_WINDOW_SIZE.lock().ok().and_then(|cache| *cache) else {
        return;
    };
    if let Err(e) = crate::config::update(app, |cfg| {
        cfg.window_width = width;
        cfg.window_height = height;
    }) {
        eprintln!("[trapop] window size save failed: {e}");
    }
}
