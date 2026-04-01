use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use api::{
    AnthropicClient, ApiError, ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStartEvent,
    InputContentBlock, InputMessage, MessageDeltaEvent, MessageRequest, OutputContentBlock,
    StreamEvent, ToolChoice, ToolDefinition,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::test]
async fn send_message_posts_json_and_parses_response() {
    let state = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let body = concat!(
        "{",
        "\"id\":\"chatcmpl-test\",",
        "\"object\":\"chat.completion\",",
        "\"model\":\"anthropic/claude-sonnet-4\",",
        "\"choices\":[{",
        "\"message\":{\"role\":\"assistant\",\"content\":\"Hello from Claude\"},",
        "\"finish_reason\":\"stop\"",
        "}],",
        "\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":4}",
        "}"
    );
    let server = spawn_server(
        state.clone(),
        vec![http_response("200 OK", "application/json", body)],
    )
    .await;

    let client = AnthropicClient::new("test-key")
        .with_auth_token(Some("proxy-token".to_string()))
        .with_base_url(server.base_url());
    let response = client
        .send_message(&sample_request(false))
        .await
        .expect("request should succeed");

    assert_eq!(response.id, "chatcmpl-test");
    assert_eq!(response.total_tokens(), 16);
    assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(
        response.content,
        vec![OutputContentBlock::Text {
            text: "Hello from Claude".to_string(),
        }]
    );

    let captured = state.lock().await;
    let request = captured.first().expect("server should capture request");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    // Auth should use Bearer for OpenRouter
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer proxy-token")
    );
    let body: serde_json::Value =
        serde_json::from_str(&request.body).expect("request body should be json");
    assert_eq!(
        body.get("model").and_then(serde_json::Value::as_str),
        Some("test-model")
    );
    // OpenAI format uses stream: false (or omits it)
    assert!(
        body.get("stream").is_none()
            || body.get("stream") == Some(&serde_json::Value::Bool(false))
    );
    assert_eq!(body["tools"][0]["function"]["name"], json!("get_weather"));
    assert_eq!(body["tool_choice"], json!("auto"));
}

#[tokio::test]
async fn stream_message_parses_sse_events_with_tool_use() {
    let state = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-stream\",\"model\":\"test-model\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-stream\",\"model\":\"test-model\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_123\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-stream\",\"model\":\"test-model\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\\\"Paris\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-stream\",\"model\":\"test-model\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":1}}\n\n",
        "data: [DONE]\n\n"
    );
    let server = spawn_server(
        state.clone(),
        vec![http_response_with_headers(
            "200 OK",
            "text/event-stream",
            sse,
            &[("x-request-id", "req_stream_456")],
        )],
    )
    .await;

    let client = AnthropicClient::new("test-key")
        .with_auth_token(Some("proxy-token".to_string()))
        .with_base_url(server.base_url());
    let mut stream = client
        .stream_message(&sample_request(false))
        .await
        .expect("stream should start");

    assert_eq!(stream.request_id(), Some("req_stream_456"));

    let mut events = Vec::new();
    while let Some(event) = stream
        .next_event()
        .await
        .expect("stream event should parse")
    {
        events.push(event);
    }

    // Expect: MessageStart, ContentBlockStart(Text), ContentBlockStop(text),
    //         ContentBlockStart(ToolUse), ContentBlockDelta(InputJsonDelta),
    //         ContentBlockStop, MessageDelta, MessageStop
    assert!(matches!(events[0], StreamEvent::MessageStart(_)));

    // Find the tool use content block start
    let tool_start = events.iter().find(|e| {
        matches!(
            e,
            StreamEvent::ContentBlockStart(ContentBlockStartEvent {
                content_block: OutputContentBlock::ToolUse { .. },
                ..
            })
        )
    });
    assert!(tool_start.is_some(), "should have tool_use content block");

    // Check tool name
    match tool_start.unwrap() {
        StreamEvent::ContentBlockStart(ContentBlockStartEvent {
            content_block: OutputContentBlock::ToolUse { name, .. },
            ..
        }) => {
            assert_eq!(name, "get_weather");
        }
        other => panic!("expected tool_use block, got {other:?}"),
    }

    // Should have InputJsonDelta
    assert!(events.iter().any(|e| matches!(
        e,
        StreamEvent::ContentBlockDelta(ContentBlockDeltaEvent {
            delta: ContentBlockDelta::InputJsonDelta { .. },
            ..
        })
    )));

    assert!(events
        .iter()
        .any(|e| matches!(e, StreamEvent::MessageDelta(MessageDeltaEvent { .. }))));
    assert!(events
        .iter()
        .any(|e| matches!(e, StreamEvent::MessageStop(_))));

    let captured = state.lock().await;
    let request = captured.first().expect("server should capture request");
    assert!(request.body.contains("\"stream\":true"));
}

#[tokio::test]
async fn retries_retryable_failures_before_succeeding() {
    let state = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let server = spawn_server(
        state.clone(),
        vec![
            http_response(
                "429 Too Many Requests",
                "application/json",
                "{\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow down\"}}",
            ),
            http_response(
                "200 OK",
                "application/json",
                "{\"id\":\"chatcmpl-retry\",\"model\":\"test\",\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"Recovered\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}",
            ),
        ],
    )
    .await;

    let client = AnthropicClient::new("test-key")
        .with_base_url(server.base_url())
        .with_retry_policy(2, Duration::from_millis(1), Duration::from_millis(2));

    let response = client
        .send_message(&sample_request(false))
        .await
        .expect("retry should eventually succeed");

    assert_eq!(response.total_tokens(), 5);
    assert_eq!(state.lock().await.len(), 2);
}

#[tokio::test]
async fn surfaces_retry_exhaustion_for_persistent_retryable_errors() {
    let state = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let server = spawn_server(
        state.clone(),
        vec![
            http_response(
                "503 Service Unavailable",
                "application/json",
                "{\"error\":{\"type\":\"overloaded_error\",\"message\":\"busy\"}}",
            ),
            http_response(
                "503 Service Unavailable",
                "application/json",
                "{\"error\":{\"type\":\"overloaded_error\",\"message\":\"still busy\"}}",
            ),
        ],
    )
    .await;

    let client = AnthropicClient::new("test-key")
        .with_base_url(server.base_url())
        .with_retry_policy(1, Duration::from_millis(1), Duration::from_millis(2));

    let error = client
        .send_message(&sample_request(false))
        .await
        .expect_err("persistent 503 should fail");

    match error {
        ApiError::RetriesExhausted {
            attempts,
            last_error,
        } => {
            assert_eq!(attempts, 2);
            assert!(matches!(
                *last_error,
                ApiError::Api {
                    status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
                    retryable: true,
                    ..
                }
            ));
        }
        other => panic!("expected retries exhausted, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires OPENROUTER_API_KEY and network access"]
async fn live_stream_smoke_test() {
    let client = AnthropicClient::from_env().expect("OPENROUTER_API_KEY must be set");
    let mut stream = client
        .stream_message(&MessageRequest {
            model: std::env::var("MODEL")
                .unwrap_or_else(|_| "anthropic/claude-sonnet-4".to_string()),
            max_tokens: 32,
            messages: vec![InputMessage::user_text(
                "Reply with exactly: hello from rust",
            )],
            system: None,
            tools: None,
            tool_choice: None,
            stream: false,
        })
        .await
        .expect("live stream should start");

    while let Some(_event) = stream
        .next_event()
        .await
        .expect("live stream should yield events")
    {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

struct TestServer {
    base_url: String,
    join_handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    fn base_url(&self) -> String {
        self.base_url.clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.join_handle.abort();
    }
}

async fn spawn_server(
    state: Arc<Mutex<Vec<CapturedRequest>>>,
    responses: Vec<String>,
) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have local addr");
    let join_handle = tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.expect("server should accept");
            let mut buffer = Vec::new();
            let mut header_end = None;

            loop {
                let mut chunk = [0_u8; 1024];
                let read = socket
                    .read(&mut chunk)
                    .await
                    .expect("request read should succeed");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(position) = find_header_end(&buffer) {
                    header_end = Some(position);
                    break;
                }
            }

            let header_end = header_end.expect("request should include headers");
            let (header_bytes, remaining) = buffer.split_at(header_end);
            let header_text =
                String::from_utf8(header_bytes.to_vec()).expect("headers should be utf8");
            let mut lines = header_text.split("\r\n");
            let request_line = lines.next().expect("request line should exist");
            let mut parts = request_line.split_whitespace();
            let method = parts.next().expect("method should exist").to_string();
            let path = parts.next().expect("path should exist").to_string();
            let mut headers = HashMap::new();
            let mut content_length = 0_usize;
            for line in lines {
                if line.is_empty() {
                    continue;
                }
                let (name, value) = line.split_once(':').expect("header should have colon");
                let value = value.trim().to_string();
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.parse().expect("content length should parse");
                }
                headers.insert(name.to_ascii_lowercase(), value);
            }

            let mut body = remaining[4..].to_vec();
            while body.len() < content_length {
                let mut chunk = vec![0_u8; content_length - body.len()];
                let read = socket
                    .read(&mut chunk)
                    .await
                    .expect("body read should succeed");
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..read]);
            }

            state.lock().await.push(CapturedRequest {
                method,
                path,
                headers,
                body: String::from_utf8(body).expect("body should be utf8"),
            });

            socket
                .write_all(response.as_bytes())
                .await
                .expect("response write should succeed");
        }
    });

    TestServer {
        base_url: format!("http://{address}"),
        join_handle,
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn http_response(status: &str, content_type: &str, body: &str) -> String {
    http_response_with_headers(status, content_type, body, &[])
}

fn http_response_with_headers(
    status: &str,
    content_type: &str,
    body: &str,
    headers: &[(&str, &str)],
) -> String {
    let mut extra_headers = String::new();
    for (name, value) in headers {
        use std::fmt::Write as _;
        write!(&mut extra_headers, "{name}: {value}\r\n").expect("header write should succeed");
    }
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\n{extra_headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn sample_request(stream: bool) -> MessageRequest {
    MessageRequest {
        model: "test-model".to_string(),
        max_tokens: 64,
        messages: vec![InputMessage {
            role: "user".to_string(),
            content: vec![
                InputContentBlock::Text {
                    text: "Say hello".to_string(),
                },
                InputContentBlock::ToolResult {
                    tool_use_id: "toolu_prev".to_string(),
                    content: vec![api::ToolResultContentBlock::Json {
                        value: json!({"forecast": "sunny"}),
                    }],
                    is_error: false,
                },
            ],
        }],
        system: Some("Use tools when needed".to_string()),
        tools: Some(vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: Some("Fetches the weather".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"]
            }),
        }]),
        tool_choice: Some(ToolChoice::Auto),
        stream,
    }
}
