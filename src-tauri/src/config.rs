use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const CONFIG_FILE: &str = "config.json";
const DEFAULT_LANG_A: &str = "日本語";
const DEFAULT_LANG_B: &str = "英語";
// tauri.conf.json の windows[0] の width/height と同値を保つこと
// (conf のサイズで表示された直後に setup が set_size で上書きするため、ズレると起動時にリサイズが見える)
pub const DEFAULT_WINDOW_WIDTH: f64 = 860.0;
pub const DEFAULT_WINDOW_HEIGHT: f64 = 600.0;

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
    #[serde(default)]
    pub engine_choice: EngineChoice,
    #[serde(default)]
    pub model_override: Option<String>,
    // None = 組み込みプロンプトを使う。{lang_a}/{lang_b} が言語名に置換される
    #[serde(default)]
    pub custom_prompt: Option<String>,
    #[serde(default = "default_lang_a")]
    pub lang_a: String,
    #[serde(default = "default_lang_b")]
    pub lang_b: String,
    #[serde(default = "default_window_width")]
    pub window_width: f64,
    #[serde(default = "default_window_height")]
    pub window_height: f64,
}

// ウィンドウサイズの妥当性(有限かつ正)。save_window_size コマンドと
// リサイズキャッシュの両方の書き手がこれを通す
pub fn is_valid_window_size(width: f64, height: f64) -> bool {
    width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0
}

fn default_lang_a() -> String {
    DEFAULT_LANG_A.to_string()
}

fn default_lang_b() -> String {
    DEFAULT_LANG_B.to_string()
}

fn default_window_width() -> f64 {
    DEFAULT_WINDOW_WIDTH
}

fn default_window_height() -> f64 {
    DEFAULT_WINDOW_HEIGHT
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            engine_choice: EngineChoice::default(),
            model_override: None,
            custom_prompt: None,
            lang_a: default_lang_a(),
            lang_b: default_lang_b(),
            window_width: default_window_width(),
            window_height: default_window_height(),
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
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

// 設定変更は read-modify-write なので、コマンドの並行実行(リサイズ保存と設定保存の
// 同時実行等)で他フィールドを古い値へ巻き戻さないよう、プロセス内 lock で直列化する。
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
        assert_eq!(config.engine_choice, EngineChoice::Auto);
        assert_eq!(config.lang_a, DEFAULT_LANG_A);
        assert_eq!(config.lang_b, DEFAULT_LANG_B);
        assert_eq!(config.window_width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(config.window_height, DEFAULT_WINDOW_HEIGHT);
    }

    #[test]
    fn round_trips_through_json() {
        let config = AppConfig {
            engine_choice: EngineChoice::Openai,
            model_override: Some("gpt-4.1".to_string()),
            custom_prompt: Some("カジュアルに訳して。{lang_a}⇔{lang_b}".to_string()),
            lang_a: "中国語".to_string(),
            lang_b: "韓国語".to_string(),
            window_width: 500.0,
            window_height: 400.0,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed = parse_config(&json);

        assert_eq!(parsed.engine_choice, EngineChoice::Openai);
        assert_eq!(parsed.model_override.as_deref(), Some("gpt-4.1"));
        assert_eq!(
            parsed.custom_prompt.as_deref(),
            Some("カジュアルに訳して。{lang_a}⇔{lang_b}")
        );
        assert_eq!(parsed.lang_a, "中国語");
        assert_eq!(parsed.lang_b, "韓国語");
        assert_eq!(parsed.window_width, 500.0);
        assert_eq!(parsed.window_height, 400.0);
    }

    #[test]
    fn missing_lang_fields_fall_back_to_defaults() {
        // 部分的な config(不足キーと未知キーの両方)から既定値へ落ちることを検証する
        let config = parse_config(r#"{"unknown_key":"x"}"#);
        assert_eq!(config.lang_a, DEFAULT_LANG_A);
        assert_eq!(config.lang_b, DEFAULT_LANG_B);
    }

    #[test]
    fn missing_window_size_falls_back_to_defaults() {
        let config = parse_config(r#"{"unknown_key":"x"}"#);
        assert_eq!(config.window_width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(config.window_height, DEFAULT_WINDOW_HEIGHT);
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
