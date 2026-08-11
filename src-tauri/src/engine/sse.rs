// SSE のバイト列をイベント単位に切り出す。チャンク境界でマルチバイト文字が
// 分断されても、イベント区切り(\n\n)は ASCII なので decode は必ず完全なイベントに対して行える。
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
        let pos = self
            .bytes
            .windows(2)
            .position(|window| window == b"\n\n")?;
        let event_bytes: Vec<u8> = self.bytes.drain(..pos + 2).collect();
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

    #[test]
    fn sse_buffer_yields_events_in_order() {
        let mut buffer = SseBuffer::new();
        buffer.push(b"data: one\n\ndata: two\n\n");
        assert!(buffer.next_event().unwrap().contains("one"));
        assert!(buffer.next_event().unwrap().contains("two"));
        assert!(buffer.next_event().is_none());
    }
}
