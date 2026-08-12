use super::{TranslationChunk, TranslationEngine, TranslationInput};
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

// 固定名のモデルは提供終了で 404 になりうるため、
// 常に最新 flash を指すエイリアスを既定にする
pub const DEFAULT_MODEL: &str = "gemini-flash-latest";
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
    custom_prompt: Option<String>,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl GeminiEngine {
    pub fn from_environment(
        model_override: Option<&str>,
        custom_prompt: Option<&str>,
    ) -> Result<Self, MissingApiKeyError> {
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
            custom_prompt: custom_prompt.map(str::to_string),
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
        target: Option<&str>,
        tx: &UnboundedSender<TranslationChunk>,
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "systemInstruction": {
                "parts": [{"text": super::system_prompt(lang_a, lang_b, input.is_html(), self.custom_prompt.as_deref(), target)}],
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
        target: Option<&str>,
        tx: UnboundedSender<TranslationChunk>,
    ) {
        if let Err(err) = self
            .stream_translation(input, lang_a, lang_b, target, &tx)
            .await
        {
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

// 翻訳(generateContent)に使えるテキスト系モデルだけに絞る。
// 注意: ListModels に載っていても実行時に拒否されるモデルはある。
const EXCLUDED_MODEL_KEYWORDS: &[&str] = &[
    "tts",
    "image",
    "robotics",
    "computer-use",
    "deep-research",
    "embedding",
    "aqa",
    "lyria",
    "nano-banana",
    "antigravity",
];

pub async fn list_models() -> Result<Vec<String>, String> {
    let api_key = resolve_api_key().ok_or("APIキーが未設定です")?;
    let response = reqwest::Client::new()
        .get(format!("{ENDPOINT_BASE}?pageSize=200"))
        .header("x-goog-api-key", &api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(super::user_facing_api_error(
            "Gemini",
            response.status().as_u16(),
        ));
    }

    let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let mut models: Vec<String> = value["models"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|m| {
                    m["supportedGenerationMethods"]
                        .as_array()
                        .is_some_and(|methods| {
                            methods.iter().any(|v| v.as_str() == Some("generateContent"))
                        })
                })
                .filter_map(|m| m["name"].as_str())
                .filter_map(|name| name.strip_prefix("models/"))
                .filter(|name| name.starts_with("gemini"))
                .filter(|name| !EXCLUDED_MODEL_KEYWORDS.iter().any(|kw| name.contains(kw)))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    // 提供終了に強い -latest エイリアスを先頭へ
    models.sort_by_key(|name| !name.contains("latest"));
    Ok(models)
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
        let result = GeminiEngine::from_environment(None, None);
        assert!(result.is_err());
    }

    #[test]
    fn html_system_prompt_extends_plain_text_prompt() {
        let plain = super::super::system_prompt("日本語", "英語", false, None, None);
        let html = super::super::system_prompt("日本語", "英語", true, None, None);
        assert!(html.contains("HTML"));
        assert!(plain.contains("日本語"));
        assert!(plain.contains("英語"));
    }

    #[test]
    fn bidirectional_prompt_mentions_both_languages_and_fallback() {
        let prompt = super::super::system_prompt("日本語", "英語", false, None, None);
        assert!(prompt.contains("日本語であれば英語に"));
        assert!(prompt.contains("英語であれば日本語に"));
        assert!(prompt.contains("どちらの言語でもない場合は日本語に"));
    }

    #[test]
    fn plain_text_prompt_forbids_html_tags() {
        let prompt = super::super::system_prompt("日本語", "英語", false, None, None);
        assert!(prompt.contains("HTMLタグは出力に一切含めないでください"));
        assert!(prompt.contains("<div>"));
    }
}
