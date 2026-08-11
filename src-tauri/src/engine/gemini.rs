use super::{TranslationChunk, TranslationEngine, TranslationInput};
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

pub const DEFAULT_MODEL: &str = "gemini-2.5-flash";
const MODEL_ENV_OVERRIDE: &str = "TRAPOP_GEMINI_MODEL";
const KEYCHAIN_SERVICE: &str = "trapop-gemini";
const API_KEY_ENV: &str = "GEMINI_API_KEY";
const ENDPOINT_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Debug)]
pub struct MissingApiKeyError;

impl std::fmt::Display for MissingApiKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "APIキーが未設定です。設定画面から登録してください。")
    }
}

impl std::error::Error for MissingApiKeyError {}

pub struct GeminiEngine {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl GeminiEngine {
    pub fn from_environment(model_override: Option<&str>) -> Result<Self, MissingApiKeyError> {
        let api_key = resolve_api_key().ok_or(MissingApiKeyError)?;
        let model = model_override
            .filter(|m| !m.trim().is_empty())
            .map(|m| m.to_string())
            .or_else(|| std::env::var(MODEL_ENV_OVERRIDE).ok())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        Ok(Self {
            api_key,
            model,
            client: reqwest::Client::new(),
        })
    }

    async fn stream_translation(
        &self,
        input: &TranslationInput,
        lang_a: &str,
        lang_b: &str,
        tx: &UnboundedSender<TranslationChunk>,
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "systemInstruction": {
                "parts": [{"text": super::system_prompt(lang_a, lang_b, input.is_html())}],
            },
            "contents": [
                {"role": "user", "parts": [{"text": input.body()}]},
            ],
        });

        let endpoint = format!(
            "{ENDPOINT_BASE}/{model}:streamGenerateContent?alt=sse",
            model = self.model
        );

        let response = self
            .client
            .post(endpoint)
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            return Err(format!("Gemini API error: {}", response.status()));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| e.to_string())?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(pos) = buffer.find("\n\n") {
                let event: String = buffer.drain(..pos + 2).collect();
                if !forward_event(&event, tx) {
                    return Ok(());
                }
            }
        }

        let _ = tx.send(TranslationChunk {
            text: String::new(),
            done: true,
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl TranslationEngine for GeminiEngine {
    fn name(&self) -> &'static str {
        "gemini"
    }

    async fn translate(
        &self,
        input: &TranslationInput,
        lang_a: &str,
        lang_b: &str,
        tx: UnboundedSender<TranslationChunk>,
    ) {
        if let Err(err) = self.stream_translation(input, lang_a, lang_b, &tx).await {
            let _ = tx.send(TranslationChunk {
                text: format!("翻訳中にエラーが発生しました: {err}"),
                done: true,
            });
        }
    }
}

fn forward_event(event: &str, tx: &UnboundedSender<TranslationChunk>) -> bool {
    for line in event.lines() {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };

        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(text) = value["candidates"][0]["content"]["parts"][0]["text"].as_str() else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        if tx
            .send(TranslationChunk {
                text: text.to_string(),
                done: false,
            })
            .is_err()
        {
            return false;
        }
    }
    true
}

fn resolve_api_key() -> Option<String> {
    keychain_api_key().or_else(|| {
        std::env::var(API_KEY_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
    })
}

fn keychain_api_key() -> Option<String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let key = String::from_utf8(output.stdout).ok()?;
    let trimmed = key.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn has_stored_key() -> bool {
    keychain_api_key().is_some()
}

pub fn store_api_key(key: &str) -> Result<(), String> {
    let status = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-a",
            "trapop",
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
            key,
        ])
        .status()
        .map_err(|e| e.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err("Keychain へのAPIキー保存に失敗しました".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_environment_errors_without_api_key() {
        std::env::remove_var(API_KEY_ENV);
        if has_stored_key() {
            return;
        }
        let result = GeminiEngine::from_environment(None);
        assert!(result.is_err());
    }

    #[test]
    fn html_system_prompt_extends_plain_text_prompt() {
        let plain = super::super::system_prompt("日本語", "英語", false);
        let html = super::super::system_prompt("日本語", "英語", true);
        assert!(html.contains("HTML"));
        assert!(plain.contains("日本語"));
        assert!(plain.contains("英語"));
    }

    #[test]
    fn bidirectional_prompt_mentions_both_languages_and_fallback() {
        let prompt = super::super::system_prompt("日本語", "英語", false);
        assert!(prompt.contains("日本語であれば英語に"));
        assert!(prompt.contains("英語であれば日本語に"));
        assert!(prompt.contains("どちらの言語でもない場合は日本語に"));
    }

    #[test]
    fn plain_text_prompt_forbids_html_tags() {
        let prompt = super::super::system_prompt("日本語", "英語", false);
        assert!(prompt.contains("HTMLタグは出力に一切含めないでください"));
        assert!(prompt.contains("<div>"));
    }
}
