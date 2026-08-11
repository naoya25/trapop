use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const CONFIG_FILE: &str = "config.json";
const DEFAULT_HOTKEY: &str = "shift+alt+super+KeyP";
const DEFAULT_LANG_A: &str = "日本語";
const DEFAULT_LANG_B: &str = "英語";
pub const DEFAULT_POPUP_WIDTH: f64 = 420.0;
pub const DEFAULT_POPUP_HEIGHT: f64 = 320.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineChoice {
    #[default]
    Auto,
    Openai,
    Gemini,
    Mock,
}

impl EngineChoice {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
            Self::Mock => "mock",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "openai" => Ok(Self::Openai),
            "gemini" => Ok(Self::Gemini),
            "mock" => Ok(Self::Mock),
            other => Err(format!("未知のエンジン指定です: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default)]
    pub engine_choice: EngineChoice,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default = "default_lang_a")]
    pub lang_a: String,
    #[serde(default = "default_lang_b")]
    pub lang_b: String,
    #[serde(default = "default_popup_width")]
    pub popup_width: f64,
    #[serde(default = "default_popup_height")]
    pub popup_height: f64,
}

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.to_string()
}

fn default_lang_a() -> String {
    DEFAULT_LANG_A.to_string()
}

fn default_lang_b() -> String {
    DEFAULT_LANG_B.to_string()
}

fn default_popup_width() -> f64 {
    DEFAULT_POPUP_WIDTH
}

fn default_popup_height() -> f64 {
    DEFAULT_POPUP_HEIGHT
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            engine_choice: EngineChoice::default(),
            model_override: None,
            lang_a: default_lang_a(),
            lang_b: default_lang_b(),
            popup_width: default_popup_width(),
            popup_height: default_popup_height(),
        }
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(CONFIG_FILE))
}

pub fn load(app: &AppHandle) -> AppConfig {
    let Ok(path) = config_path(app) else {
        return AppConfig::default();
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    parse_config(&content)
}

pub fn save(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    // fs::write は truncate→write で非原子。書き込み途中に load() が走ると
    // 壊れた JSON を読んで全設定が既定値に落ちるため、temp + rename で原子的に置換する。
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

// 設定変更は read-modify-write なので、複数 popup からの同時保存(リサイズ等)で
// 他フィールドを古い値へ巻き戻さないよう、プロセス内 lock で直列化する。
static UPDATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn update(
    app: &AppHandle,
    mutate: impl FnOnce(&mut AppConfig),
) -> Result<AppConfig, String> {
    let _guard = UPDATE_LOCK.lock().map_err(|e| e.to_string())?;
    let mut cfg = load(app);
    mutate(&mut cfg);
    save(app, &cfg)?;
    Ok(cfg)
}

fn parse_config(content: &str) -> AppConfig {
    serde_json::from_str(content).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_default_on_invalid_json() {
        let config = parse_config("not json");
        assert_eq!(config.hotkey, DEFAULT_HOTKEY);
        assert_eq!(config.engine_choice, EngineChoice::Auto);
        assert_eq!(config.lang_a, DEFAULT_LANG_A);
        assert_eq!(config.lang_b, DEFAULT_LANG_B);
        assert_eq!(config.popup_width, DEFAULT_POPUP_WIDTH);
        assert_eq!(config.popup_height, DEFAULT_POPUP_HEIGHT);
    }

    #[test]
    fn round_trips_through_json() {
        let config = AppConfig {
            hotkey: "control+alt+KeyJ".to_string(),
            engine_choice: EngineChoice::Openai,
            model_override: Some("gpt-4.1".to_string()),
            lang_a: "中国語".to_string(),
            lang_b: "韓国語".to_string(),
            popup_width: 500.0,
            popup_height: 400.0,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed = parse_config(&json);

        assert_eq!(parsed.hotkey, config.hotkey);
        assert_eq!(parsed.engine_choice, EngineChoice::Openai);
        assert_eq!(parsed.model_override.as_deref(), Some("gpt-4.1"));
        assert_eq!(parsed.lang_a, "中国語");
        assert_eq!(parsed.lang_b, "韓国語");
        assert_eq!(parsed.popup_width, 500.0);
        assert_eq!(parsed.popup_height, 400.0);
    }

    #[test]
    fn missing_lang_fields_fall_back_to_defaults() {
        let config = parse_config(r#"{"hotkey":"control+alt+KeyJ"}"#);
        assert_eq!(config.lang_a, DEFAULT_LANG_A);
        assert_eq!(config.lang_b, DEFAULT_LANG_B);
    }

    #[test]
    fn missing_popup_size_falls_back_to_defaults() {
        let config = parse_config(r#"{"hotkey":"control+alt+KeyJ"}"#);
        assert_eq!(config.popup_width, DEFAULT_POPUP_WIDTH);
        assert_eq!(config.popup_height, DEFAULT_POPUP_HEIGHT);
    }

    #[test]
    fn engine_choice_round_trips_through_str() {
        for choice in [
            EngineChoice::Auto,
            EngineChoice::Openai,
            EngineChoice::Gemini,
            EngineChoice::Mock,
        ] {
            let parsed = EngineChoice::parse(choice.as_str()).unwrap();
            assert_eq!(parsed, choice);
        }
    }

    #[test]
    fn rejects_unknown_engine_choice() {
        assert!(EngineChoice::parse("anthropic").is_err());
    }
}
