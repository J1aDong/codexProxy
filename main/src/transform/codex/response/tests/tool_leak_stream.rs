use super::*;
#[test]
fn test_leaked_tool_text_is_promoted_to_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "assistant to=multi_tool_use.parallel {\"tool_uses\":[{\"recipient_name\":\"functions.Write\",\"parameters\":{\"file_path\":\"/tmp/a.ts\"}},{\"recipient_name\":\"functions.Read\",\"parameters\":{\"file_path\":\"/tmp/b.ts\"}}]}\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    let tool_use_count = joined.matches("\"type\":\"tool_use\"").count();
    assert!(
        tool_use_count == 2,
        "parallel leaked tool text should be split into 2 tool_use blocks, got {}",
        tool_use_count
    );
    assert!(
        joined.contains("\"name\":\"Write\"") && joined.contains("\"name\":\"Read\""),
        "parallel leaked tool targets should be normalized into concrete tool names"
    );
    assert!(
        joined.contains("\\\"file_path\\\":\\\"/tmp/a.ts\\\"")
            && joined.contains("\\\"file_path\\\":\\\"/tmp/b.ts\\\""),
        "split tool_use blocks should preserve parameters for each leaked call"
    );
}

#[test]
fn test_leaked_tool_suffix_keeps_prefix_text_visible() {
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
        "prefix text before leaked tool marker should stay visible"
    );
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Edit\""),
        "leaked tool suffix should still be promoted to tool_use"
    );
    assert!(
        !joined.contains("assistant to=functions.Edit"),
        "leaked tool marker should not appear in visible text output"
    );
}

#[test]
fn test_leaked_tool_name_compat_map_and_fallback() {
    assert_eq!(
        TransformResponse::normalize_leaked_tool_name("functions.Write"),
        "Write"
    );
    assert_eq!(
        TransformResponse::normalize_leaked_tool_name("functions.Bash"),
        "Bash"
    );
    assert_eq!(
        TransformResponse::normalize_leaked_tool_name("functions.exec_command"),
        "exec_command",
        "unknown functions.* names should fall back to prefix stripping"
    );
    assert_eq!(
        TransformResponse::normalize_leaked_tool_name("multi_tool_use.parallel"),
        "multi_tool_use.parallel",
        "non-functions names should be kept unchanged unless explicitly mapped"
    );
}

#[test]
fn test_malformed_parallel_leak_is_dropped() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "assistant to=multi_tool_use.parallel մեկնաբանություն\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "malformed parallel leak should not fabricate a tool_use block"
    );
    assert!(
        !joined.contains("մեկնաբանություն"),
        "leaked tool line suffix should not appear in visible text output"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "malformed parallel leak should be dropped from visible text"
    );
}

#[test]
fn test_leaked_functions_tool_line_without_assistant_prefix_is_promoted() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "numerusform to=functions.Bash {\"command\":\"pwd\",\"description\":\"Check cwd\"}\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Bash\""),
        "functions leak should be promoted even without assistant prefix"
    );
    assert!(
        joined.contains("\\\"command\\\":\\\"pwd\\\"")
            && joined.contains("\\\"description\\\":\\\"Check cwd\\\""),
        "valid leaked json payload should be forwarded as tool arguments"
    );
    assert!(
        !joined.contains("to=functions.Bash"),
        "functions leak marker should not appear in visible assistant output"
    );
}

#[test]
fn test_leaked_functions_tool_line_split_across_chunks_is_promoted() {
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
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Read\""),
        "split leaked functions line should still be promoted to tool_use"
    );
    assert!(
        joined.contains("\\\"file_path\\\":\\\"/tmp/a.txt\\\""),
        "split leaked json payload should be forwarded as tool arguments"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "split leaked line should not fall through to text output"
    );
}

#[test]
fn test_split_marker_across_chunks_keeps_prefix_and_promotes_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "先做这个 assistant t"
        })
    );
    let line_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "o=functions.Read {\"file_path\":\"/tmp/chunk.txt\"}\n"
        })
    );

    let events_1 = transformer.transform_sse_line(&line_1);
    let events_2 = transformer.transform_sse_line(&line_2);
    let joined = format!("{}{}", events_1.join(""), events_2.join(""));

    assert!(
        joined.contains("\"type\":\"text_delta\"") && joined.contains("\"text\":\"先做这个 \""),
        "prefix text should remain visible even when marker is split across chunks"
    );
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Read\""),
        "split marker across chunks should still be promoted to tool_use"
    );
    assert!(
        !joined.contains("assistant to=functions.Read"),
        "leaked marker text should not appear in visible output"
    );
}

#[test]
fn test_leaked_tool_text_from_content_part_added_is_promoted_to_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.content_part.added",
            "part": {
                "type": "output_text",
                "text": "assistant to=functions.Write {\"file_path\":\"/tmp/design.md\"}\n"
            }
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Write\""),
        "content_part leak should be promoted to tool_use"
    );
    assert!(
        joined.contains("\\\"file_path\\\":\\\"/tmp/design.md\\\""),
        "content_part leaked json payload should be preserved as tool arguments"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "promoted content_part leak should not emit text_delta"
    );
}

#[test]
fn test_leaked_functions_tool_line_split_across_mixed_events_is_promoted() {
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
            "type": "response.content_part.added",
            "part": {
                "type": "output_text",
                "text": "{\"file_path\":\"/tmp/mixed.txt\"}\n"
            }
        })
    );

    let events_1 = transformer.transform_sse_line(&line_1);
    let events_2 = transformer.transform_sse_line(&line_2);
    let joined = format!("{}{}", events_1.join(""), events_2.join(""));
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Read\""),
        "mixed-event leaked functions line should still be promoted to tool_use"
    );
    assert!(
        joined.contains("\\\"file_path\\\":\\\"/tmp/mixed.txt\\\""),
        "mixed-event leaked json payload should be forwarded as tool arguments"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "mixed-event leaked line should not fall through to text output"
    );
}

#[test]
fn test_parallel_leak_with_partial_invalid_entries_keeps_valid_calls() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "assistant to=multi_tool_use.parallel {\"tool_uses\":[{\"recipient_name\":\"functions.Write\",\"parameters\":{\"file_path\":\"/tmp/design.md\"}},{\"parameters\":{\"foo\":\"bar\"}},{\"recipient_name\":\"functions.Bash\",\"parameters\":{\"command\":\"pwd\"}}]}\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    let tool_use_count = joined.matches("\"type\":\"tool_use\"").count();
    assert_eq!(
        tool_use_count, 2,
        "only valid tool_uses should be emitted as tool_use blocks"
    );
    assert!(
        joined.contains("\"name\":\"Write\"") && joined.contains("\"name\":\"Bash\""),
        "valid entries should be preserved and normalized"
    );
    assert!(
        joined.contains("\\\"file_path\\\":\\\"/tmp/design.md\\\"")
            && joined.contains("\\\"command\\\":\\\"pwd\\\""),
        "valid parameters should be forwarded to emitted tool_use blocks"
    );
}

#[test]
fn test_malformed_functions_leak_is_dropped() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "assistant to=functions.Write {\"file_path\":\"/tmp/a.ts\"\n"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "malformed functions leak should not emit tool_use"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "malformed functions leak should not be shown as plain text"
    );
}
