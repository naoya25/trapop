// SSE のバイト列をイベント単位に切り出す。チャンク境界でマルチバイト文字が
// 分断されても、イベント区切り(空行)は ASCII なので decode は必ず完全なイベントに対して行える。
// 区切りは LF(\n\n) と CRLF(\r\n\r\n) の両方を受ける(Gemini API は CRLF を送ってくる)。
pub struct SseBuffer {
    bytes: Vec<u8>,
}

impl SseBuffer {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
    }

    pub fn next_event(&mut self) -> Option<String> {
        let lf = self.bytes.windows(2).position(|w| w == b"\n\n");
        let crlf = self.bytes.windows(4).position(|w| w == b"\r\n\r\n");
        let (pos, delim_len) = match (lf, crlf) {
            (Some(l), Some(c)) if c < l => (c, 4),
            (Some(l), _) => (l, 2),
            (None, Some(c)) => (c, 4),
            (None, None) => return None,
        };
        let event_bytes: Vec<u8> = self.bytes.drain(..pos + delim_len).collect();
        Some(String::from_utf8_lossy(&event_bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_buffer_handles_multibyte_split_across_chunks() {
        let payload = "data: {\"text\":\"日本語の翻訳\"}\n\n".as_bytes();
        let (first, second) = payload.split_at(payload.len() / 2 + 1);

        let mut buffer = SseBuffer::new();
        buffer.push(first);
        assert!(buffer.next_event().is_none());

        buffer.push(second);
        let event = buffer.next_event().expect("event should be complete");
        assert!(event.contains("日本語の翻訳"));
        assert!(!event.contains('\u{FFFD}'));
    }

    // Gemini API は CRLF 区切りで送ってくる。LF しか見ないと
    // イベントが一切切り出せず「完了なのに本文が空」になる
    #[test]
    fn sse_buffer_handles_crlf_delimited_events() {
        let mut buffer = SseBuffer::new();
        buffer.push(b"data: one\r\n\r\ndata: two\r\n\r\n");
        assert!(buffer.next_event().unwrap().contains("one"));
        assert!(buffer.next_event().unwrap().contains("two"));
        assert!(buffer.next_event().is_none());
    }

    #[test]
    fn sse_buffer_handles_mixed_lf_and_crlf_events() {
        let mut buffer = SseBuffer::new();
        buffer.push(b"data: one\n\ndata: two\r\n\r\ndata: three\n\n");
        assert!(buffer.next_event().unwrap().contains("one"));
        assert!(buffer.next_event().unwrap().contains("two"));
        assert!(buffer.next_event().unwrap().contains("three"));
        assert!(buffer.next_event().is_none());
    }

    #[test]
    fn sse_buffer_yields_events_in_order() {
        let mut buffer = SseBuffer::new();
        buffer.push(b"data: one\n\ndata: two\n\n");
        assert!(buffer.next_event().unwrap().contains("one"));
        assert!(buffer.next_event().unwrap().contains("two"));
        assert!(buffer.next_event().is_none());
    }
}
