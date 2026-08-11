pub mod gemini;
pub mod keys;
pub mod mock;
pub mod openai;
pub mod sse;

use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, serde::Serialize)]
pub struct TranslationChunk {
    pub text: String,
    pub done: bool,
    pub error: bool,
}

impl TranslationChunk {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            done: false,
            error: false,
        }
    }

    pub fn done() -> Self {
        Self {
            text: String::new(),
            done: true,
            error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            text: message.into(),
            done: true,
            error: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TranslationInput {
    PlainText(String),
    Html(String),
}

impl TranslationInput {
    pub fn from_paste(plain_text: Option<String>, html: Option<String>) -> Option<Self> {
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
            記法そのものは変更せず本文のテキストのみを翻訳してください。コードフェンス内のコードは翻訳せず原文のまま保持してください。\
            入力はHTMLではありません。<div>・<p>・<span>などのHTMLタグは出力に一切含めないでください。\
            出力はMarkdown記法(または装飾のない平文)のみとしてください。"
        )
    }
}

// API エラーは典型ステータスだけ日本語に写像し、生のレスポンス本文は
// 呼び出し側で stderr に落とす(UI には1行の理由だけ出す)。
pub fn user_facing_api_error(provider: &str, status: u16) -> String {
    match status {
        401 | 403 => format!("{provider} のAPIキーが無効です。設定画面で確認してください。"),
        429 => format!("{provider} がレート制限中です。しばらく待って再試行してください。"),
        500..=599 => format!("{provider} 側で障害が発生しています。しばらく待って再試行してください。"),
        _ => format!("{provider} APIエラー(HTTP {status})。再試行してください。"),
    }
}

pub struct ResolvedEngine {
    pub engine: Box<dyn TranslationEngine>,
    pub unavailable_reason: Option<String>,
}

pub fn resolve(choice: &str, model_override: Option<&str>) -> ResolvedEngine {
    let missing_key = |provider: &str| {
        Some(format!(
            "{provider} のAPIキーが未設定です。設定画面から登録してください。"
        ))
    };

    match choice {
        "mock" => ResolvedEngine {
            engine: Box::new(mock::MockEngine),
            unavailable_reason: None,
        },
        "openai" => match openai::OpenAiEngine::from_environment(model_override) {
            Ok(engine) => ResolvedEngine {
                engine: Box::new(engine),
                unavailable_reason: None,
            },
            Err(_) => ResolvedEngine {
                engine: Box::new(mock::MockEngine),
                unavailable_reason: missing_key("OpenAI"),
            },
        },
        "gemini" => match gemini::GeminiEngine::from_environment(model_override) {
            Ok(engine) => ResolvedEngine {
                engine: Box::new(engine),
                unavailable_reason: None,
            },
            Err(_) => ResolvedEngine {
                engine: Box::new(mock::MockEngine),
                unavailable_reason: missing_key("Gemini"),
            },
        },
        other => {
            // ここに来てよいのは "auto" だけ。エンジン追加時に match 漏れのまま
            // 黙って auto 挙動になるのを開発中に検出する。
            debug_assert!(other == "auto", "unknown engine choice: {other}");
            if let Ok(engine) = openai::OpenAiEngine::from_environment(model_override) {
                return ResolvedEngine {
                    engine: Box::new(engine),
                    unavailable_reason: None,
                };
            }
            if let Ok(engine) = gemini::GeminiEngine::from_environment(model_override) {
                return ResolvedEngine {
                    engine: Box::new(engine),
                    unavailable_reason: None,
                };
            }
            ResolvedEngine {
                engine: Box::new(mock::MockEngine),
                unavailable_reason: missing_key("OpenAI または Gemini"),
            }
        }
    }
}
