use super::*;
#[test]
fn test_plain_content_part_text_is_not_misclassified_as_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.content_part.added",
            "part": {
                "type": "output_text",
                "text": "普通文本内容\n"
            }
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    let has_text_block_payload = joined
        .contains("\"content_block\":{\"text\":\"\",\"type\":\"text\"}")
        || joined.contains("\"content_block\":{\"type\":\"text\",\"text\":\"\"}");
    assert!(
        joined.contains("\"content_block_start\"") && has_text_block_payload,
        "plain content_part text should open a text block"
    );
    assert!(
        joined.contains("\"type\":\"text_delta\"")
            && joined.contains("\"text\":\"普通文本内容\\n\""),
        "plain content_part text should be emitted as text_delta"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "plain content_part text must not be promoted to tool_use"
    );
}

#[test]
fn test_plain_text_is_not_misclassified_as_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "你好\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    let has_text_block_payload = joined
        .contains("\"content_block\":{\"text\":\"\",\"type\":\"text\"}")
        || joined.contains("\"content_block\":{\"type\":\"text\",\"text\":\"\"}");
    assert!(
        joined.contains("\"content_block_start\"") && has_text_block_payload,
        "plain text should open a text block"
    );
    assert!(
        joined.contains("\"type\":\"text_delta\"") && joined.contains("\"text\":\"你好\\n\""),
        "plain text should emit text_delta and preserve newline"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "plain text must not be promoted to tool_use"
    );
}

#[test]
fn test_plain_text_preserves_markdown_line_breaks() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "## Rust 入门\n\n1. 语法基础\n2. 核心机制\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("\"text\":\"## Rust 入门\\n\\n1. 语法基础\\n2. 核心机制\\n\""),
        "markdown text should keep line breaks to avoid collapsed layout"
    );
}

#[test]
fn test_plain_text_without_proposed_plan_wrapper_is_unchanged() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "普通说明文字，不含任何 plan wrapper。"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("普通说明文字，不含任何 plan wrapper。"),
        "ordinary visible text should remain unchanged when no proposed_plan wrapper is present"
    );
    assert!(
        !joined.contains("proposed_plan"),
        "ordinary text should not gain proposed_plan markers during hygiene"
    );
}

#[test]
fn test_proposed_plan_tags_are_preserved_in_visible_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "<proposed_plan>\n1. 先检查请求链路\n2. 等你确认后再执行修改\n</proposed_plan>"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("<proposed_plan>") && joined.contains("</proposed_plan>"),
        "proposed_plan wrapper tags should remain visible so Claude clients can render plan previews"
    );
    assert!(
        joined.contains("先检查请求链路") && joined.contains("等你确认后再执行修改"),
        "plan body should remain visible after stripping wrapper tags"
    );
}

#[test]
fn test_proposed_plan_tags_split_across_chunks_are_preserved() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let first = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "<proposed_plan>\n1. 先检查请求链路\n"
        })
    );
    let second = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "2. 等你确认后再执行修改\n</proposed_plan>"
        })
    );

    let mut events = transformer.transform_sse_line(&first);
    events.extend(transformer.transform_sse_line(&second));
    let joined = events.join("");

    assert!(
        joined.contains("<proposed_plan>") && joined.contains("</proposed_plan>"),
        "proposed_plan wrapper tags should remain visible even when split across chunks"
    );
    assert!(
        joined.contains("先检查请求链路") && joined.contains("等你确认后再执行修改"),
        "plan body should remain visible across chunk boundaries"
    );
}

#[test]
fn test_proposed_plan_detection_is_recorded_in_diagnostics() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let plan_body = "# Clock Plan\n\n## Summary\n- inspect request path\n- patch converter";
    let first = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "<proposed_plan>\n# Clock Plan\n\n## Summary\n- inspect request path\n"
        })
    );
    let second = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "- patch converter\n</proposed_plan>"
        })
    );
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": { "status": "completed" }
        })
    );

    let _ = transformer.transform_sse_line(&first);
    let _ = transformer.transform_sse_line(&second);
    let _ = transformer.transform_sse_line(&completed);

    let summary =
        <TransformResponse as crate::transform::ResponseTransformer>::take_diagnostics_summary(
            &mut transformer,
        )
        .expect("diagnostics summary should exist after proposed plan extraction");

    assert_eq!(
        summary
            .pointer("/counters/detected_proposed_plan_blocks")
            .and_then(|v| v.as_u64()),
        Some(1),
        "complete proposed_plan blocks should be counted for future bridge instrumentation"
    );
    assert_eq!(
        summary
            .pointer("/counters/extracted_proposed_plan_body_chars")
            .and_then(|v| v.as_u64()),
        Some(plan_body.chars().count() as u64),
        "diagnostics should record extracted proposed_plan body size"
    );
}

#[test]
fn test_proposed_plan_bridge_writes_plan_file_and_emits_exit_plan_mode() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let plan_path = std::env::temp_dir().join(format!(
        "codex_proxy_plan_bridge_{}.md",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    <TransformResponse as crate::transform::ResponseTransformer>::configure_request_context(
        &mut transformer,
        &crate::transform::ResponseTransformRequestContext {
            codex_plan_file_path: Some(plan_path.to_string_lossy().to_string()),
        },
    );

    let plan_body = "# Clock Plan\n\n## Summary\n- inspect request path\n- patch converter";
    let first = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "<proposed_plan>\n# Clock Plan\n\n## Summary\n- inspect request path\n"
        })
    );
    let second = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "- patch converter\n</proposed_plan>"
        })
    );
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": { "status": "completed" }
        })
    );

    let mut events = transformer.transform_sse_line(&first);
    events.extend(transformer.transform_sse_line(&second));
    events.extend(transformer.transform_sse_line(&completed));
    let joined = events.join("");

    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"ExitPlanMode\""),
        "plan bridge should emit a synthetic ExitPlanMode tool_use"
    );
    assert!(
        !joined.contains("<proposed_plan>") && !joined.contains("Clock Plan"),
        "raw proposed_plan text should be suppressed once the bridge succeeds"
    );
    assert!(
        joined.contains("\"stop_reason\":\"tool_use\""),
        "terminal stop_reason should switch to tool_use when ExitPlanMode is bridged"
    );
    assert_eq!(
        std::fs::read_to_string(&plan_path).expect("plan file should be written"),
        plan_body,
        "plan bridge should persist extracted proposed_plan body to Claude's plan file path"
    );

    let summary =
        <TransformResponse as crate::transform::ResponseTransformer>::take_diagnostics_summary(
            &mut transformer,
        )
        .expect("diagnostics summary should exist after plan bridge");
    assert_eq!(
        summary
            .pointer("/counters/plan_bridge_write_successes")
            .and_then(|v| v.as_u64()),
        Some(1),
        "diagnostics should record successful plan-file writes"
    );
    assert_eq!(
        summary
            .pointer("/counters/plan_bridge_exit_plan_mode_emitted")
            .and_then(|v| v.as_u64()),
        Some(1),
        "diagnostics should record synthetic ExitPlanMode emission"
    );

    let _ = std::fs::remove_file(plan_path);
}

#[test]
fn test_log_sample_overflow_text_is_safely_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // Sample extracted from fronted-tauri/src-tauri/logs/proxy_20260222_104127.log
    let output_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "**Aligning document chat model path****Aligning document chat model path**"
        })
    );
    let leaked_tail = format!(
        "data: {}",
        json!({
            "type": "response.content_part.added",
            "part": {
                "type": "output_text",
                "text": "{\"file_path\":\"/Users/mr.j/myRoom/code/ai/MyProjects/Proma/apps/electron/src/main/lib/plugins/document-chat-bridge.ts\",\"new_string\":\"import { computeEmbedding, loadKnowledgeBaseIndex, resolvePluginModelOrThrow } from './ai-indexing-service'\\n\",\"old_string\":\"import { computeEmbedding, loadKnowledgeBaseIndex } from './ai-indexing-service'\",\"replace_all\":false}"
            }
        })
    );

    let mut events = transformer.transform_sse_line(&output_delta);
    events.extend(transformer.transform_sse_line(&leaked_tail));
    let joined = events.join("");

    assert!(
        joined.contains("Aligning document chat model path")
            && !joined.contains("\\\"file_path\\\"")
            && !joined.contains("\\\"new_string\\\"")
            && !joined.contains("\\\"old_string\\\"")
            && !joined.contains("\\\"replace_all\\\""),
        "log-derived leaked tool json tail must be suppressed while keeping readable prefix"
    );
    assert!(
        joined.contains("\"type\":\"tool_use\"")
            && joined.contains("\"name\":\"AskUserQuestion\"")
            && !joined.contains("\"name\":\"Edit\""),
        "high-risk log-derived leaked payload should emit AskUserQuestion and keep real tool blocked"
    );
}

#[test]
fn test_plain_business_json_text_is_not_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "业务示例：{\"name\":\"alice\",\"age\":18,\"ok\":true}"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    assert!(
        joined.contains("业务示例"),
        "plain business json text should remain visible"
    );
    assert!(
        joined.contains("\\\"name\\\":\\\"alice\\\""),
        "business json should not be suppressed when it does not match tool payload"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "business json text must not create tool_use"
    );
}

#[test]
fn test_unparsable_leaked_tool_prefix_is_suppressed_from_visible_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "assistant to=multi_tool_use.parallelExtra {\"tool_uses\":[]}\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "unparsable leaked tool prefix should not fall through to text output"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "unparsable leaked tool prefix should not fabricate a tool_use block"
    );
    assert!(
        !joined.contains("parallelExtra"),
        "suppressed unparsable leaked marker should stay hidden from client text"
    );
}

#[test]
fn test_codex_input_strips_leaked_tool_suffix_from_message_text() {
    let request = AnthropicRequest {
        model: Some("claude-sonnet-4-5-20250929".to_string()),
        messages: vec![Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                text: "先起草 design。 to=functions.Write {\"file_path\":\"/tmp/design.md\"}"
                    .to_string(),
            }])),
        }],
        system: None,
        metadata: None,
        stream: true,
        tools: None,
        tool_choice: None,
        thinking: None,
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
        texts.iter().any(|text| text.contains("先起草 design。")),
        "normal prefix text should be preserved"
    );
    assert!(
        texts
            .iter()
            .all(|text| !text.contains("to=functions.Write")),
        "leaked tool marker should be stripped from outbound message text"
    );
}
