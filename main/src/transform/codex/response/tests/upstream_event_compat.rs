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
