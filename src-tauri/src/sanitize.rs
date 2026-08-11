use ammonia::Builder;

pub fn sanitize_translation_html(html: &str) -> String {
    Builder::default().clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_script_and_event_handlers() {
        let dirty = r#"<p onclick="alert(1)">hello</p><script>alert(1)</script><iframe src="evil"></iframe>"#;
        let clean = sanitize_translation_html(dirty);

        assert!(!clean.contains("<script"));
        assert!(!clean.contains("<iframe"));
        assert!(!clean.contains("onclick"));
        assert!(clean.contains("hello"));
    }

    #[test]
    fn preserves_lists_code_and_bold_structure() {
        let dirty = "<ul><li>one</li><li>two</li></ul><p>see <code>foo()</code> and <strong>bold</strong></p>";
        let clean = sanitize_translation_html(dirty);

        assert!(clean.contains("<ul>"));
        assert!(clean.contains("<li>one</li>"));
        assert!(clean.contains("<code>foo()</code>"));
        assert!(clean.contains("<strong>bold</strong>"));
    }
}
