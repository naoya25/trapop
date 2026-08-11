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
    client: reqwest::Client,
}

impl OpenAiEngine {
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
        tx: &UnboundedSender<TranslationChunk>,
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "model": self.model,
            "stream": true,
            "messages": [
                {"role": "system", "content": system_prompt(input.is_html())},
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
            return Err(format!("OpenAI API error: {}", response.status()));
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
impl TranslationEngine for OpenAiEngine {
    fn name(&self) -> &'static str {
        "openai"
    }

    async fn translate(&self, input: &TranslationInput, tx: UnboundedSender<TranslationChunk>) {
        if let Err(err) = self.stream_translation(input, &tx).await {
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
        if data == "[DONE]" {
            return tx
                .send(TranslationChunk {
                    text: String::new(),
                    done: true,
                })
                .is_ok();
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

fn system_prompt(is_html: bool) -> String {
    let base = "あなたはプロの翻訳者です。与えられたテキストを自然な日本語に全文翻訳してください。\
        要約・省略・意訳による情報の削減は禁止です。原文にある情報はすべて訳文に含めてください。\
        出力は訳文のみとし、前置きや説明を含めないでください。";

    if is_html {
        format!(
            "{base} 入力はHTMLです。タグ構造(見出し・リスト・コードブロック・太字・リンク等)は変更せず、\
            タグ内のテキストノードのみを翻訳してください。<code>と<pre>の中身は翻訳せず原文のまま保持してください。\
            出力もHTMLのみとし、Markdownのコードフェンスや説明文を付けないでください。"
        )
    } else {
        base.to_string()
    }
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
