use super::*;
#[test]
fn test_codex_input_strips_signature_fields_and_normalizes_thinking_type() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_123".to_string()),
                    name: "WebFetch".to_string(),
                    input: json!({"url": "https://example.com"}),
                    signature: Some("sig_tool_abc".to_string()),
                }])),
            },
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::Thinking {
                    thinking: "internal".to_string(),
                    signature: Some("sig_thinking_abc".to_string()),
                }])),
            },
        ],
        system: None,
        stream: true,
        tools: None,
        max_tokens: None,
        temperature: None,
        top_p: None,
        top_k: None,
        stop_sequences: None,
    };

    let mapping = ReasoningEffortMapping::default();
    let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

    let input = body
        .get("input")
        .and_then(|v| v.as_array())
        .expect("input should be an array");

    let tool_call = input
        .iter()
        .find(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .expect("function_call item should exist");
    assert!(
        tool_call.get("signature").is_none(),
        "function_call signature should be stripped for codex"
    );

    let normalized_block = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .find(|block| block.get("type").and_then(|v| v.as_str()) == Some("output_text"))
        .expect("thinking block should be normalized to output_text");
    assert!(
        normalized_block.get("signature").is_none(),
        "normalized block signature should be stripped for codex"
    );
    assert_eq!(
        normalized_block
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "internal",
        "thinking text should be preserved after normalization"
    );
    assert!(
        normalized_block.get("thinking").is_none(),
        "legacy thinking field should be removed after normalization"
    );

    let has_thinking_type = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .any(|block| block.get("type").and_then(|v| v.as_str()) == Some("thinking"));
    assert!(
        !has_thinking_type,
        "codex payload must not contain thinking type"
    );

    let has_summary_text_type = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .any(|block| block.get("type").and_then(|v| v.as_str()) == Some("summary_text"));
    assert!(
        !has_summary_text_type,
        "codex message.content should not use summary_text type"
    );
}

#[test]
fn test_codex_input_normalizes_multiple_thinking_blocks() {
    let request = AnthropicRequest {
        model: Some("claude-opus-4-6".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![
                ContentBlock::Thinking {
                    thinking: "first".to_string(),
                    signature: Some("sig_1".to_string()),
                },
                ContentBlock::Text {
                    text: "visible".to_string(),
                },
                ContentBlock::Thinking {
                    thinking: "second".to_string(),
                    signature: Some("sig_2".to_string()),
                },
            ])),
        }],
        system: None,
        stream: true,
        tools: None,
        max_tokens: None,
        temperature: None,
        top_p: None,
        top_k: None,
        stop_sequences: None,
    };

    let mapping = ReasoningEffortMapping::default();
    let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
    let input = body
        .get("input")
        .and_then(|v| v.as_array())
        .expect("input should be an array");

    let normalized_texts: Vec<String> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("output_text"))
        .filter_map(|block| {
            block
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    assert!(
        normalized_texts.contains(&"first".to_string()),
        "first thinking block should be normalized to output_text"
    );
    assert!(
        normalized_texts.contains(&"second".to_string()),
        "second thinking block should be normalized to output_text"
    );
}

#[test]
fn test_codex_input_sanitizes_function_call_name() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: Some("call_abc".to_string()),
                name: "functions.exec_command".to_string(),
                input: json!({"cmd": "echo hi"}),
                signature: None,
            }])),
        }],
        system: None,
        stream: true,
        tools: None,
        max_tokens: None,
        temperature: None,
        top_p: None,
        top_k: None,
        stop_sequences: None,
    };

    let mapping = ReasoningEffortMapping::default();
    let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

    let input = body
        .get("input")
        .and_then(|v| v.as_array())
        .expect("input should be an array");

    let tool_call = input
        .iter()
        .find(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .expect("function_call item should exist");
    assert_eq!(
        tool_call.get("name").and_then(|v| v.as_str()),
        Some("functions_exec_command"),
        "function_call name should be sanitized to codex-accepted pattern"
    );
}

#[test]
fn test_codex_input_all_function_call_names_match_pattern() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![
                ContentBlock::ToolUse {
                    id: Some("call_1".to_string()),
                    name: "functions.exec_command".to_string(),
                    input: json!({"cmd": "echo hi"}),
                    signature: None,
                },
                ContentBlock::ToolUse {
                    id: Some("call_2".to_string()),
                    name: "multi_tool_use.parallel".to_string(),
                    input: json!({"tool_uses": []}),
                    signature: None,
                },
                ContentBlock::ToolUse {
                    id: Some("call_3".to_string()),
                    name: "Valid_Name-01".to_string(),
                    input: json!({"ok": true}),
                    signature: None,
                },
            ])),
        }],
        system: None,
        stream: true,
        tools: None,
        max_tokens: None,
        temperature: None,
        top_p: None,
        top_k: None,
        stop_sequences: None,
    };

    let mapping = ReasoningEffortMapping::default();
    let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

    let input = body
        .get("input")
        .and_then(|v| v.as_array())
        .expect("input should be an array");

    let call_names: Vec<String> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .filter_map(|item| {
            item.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    assert_eq!(
        call_names.len(),
        3,
        "expected all tool_use blocks to become function_call"
    );
    for name in call_names {
        assert!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'),
            "function_call name '{}' must match ^[a-zA-Z0-9_-]+$",
            name
        );
    }
}
