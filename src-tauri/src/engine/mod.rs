pub mod mock;

use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, serde::Serialize)]
pub struct TranslationChunk {
    pub text: String,
    pub done: bool,
}

#[async_trait::async_trait]
pub trait TranslationEngine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn translate(&self, input: &str, tx: UnboundedSender<TranslationChunk>);
}
