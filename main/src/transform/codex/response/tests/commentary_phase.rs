use super::*;

#[test]
fn commentary_text_redirected_to_thinking_blocks() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // 1. Message item added with phase: commentary
    let item_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "message",
                "role": "assistant",
                "phase": "commentary"
            }
        })
    );
    let _ = transformer.transform_sse_line(&item_added);

    // 2. Text delta during commentary phase
    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Using the openspec-apply-change skill for this request."
        })
    );
    let events = transformer.transform_sse_line(&text_delta);
    let joined = events.join("");

    // Should be thinking, NOT text
    assert!(
        joined.contains("\"type\":\"thinking_delta\""),
        "commentary text should be emitted as thinking_delta, not text_delta"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "commentary text must not appear as text_delta"
    );
    assert!(
        joined.contains("Using the openspec-apply-change skill"),
        "commentary content should be preserved in thinking block"
    );
}

#[test]
fn final_answer_text_remains_as_text_blocks() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // Message item with phase: final_answer
    let item_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "message",
                "role": "assistant",
                "phase": "final_answer"
            }
        })
    );
    let _ = transformer.transform_sse_line(&item_added);

    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Here is your answer."
        })
    );
    let events = transformer.transform_sse_line(&text_delta);
    let joined = events.join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "final_answer text should be emitted as text_delta"
    );
    assert!(
        !joined.contains("\"type\":\"thinking_delta\""),
        "final_answer text must not be redirected to thinking"
    );
}

#[test]
fn no_phase_text_defaults_to_text_blocks() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // Message item without phase field (legacy model compatibility)
    let item_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "message",
                "role": "assistant"
            }
        })
    );
    let _ = transformer.transform_sse_line(&item_added);

    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Normal text output."
        })
    );
    let events = transformer.transform_sse_line(&text_delta);
    let joined = events.join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "text without phase should default to text_delta"
    );
}

#[test]
fn commentary_then_tool_call_produces_thinking_plus_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // 1. Commentary message item
    let commentary_item = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "message",
                "role": "assistant",
                "phase": "commentary"
            }
        })
    );
    transformer.transform_sse_line(&commentary_item);

    // 2. Commentary text
    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Let me read this file first."
        })
    );
    let text_events = transformer.transform_sse_line(&text_delta);

    // 3. Commentary item done
    let item_done = format!(
        "data: {}",
        json!({ "type": "response.output_item.done" })
    );
    let done_events = transformer.transform_sse_line(&item_done);

    // 4. Function call item
    let fc_item = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "call_id": "call_123",
                "name": "Read"
            }
        })
    );
    let fc_events = transformer.transform_sse_line(&fc_item);

    // 5. Function call args
    let fc_args = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "delta": "{\"file_path\":\"/tmp/a.txt\"}"
        })
    );
    let arg_events = transformer.transform_sse_line(&fc_args);

    let all_joined = format!(
        "{}{}{}{}",
        text_events.join(""),
        done_events.join(""),
        fc_events.join(""),
        arg_events.join("")
    );

    // Commentary should be thinking
    assert!(
        all_joined.contains("\"type\":\"thinking_delta\"")
            && all_joined.contains("Let me read this file first."),
        "commentary text should appear as thinking_delta"
    );

    // Tool call should be present
    assert!(
        all_joined.contains("\"type\":\"tool_use\"") && all_joined.contains("\"name\":\"Read\""),
        "function_call should produce tool_use block"
    );

    // No text_delta should exist
    assert!(
        !all_joined.contains("\"type\":\"text_delta\""),
        "commentary text must not leak as text_delta"
    );
}

#[test]
fn commentary_phase_resets_after_item_done() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // Commentary phase
    let commentary_item = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "message",
                "role": "assistant",
                "phase": "commentary"
            }
        })
    );
    transformer.transform_sse_line(&commentary_item);
    assert!(transformer.in_commentary_phase);

    // Item done resets
    let item_done = format!(
        "data: {}",
        json!({ "type": "response.output_item.done" })
    );
    transformer.transform_sse_line(&item_done);
    assert!(!transformer.in_commentary_phase);
}

#[test]
fn text_without_preceding_item_added_defaults_to_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // Text delta arrives without any preceding output_item.added
    // (e.g., older API version or direct text streaming)
    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Direct text without item metadata."
        })
    );
    let events = transformer.transform_sse_line(&text_delta);
    let joined = events.join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "text without item metadata should default to text_delta for backward compatibility"
    );
}

// ─── Fallback detection: reasoning presence without message item_added ────

#[test]
fn text_after_reasoning_without_item_added_redirected_to_thinking() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // 1. Reasoning summary (sets had_reasoning_in_response = true)
    let reasoning_part = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_part.added",
            "part": { "type": "summary_text" }
        })
    );
    transformer.transform_sse_line(&reasoning_part);

    let reasoning_delta = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": "Thinking about the problem..."
        })
    );
    transformer.transform_sse_line(&reasoning_delta);

    let reasoning_done = format!(
        "data: {}",
        json!({ "type": "response.reasoning_summary_part.done" })
    );
    transformer.transform_sse_line(&reasoning_done);

    // 2. Text delta arrives WITHOUT any output_item.added for message
    //    (Codex API sometimes omits it — the key bug this fallback fixes)
    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Using the openspec-apply-change skill for this request."
        })
    );
    let events = transformer.transform_sse_line(&text_delta);
    let joined = events.join("");

    // Should be redirected to thinking via fallback
    assert!(
        joined.contains("\"type\":\"thinking_delta\""),
        "text after reasoning without item_added should be redirected to thinking_delta"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "text after reasoning without item_added must not appear as text_delta"
    );
    assert!(
        joined.contains("Using the openspec-apply-change skill"),
        "fallback commentary content should be preserved in thinking block"
    );
}

#[test]
fn text_after_reasoning_with_final_answer_item_added_remains_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // 1. Reasoning summary
    let reasoning_delta = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": "Thinking..."
        })
    );
    transformer.transform_sse_line(&reasoning_delta);

    let reasoning_part_done = format!(
        "data: {}",
        json!({ "type": "response.reasoning_summary_part.done" })
    );
    transformer.transform_sse_line(&reasoning_part_done);

    // 2. Message item_added with phase: final_answer
    //    (saw_message_item_added = true, overrides fallback)
    let item_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "message",
                "role": "assistant",
                "phase": "final_answer"
            }
        })
    );
    transformer.transform_sse_line(&item_added);

    // 3. Text delta — should be normal text, NOT thinking
    let text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Here is the final answer."
        })
    );
    let events = transformer.transform_sse_line(&text_delta);
    let joined = events.join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "final_answer text after reasoning should remain as text_delta"
    );
    assert!(
        !joined.contains("\"type\":\"thinking_delta\""),
        "final_answer text must not be redirected to thinking"
    );
}

#[test]
fn fallback_commentary_then_tool_call_no_text_leak() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // 1. Reasoning summary
    let reasoning = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": "Let me analyze..."
        })
    );
    transformer.transform_sse_line(&reasoning);

    let reasoning_done = format!(
        "data: {}",
        json!({ "type": "response.reasoning_summary_part.done" })
    );
    transformer.transform_sse_line(&reasoning_done);

    // 2. Text delta WITHOUT output_item.added (fallback commentary)
    let text1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Let me read "
        })
    );
    let text2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "the file first."
        })
    );
    let ev1 = transformer.transform_sse_line(&text1);
    let ev2 = transformer.transform_sse_line(&text2);

    // 3. Function call arrives
    let fc_item = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "call_id": "call_456",
                "name": "Read"
            }
        })
    );
    let fc_events = transformer.transform_sse_line(&fc_item);

    let fc_args = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "delta": "{\"file_path\":\"/tmp/b.txt\"}"
        })
    );
    let arg_events = transformer.transform_sse_line(&fc_args);

    let all = format!(
        "{}{}{}{}",
        ev1.join(""),
        ev2.join(""),
        fc_events.join(""),
        arg_events.join("")
    );

    // Commentary redirected to thinking
    assert!(
        all.contains("\"type\":\"thinking_delta\""),
        "fallback commentary should produce thinking_delta"
    );
    // Tool call present
    assert!(
        all.contains("\"type\":\"tool_use\"") && all.contains("\"name\":\"Read\""),
        "function_call should produce tool_use block"
    );
    // No text_delta leak
    assert!(
        !all.contains("\"type\":\"text_delta\""),
        "fallback commentary must not leak as text_delta"
    );
}

#[test]
fn consecutive_fallback_commentary_deltas_share_thinking_block() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // Reasoning to activate fallback
    let reasoning = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": "Thinking..."
        })
    );
    transformer.transform_sse_line(&reasoning);

    let reasoning_done = format!(
        "data: {}",
        json!({ "type": "response.reasoning_summary_part.done" })
    );
    transformer.transform_sse_line(&reasoning_done);

    // Multiple text deltas without output_item.added
    let delta1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Part 1. "
        })
    );
    let delta2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Part 2. "
        })
    );
    let delta3 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Part 3."
        })
    );

    let ev1 = transformer.transform_sse_line(&delta1);
    let ev2 = transformer.transform_sse_line(&delta2);
    let ev3 = transformer.transform_sse_line(&delta3);

    let all = format!("{}{}{}", ev1.join(""), ev2.join(""), ev3.join(""));

    // All should be thinking_delta
    assert!(
        !all.contains("\"type\":\"text_delta\""),
        "consecutive fallback commentary must not produce text_delta"
    );

    // Should have exactly 1 content_block_start (thinking), NOT 3
    let block_start_count = all.matches("\"type\":\"content_block_start\"").count();
    assert_eq!(
        block_start_count, 1,
        "consecutive commentary deltas should share a single thinking block, got {} block starts",
        block_start_count
    );

    // Should have 3 thinking_delta events
    let thinking_delta_count = all.matches("\"type\":\"thinking_delta\"").count();
    assert_eq!(
        thinking_delta_count, 3,
        "should have 3 thinking_delta events, got {}",
        thinking_delta_count
    );
}
