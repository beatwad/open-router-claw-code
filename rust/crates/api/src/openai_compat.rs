//! OpenAI-compatible wire format types and conversion to/from internal types.
//! Used for OpenRouter API integration.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{
    InputContentBlock, InputMessage, MessageRequest, MessageResponse, OutputContentBlock,
    StreamEvent, ToolChoice, ToolDefinition, ToolResultContentBlock, Usage,
};

// ── Request types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ChatToolChoiceValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
}

#[derive(Debug, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatFunctionDef,
}

#[derive(Debug, Serialize)]
pub struct ChatFunctionDef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ChatToolChoiceValue {
    String(String),
    Named {
        #[serde(rename = "type")]
        kind: String,
        function: ChatToolChoiceFunction,
    },
}

#[derive(Debug, Serialize)]
pub struct ChatToolChoiceFunction {
    pub name: String,
}

// ── Response types (non-streaming) ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    #[serde(default)]
    pub model: String,
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: ChatChoiceMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoiceMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

// ── Streaming chunk types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatCompletionChunk {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    pub choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
pub struct ChunkChoice {
    pub delta: ChunkDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChunkDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChunkToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct ChunkToolCall {
    pub index: u32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<ChunkFunction>,
}

#[derive(Debug, Deserialize)]
pub struct ChunkFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

// ── Request conversion ─────────────────────────────────────────────────────

pub fn to_chat_request(request: &MessageRequest) -> ChatCompletionRequest {
    let mut messages = Vec::new();

    // System prompt → system message
    if let Some(system) = &request.system {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(system.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Convert each InputMessage
    for msg in &request.messages {
        convert_input_message(msg, &mut messages);
    }

    let tools = request.tools.as_ref().map(|tools| {
        tools
            .iter()
            .map(|tool| convert_tool_def(tool))
            .collect()
    });

    let tool_choice = request.tool_choice.as_ref().map(convert_tool_choice);

    let stream_options = request.stream.then_some(StreamOptions {
        include_usage: true,
    });

    ChatCompletionRequest {
        model: request.model.clone(),
        messages,
        max_tokens: Some(request.max_tokens),
        stream: request.stream,
        tools,
        tool_choice,
        stream_options,
    }
}

fn convert_input_message(msg: &InputMessage, out: &mut Vec<ChatMessage>) {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();

    for block in &msg.content {
        match block {
            InputContentBlock::Text { text } => {
                text_parts.push(text.clone());
            }
            InputContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(ChatToolCall {
                    id: id.clone(),
                    kind: "function".to_string(),
                    function: ChatFunctionCall {
                        name: name.clone(),
                        arguments: serde_json::to_string(input).unwrap_or_default(),
                    },
                });
            }
            InputContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let text = content
                    .iter()
                    .map(|block| match block {
                        ToolResultContentBlock::Text { text } => text.clone(),
                        ToolResultContentBlock::Json { value } => {
                            serde_json::to_string(value).unwrap_or_default()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = if *is_error {
                    format!("[ERROR] {text}")
                } else {
                    text
                };
                tool_results.push((tool_use_id.clone(), text));
            }
        }
    }

    // Emit assistant message with text + tool_calls
    if msg.role == "assistant" {
        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };
        let tc = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        };
        out.push(ChatMessage {
            role: "assistant".to_string(),
            content,
            tool_calls: tc,
            tool_call_id: None,
        });
    } else {
        // User message: emit text as user message, tool results as tool messages
        if !text_parts.is_empty() {
            out.push(ChatMessage {
                role: "user".to_string(),
                content: Some(text_parts.join("\n")),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        for (tool_call_id, text) in tool_results {
            out.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(text),
                tool_calls: None,
                tool_call_id: Some(tool_call_id),
            });
        }
    }
}

fn convert_tool_def(tool: &ToolDefinition) -> ChatTool {
    ChatTool {
        kind: "function".to_string(),
        function: ChatFunctionDef {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        },
    }
}

fn convert_tool_choice(choice: &ToolChoice) -> ChatToolChoiceValue {
    match choice {
        ToolChoice::Auto => ChatToolChoiceValue::String("auto".to_string()),
        ToolChoice::Any => ChatToolChoiceValue::String("required".to_string()),
        ToolChoice::Tool { name } => ChatToolChoiceValue::Named {
            kind: "function".to_string(),
            function: ChatToolChoiceFunction { name: name.clone() },
        },
    }
}

// ── Response conversion (non-streaming) ────────────────────────────────────

pub fn from_chat_response(response: ChatCompletionResponse) -> MessageResponse {
    let choice = response.choices.into_iter().next();
    let finish_reason = choice.as_ref().and_then(|c| c.finish_reason.clone());
    let stop_reason = finish_reason.map(|r| map_finish_reason(&r));

    let mut content = Vec::new();
    if let Some(choice) = choice {
        if let Some(text) = choice.message.content {
            if !text.is_empty() {
                content.push(OutputContentBlock::Text { text });
            }
        }
        if let Some(tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input: Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Object(
                        serde_json::Map::new(),
                    ));
                content.push(OutputContentBlock::ToolUse {
                    id: tc.id,
                    name: tc.function.name,
                    input,
                });
            }
        }
    }

    let usage = response.usage.map_or_else(
        || Usage {
            input_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: 0,
        },
        |u| Usage {
            input_tokens: u.prompt_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: u.completion_tokens,
        },
    );

    MessageResponse {
        id: response.id,
        kind: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: response.model,
        stop_reason,
        stop_sequence: None,
        usage,
        request_id: None,
    }
}

// ── Streaming conversion ───────────────────────────────────────────────────

/// Stateful converter that turns OpenAI streaming chunks into internal StreamEvents.
#[derive(Debug, Default)]
pub struct OpenAiStreamConverter {
    message_started: bool,
    block_index: i32,
    has_text_block: bool,
    message_id: String,
    message_model: String,
}

impl OpenAiStreamConverter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a single OpenAI chunk and produce zero or more StreamEvents.
    pub fn process_chunk(&mut self, chunk: ChatCompletionChunk) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        if !self.message_id.is_empty() && chunk.id != self.message_id {
            // Different message ID — shouldn't happen in normal flow
        }
        if !chunk.id.is_empty() {
            self.message_id.clone_from(&chunk.id);
        }
        if !chunk.model.is_empty() {
            self.message_model.clone_from(&chunk.model);
        }

        let Some(choice) = chunk.choices.into_iter().next() else {
            // Usage-only chunk (no choices) at end of stream
            if let Some(usage) = chunk.usage {
                self.emit_usage(&mut events, &usage, None);
            }
            return events;
        };

        // Ensure MessageStart is emitted
        if !self.message_started {
            self.message_started = true;
            events.push(StreamEvent::MessageStart(
                crate::types::MessageStartEvent {
                    message: MessageResponse {
                        id: self.message_id.clone(),
                        kind: "message".to_string(),
                        role: "assistant".to_string(),
                        content: Vec::new(),
                        model: self.message_model.clone(),
                        stop_reason: None,
                        stop_sequence: None,
                        usage: Usage {
                            input_tokens: 0,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                            output_tokens: 0,
                        },
                        request_id: None,
                    },
                },
            ));
        }

        let delta = &choice.delta;

        // Handle text content
        if let Some(text) = &delta.content {
            if !self.has_text_block {
                self.has_text_block = true;
                self.block_index += 1;
                events.push(StreamEvent::ContentBlockStart(
                    crate::types::ContentBlockStartEvent {
                        index: self.block_index as u32,
                        content_block: OutputContentBlock::Text {
                            text: String::new(),
                        },
                    },
                ));
            }
            if !text.is_empty() {
                events.push(StreamEvent::ContentBlockDelta(
                    crate::types::ContentBlockDeltaEvent {
                        index: self.block_index as u32,
                        delta: crate::types::ContentBlockDelta::TextDelta {
                            text: text.clone(),
                        },
                    },
                ));
            }
        }

        // Handle tool calls
        if let Some(tool_calls) = &delta.tool_calls {
            for tc in tool_calls {
                if let Some(id) = &tc.id {
                    // New tool call starting — close text block if open
                    if self.has_text_block {
                        events.push(StreamEvent::ContentBlockStop(
                            crate::types::ContentBlockStopEvent {
                                index: self.block_index as u32,
                            },
                        ));
                        self.has_text_block = false;
                    }
                    self.block_index += 1;
                    let name = tc
                        .function
                        .as_ref()
                        .and_then(|f| f.name.clone())
                        .unwrap_or_default();
                    events.push(StreamEvent::ContentBlockStart(
                        crate::types::ContentBlockStartEvent {
                            index: self.block_index as u32,
                            content_block: OutputContentBlock::ToolUse {
                                id: id.clone(),
                                name,
                                input: Value::Object(serde_json::Map::new()),
                            },
                        },
                    ));
                }
                if let Some(func) = &tc.function {
                    if let Some(args) = &func.arguments {
                        if !args.is_empty() {
                            events.push(StreamEvent::ContentBlockDelta(
                                crate::types::ContentBlockDeltaEvent {
                                    index: self.block_index as u32,
                                    delta: crate::types::ContentBlockDelta::InputJsonDelta {
                                        partial_json: args.clone(),
                                    },
                                },
                            ));
                        }
                    }
                }
            }
        }

        // Handle finish
        if let Some(finish_reason) = &choice.finish_reason {
            // Close any open content block
            if self.block_index >= 0 {
                events.push(StreamEvent::ContentBlockStop(
                    crate::types::ContentBlockStopEvent {
                        index: self.block_index as u32,
                    },
                ));
            }

            let stop_reason = map_finish_reason(finish_reason);
            let usage = chunk.usage.unwrap_or(ChatUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
            });
            self.emit_usage(&mut events, &usage, Some(stop_reason));
            events.push(StreamEvent::MessageStop(
                crate::types::MessageStopEvent {},
            ));
        }

        events
    }

    fn emit_usage(&self, events: &mut Vec<StreamEvent>, usage: &ChatUsage, stop_reason: Option<String>) {
        events.push(StreamEvent::MessageDelta(
            crate::types::MessageDeltaEvent {
                delta: crate::types::MessageDelta {
                    stop_reason,
                    stop_sequence: None,
                },
                usage: Usage {
                    input_tokens: usage.prompt_tokens,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                    output_tokens: usage.completion_tokens,
                },
            },
        ));
    }
}

fn map_finish_reason(reason: &str) -> String {
    match reason {
        "stop" => "end_turn".to_string(),
        "tool_calls" => "tool_use".to_string(),
        "length" => "max_tokens".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InputContentBlock, InputMessage, ToolChoice, ToolDefinition};

    #[test]
    fn converts_simple_request() {
        let request = MessageRequest {
            model: "openai/gpt-4o".to_string(),
            max_tokens: 4096,
            messages: vec![InputMessage::user_text("Hello")],
            system: Some("You are helpful.".to_string()),
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let chat_req = to_chat_request(&request);
        assert_eq!(chat_req.messages.len(), 2);
        assert_eq!(chat_req.messages[0].role, "system");
        assert_eq!(
            chat_req.messages[0].content.as_deref(),
            Some("You are helpful.")
        );
        assert_eq!(chat_req.messages[1].role, "user");
        assert_eq!(chat_req.messages[1].content.as_deref(), Some("Hello"));
    }

    #[test]
    fn converts_tool_result_to_tool_message() {
        let request = MessageRequest {
            model: "test".to_string(),
            max_tokens: 100,
            messages: vec![InputMessage::user_tool_result("call_1", "result text", false)],
            system: None,
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let chat_req = to_chat_request(&request);
        assert_eq!(chat_req.messages.len(), 1);
        assert_eq!(chat_req.messages[0].role, "tool");
        assert_eq!(
            chat_req.messages[0].tool_call_id.as_deref(),
            Some("call_1")
        );
    }

    #[test]
    fn converts_assistant_tool_use() {
        let request = MessageRequest {
            model: "test".to_string(),
            max_tokens: 100,
            messages: vec![InputMessage {
                role: "assistant".to_string(),
                content: vec![InputContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    input: serde_json::json!({"path": "/tmp/test"}),
                }],
            }],
            system: None,
            tools: None,
            tool_choice: None,
            stream: false,
        };

        let chat_req = to_chat_request(&request);
        assert_eq!(chat_req.messages.len(), 1);
        assert_eq!(chat_req.messages[0].role, "assistant");
        assert!(chat_req.messages[0].content.is_none());
        let tc = chat_req.messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tc[0].function.name, "read_file");
    }

    #[test]
    fn converts_tool_definitions() {
        let request = MessageRequest {
            model: "test".to_string(),
            max_tokens: 100,
            messages: vec![],
            system: None,
            tools: Some(vec![ToolDefinition {
                name: "read_file".to_string(),
                description: Some("Read a file".to_string()),
                input_schema: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            }]),
            tool_choice: Some(ToolChoice::Auto),
            stream: false,
        };

        let chat_req = to_chat_request(&request);
        let tools = chat_req.tools.unwrap();
        assert_eq!(tools[0].kind, "function");
        assert_eq!(tools[0].function.name, "read_file");
        assert_eq!(
            chat_req.tool_choice.map(|tc| match tc {
                ChatToolChoiceValue::String(s) => s,
                _ => panic!("expected string"),
            }),
            Some("auto".to_string())
        );
    }

    #[test]
    fn converts_non_streaming_response() {
        let response = ChatCompletionResponse {
            id: "chatcmpl-123".to_string(),
            model: "openai/gpt-4o".to_string(),
            choices: vec![ChatChoice {
                message: ChatChoiceMessage {
                    content: Some("Hello!".to_string()),
                    tool_calls: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(ChatUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            }),
        };

        let msg = from_chat_response(response);
        assert_eq!(msg.id, "chatcmpl-123");
        assert_eq!(msg.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(msg.content.len(), 1);
        assert_eq!(msg.usage.input_tokens, 10);
        assert_eq!(msg.usage.output_tokens, 5);
    }

    #[test]
    fn stream_converter_produces_events_for_text() {
        let mut converter = OpenAiStreamConverter::new();

        let chunk = ChatCompletionChunk {
            id: "chatcmpl-1".to_string(),
            model: "test-model".to_string(),
            choices: vec![ChunkChoice {
                delta: ChunkDelta {
                    role: Some("assistant".to_string()),
                    content: Some("Hello".to_string()),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        };

        let events = converter.process_chunk(chunk);
        // Should produce: MessageStart, ContentBlockStart(Text), ContentBlockDelta(TextDelta)
        assert!(events.len() >= 3);
        assert!(matches!(events[0], StreamEvent::MessageStart(_)));
        assert!(matches!(events[1], StreamEvent::ContentBlockStart(_)));
        assert!(matches!(events[2], StreamEvent::ContentBlockDelta(_)));
    }

    #[test]
    fn stream_converter_handles_finish() {
        let mut converter = OpenAiStreamConverter::new();

        // First chunk with text
        let _ = converter.process_chunk(ChatCompletionChunk {
            id: "c1".to_string(),
            model: "m".to_string(),
            choices: vec![ChunkChoice {
                delta: ChunkDelta {
                    role: None,
                    content: Some("Hi".to_string()),
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        });

        // Finish chunk
        let events = converter.process_chunk(ChatCompletionChunk {
            id: "c1".to_string(),
            model: "m".to_string(),
            choices: vec![ChunkChoice {
                delta: ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(ChatUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            }),
        });

        // Should produce: ContentBlockStop, MessageDelta, MessageStop
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ContentBlockStop(_))));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::MessageDelta(_))));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::MessageStop(_))));
    }

    #[test]
    fn maps_finish_reasons() {
        assert_eq!(map_finish_reason("stop"), "end_turn");
        assert_eq!(map_finish_reason("tool_calls"), "tool_use");
        assert_eq!(map_finish_reason("length"), "max_tokens");
        assert_eq!(map_finish_reason("other"), "other");
    }
}
