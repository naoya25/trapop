use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

pub struct HotkeySpec {
    pub modifiers: Modifiers,
    pub code: Code,
}

impl Default for HotkeySpec {
    fn default() -> Self {
        Self {
            modifiers: Modifiers::SUPER | Modifiers::ALT | Modifiers::SHIFT,
            code: Code::KeyP,
        }
    }
}

impl HotkeySpec {
    pub fn shortcut(&self) -> Shortcut {
        Shortcut::new(Some(self.modifiers), self.code)
    }
}

pub fn register(app: &AppHandle, spec: &HotkeySpec) -> Result<(), String> {
    app.global_shortcut()
        .register(spec.shortcut())
        .map_err(|e| e.to_string())
}

// 設定画面からのホットキー変更用に公開しておくフック。設定 UI は未実装のため呼び出し元がまだ無い。
#[allow(dead_code)]
pub fn set_hotkey(app: &AppHandle, spec: &HotkeySpec) -> Result<(), String> {
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;
    register(app, spec)
}
