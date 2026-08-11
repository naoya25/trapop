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
            // モデル名はエンドポイント URL に連結するため、パス/クエリを
            // 差し替えられる文字(`/` `?` `#` 等)が入っていたら既定に落とす。
            .filter(|m| is_safe_model_name(m))
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
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let body_head: String = body.chars().take(200).collect();
            // 生のレスポンス本文はユーザーに出さず stderr に落とす
            eprintln!("[trapop] Gemini API error: {status} {body_head}");
            return Err(super::user_facing_api_error("Gemini", status.as_u16()));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = super::sse::SseBuffer::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| e.to_string())?;
            buffer.push(&bytes);

            while let Some(event) = buffer.next_event() {
                if !forward_event(&event, tx) {
                    return Ok(());
                }
            }
        }

        let _ = tx.send(TranslationChunk::done());
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
            let _ = tx.send(TranslationChunk::error(format!(
                "翻訳中にエラーが発生しました: {err}"
            )));
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
        if tx.send(TranslationChunk::text(text)).is_err() {
            return false;
        }
    }
    true
}

fn is_safe_model_name(model: &str) -> bool {
    !model.is_empty()
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn resolve_api_key() -> Option<String> {
    super::keys::resolve_api_key(KEYCHAIN_SERVICE, API_KEY_ENV)
}

pub fn has_stored_key() -> bool {
    super::keys::has_stored_key(KEYCHAIN_SERVICE)
}

pub fn store_api_key(key: &str) -> Result<(), String> {
    super::keys::store_api_key(KEYCHAIN_SERVICE, key)
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
