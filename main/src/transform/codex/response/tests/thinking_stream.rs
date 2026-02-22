use super::*;
#[test]
fn test_reasoning_summary_events_are_mapped_to_thinking_deltas() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line_part_added = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_part.added",
            "summary_index": 0
        })
    );
    let line_delta_1 = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "summary_index": 0,
            "delta": "先分析上下文。"
        })
    );
    let line_delta_2 = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "summary_index": 0,
            "delta": "再给结论。"
        })
    );
    let line_part_done = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_part.done",
            "summary_index": 0
        })
    );

    let joined = format!(
        "{}{}{}{}",
        transformer.transform_sse_line(&line_part_added).join(""),
        transformer.transform_sse_line(&line_delta_1).join(""),
        transformer.transform_sse_line(&line_delta_2).join(""),
        transformer.transform_sse_line(&line_part_done).join("")
    );

    let has_thinking_block_payload = joined
        .contains("\"content_block\":{\"thinking\":\"\",\"type\":\"thinking\"}")
        || joined.contains("\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}");
    assert!(
        joined.contains("\"content_block_start\"") && has_thinking_block_payload,
        "reasoning summary should open a thinking block"
    );
    assert!(
        joined.contains("\"type\":\"thinking_delta\"")
            && joined.contains("\"thinking\":\"先分析上下文。\"")
            && joined.contains("\"thinking\":\"再给结论。\""),
        "reasoning summary deltas should be mapped to thinking_delta"
    );
    assert!(
        joined.contains("\"type\":\"content_block_stop\""),
        "reasoning summary part.done should close the thinking block"
    );
}

#[test]
fn test_reasoning_summary_block_closes_before_output_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line_part_added = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_part.added",
            "summary_index": 0
        })
    );
    let line_reasoning_delta = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "summary_index": 0,
            "delta": "推理中"
        })
    );
    let line_part_done = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_part.done",
            "summary_index": 0
        })
    );
    // Real API sends output_item.added for final answer message
    let line_message_item = format!(
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
    let line_text_delta = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "最终答案"
        })
    );

    let joined = format!(
        "{}{}{}{}{}",
        transformer.transform_sse_line(&line_part_added).join(""),
        transformer
            .transform_sse_line(&line_reasoning_delta)
            .join(""),
        transformer.transform_sse_line(&line_part_done).join(""),
        transformer
            .transform_sse_line(&line_message_item)
            .join(""),
        transformer.transform_sse_line(&line_text_delta).join("")
    );

    let pos_thinking_delta = joined
        .find("\"type\":\"thinking_delta\"")
        .expect("thinking_delta should exist");
    let pos_block_stop = joined[pos_thinking_delta..]
        .find("\"type\":\"content_block_stop\"")
        .map(|pos| pos + pos_thinking_delta)
        .expect("thinking block should be closed before switching to text");
    let pos_text_delta = joined
        .find("\"type\":\"text_delta\"")
        .expect("text_delta should exist");
    assert!(
        pos_block_stop < pos_text_delta,
        "thinking block must close before text delta is emitted"
    );
    assert!(
        joined.contains("\"text\":\"最终答案\""),
        "final answer text should still stream as text_delta"
    );
}

#[test]
fn test_response_completed_closes_open_thinking_block() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line_part_added = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_part.added",
            "summary_index": 0
        })
    );
    let line_reasoning_delta = format!(
        "data: {}",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "summary_index": 0,
            "delta": "仍在推理"
        })
    );
    let line_completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "usage": { "input_tokens": 10, "output_tokens": 20 }
            }
        })
    );

    let joined = format!(
        "{}{}{}",
        transformer.transform_sse_line(&line_part_added).join(""),
        transformer
            .transform_sse_line(&line_reasoning_delta)
            .join(""),
        transformer.transform_sse_line(&line_completed).join("")
    );
    assert!(
        joined.contains("\"type\":\"content_block_stop\""),
        "response.completed should close any open thinking block"
    );
    assert!(
        joined.contains("\"type\":\"message_delta\"")
            && joined.contains("\"type\":\"message_stop\""),
        "response.completed should still emit message_delta + message_stop"
    );
}
