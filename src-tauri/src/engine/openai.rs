use super::{TranslationChunk, TranslationEngine, TranslationInput};
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

pub const DEFAULT_MODEL: &str = "gpt-4.1-mini";
const MODEL_ENV_OVERRIDE: &str = "TRAPOP_OPENAI_MODEL";
const KEYCHAIN_SERVICE: &str = "trapop-openai";
const API_KEY_ENV: &str = "OPENAI_API_KEY";
const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

#[derive(Debug)]
pub struct MissingApiKeyError;

impl std::fmt::Display for MissingApiKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "APIキーが未設定です。設定画面から登録してください。")
    }
}

impl std::error::Error for MissingApiKeyError {}

pub struct OpenAiEngine {
    api_key: String,
    model: String,
    custom_prompt: Option<String>,
    client: reqwest::Client,
}

impl OpenAiEngine {
    pub fn from_environment(
        model_override: Option<&str>,
        custom_prompt: Option<&str>,
    ) -> Result<Self, MissingApiKeyError> {
        let api_key = resolve_api_key().ok_or(MissingApiKeyError)?;
        let model = model_override
            .filter(|m| !m.trim().is_empty())
            .map(|m| m.to_string())
            .or_else(|| std::env::var(MODEL_ENV_OVERRIDE).ok())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        Ok(Self {
            api_key,
            model,
            custom_prompt: custom_prompt.map(str::to_string),
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
            "model": self.model,
            "stream": true,
            "messages": [
                {"role": "system", "content": super::system_prompt(lang_a, lang_b, input.is_html(), self.custom_prompt.as_deref())},
                {"role": "user", "content": input.body()},
            ],
        });

        let response = self
            .client
            .post(ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let body_head: String = body.chars().take(200).collect();
            // 生のレスポンス本文はユーザーに出さず stderr に落とす
            eprintln!("[trapop] OpenAI API error: {status} {body_head}");
            return Err(super::user_facing_api_error("OpenAI", status.as_u16()));
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
impl TranslationEngine for OpenAiEngine {
    fn name(&self) -> &'static str {
        "openai"
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
        if data == "[DONE]" {
            // 終端を送ったらストリーム読みも止める(true を返すと外側 while が
            // 回り続け、フォールバックの done() と合わせて終端が2回飛ぶ)。
            let _ = tx.send(TranslationChunk::done());
            return false;
        }

        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(text) = value["choices"][0]["delta"]["content"].as_str() else {
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

// chat/completions で翻訳に使えるテキスト系モデルだけに絞る
const EXCLUDED_MODEL_KEYWORDS: &[&str] = &[
    "audio",
    "realtime",
    "image",
    "tts",
    "transcribe",
    "whisper",
    "embedding",
    "moderation",
    "dall-e",
    "search",
    "computer-use",
    "codex",
    "instruct",
];

pub async fn list_models() -> Result<Vec<String>, String> {
    let api_key = resolve_api_key().ok_or("APIキーが未設定です")?;
    let response = reqwest::Client::new()
        .get("https://api.openai.com/v1/models")
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(super::user_facing_api_error(
            "OpenAI",
            response.status().as_u16(),
        ));
    }

    let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let mut models: Vec<String> = value["data"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| m["id"].as_str())
                .filter(|id| id.starts_with("gpt-") || id.starts_with("o"))
                .filter(|id| !EXCLUDED_MODEL_KEYWORDS.iter().any(|kw| id.contains(kw)))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    models.sort();
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
