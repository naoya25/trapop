pub mod mock;
pub mod openai;

use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, serde::Serialize)]
pub struct TranslationChunk {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub enum TranslationInput {
    PlainText(String),
    Html(String),
}

impl TranslationInput {
    pub fn from_capture(plain_text: Option<String>, html: Option<String>) -> Option<Self> {
        if let Some(html) = html.filter(|h| !h.trim().is_empty()) {
            return Some(Self::Html(html));
        }
        plain_text
            .filter(|p| !p.trim().is_empty())
            .map(Self::PlainText)
    }

    pub fn body(&self) -> &str {
        match self {
            Self::PlainText(s) => s,
            Self::Html(s) => s,
        }
    }

    pub fn is_html(&self) -> bool {
        matches!(self, Self::Html(_))
    }
}

#[async_trait::async_trait]
pub trait TranslationEngine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn translate(&self, input: &TranslationInput, tx: UnboundedSender<TranslationChunk>);
}

pub fn resolve() -> Box<dyn TranslationEngine> {
    match openai::OpenAiEngine::from_environment() {
        Ok(engine) => Box::new(engine),
        Err(_) => Box::new(mock::MockEngine),
    }
}
