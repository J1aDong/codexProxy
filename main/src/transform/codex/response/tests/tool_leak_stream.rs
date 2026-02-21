use super::*;

#[test]
fn leaked_tool_text_is_suppressed_not_promoted() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "assistant to=functions.Write {\"file_path\":\"/tmp/a.ts\"}\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "leaked tool text must not be promoted into tool_use"
    );
    assert!(
        !joined.contains("assistant to=functions.Write"),
        "leaked marker must not appear in visible output"
    );
}

#[test]
fn leaked_tool_suffix_keeps_prefix_text_visible() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "先修正 loader。 assistant to=functions.Edit {\"file_path\":\"/tmp/loader.ts\"}\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"text_delta\"")
            && joined.contains("\"text\":\"先修正 loader。 \""),
        "prefix text before leaked marker should remain visible"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "leaked suffix must not be promoted to tool_use"
    );
}

#[test]
fn split_leaked_tool_line_across_chunks_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "to=functions.Read "
        })
    );
    let line_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "{\"file_path\":\"/tmp/a.txt\"}\n"
        })
    );

    let events_1 = transformer.transform_sse_line(&line_1);
    let events_2 = transformer.transform_sse_line(&line_2);
    let joined = format!("{}{}", events_1.join(""), events_2.join(""));

    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "split leaked tool line must not create tool_use"
    );
    assert!(
        !joined.contains("to=functions.Read") && !joined.contains("/tmp/a.txt"),
        "split leaked fragments must be hidden from visible text"
    );
}

#[test]
fn raw_parallel_tool_json_without_marker_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Reviewing specs first。 {\"tool_uses\":[{\"recipient_name\":\"functions.Read\",\"parameters\":{\"file_path\":\"/tmp/spec-a.md\"}}]}"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"text_delta\"")
            && joined.contains("\"text\":\"Reviewing specs first。 \""),
        "normal prefix text should stay visible"
    );
    assert!(
        !joined.contains("\"recipient_name\"") && !joined.contains("\"type\":\"tool_use\""),
        "raw leaked tool json must be hidden and not promoted"
    );
}

#[test]
fn structured_function_call_events_still_produce_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "call_id": "call_1",
                "name": "Read"
            }
        })
    );
    let delta_line = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "delta": "{\"file_path\":\"/tmp/a.txt\"}"
        })
    );
    let done_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done"
        })
    );

    let mut events = transformer.transform_sse_line(&add_line);
    events.extend(transformer.transform_sse_line(&delta_line));
    events.extend(transformer.transform_sse_line(&done_line));

    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Read\""),
        "structured function_call must produce tool_use"
    );
    assert!(
        joined.contains("\"type\":\"input_json_delta\"")
            && joined.contains("\\\"file_path\\\":\\\"/tmp/a.txt\\\""),
        "structured function_call arguments must be preserved"
    );
}

#[test]
fn markdown_bash_interception_still_works() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Let me run this.\n```bash\necho \"hello\"\n"
        })
    );
    let line_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "```\nDone."
        })
    );

    let mut events = transformer.transform_sse_line(&line_1);
    events.extend(transformer.transform_sse_line(&line_2));

    let completed_line = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": { "status": "completed" }
        })
    );
    events.extend(transformer.transform_sse_line(&completed_line));

    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Bash\""),
        "markdown bash blocks should be converted into Bash tool calls"
    );
    assert!(
        joined.contains("\"type\":\"text_delta\"")
            && joined.contains("\"text\":\"Let me run this.\\n\""),
        "prefix text before markdown block should remain visible"
    );
}
