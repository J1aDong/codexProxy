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

#[test]
fn test_codex_input_strips_markerless_high_confidence_tool_json_tail() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "继续处理。 {\"file_path\":\"/tmp/a.ts\",\"new_string\":\"x\",\"old_string\":\"y\",\"replace_all\":false}"
                    .to_string(),
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

    let texts: Vec<String> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .filter_map(|block| {
            block
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    assert!(
        texts.iter().any(|text| text.contains("继续处理。")),
        "normal prefix text should be preserved"
    );
    assert!(
        texts.iter().all(|text| !text.contains("\"file_path\"")
            && !text.contains("\"new_string\"")
            && !text.contains("\"old_string\"")
            && !text.contains("\"replace_all\"")),
        "high-confidence tool json tail should be stripped from outbound request"
    );
}

#[test]
fn test_codex_input_strips_markerless_exec_command_json_tail() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "继续执行。 {\"command\":\"npm run lint\",\"description\":\"Run lint\",\"timeout\":600000}".to_string(),
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

    let texts: Vec<String> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .filter_map(|block| {
            block
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    assert!(
        texts.iter().any(|text| text.contains("继续执行。")),
        "normal prefix text should be preserved"
    );
    assert!(
        texts.iter().all(|text| !text.contains("\"command\"")
            && !text.contains("\"description\"")
            && !text.contains("\"timeout\"")),
        "markerless exec-command json tail should be stripped from outbound request"
    );
}

#[test]
fn test_codex_request_without_tools_uses_none_tool_choice_and_empty_tools() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "hello".to_string(),
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

    let tools = body
        .get("tools")
        .and_then(|v| v.as_array())
        .expect("tools should be an array");
    assert!(
        tools.is_empty(),
        "tools should stay empty when request has no tools"
    );
    assert_eq!(
        body.get("tool_choice").and_then(|v| v.as_str()),
        Some("none"),
        "tool_choice should be none when tools are empty"
    );
}

#[test]
fn test_codex_request_does_not_inject_template_input() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "say hi".to_string(),
            }])),
        }],
        system: None,
        stream: true,
        tools: Some(vec![]),
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
    let serialized = serde_json::to_string(input).expect("input should serialize");

    assert!(
        !serialized.contains("name: engineer-professional"),
        "legacy codex-request template content must not be injected"
    );
}

#[test]
fn test_codex_input_sanitizes_stuck_tool_json_and_drops_suggestion_prompt() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                    text: "**Verifying installed command****Verifying installed command** +#+#+#+#+#+{\"command\":\"node \\\"/Users/mr.j/.local/ddg-search/node_modules/@oevortex/ddg_search/dist/index.js\\\" --help\",\"description\":\"验证本地安装的ddg_search可执行\"}".to_string(),
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                    text: "[SUGGESTION MODE: Suggest what the user might naturally type next into Claude Code.]\n\nReply with ONLY the suggestion, no quotes or explanation.".to_string(),
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

    let message_texts: Vec<String> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .collect();

    assert!(
        message_texts
            .iter()
            .any(|text| text.contains("**Verifying installed command**")),
        "normal assistant prefix should be preserved"
    );
    assert!(
        message_texts
            .iter()
            .all(|text| !text
                .contains("**Verifying installed command****Verifying installed command**")),
        "duplicated markdown bold fragments should be collapsed"
    );
    assert!(
        message_texts
            .iter()
            .all(|text| !text.contains("\"command\":\"node")),
        "stuck tool json should be stripped from message text"
    );
    assert!(
        message_texts
            .iter()
            .all(|text| !text.contains("SUGGESTION MODE")),
        "suggestion-mode prompt should be dropped before upstream request"
    );
    assert!(
        message_texts
            .iter()
            .all(|text| !text.contains(" +#+#+#+#+#+")),
        "known trailing noise after stripped tool json should be removed"
    );
}

#[test]
fn test_codex_input_strips_system_reminder_from_function_call_output() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_vKYfzxA90qhkE8OO8PoOPBem".to_string()),
                    name: "Bash".to_string(),
                    input: json!({
                        "command": "npm install @oevortex/ddg_search ajv",
                        "description": "创建本地目录并安装ddg_search及ajv依赖",
                        "timeout": 600000
                    }),
                    signature: None,
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: Some("call_vKYfzxA90qhkE8OO8PoOPBem".to_string()),
                    id: Some("result_id".to_string()),
                    content: Some(json!(
                        "added 138 packages\n\n<system-reminder>\nignore this reminder\n</system-reminder>"
                    )),
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

    let function_call_output = input
        .iter()
        .find(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call_output"))
        .expect("function_call_output should exist");
    let output_text = function_call_output
        .get("output")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    assert!(
        output_text.contains("added 138 packages"),
        "normal tool output should be preserved"
    );
    assert!(
        !output_text.contains("<system-reminder>") && !output_text.contains("ignore this reminder"),
        "system reminder block should be removed from tool output replay"
    );
}

#[test]
fn test_codex_input_preserves_skill_system_reminder_in_assistant_text() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "Skill meta:\n<system-reminder>\n### Available skills\n- defuddle (file: /tmp/defuddle/SKILL.md)\n</system-reminder>\nDone.".to_string(),
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

    let text_blocks: Vec<&str> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|msg| msg.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .collect();

    assert!(
        text_blocks
            .iter()
            .any(|text| text.contains("### Available skills") && text.contains("SKILL.md")),
        "skill catalog reminder should be preserved for downstream skill awareness"
    );
}

#[test]
fn test_codex_input_keeps_empty_function_call_output() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_empty_output".to_string()),
                    name: "Bash".to_string(),
                    input: json!({
                        "command": "sqlite3 /tmp/demo.db \"select 1 where 1=0\""
                    }),
                    signature: None,
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: Some("call_empty_output".to_string()),
                    id: Some("result_empty".to_string()),
                    content: Some(json!("")),
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

    let function_call_output = input
        .iter()
        .find(|item| {
            item.get("type").and_then(|v| v.as_str()) == Some("function_call_output")
                && item.get("call_id").and_then(|v| v.as_str()) == Some("call_empty_output")
        })
        .expect("empty function_call_output should be retained for tool-call pairing");

    assert_eq!(
        function_call_output
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>"),
        "(No output)",
        "empty tool output should be replaced with explicit placeholder text"
    );
}

#[test]
fn test_codex_input_strips_markerless_taskoutput_json_tail() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "**Checking debug output for failing path**numerusform{\"block\":true,\"task_id\":\"bf6ea6d\",\"timeout\":20000}".to_string(),
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

    let text_blocks: Vec<&str> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|msg| msg.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .collect();

    assert!(
        text_blocks
            .iter()
            .any(|text| text.contains("Checking debug output for failing path")),
        "normal human-readable prefix text should remain"
    );
    assert!(
        text_blocks
            .iter()
            .all(|text| !text.contains("\"task_id\"") && !text.contains("\"timeout\"")),
        "markerless task-output args json tail should be stripped from outbound message text"
    );
    assert!(
        text_blocks.iter().all(|text| !text.contains("numerusform")),
        "known connector noise should be removed after stripping leaked json tail"
    );
}

#[test]
fn test_codex_input_strips_markerless_read_json_tail() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "Removing obsolete payload branch +#+#+#+#+#+{\"file_path\":\"/Users/mr.j/myRoom/YAT/yat_commad_check/index.html\",\"offset\":260,\"limit\":220}".to_string(),
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

    let text_blocks: Vec<&str> = input
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
        .filter_map(|msg| msg.get("content").and_then(|v| v.as_array()))
        .flat_map(|content| content.iter())
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .collect();

    assert!(
        text_blocks
            .iter()
            .any(|text| text.contains("Removing obsolete payload branch")),
        "normal readable prefix should remain"
    );
    assert!(
        text_blocks.iter().all(|text| {
            !text.contains("\"file_path\"")
                && !text.contains("\"offset\"")
                && !text.contains("\"limit\"")
        }),
        "markerless read-args json tail should be stripped from outbound message text"
    );
}

#[test]
fn test_codex_input_injects_missing_function_call_output_placeholder() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: Some("call_missing_result".to_string()),
                name: "Bash".to_string(),
                input: json!({
                    "command": "echo hello"
                }),
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

    let synthesized_output = input
        .iter()
        .find(|item| {
            item.get("type").and_then(|v| v.as_str()) == Some("function_call_output")
                && item.get("call_id").and_then(|v| v.as_str()) == Some("call_missing_result")
        })
        .expect("missing function_call_output should be synthesized");

    assert_eq!(
        synthesized_output
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>"),
        "(No output)",
        "synthesized function_call_output should use stable placeholder text"
    );
}
