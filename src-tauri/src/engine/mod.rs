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
        target: Option<&str>,
        tx: UnboundedSender<TranslationChunk>,
    );
}

// 設定画面の placeholder 表示にも使うため、テンプレートとして公開する。
// {lang_a}/{lang_b} は実行時に言語名へ置換される
pub const DEFAULT_PROMPT_TEMPLATE: &str = "あなたはプロの翻訳者です。入力テキストが{lang_a}であれば{lang_b}に、\
{lang_b}であれば{lang_a}に全文翻訳してください。どちらの言語でもない場合は{lang_a}に翻訳してください。\
要約・省略・意訳による情報の削減は禁止です。原文にある情報はすべて訳文に含めてください。\
出力は訳文のみとし、前置きや説明を含めないでください。";

// target=None(自動判定)のときだけ後置する。入力に複数言語が混在すると
// LLM が誤判定して意図しない方向に訳す(例: 英語主体の文に日本語が混ざり
// 日本語入力と誤認して英語へ訳す)ため、多数決ルールで自動判定を補強する
const AUTO_MIXED_LANGUAGE_RULE: &str =
    " 入力に複数の言語が混在する場合は、文字量の多い主要な言語を入力言語とみなし、\
必ず全文をどちらか一方の言語に統一してください。";

// custom_template はユーザー編集の翻訳ポリシー部分。HTML/Markdown の書式保持
// ルールは編集対象にしない(消せると描画が壊れるため、常にここで後置する)。
// target が Some のときは、その言語での出力を明示指定する(ソース言語の自動判定を
// 使わない)。custom_template 使用時も同様にこの指定を後置して両立させる
pub fn system_prompt(
    lang_a: &str,
    lang_b: &str,
    is_html: bool,
    custom_template: Option<&str>,
    target: Option<&str>,
) -> String {
    let template = custom_template
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(DEFAULT_PROMPT_TEMPLATE);
    let mut base = template
        .replace("{lang_a}", lang_a)
        .replace("{lang_b}", lang_b);

    match target {
        Some(target_lang) => {
            base.push_str(&format!(
                " 必ず{target_lang}で出力してください。入力の一部が既に{target_lang}でも、\
そのまま残さず自然な{target_lang}に統一してください。"
            ));
        }
        None => base.push_str(AUTO_MIXED_LANGUAGE_RULE),
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_prompt_replaces_language_placeholders() {
        let prompt = system_prompt(
            "日本語",
            "英語",
            false,
            Some("カジュアルに。{lang_a}⇔{lang_b}"),
            None,
        );
        assert!(prompt.starts_with("カジュアルに。日本語⇔英語"));
    }

    #[test]
    fn custom_prompt_keeps_format_rules_appended() {
        let plain = system_prompt("日本語", "英語", false, Some("カジュアルに"), None);
        assert!(plain.contains("Markdown記法"));
        let html = system_prompt("日本語", "英語", true, Some("カジュアルに"), None);
        assert!(html.contains("入力はHTMLです"));
    }

    #[test]
    fn blank_custom_prompt_falls_back_to_default() {
        let with_blank = system_prompt("日本語", "英語", false, Some("   "), None);
        let with_none = system_prompt("日本語", "英語", false, None, None);
        assert_eq!(with_blank, with_none);
        assert!(with_none.contains("あなたはプロの翻訳者です"));
    }

    #[test]
    fn auto_target_adds_mixed_language_majority_rule() {
        let prompt = system_prompt("日本語", "英語", false, None, None);
        assert!(prompt.contains("文字量の多い主要な言語を入力言語とみなし"));
    }

    #[test]
    fn explicit_target_forces_output_language() {
        let prompt = system_prompt("日本語", "英語", false, None, Some("英語"));
        assert!(prompt.contains("必ず英語で出力してください"));
        assert!(!prompt.contains("文字量の多い主要な言語を入力言語とみなし"));
    }

    #[test]
    fn explicit_target_coexists_with_custom_prompt() {
        let prompt = system_prompt(
            "日本語",
            "英語",
            false,
            Some("カジュアルに訳して"),
            Some("英語"),
        );
        assert!(prompt.starts_with("カジュアルに訳して"));
        assert!(prompt.contains("必ず英語で出力してください"));
    }
}

pub struct ResolvedEngine {
    pub engine: Box<dyn TranslationEngine>,
    pub unavailable_reason: Option<String>,
}

pub fn resolve(
    choice: &str,
    model_override: Option<&str>,
    custom_prompt: Option<&str>,
) -> ResolvedEngine {
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
        "openai" => match openai::OpenAiEngine::from_environment(model_override, custom_prompt) {
            Ok(engine) => ResolvedEngine {
                engine: Box::new(engine),
                unavailable_reason: None,
            },
            Err(_) => ResolvedEngine {
                engine: Box::new(mock::MockEngine),
                unavailable_reason: missing_key("OpenAI"),
            },
        },
        "gemini" => match gemini::GeminiEngine::from_environment(model_override, custom_prompt) {
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
            if let Ok(engine) = openai::OpenAiEngine::from_environment(model_override, custom_prompt)
            {
                return ResolvedEngine {
                    engine: Box::new(engine),
                    unavailable_reason: None,
                };
            }
            if let Ok(engine) = gemini::GeminiEngine::from_environment(model_override, custom_prompt)
            {
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
