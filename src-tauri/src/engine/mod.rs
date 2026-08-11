pub mod gemini;
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
    async fn translate(
        &self,
        input: &TranslationInput,
        lang_a: &str,
        lang_b: &str,
        tx: UnboundedSender<TranslationChunk>,
    );
}

pub fn system_prompt(lang_a: &str, lang_b: &str, is_html: bool) -> String {
    let base = format!(
        "あなたはプロの翻訳者です。入力テキストが{lang_a}であれば{lang_b}に、\
        {lang_b}であれば{lang_a}に全文翻訳してください。どちらの言語でもない場合は{lang_a}に翻訳してください。\
        要約・省略・意訳による情報の削減は禁止です。原文にある情報はすべて訳文に含めてください。\
        出力は訳文のみとし、前置きや説明を含めないでください。"
    );

    if is_html {
        format!(
            "{base} 入力はHTMLです。タグ構造(見出し・リスト・コードブロック・太字・リンク等)は変更せず、\
            タグ内のテキストノードのみを翻訳してください。<code>と<pre>の中身は翻訳せず原文のまま保持してください。\
            出力もHTMLのみとし、Markdownのコードフェンスや説明文を付けないでください。"
        )
    } else {
        format!(
            "{base} 入力がMarkdown記法(見出し・リスト・コードフェンス・強調・リンク等)を含む場合は、\
            記法そのものは変更せず本文のテキストのみを翻訳してください。コードフェンス内のコードは翻訳せず原文のまま保持してください。"
        )
    }
}

pub fn resolve(choice: &str, model_override: Option<&str>) -> Box<dyn TranslationEngine> {
    match choice {
        "mock" => Box::new(mock::MockEngine),
        "openai" => match openai::OpenAiEngine::from_environment(model_override) {
            Ok(engine) => Box::new(engine),
            Err(_) => Box::new(mock::MockEngine),
        },
        "gemini" => match gemini::GeminiEngine::from_environment(model_override) {
            Ok(engine) => Box::new(engine),
            Err(_) => Box::new(mock::MockEngine),
        },
        _ => {
            if let Ok(engine) = openai::OpenAiEngine::from_environment(model_override) {
                return Box::new(engine);
            }
            if let Ok(engine) = gemini::GeminiEngine::from_environment(model_override) {
                return Box::new(engine);
            }
            Box::new(mock::MockEngine)
        }
    }
}
