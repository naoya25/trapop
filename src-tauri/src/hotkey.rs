use std::str::FromStr;
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

    pub fn from_accelerator(accelerator: &str) -> Result<Self, String> {
        let shortcut = Shortcut::from_str(accelerator).map_err(|e| e.to_string())?;
        Ok(Self {
            modifiers: shortcut.mods,
            code: shortcut.key,
        })
    }

    pub fn to_accelerator(&self) -> String {
        self.shortcut().to_string()
    }
}

pub fn register(app: &AppHandle, spec: &HotkeySpec) -> Result<(), String> {
    app.global_shortcut()
        .register(spec.shortcut())
        .map_err(|e| e.to_string())
}

pub fn set_hotkey(app: &AppHandle, spec: &HotkeySpec) -> Result<(), String> {
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;
    register(app, spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_accelerator_string() {
        let spec = HotkeySpec::default();
        let accelerator = spec.to_accelerator();
        let parsed = HotkeySpec::from_accelerator(&accelerator).expect("should parse");

        assert_eq!(parsed.modifiers, spec.modifiers);
        assert_eq!(parsed.code, spec.code);
    }

    #[test]
    fn rejects_invalid_accelerator() {
        assert!(HotkeySpec::from_accelerator("not a real hotkey").is_err());
    }
}
