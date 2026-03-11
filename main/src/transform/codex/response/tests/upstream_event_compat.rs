use super::*;
use serde_json::json;
#[test]
fn response_incomplete_emits_terminal_events_with_max_tokens() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let text_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "partial answer"
        })
    );
    let incomplete_line = format!(
        "data: {}",
        json!({
            "type": "response.incomplete",
            "response": {
                "status": "incomplete",
                "incomplete_details": { "reason": "max_output_tokens" },
                "usage": { "input_tokens": 12, "output_tokens": 34 }
            }
        })
    );

    let mut events = transformer.transform_sse_line(&text_line);
    events.extend(transformer.transform_sse_line(&incomplete_line));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"message_delta\""));
    assert!(joined.contains("\"stop_reason\":\"max_tokens\""));
    assert!(joined.contains("\"input_tokens\":12"));
    assert!(joined.contains("\"output_tokens\":34"));
    assert!(joined.contains("\"type\":\"message_stop\""));
}

#[test]
fn response_incomplete_maps_context_window_to_anthropic_stop_reason() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let incomplete_line = format!(
        "data: {}",
        json!({
            "type": "response.incomplete",
            "response": {
                "status": "incomplete",
                "incomplete_details": { "reason": "model_context_window_exceeded" }
            }
        })
    );

    let joined = transformer.transform_sse_line(&incomplete_line).join("");
    assert!(joined.contains("\"stop_reason\":\"model_context_window_exceeded\""));
    assert!(joined.contains("\"type\":\"message_stop\""));
}

#[test]
fn response_done_alias_emits_terminal_events() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let text_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "final answer"
        })
    );
    let done_line = format!(
        "data: {}",
        json!({
            "type": "response.done",
            "response": {
                "status": "completed",
                "usage": { "input_tokens": 9, "output_tokens": 21 }
            }
        })
    );

    let mut events = transformer.transform_sse_line(&text_line);
    events.extend(transformer.transform_sse_line(&done_line));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"message_delta\""));
    assert!(joined.contains("\"stop_reason\":\"end_turn\""));
    assert!(joined.contains("\"input_tokens\":9"));
    assert!(joined.contains("\"output_tokens\":21"));
    assert!(joined.contains("\"type\":\"message_stop\""));
}

#[test]
fn message_output_item_done_force_closes_text_before_tool_item() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "output_index": 0,
            "item_id": "msg_1",
            "delta": "need read file"
        })
    );
    let message_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": { "id": "msg_1", "type": "message" }
        })
    );
    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_1",
                "type": "function_call",
                "call_id": "call_1",
                "name": "Read"
            }
        })
    );
    let tool_args_delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "item_id": "fc_1",
            "call_id": "call_1",
            "delta": "{\"file_path\":\"/tmp/a.txt\"}"
        })
    );
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_1",
                "type": "function_call",
                "call_id": "call_1",
                "name": "Read"
            }
        })
    );

    let _ = transformer.transform_sse_line(&text_delta);
    let done_events = transformer.transform_sse_line(&message_done).join("");
    assert!(
        done_events.contains("\"type\":\"content_block_stop\""),
        "message output_item.done should force-close active text block"
    );

    let mut tool_events = transformer.transform_sse_line(&tool_added);
    tool_events.extend(transformer.transform_sse_line(&tool_args_delta));
    tool_events.extend(transformer.transform_sse_line(&tool_done));
    let joined = tool_events.join("");
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Read\""),
        "tool sequence should remain healthy after message-done fence"
    );
}

/// 验证事件交错时 Message 文本不会在工具窗口期间被丢弃。
/// 场景：text delta 在 function_call output_item.added 之后到达（事件交错）
#[test]
fn interleaved_message_text_during_tool_window_is_preserved() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // 1. 注册 message item
    let msg_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": { "id": "msg_1", "type": "message" }
        })
    );
    // 2. 第一段文本
    let text1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "output_index": 0,
            "item_id": "msg_1",
            "delta": "I'll read"
        })
    );
    // 3. function_call 注册（此时文本 block 被关闭、工具被缓冲）
    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_1",
                "type": "function_call",
                "call_id": "call_1",
                "name": "Read"
            }
        })
    );
    // 4. 第二段文本（交错到达，属于 message output_index=0）
    let text2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "output_index": 0,
            "item_id": "msg_1",
            "delta": " the file"
        })
    );
    // 5. 工具参数
    let tool_args = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "item_id": "fc_1",
            "call_id": "call_1",
            "delta": "{\"file_path\":\"/tmp/a.txt\"}"
        })
    );
    // 6. 工具完成
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_1",
                "type": "function_call",
                "call_id": "call_1",
                "name": "Read"
            }
        })
    );
    // 7. 响应完成
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "usage": { "input_tokens": 10, "output_tokens": 20 }
            }
        })
    );

    let mut all_output = Vec::new();
    all_output.extend(transformer.transform_sse_line(&msg_added));
    all_output.extend(transformer.transform_sse_line(&text1));
    all_output.extend(transformer.transform_sse_line(&tool_added));
    all_output.extend(transformer.transform_sse_line(&text2));
    all_output.extend(transformer.transform_sse_line(&tool_args));
    all_output.extend(transformer.transform_sse_line(&tool_done));
    all_output.extend(transformer.transform_sse_line(&completed));

    let joined = all_output.join("");

    // 验证：第一段文本被发射
    assert!(
        joined.contains("I'll read"),
        "First text fragment should be emitted"
    );

    // 验证：第二段文本（交错到达）被保留（延迟发射），不应被丢弃
    assert!(
        joined.contains(" the file"),
        "Interleaved message text during tool window should be preserved (deferred), not dropped. Output: {}",
        joined
    );

    // 验证：工具调用正常
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Read\""),
        "Tool use block should be emitted correctly"
    );

    // 验证：有正确的终止事件
    assert!(
        joined.contains("\"type\":\"message_stop\""),
        "Message stop should be emitted"
    );
}

#[test]
fn response_refusal_stream_maps_to_text_blocks_and_refusal_stop_reason() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let refusal_delta = format!(
        "data: {}",
        json!({
            "type": "response.refusal.delta",
            "delta": "I can't "
        })
    );
    let refusal_done = format!(
        "data: {}",
        json!({
            "type": "response.refusal.done",
            "refusal": "I can't help with that."
        })
    );
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed"
            }
        })
    );

    let mut events = transformer.transform_sse_line(&refusal_delta);
    events.extend(transformer.transform_sse_line(&refusal_done));
    events.extend(transformer.transform_sse_line(&completed));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"content_block_start\""));
    assert!(joined.contains("\"type\":\"text_delta\""));
    assert!(joined.contains("I can't "));
    assert!(joined.contains("help with that."));
    assert!(joined.contains("\"stop_reason\":\"refusal\""));
    assert!(joined.contains("\"type\":\"message_stop\""));
}

#[test]
fn agent_tool_start_emits_generic_background_progress_before_done() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_agent_generic_1",
                "type": "function_call",
                "call_id": "call_agent_generic_1",
                "name": "Agent"
            }
        })
    );

    let joined = transformer.transform_sse_line(&tool_added).join("");
    println!("JOINED={}", joined);

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("正在启动后台 explorer"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
}

#[test]
fn response_in_progress_emits_single_lifecycle_heartbeat() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let in_progress = format!(
        "data: {}",
        json!({
            "type": "response.in_progress",
            "response": { "status": "in_progress" }
        })
    );

    let first = transformer.transform_sse_line(&in_progress).join("");
    let second = transformer.transform_sse_line(&in_progress).join("");

    assert!(first.contains("模型正在处理中"));
    assert!(first.contains("\"type\":\"thinking_delta\""));
    assert!(!second.contains("模型正在处理中"));
}

#[test]
fn web_search_tool_stream_is_still_visible_when_thinking_disabled() {
    let mut transformer = TransformResponse::new_with_visible_thinking("gpt-5.3-codex", false);

    let search_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "ws_1",
                "type": "web_search_call",
                "status": "in_progress"
            }
        })
    );
    let search_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "ws_1",
                "type": "web_search_call",
                "status": "completed",
                "action": {
                    "type": "search",
                    "query": "popular ai tools",
                    "sources": [
                        {
                            "type": "url",
                            "url": "https://example.com/tools",
                            "title": "Popular AI Tools"
                        }
                    ]
                }
            }
        })
    );

    let joined = format!(
        "{}{}",
        transformer.transform_sse_line(&search_added).join(""),
        transformer.transform_sse_line(&search_done).join("")
    );

    assert!(joined.contains("\"type\":\"server_tool_use\""));
    assert!(joined.contains("\"type\":\"input_json_delta\""));
    assert!(joined.contains("popular ai tools"));
    assert!(joined.contains("\"type\":\"web_search_tool_result\""));
    assert!(!joined.contains("\"type\":\"thinking_delta\""));
}

#[test]
fn web_search_call_emits_server_tool_use_and_result_blocks() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let search_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "ws_1",
                "type": "web_search_call",
                "status": "in_progress"
            }
        })
    );
    let search_searching = format!(
        "data: {}",
        json!({
            "type": "response.web_search_call.searching",
            "output_index": 1,
            "item_id": "ws_1"
        })
    );
    let search_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "ws_1",
                "type": "web_search_call",
                "status": "completed",
                "action": {
                    "type": "search",
                    "query": "popular ai tools",
                    "sources": [
                        {
                            "type": "url",
                            "url": "https://example.com/tools",
                            "title": "Popular AI Tools"
                        },
                        {
                            "type": "url",
                            "url": "https://example.com/trends"
                        }
                    ]
                }
            }
        })
    );
    let final_message_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 2,
            "item": {
                "id": "msg_1",
                "type": "message",
                "status": "in_progress",
                "phase": "final_answer",
                "content": [],
                "role": "assistant"
            }
        })
    );
    let final_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "output_index": 2,
            "item_id": "msg_1",
            "content_index": 0,
            "delta": "整理好了"
        })
    );

    let mut events = Vec::new();
    events.extend(transformer.transform_sse_line(&search_added));
    events.extend(transformer.transform_sse_line(&search_searching));
    events.extend(transformer.transform_sse_line(&search_done));
    events.extend(transformer.transform_sse_line(&final_message_added));
    events.extend(transformer.transform_sse_line(&final_text));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"server_tool_use\""));
    assert!(joined.contains("\"name\":\"web_search\""));
    assert!(joined.contains("\"type\":\"input_json_delta\""));
    assert!(joined.contains("popular ai tools"));
    assert!(joined.contains("\"type\":\"web_search_tool_result\""));
    assert!(joined.contains("https://example.com/tools"));
    assert!(joined.contains("https://example.com/trends"));
    assert!(!joined.contains("正在发起网页搜索"));
    assert!(!joined.contains("正在检索搜索结果"));
    assert!(!joined.contains("已拿到搜索结果，继续整理"));
    assert!(!joined.contains("\"type\":\"thinking_delta\""));
    assert!(joined.contains("整理好了"));
}

#[test]
fn response_failed_emits_error_with_upstream_message_and_code() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let failed = format!(
        "data: {}",
        json!({
            "type": "response.failed",
            "response": {
                "error": {
                    "message": "Sibling tool call errored: Invalid tool parameters",
                    "code": "invalid_tool_arguments"
                }
            }
        })
    );

    let joined = transformer.transform_sse_line(&failed).join("");
    assert!(joined.contains("event: error"));
    assert!(joined.contains("Sibling tool call errored: Invalid tool parameters"));
    assert!(joined.contains("\"code\":\"invalid_tool_arguments\""));
    assert!(joined.contains("\"type\":\"message_stop\""));
}

#[test]
fn error_event_emits_error_with_message_and_code() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let error_line = format!(
        "data: {}",
        json!({
            "type": "error",
            "error": {
                "message": "Rate limit exceeded",
                "code": "rate_limit_exceeded"
            }
        })
    );

    let joined = transformer.transform_sse_line(&error_line).join("");
    assert!(joined.contains("event: error"));
    assert!(joined.contains("Rate limit exceeded"));
    assert!(joined.contains("\"code\":\"rate_limit_exceeded\""));
    assert!(joined.contains("\"type\":\"message_stop\""));
}

#[test]
fn response_failed_without_message_uses_default_error_message() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let failed = format!(
        "data: {}",
        json!({
            "type": "response.failed",
            "response": {
                "error": {
                    "code": "unknown_error"
                }
            }
        })
    );

    let joined = transformer.transform_sse_line(&failed).join("");
    assert!(joined.contains("Upstream returned response.failed and terminated the stream."));
    assert!(joined.contains("\"code\":\"unknown_error\""));
    assert!(joined.contains("\"type\":\"message_stop\""));
}

#[test]
fn function_call_arguments_whitespace_flood_triggers_controlled_error_stop() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_ws_guard",
                "type": "function_call",
                "call_id": "call_ws_guard",
                "name": "Read"
            }
        })
    );
    let flood_delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "fc_ws_guard",
            "delta": " ".repeat(TransformResponse::MAX_FUNCTION_ARGS_WHITESPACE_RUN + 1)
        })
    );

    let _ = transformer.transform_sse_line(&add);
    let joined = transformer.transform_sse_line(&flood_delta).join("");

    assert!(
        joined.contains("event: error")
            && joined.contains("\"code\":\"function_args_whitespace_overflow\""),
        "whitespace flood should trigger deterministic error path"
    );
    assert!(
        joined.contains("\"type\":\"message_stop\""),
        "overflow guard should terminate stream cleanly"
    );
}

#[test]
fn mismatched_item_id_is_normalized_by_output_index_for_tool_arguments() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 7,
            "item": {
                "id": "fc_sync_primary",
                "type": "function_call",
                "call_id": "call_sync_primary",
                "name": "Read"
            }
        })
    );
    let mismatched_delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 7,
            "item_id": "fc_sync_mismatch",
            "delta": "{\"file_path\":\"/tmp/id-sync.ts\"}"
        })
    );
    let done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 7,
            "item": {
                "id": "fc_sync_done_mismatch",
                "type": "function_call",
                "call_id": "call_sync_primary",
                "name": "Read"
            }
        })
    );

    let add_events = transformer.transform_sse_line(&add).join("");
    let delta_events = transformer.transform_sse_line(&mismatched_delta).join("");
    let done_events = transformer.transform_sse_line(&done).join("");
    let joined = format!("{add_events}{delta_events}{done_events}");

    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"id\":\"call_sync_primary\""),
        "mismatched item_id should still resolve to original tool call"
    );
    assert!(
        joined.contains("id-sync.ts"),
        "arguments routed by normalized output_index should stay attached"
    );
    assert!(
        transformer.diagnostics.normalized_item_id_mismatches >= 1,
        "normalization diagnostics should record item_id mismatch handling"
    );
}

#[test]
fn output_index_routing_wins_over_conflicting_item_id_for_tool_arguments() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_first = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_first",
                "type": "function_call",
                "call_id": "call_first",
                "name": "Read"
            }
        })
    );
    let add_second = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 2,
            "item": {
                "id": "fc_second",
                "type": "function_call",
                "call_id": "call_second",
                "name": "Read"
            }
        })
    );
    let conflicting_delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 2,
            "item_id": "fc_first",
            "delta": "{\"file_path\":\"/tmp/index-priority.txt\"}"
        })
    );
    let done_second = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 2,
            "item": {
                "id": "fc_second",
                "type": "function_call",
                "call_id": "call_second",
                "name": "Read"
            }
        })
    );
    let done_first = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_first",
                "type": "function_call",
                "call_id": "call_first",
                "name": "Read"
            }
        })
    );

    let _ = transformer.transform_sse_line(&add_first);
    let _ = transformer.transform_sse_line(&add_second);
    let _ = transformer.transform_sse_line(&conflicting_delta);
    let _ = transformer.transform_sse_line(&done_second);
    let joined = transformer.transform_sse_line(&done_first).join("");

    let call_second_pos = joined
        .find("\"id\":\"call_second\"")
        .expect("second tool call should be emitted");
    let path_pos = joined
        .find("index-priority.txt")
        .expect("argument payload should be emitted");

    assert!(
        path_pos > call_second_pos,
        "conflicting item_id must not steal args from the output_index-matched tool call"
    );
}

#[test]
fn terminal_invariant_violation_auto_clears_orphaned_tool_argument_updates() {
    // When upstream sends inconsistent tool call data (e.g., tool argument updates
    // without a corresponding function_call), we auto-clear them at terminal phase
    // and emit a warning instead of failing the entire response.
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    transformer
        .pending_tool_argument_updates
        .push(PendingToolArgumentUpdate {
            output_index: Some(999),
            item_id: Some("orphan_item".to_string()),
            call_id: Some("orphan_call".to_string()),
            kind: PendingToolArgumentUpdateKind::Delta("{".to_string()),
        });

    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed"
            }
        })
    );

    let joined = transformer.transform_sse_line(&completed).join("");
    // Should NOT emit error - instead auto-clear and continue normally
    assert!(!joined.contains("event: error"));
    // Should emit normal message_stop
    assert!(joined.contains("\"type\":\"message_stop\""));
    // Pending updates should be cleared
    assert!(transformer.pending_tool_argument_updates.is_empty());
}

#[test]
fn serialized_agent_tool_use_emits_background_progress_hint() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_agent_1",
                "type": "function_call",
                "call_id": "call_agent_1",
                "name": "Agent"
            }
        })
    );
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_agent_1",
                "type": "function_call",
                "call_id": "call_agent_1",
                "name": "Agent",
                "arguments": r#"{"description":"Search remote Claude Code tools","prompt":"Find remote wrappers","run_in_background":true}"#
            }
        })
    );

    let mut events = transformer.transform_sse_line(&tool_added);
    events.extend(transformer.transform_sse_line(&tool_done));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("已启动后台 explorer：Search remote Claude Code tools"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
}

#[test]
fn serialized_task_output_tool_use_emits_polling_progress_hint() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_task_output_1",
                "type": "function_call",
                "call_id": "call_task_output_1",
                "name": "TaskOutput"
            }
        })
    );
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_task_output_1",
                "type": "function_call",
                "call_id": "call_task_output_1",
                "name": "TaskOutput",
                "arguments": r#"{"task_id":"task_123","block":false,"timeout":5000}"#
            }
        })
    );

    let mut events = transformer.transform_sse_line(&tool_added);
    events.extend(transformer.transform_sse_line(&tool_done));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("正在轮询后台任务结果"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
}

#[test]
fn retrieval_status_timeout_text_is_bridged_to_thinking_progress() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let timeout_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": r#"<retrieval_status>timeout</retrieval_status>

<system-reminder>ignore me</system-reminder>"#
        })
    );

    let joined = transformer.transform_sse_line(&timeout_line).join("");

    assert!(joined.contains("某个 explorer 仍在运行，我继续等待结果"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
    assert!(!joined.contains("retrieval_status"));
    assert!(!joined.contains("system-reminder"));
}

#[test]
fn task_notification_completion_text_is_bridged_to_thinking_progress() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let notification_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": r#"<task-notification>
<task-id>task_123</task-id>
<status>completed</status>
<summary>Agent "Search remote Claude Code tools" completed</summary>
</task-notification>
Full transcript available at: /tmp/task_123.output"#
        })
    );

    let joined = transformer.transform_sse_line(&notification_line).join("");

    assert!(joined.contains("后台 explorer 已完成：Search remote Claude Code tools"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
    assert!(!joined.contains("task-notification"));
    assert!(!joined.contains("Full transcript available"));
}

#[test]
fn task_output_running_text_is_suppressed_when_visible_thinking_disabled() {
    let mut transformer = TransformResponse::new_with_visible_thinking("gpt-5.3-codex", false);

    let running_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Task is still running…"
        })
    );

    let joined = transformer.transform_sse_line(&running_line).join("");

    assert!(
        !joined.contains("\"type\":\"thinking_delta\""),
        "thinking-disabled requests should suppress task progress thinking deltas"
    );
    assert!(
        !joined.contains("某个 explorer 仍在运行，我继续等待结果"),
        "suppressed task progress should not emit the rewritten waiting message"
    );
}

#[test]
fn task_output_running_text_is_bridged_to_thinking_progress() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let running_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Task is still running…"
        })
    );

    let joined = transformer.transform_sse_line(&running_line).join("");

    assert!(joined.contains("某个 explorer 仍在运行，我继续等待结果"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
    assert!(!joined.contains("Task is still running"));
}

#[test]
fn task_output_no_output_text_is_bridged_to_thinking_progress() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let no_output_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "No task output available"
        })
    );

    let joined = transformer.transform_sse_line(&no_output_line).join("");

    assert!(joined.contains("后台任务暂时还没有新输出，我继续等待"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
    assert!(!joined.contains("No task output available"));
}

#[test]
fn task_output_missing_task_text_is_bridged_to_thinking_progress() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let missing_task_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Error: No task found with ID: a20f0fd7a883a0782"
        })
    );

    let joined = transformer.transform_sse_line(&missing_task_line).join("");

    assert!(joined.contains("某个后台任务已结束或状态失效，我继续汇总现有结果"));
    assert!(joined.contains("\"type\":\"thinking_delta\""));
    assert!(!joined.contains("No task found with ID"));
}

#[test]
fn bash_tool_codex_exec_command_payload_is_normalized_before_emitting_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_bash_1",
                "type": "function_call",
                "call_id": "call_bash_1",
                "name": "Bash"
            }
        })
    );
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_bash_1",
                "type": "function_call",
                "call_id": "call_bash_1",
                "name": "Bash",
                "arguments": serde_json::to_string(&json!({
                    "cmd": "ls -la",
                    "workdir": "/tmp/demo dir",
                    "justification": "List project files",
                    "yield_time_ms": 1000,
                    "max_output_tokens": 4000
                })).unwrap()
            }
        })
    );

    let mut events = transformer.transform_sse_line(&tool_added);
    events.extend(transformer.transform_sse_line(&tool_done));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("\"name\":\"Bash\""));
    assert!(joined.contains(r#"\"command\":\"cd '/tmp/demo dir' && ls -la\""#));
    assert!(joined.contains(r#"\"description\":\"List project files\""#));
    assert!(!joined.contains(r#"\"cmd\":"#));
    assert!(!joined.contains(r#"\"workdir\":"#));
    assert!(!joined.contains(r#"\"yield_time_ms\":"#));
}

#[test]
fn bash_tool_argument_delta_does_not_stream_raw_exec_command_aliases() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_bash_stream_1",
                "type": "function_call",
                "call_id": "call_bash_stream_1",
                "name": "Bash"
            }
        })
    );
    let tool_args_delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "item_id": "fc_bash_stream_1",
            "call_id": "call_bash_stream_1",
            "delta": "{\"cmd\":\"ls -la\",\"workdir\":\"/tmp/demo dir\"}"
        })
    );

    let mut events = transformer.transform_sse_line(&tool_added);
    events.extend(transformer.transform_sse_line(&tool_args_delta));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("\"name\":\"Bash\""));
    assert!(!joined.contains(r#"\"cmd\":"#));
    assert!(!joined.contains(r#"\"workdir\":"#));
}

#[test]
fn task_output_defaults_are_filled_before_emitting_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_task_output_1",
                "type": "function_call",
                "call_id": "call_task_output_1",
                "name": "TaskOutput"
            }
        })
    );
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_task_output_1",
                "type": "function_call",
                "call_id": "call_task_output_1",
                "name": "TaskOutput",
                "arguments": serde_json::to_string(&json!({
                    "taskId": "task_123"
                })).unwrap()
            }
        })
    );

    let mut events = transformer.transform_sse_line(&tool_added);
    events.extend(transformer.transform_sse_line(&tool_done));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("\"name\":\"TaskOutput\""));
    assert!(joined.contains(r#"\"task_id\":\"task_123\""#));
    assert!(joined.contains(r#"\"block\":true"#));
    assert!(joined.contains(r#"\"timeout\":30000"#));
    assert!(!joined.contains(r#"\"taskId\":"#));
}

#[test]
fn edit_tool_aliases_are_normalized_before_emitting_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_edit_1",
                "type": "function_call",
                "call_id": "call_edit_1",
                "name": "Edit"
            }
        })
    );
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_edit_1",
                "type": "function_call",
                "call_id": "call_edit_1",
                "name": "Edit",
                "arguments": serde_json::to_string(&json!({
                    "filePath": "/tmp/a.txt",
                    "oldText": "a",
                    "newText": "b"
                })).unwrap()
            }
        })
    );

    let mut events = transformer.transform_sse_line(&tool_added);
    events.extend(transformer.transform_sse_line(&tool_done));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("\"name\":\"Edit\""));
    assert!(joined.contains(r#"\"file_path\":\"/tmp/a.txt\""#));
    assert!(joined.contains(r#"\"old_string\":\"a\""#));
    assert!(joined.contains(r#"\"new_string\":\"b\""#));
    assert!(!joined.contains(r#"\"filePath\":"#));
    assert!(!joined.contains(r#"\"oldText\":"#));
    assert!(!joined.contains(r#"\"newText\":"#));
}

#[test]
fn skill_tool_command_payload_is_normalized_before_emitting_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let tool_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_skill_1",
                "type": "function_call",
                "call_id": "call_skill_1",
                "name": "Skill"
            }
        })
    );
    let tool_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_skill_1",
                "type": "function_call",
                "call_id": "call_skill_1",
                "name": "Skill",
                "arguments": "{\"command\":\"review-pr 123\"}"
            }
        })
    );

    let mut events = transformer.transform_sse_line(&tool_added);
    events.extend(transformer.transform_sse_line(&tool_done));
    let joined = events.join("");

    assert!(joined.contains("\"type\":\"tool_use\""));
    assert!(joined.contains("\"name\":\"Skill\""));
    assert!(joined.contains(r#"\"skill\":\"review-pr\""#));
    assert!(joined.contains(r#"\"args\":\"123\""#));
    assert!(!joined.contains(r#"\"command\":\"review-pr 123\""#));
}
