use crate::error::ApiError;
use crate::openai_compat::{ChatCompletionChunk, OpenAiStreamConverter};
use crate::types::StreamEvent;

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    converter: Option<OpenAiStreamConverter>,
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            converter: None,
        }
    }

    /// Push raw bytes and return parsed StreamEvents (via OpenAI chunk conversion).
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<StreamEvent>, ApiError> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some(frame) = self.next_frame() {
            if let Some(payload) = extract_data_payload(&frame) {
                let converter = self
                    .converter
                    .get_or_insert_with(OpenAiStreamConverter::new);
                let chunk: ChatCompletionChunk =
                    serde_json::from_str(&payload).map_err(ApiError::from)?;
                events.extend(converter.process_chunk(chunk));
            }
        }

        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Vec<StreamEvent>, ApiError> {
        if self.buffer.is_empty() {
            return Ok(Vec::new());
        }

        let trailing = std::mem::take(&mut self.buffer);
        let frame = String::from_utf8_lossy(&trailing);
        if let Some(payload) = extract_data_payload(&frame) {
            let converter = self
                .converter
                .get_or_insert_with(OpenAiStreamConverter::new);
            let chunk: ChatCompletionChunk =
                serde_json::from_str(&payload).map_err(ApiError::from)?;
            Ok(converter.process_chunk(chunk))
        } else {
            Ok(Vec::new())
        }
    }

    fn next_frame(&mut self) -> Option<String> {
        let separator = self
            .buffer
            .windows(2)
            .position(|window| window == b"\n\n")
            .map(|position| (position, 2))
            .or_else(|| {
                self.buffer
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|position| (position, 4))
            })?;

        let (position, separator_len) = separator;
        let frame = self
            .buffer
            .drain(..position + separator_len)
            .collect::<Vec<_>>();
        let frame_len = frame.len().saturating_sub(separator_len);
        Some(String::from_utf8_lossy(&frame[..frame_len]).into_owned())
    }
}

/// Extract the data payload from an SSE frame, returning None for pings, [DONE], and empty frames.
fn extract_data_payload(frame: &str) -> Option<String> {
    let trimmed = frame.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut data_lines = Vec::new();

    for line in trimmed.lines() {
        if line.starts_with(':') {
            continue;
        }
        if let Some(name) = line.strip_prefix("event:") {
            let name = name.trim();
            if name == "ping" {
                return None;
            }
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim_start());
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let payload = data_lines.join("\n");
    if payload == "[DONE]" {
        return None;
    }

    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::{extract_data_payload, SseParser};
    use crate::types::{ContentBlockDelta, StreamEvent};

    #[test]
    fn extracts_data_payload() {
        let frame = "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}";
        let payload = extract_data_payload(frame);
        assert!(payload.is_some());
        assert!(payload.unwrap().contains("chatcmpl-1"));
    }

    #[test]
    fn ignores_done_marker() {
        assert_eq!(extract_data_payload("data: [DONE]"), None);
    }

    #[test]
    fn ignores_ping() {
        assert_eq!(extract_data_payload("event: ping\ndata: {}"), None);
    }

    #[test]
    fn parses_chunked_openai_stream() {
        let mut parser = SseParser::new();
        // First chunk: role assignment + text start
        let first = br#"data: {"id":"c1","model":"m","choices":[{"delta":{"role":"assistant","content":"Hel"},"finish_reason":null}]}"#;
        let first_with_sep = [&first[..], b"\n\n"].concat();

        let events = parser.push(&first_with_sep).expect("first chunk");
        // Should get MessageStart + ContentBlockStart + ContentBlockDelta
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::MessageStart(_))));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentBlockStart(_))));

        // Second chunk: more text
        let second = b"data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n";
        let events = parser.push(second).expect("second chunk");
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentBlockDelta(d) if matches!(&d.delta, ContentBlockDelta::TextDelta { text } if text == "lo"))));

        // Finish chunk
        let finish = b"data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n";
        let events = parser.push(finish).expect("finish chunk");
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::MessageStop(_))));
    }

    #[test]
    fn handles_done_in_stream() {
        let mut parser = SseParser::new();
        let payload = b"data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Hi\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
        let events = parser.push(payload).expect("stream");
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::MessageStop(_))));
    }
}
