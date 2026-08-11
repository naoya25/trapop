use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const CONFIG_FILE: &str = "config.json";
const DEFAULT_HOTKEY: &str = "shift+alt+super+KeyP";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineChoice {
    #[default]
    Auto,
    Openai,
    Mock,
}

impl EngineChoice {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Openai => "openai",
            Self::Mock => "mock",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "openai" => Ok(Self::Openai),
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
}

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            engine_choice: EngineChoice::default(),
            model_override: None,
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
    fs::write(path, json).map_err(|e| e.to_string())
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
    }

    #[test]
    fn round_trips_through_json() {
        let config = AppConfig {
            hotkey: "control+alt+KeyJ".to_string(),
            engine_choice: EngineChoice::Openai,
            model_override: Some("gpt-4.1".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed = parse_config(&json);

        assert_eq!(parsed.hotkey, config.hotkey);
        assert_eq!(parsed.engine_choice, EngineChoice::Openai);
        assert_eq!(parsed.model_override.as_deref(), Some("gpt-4.1"));
    }

    #[test]
    fn engine_choice_round_trips_through_str() {
        for choice in [EngineChoice::Auto, EngineChoice::Openai, EngineChoice::Mock] {
            let parsed = EngineChoice::parse(choice.as_str()).unwrap();
            assert_eq!(parsed, choice);
        }
    }

    #[test]
    fn rejects_unknown_engine_choice() {
        assert!(EngineChoice::parse("gemini").is_err());
    }
}
