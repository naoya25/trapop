use super::{TranslationChunk, TranslationEngine};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const STREAM_INTERVAL_MS: u64 = 100;

pub struct MockEngine;

#[async_trait::async_trait]
impl TranslationEngine for MockEngine {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn translate(&self, input: &str, tx: UnboundedSender<TranslationChunk>) {
        let body = fixed_translation(input);
        for word in body.split_inclusive(' ') {
            if tx
                .send(TranslationChunk {
                    text: word.to_string(),
                    done: false,
                })
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(STREAM_INTERVAL_MS)).await;
        }
        let _ = tx.send(TranslationChunk {
            text: String::new(),
            done: true,
        });
    }
}

fn fixed_translation(input: &str) -> String {
    let preview: String = input.chars().take(24).collect();
    format!(
        "これはモックエンジンによる固定の和訳です。原文の冒頭は「{preview}」でした。 \
         実際の翻訳エンジン(OpenAI)は次のタスクで接続されます。 \
         このストリームは100ミリ秒間隔で単語ごとに流れます。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn streams_chunks_and_terminates_with_done() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let engine = MockEngine;

        engine.translate("hello world", tx).await;

        let mut received_text = String::new();
        let mut saw_done = false;
        while let Some(chunk) = rx.recv().await {
            received_text.push_str(&chunk.text);
            if chunk.done {
                saw_done = true;
            }
        }

        assert!(saw_done, "stream must terminate with a done chunk");
        assert!(!received_text.is_empty(), "stream must yield translated text");
    }
}
