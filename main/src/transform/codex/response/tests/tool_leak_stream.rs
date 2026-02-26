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
fn partial_marker_then_tool_json_is_fully_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // Marker starts, but does not finish with newline in this chunk.
    let chunk1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "**Aligning document chat model path**assistant to=functions.Edit"
        })
    );

    // Next chunk is raw tool JSON tail (same leak pattern seen in logs)
    let chunk2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": " {\"file_path\":\"/tmp/a.ts\",\"new_string\":\"x\",\"old_string\":\"y\",\"replace_all\":false}"
        })
    );

    // End of text part
    let done = format!(
        "data: {}",
        json!({
            "type": "response.content_part.done",
            "part": {"type": "output_text"}
        })
    );

    let e1 = transformer.transform_sse_line(&chunk1);
    let e2 = transformer.transform_sse_line(&chunk2);
    let e3 = transformer.transform_sse_line(&done);
    let joined = format!("{}{}{}", e1.join(""), e2.join(""), e3.join(""));

    // Should keep only normal prefix text, suppress marker/json leakage completely.
    assert!(
        joined.contains("\"type\":\"text_delta\"")
            && joined.contains("Aligning document chat model path"),
        "normal prefix text should remain visible"
    );
    assert!(
        !joined.contains("assistant to=functions.Edit")
            && !joined.contains("\"file_path\"")
            && !joined.contains("\"new_string\"")
            && !joined.contains("\"old_string\"")
            && !joined.contains("\"replace_all\""),
        "leaked marker and tool json tail must be fully suppressed"
    );
}

#[test]
fn raw_edit_json_without_tool_marker_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Preparing edit. {\"file_path\":\"/tmp/mod.ts\",\"new_string\":\"a\",\"old_string\":\"b\",\"replace_all\":false}"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"text_delta\"") && joined.contains("Preparing edit. "),
        "plain prefix should remain visible"
    );
    assert!(
        !joined.contains("\"file_path\"")
            && !joined.contains("\"new_string\"")
            && !joined.contains("\"old_string\"")
            && !joined.contains("\"replace_all\""),
        "raw edit json should be suppressed"
    );
}

#[test]
fn raw_exec_command_json_without_marker_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Rebuilding Tailwind and verifying border****Rebuilding Tailwind and verifying border ####json {\"command\":\"npx --prefix /tmp/demo tailwindcss -i /tmp/in.css -o /tmp/out.css\",\"description\":\"Rebuild Tailwind CSS\",\"timeout\":600000}"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    let visible_repeat_count = joined
        .matches("Rebuilding Tailwind and verifying border")
        .count();
    assert_eq!(
        visible_repeat_count, 1,
        "duplicated stitched prefix should collapse into a single readable sentence"
    );
    assert!(
        !joined.contains("\\\"command\\\"")
            && !joined.contains("\\\"description\\\"")
            && !joined.contains("\\\"timeout\\\""),
        "raw exec-command tool args json should be suppressed from visible text"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "suppressed leaked exec-command json must not create tool_use blocks"
    );
}

#[test]
fn raw_taskoutput_json_without_marker_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "**Checking debug output for failing path**numerusform{\"block\":true,\"task_id\":\"bf6ea6d\",\"timeout\":20000}{\"block\":true,\"task_id\":\"bf6ea6d\",\"timeout\":20000}"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    assert!(
        joined.contains("Checking debug output for failing path"),
        "normal prefix text should remain visible"
    );
    assert!(
        !joined.contains("\\\"task_id\\\"")
            && !joined.contains("\\\"timeout\\\"")
            && !joined.contains("numerusform"),
        "raw task-output args json tail and connector noise should be suppressed"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "suppressed task-output args json must not create tool_use blocks"
    );
}

#[test]
fn raw_read_json_without_marker_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Removing obsolete payload branch +#+#+#+#+#+{\"file_path\":\"/Users/mr.j/myRoom/YAT/yat_commad_check/index.html\",\"offset\":260,\"limit\":220}{\"file_path\":\"/Users/mr.j/myRoom/YAT/yat_commad_check/index.html\",\"offset\":260,\"limit\":220}"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    assert!(
        joined.contains("Removing obsolete payload branch"),
        "normal prefix text should remain visible"
    );
    assert!(
        !joined.contains("\\\"file_path\\\"")
            && !joined.contains("\\\"offset\\\"")
            && !joined.contains("\\\"limit\\\""),
        "raw read-args json tail should be suppressed from visible text"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "suppressed read-args json must not create tool_use blocks"
    );
}

#[test]
fn same_chunk_natural_language_json_and_suffix_only_suppresses_tool_json() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "先分析。 {\"file_path\":\"/tmp/mod.ts\",\"new_string\":\"x\",\"old_string\":\"y\",\"replace_all\":false} 再继续。"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    assert!(
        joined.contains("先分析。 "),
        "prefix natural language should remain visible"
    );
    assert!(
        joined.contains("再继续。"),
        "suffix natural language should remain visible"
    );
    assert!(
        !joined.contains("\"file_path\"")
            && !joined.contains("\"new_string\"")
            && !joined.contains("\"old_string\"")
            && !joined.contains("\"replace_all\""),
        "tool json payload should be fully suppressed"
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
fn function_call_arguments_done_without_delta_is_still_streamed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_1",
                "type": "function_call",
                "call_id": "call_done_only",
                "name": "Edit"
            }
        })
    );
    let done_args_line = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.done",
            "output_index": 0,
            "item_id": "fc_1",
            "arguments": "{\"file_path\":\"/tmp/done-only.ts\"}"
        })
    );
    let item_done_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_1",
                "type": "function_call",
                "call_id": "call_done_only",
                "name": "Edit",
                "arguments": "{\"file_path\":\"/tmp/done-only.ts\"}"
            }
        })
    );

    let mut events = transformer.transform_sse_line(&add_line);
    events.extend(transformer.transform_sse_line(&done_args_line));
    events.extend(transformer.transform_sse_line(&item_done_line));

    let joined = events.join("");
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"id\":\"call_done_only\""),
        "function call block should still be created"
    );
    assert!(
        joined.contains("\"type\":\"input_json_delta\"") && joined.contains("done-only.ts"),
        "arguments.done should still be converted into input_json_delta"
    );
}

#[test]
fn orphan_delta_before_output_item_added_is_replayed_after_binding() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let orphan_delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 3,
            "item_id": "fc_late_bind",
            "call_id": "call_late_bind",
            "delta": "{\"file_path\":\"/tmp/late-bind.ts\"}"
        })
    );
    let add_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 3,
            "item": {
                "id": "fc_late_bind",
                "type": "function_call",
                "call_id": "call_late_bind",
                "name": "Edit"
            }
        })
    );
    let done_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 3,
            "item": {
                "id": "fc_late_bind",
                "type": "function_call",
                "call_id": "call_late_bind",
                "name": "Edit"
            }
        })
    );

    let orphan_events = transformer.transform_sse_line(&orphan_delta).join("");
    let _ = transformer.transform_sse_line(&add_line);
    let done_events = transformer.transform_sse_line(&done_line).join("");

    assert!(
        !orphan_events.contains("late-bind.ts"),
        "orphan delta should stay buffered until it can be bound to a function_call"
    );
    assert!(
        done_events.contains("\"type\":\"tool_use\"")
            && done_events.contains("\"id\":\"call_late_bind\""),
        "late-bound function_call should still produce tool_use"
    );
    assert!(
        done_events.contains("late-bind.ts"),
        "queued delta should replay once the function_call binding exists"
    );
}

#[test]
fn orphan_done_arguments_before_output_item_added_is_replayed_after_binding() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let orphan_done_args = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.done",
            "output_index": 5,
            "item_id": "fc_late_done",
            "call_id": "call_late_done",
            "arguments": "{\"file_path\":\"/tmp/late-done.ts\"}"
        })
    );
    let add_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 5,
            "item": {
                "id": "fc_late_done",
                "type": "function_call",
                "call_id": "call_late_done",
                "name": "Read"
            }
        })
    );
    let done_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 5,
            "item": {
                "id": "fc_late_done",
                "type": "function_call",
                "call_id": "call_late_done",
                "name": "Read"
            }
        })
    );

    let orphan_events = transformer.transform_sse_line(&orphan_done_args).join("");
    let _ = transformer.transform_sse_line(&add_line);
    let done_events = transformer.transform_sse_line(&done_line).join("");

    assert!(
        !orphan_events.contains("late-done.ts"),
        "orphan done-arguments should be buffered until function_call exists"
    );
    assert!(
        done_events.contains("\"type\":\"tool_use\"")
            && done_events.contains("\"id\":\"call_late_done\""),
        "late-bound function_call should still produce tool_use"
    );
    assert!(
        done_events.contains("late-done.ts"),
        "queued done-arguments snapshot should replay once binding exists"
    );
}

#[test]
fn duplicate_active_call_id_is_idempotent_and_does_not_create_extra_tool_use() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_first = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_dup_first",
                "type": "function_call",
                "call_id": "call_duplicate_live",
                "name": "Edit"
            }
        })
    );
    let add_duplicate = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_dup_second",
                "type": "function_call",
                "call_id": "call_duplicate_live",
                "name": "Edit"
            }
        })
    );
    let done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_dup_first",
                "type": "function_call",
                "call_id": "call_duplicate_live",
                "name": "Edit"
            }
        })
    );
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": { "status": "completed" }
        })
    );

    let _ = transformer.transform_sse_line(&add_first);
    let _ = transformer.transform_sse_line(&add_duplicate);
    let mut events = transformer.transform_sse_line(&done);
    events.extend(transformer.transform_sse_line(&completed));
    let joined = events.join("");

    let tool_use_count = joined.matches("\"type\":\"tool_use\"").count();
    assert_eq!(
        tool_use_count, 1,
        "duplicate active call_id should be treated as idempotent and not emit duplicate tool_use"
    );
}

#[test]
fn call_id_precedence_prevents_output_index_conflict_argument_hijack() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_first = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_conflict_first",
                "type": "function_call",
                "call_id": "call_conflict_first",
                "name": "Edit"
            }
        })
    );
    let add_second_conflict = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_conflict_second",
                "type": "function_call",
                "call_id": "call_conflict_second",
                "name": "Edit"
            }
        })
    );
    let delta_first = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "fc_conflict_first",
            "call_id": "call_conflict_first",
            "delta": "{\"file_path\":\"/tmp/conflict-first.ts\"}"
        })
    );
    let delta_second = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "fc_conflict_second",
            "call_id": "call_conflict_second",
            "delta": "{\"file_path\":\"/tmp/conflict-second.ts\"}"
        })
    );
    let done_first = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_conflict_first",
                "type": "function_call",
                "call_id": "call_conflict_first",
                "name": "Edit"
            }
        })
    );
    let done_second = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_conflict_second",
                "type": "function_call",
                "call_id": "call_conflict_second",
                "name": "Edit"
            }
        })
    );

    let _ = transformer.transform_sse_line(&add_first);
    let _ = transformer.transform_sse_line(&add_second_conflict);
    let _ = transformer.transform_sse_line(&delta_first);
    let _ = transformer.transform_sse_line(&delta_second);
    let mut events = transformer.transform_sse_line(&done_first);
    events.extend(transformer.transform_sse_line(&done_second));
    let joined = events.join("");

    assert!(
        joined.contains("\"id\":\"call_conflict_first\"")
            && joined.contains("\"id\":\"call_conflict_second\""),
        "both function calls should still be emitted"
    );

    let first_start = joined
        .find("\"id\":\"call_conflict_first\"")
        .expect("first call block should exist");
    let first_stop = joined[first_start..]
        .find("\"type\":\"content_block_stop\"")
        .map(|pos| first_start + pos)
        .expect("first call block should stop");
    let first_block = &joined[first_start..first_stop];

    let second_start = joined
        .find("\"id\":\"call_conflict_second\"")
        .expect("second call block should exist");
    let second_stop = joined[second_start..]
        .find("\"type\":\"content_block_stop\"")
        .map(|pos| second_start + pos)
        .expect("second call block should stop");
    let second_block = &joined[second_start..second_stop];

    assert!(
        first_block.contains("conflict-first.ts") && !first_block.contains("conflict-second.ts"),
        "first call block should keep its own arguments only"
    );
    assert!(
        second_block.contains("conflict-second.ts") && !second_block.contains("conflict-first.ts"),
        "second call block should keep its own arguments only"
    );
}

#[test]
fn call_id_reuse_after_close_is_dropped_in_same_response() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_first = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_reuse_first",
                "type": "function_call",
                "call_id": "call_reuse_once",
                "name": "Read"
            }
        })
    );
    let delta_first = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "fc_reuse_first",
            "call_id": "call_reuse_once",
            "delta": "{\"file_path\":\"/tmp/reuse-first.ts\"}"
        })
    );
    let done_first = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_reuse_first",
                "type": "function_call",
                "call_id": "call_reuse_once",
                "name": "Read"
            }
        })
    );
    let add_reused = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_reuse_second",
                "type": "function_call",
                "call_id": "call_reuse_once",
                "name": "Read"
            }
        })
    );
    let delta_reused = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "item_id": "fc_reuse_second",
            "call_id": "call_reuse_once",
            "delta": "{\"file_path\":\"/tmp/reuse-second.ts\"}"
        })
    );
    let done_reused = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_reuse_second",
                "type": "function_call",
                "call_id": "call_reuse_once",
                "name": "Read"
            }
        })
    );
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": { "status": "completed" }
        })
    );

    let mut events = transformer.transform_sse_line(&add_first);
    events.extend(transformer.transform_sse_line(&delta_first));
    events.extend(transformer.transform_sse_line(&done_first));
    events.extend(transformer.transform_sse_line(&add_reused));
    events.extend(transformer.transform_sse_line(&delta_reused));
    events.extend(transformer.transform_sse_line(&done_reused));
    events.extend(transformer.transform_sse_line(&completed));
    let joined = events.join("");

    let tool_use_count = joined.matches("\"type\":\"tool_use\"").count();
    assert_eq!(
        tool_use_count, 1,
        "reused call_id after closure should be ignored in the same response"
    );
    assert!(
        joined.contains("reuse-first.ts"),
        "first call payload should remain intact"
    );
    assert!(
        !joined.contains("reuse-second.ts"),
        "reused call payload should be dropped to avoid ambiguous lifecycle"
    );
}

#[test]
fn diagnostics_counters_track_defer_and_quarantine_paths() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_text_gate = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_diag_gate",
                "type": "function_call",
                "call_id": "call_diag_gate",
                "name": "Edit"
            }
        })
    );
    let unscoped_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "this text should be deferred while tool window is open"
        })
    );
    let done_text_gate = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_diag_gate",
                "type": "function_call",
                "call_id": "call_diag_gate",
                "name": "Edit"
            }
        })
    );
    let raw_leak = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Preparing edit. {\"file_path\":\"/tmp/mod.ts\",\"new_string\":\"a\",\"old_string\":\"b\",\"replace_all\":false}"
        })
    );
    let orphan_no_hint = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "delta": "{\"file_path\":\"/tmp/no-hint.ts\"}"
        })
    );
    let orphan_with_hint = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 9,
            "item_id": "fc_diag_orphan",
            "call_id": "call_diag_orphan",
            "delta": "{\"file_path\":\"/tmp/orphan.ts\"}"
        })
    );
    let add_orphan_binding = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 9,
            "item": {
                "id": "fc_diag_orphan",
                "type": "function_call",
                "call_id": "call_diag_orphan",
                "name": "Read"
            }
        })
    );
    let done_orphan_binding = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 9,
            "item": {
                "id": "fc_diag_orphan",
                "type": "function_call",
                "call_id": "call_diag_orphan",
                "name": "Read"
            }
        })
    );

    let _ = transformer.transform_sse_line(&add_text_gate);
    let _ = transformer.transform_sse_line(&unscoped_text);
    let _ = transformer.transform_sse_line(&done_text_gate);
    let _ = transformer.transform_sse_line(&raw_leak);
    let _ = transformer.transform_sse_line(&orphan_no_hint);
    let _ = transformer.transform_sse_line(&orphan_with_hint);
    let _ = transformer.transform_sse_line(&add_orphan_binding);
    let _ = transformer.transform_sse_line(&done_orphan_binding);

    assert!(
        transformer.diagnostics.deferred_unscoped_text_chunks >= 1,
        "deferred text chunks should be counted"
    );
    assert!(
        transformer.diagnostics.deferred_unscoped_text_flushes >= 1,
        "deferred text flushes should be counted"
    );
    assert!(
        transformer.diagnostics.dropped_raw_tool_json_fragments >= 1,
        "raw leaked tool json drops should be counted"
    );
    assert_eq!(
        transformer
            .diagnostics
            .dropped_orphan_tool_argument_updates_no_hint,
        1,
        "orphan tool-arg updates without routing hints should be counted"
    );
    assert!(
        transformer.diagnostics.queued_orphan_tool_argument_updates >= 1,
        "orphan tool-arg updates with routing hints should be queued"
    );
    assert!(
        transformer.diagnostics.applied_orphan_tool_argument_updates >= 1,
        "queued orphan tool-arg updates should be applied after binding appears"
    );
    assert!(
        transformer.diagnostics.has_activity(),
        "diagnostics summary should report non-zero activity"
    );
}

#[test]
fn interleaved_parallel_function_calls_keep_separate_tool_blocks() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "item_fc_1",
                "type": "function_call",
                "call_id": "call_parallel_1",
                "name": "Edit"
            }
        })
    );
    let delta_1 = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "item_fc_1",
            "delta": "{\"file_path\":\"/tmp/a.ts\"}"
        })
    );
    let add_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "item_fc_2",
                "type": "function_call",
                "call_id": "call_parallel_2",
                "name": "Edit"
            }
        })
    );
    let delta_2 = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "item_id": "item_fc_2",
            "delta": "{\"file_path\":\"/tmp/b.ts\"}"
        })
    );
    let done_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "item_fc_1",
                "type": "function_call",
                "call_id": "call_parallel_1",
                "name": "Edit",
                "arguments": "{\"file_path\":\"/tmp/a.ts\"}"
            }
        })
    );
    let done_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "item_fc_2",
                "type": "function_call",
                "call_id": "call_parallel_2",
                "name": "Edit",
                "arguments": "{\"file_path\":\"/tmp/b.ts\"}"
            }
        })
    );

    let mut events = transformer.transform_sse_line(&add_1);
    events.extend(transformer.transform_sse_line(&delta_1));
    events.extend(transformer.transform_sse_line(&add_2));
    events.extend(transformer.transform_sse_line(&delta_2));
    events.extend(transformer.transform_sse_line(&done_1));
    events.extend(transformer.transform_sse_line(&done_2));

    let joined = events.join("");
    let tool_use_count = joined.matches("\"type\":\"tool_use\"").count();
    assert_eq!(
        tool_use_count, 2,
        "parallel function calls should create separate tool_use blocks"
    );
    assert!(
        joined.contains("\"id\":\"call_parallel_1\"")
            && joined.contains("\"id\":\"call_parallel_2\""),
        "both call ids should be preserved"
    );
    assert!(
        joined.contains("a.ts") && joined.contains("b.ts"),
        "arguments should not be merged across parallel calls"
    );

    let start_1 = joined
        .find("\"id\":\"call_parallel_1\"")
        .expect("call_parallel_1 start must exist");
    let start_2 = joined
        .find("\"id\":\"call_parallel_2\"")
        .expect("call_parallel_2 start must exist");
    assert!(
        start_1 < start_2,
        "tool blocks must be emitted in deterministic order"
    );

    let first_stop_after_1 = joined[start_1..]
        .find("\"type\":\"content_block_stop\"")
        .map(|pos| start_1 + pos)
        .expect("first tool block stop must exist");
    assert!(
        first_stop_after_1 < start_2,
        "call_parallel_1 block must be fully closed before call_parallel_2 starts"
    );

    let a_pos = joined.find("a.ts").expect("a.ts args must exist");
    let b_pos = joined.find("b.ts").expect("b.ts args must exist");
    assert!(
        a_pos < start_2 && b_pos > start_2,
        "arguments must not be interleaved across tool blocks"
    );
}

#[test]
fn out_of_order_done_events_wait_for_head_then_flush_in_order() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_head",
                "type": "function_call",
                "call_id": "call_head",
                "name": "Edit"
            }
        })
    );
    let add_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_tail",
                "type": "function_call",
                "call_id": "call_tail",
                "name": "Edit"
            }
        })
    );
    let delta_1 = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "fc_head",
            "delta": "{\"file_path\":\"/tmp/head.ts\"}"
        })
    );
    let delta_2 = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 1,
            "item_id": "fc_tail",
            "delta": "{\"file_path\":\"/tmp/tail.ts\"}"
        })
    );
    let done_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_tail",
                "type": "function_call",
                "call_id": "call_tail",
                "name": "Edit",
                "arguments": "{\"file_path\":\"/tmp/tail.ts\"}"
            }
        })
    );
    let done_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_head",
                "type": "function_call",
                "call_id": "call_head",
                "name": "Edit",
                "arguments": "{\"file_path\":\"/tmp/head.ts\"}"
            }
        })
    );

    let _ = transformer.transform_sse_line(&add_1);
    let _ = transformer.transform_sse_line(&add_2);
    let _ = transformer.transform_sse_line(&delta_1);
    let _ = transformer.transform_sse_line(&delta_2);

    let done_2_events = transformer.transform_sse_line(&done_2).join("");
    assert!(
        !done_2_events.contains("\"type\":\"tool_use\""),
        "tail completion must wait for head completion before flushing"
    );

    let done_1_events = transformer.transform_sse_line(&done_1).join("");
    let tool_use_count = done_1_events.matches("\"type\":\"tool_use\"").count();
    assert_eq!(
        tool_use_count, 2,
        "once head completes, both buffered calls should flush in order"
    );
    let head_pos = done_1_events
        .find("\"id\":\"call_head\"")
        .expect("head call should exist");
    let tail_pos = done_1_events
        .find("\"id\":\"call_tail\"")
        .expect("tail call should exist");
    assert!(
        head_pos < tail_pos,
        "buffer flush order must follow tool queue order"
    );
    assert!(
        done_1_events.contains("head.ts") && done_1_events.contains("tail.ts"),
        "buffered arguments must remain attached to their original calls"
    );
}

#[test]
fn completed_event_flushes_incomplete_buffered_tool_call() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_incomplete",
                "type": "function_call",
                "call_id": "call_incomplete",
                "name": "Edit"
            }
        })
    );
    let delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "fc_incomplete",
            "delta": "{\"file_path\":\"/tmp/incomplete.ts\"}"
        })
    );
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": { "status": "completed" }
        })
    );

    let _ = transformer.transform_sse_line(&add);
    let _ = transformer.transform_sse_line(&delta);
    let completed_events = transformer.transform_sse_line(&completed).join("");

    assert!(
        completed_events.contains("\"type\":\"tool_use\"")
            && completed_events.contains("\"id\":\"call_incomplete\""),
        "response.completed must flush buffered tool_use blocks even if item.done is missing"
    );
    assert!(
        completed_events.contains("\"type\":\"input_json_delta\"")
            && completed_events.contains("incomplete.ts"),
        "response.completed flush must preserve buffered arguments"
    );
    assert!(
        completed_events.contains("\"type\":\"message_stop\""),
        "stream should still terminate with message_stop"
    );
}

#[test]
fn split_natural_language_json_suffix_across_chunks_preserves_safe_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let chunk1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "正在处理。 {\"file_path\":\"/tmp/a.ts\",\"new_string\":\"x\""
        })
    );
    let chunk2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": ",\"old_string\":\"y\",\"replace_all\":false} 完成。"
        })
    );

    let events1 = transformer.transform_sse_line(&chunk1);
    let events2 = transformer.transform_sse_line(&chunk2);
    let joined = format!("{}{}", events1.join(""), events2.join(""));

    assert!(
        joined.contains("正在处理。 "),
        "prefix text should remain visible across chunks"
    );
    assert!(
        joined.contains("完成。"),
        "suffix text should remain visible after json suppression"
    );
    assert!(
        !joined.contains("\"file_path\"")
            && !joined.contains("\"new_string\"")
            && !joined.contains("\"old_string\"")
            && !joined.contains("\"replace_all\""),
        "split leaked json should be suppressed"
    );
}

#[test]
fn log_sample_875_replay_suppresses_leaked_json_and_preserves_tool_pipeline() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let leaked_text = "**Aligning document chat model path****Aligning document chat model path**{\"file_path\":\"/Users/mr.j/myRoom/code/ai/MyProjects/Proma/apps/electron/src/main/lib/plugins/document-chat-bridge.ts\",\"new_string\":\"import { computeEmbedding, loadKnowledgeBaseIndex, resolvePluginModelOrThrow } from './ai-indexing-service'\\n\",\"old_string\":\"import { computeEmbedding, loadKnowledgeBaseIndex } from './ai-indexing-service'\",\"replace_all\":false}";

    let leaked_line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": leaked_text
        })
    );

    let add_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "call_id": "call_log_875",
                "name": "Edit"
            }
        })
    );

    let args_line = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "delta": "{\"file_path\":\"/tmp/from-structured-call.ts\",\"old_string\":\"a\",\"new_string\":\"b\",\"replace_all\":false}"
        })
    );

    let done_line = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done"
        })
    );

    let mut events = transformer.transform_sse_line(&leaked_line);
    events.extend(transformer.transform_sse_line(&add_line));
    events.extend(transformer.transform_sse_line(&args_line));
    events.extend(transformer.transform_sse_line(&done_line));

    let joined = events.join("");

    assert!(
        joined.contains("Aligning document chat model path"),
        "visible output should keep natural language prefix"
    );
    assert!(
        !joined.contains("/Users/mr.j/myRoom/code/ai/MyProjects/Proma/apps/electron/src/main/lib/plugins/document-chat-bridge.ts")
            && !joined.contains("resolvePluginModelOrThrow")
            && !joined.contains("\"old_string\":\"import { computeEmbedding, loadKnowledgeBaseIndex } from './ai-indexing-service'\"")
            && !joined.contains("\"replace_all\":false"),
        "leaked raw tool json keys from log sample should not be visible"
    );
    assert!(
        joined.contains("\"type\":\"tool_use\"") && joined.contains("\"name\":\"Edit\""),
        "structured function_call should still produce tool_use"
    );
    assert!(
        joined.contains("\"type\":\"input_json_delta\"")
            && joined.contains("from-structured-call.ts"),
        "structured function_call arguments should still stream as input_json_delta"
    );
}

#[test]
fn contextual_note_json_leak_with_suspicious_tail_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    // 模拟日志中的模式：**Re-running...** + ```json + note + 异常尾巴
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "**Re-running targeted tests after syntax fixes****Re-running targeted tests after syntax fixes**```json\n{\"note\":\"Running tool_leak_stream, text_hygiene, request_payload again now.\"}```numerusform  天天中彩票user "
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    println!("DEBUG: Input delta: {}", "**Re-running targeted tests after syntax fixes****Re-running targeted tests after syntax fixes**```json\n{\"note\":\"Running tool_leak_stream, text_hygiene, request_payload again now.\"}```numerusform  天天中彩票user ");
    println!("DEBUG: Output events: {}", joined);

    assert!(
        joined.contains("Re-running targeted tests after syntax fixes"),
        "normal prefix text should remain visible"
    );
    assert!(
        !joined.contains("\"note\"")
            && !joined.contains("tool_leak_stream")
            && !joined.contains("numerusform")
            && !joined.contains("天天中彩票user"),
        "contextual note-json leak and suspicious tail should be suppressed"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "contextual note-json must not create tool_use"
    );
}

#[test]
fn contextual_running_prefix_note_json_is_suppressed_and_prefix_deduped() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "**Running build verification****Running build verification**```json\n{\"note\":\"Running build verification now.\"}```ರಣ "
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    assert!(
        joined.contains("**Running build verification**"),
        "normal running prefix should remain visible"
    );
    assert!(
        !joined.contains("**Running build verification****Running build verification**"),
        "duplicated markdown bold prefix should be collapsed"
    );
    assert!(
        !joined.contains("\"note\"") && !joined.contains("ರಣ"),
        "contextual note-json and suspicious tail should be suppressed"
    );
}

#[test]
fn leaked_marker_suffix_running_note_json_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "assistant to=functions.exec_command {\"cmd\":\"npm run build:fe\"}\n**Running build verification****Running build verification**```json\n{\"note\":\"Running build verification now.\"}```ರಣ "
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    assert!(
        joined.contains("**Running build verification**"),
        "suffix readable prefix should remain visible after leaked marker is removed"
    );
    assert!(
        !joined.contains("assistant to=functions.exec_command")
            && !joined.contains("\"note\"")
            && !joined.contains("ರಣ"),
        "leaked marker line and note-json/tail noise should be suppressed"
    );
    assert!(
        !joined.contains("**Running build verification****Running build verification**"),
        "suffix duplicated markdown bold prefix should be collapsed"
    );
}

#[test]
fn split_contextual_note_json_across_chunks_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let chunk1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "**Re-running targeted tests**```json\n{\"note\":\"Running"
        })
    );

    let chunk2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": " tests now.\"}```numerusform user"
        })
    );

    let events1 = transformer.transform_sse_line(&chunk1);
    let events2 = transformer.transform_sse_line(&chunk2);
    let joined = format!("{}{}", events1.join(""), events2.join(""));

    println!("DEBUG: Chunk1: {}", chunk1);
    println!("DEBUG: Chunk2: {}", chunk2);
    println!("DEBUG: Events1: {:?}", events1);
    println!("DEBUG: Events2: {:?}", events2);
    println!("DEBUG: Joined output: {}", joined);

    assert!(
        joined.contains("Re-running targeted tests"),
        "prefix text should remain visible across chunks"
    );
    assert!(
        !joined.contains("\"note\"")
            && !joined.contains("Running tests now")
            && !joined.contains("numerusform"),
        "split contextual note-json leak should be suppressed"
    );
}

#[test]
fn legitimate_business_note_json_is_not_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "业务配置示例：{\"note\":\"用户偏好设置\",\"theme\":\"dark\",\"language\":\"zh\"}"
        })
    );

    let events = transformer.transform_sse_line(&line);
    let joined = events.join("");

    assert!(
        joined.contains("业务配置示例"),
        "business context should remain visible"
    );
    assert!(
        joined.contains("\\\"note\\\":\\\"用户偏好设置\\\""),
        "legitimate business note-json should not be suppressed"
    );
    assert!(
        !joined.contains("\"type\":\"tool_use\""),
        "business json must not create tool_use"
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

#[test]
fn user_reported_long_edit_payload_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let chunk1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Adding failing deep/standard assertions first****Adding failing deep/standard assertions first****{\"file_path\":\"/Users/mr.j/myRoom/code/ai/MyProj"
        })
    );
    let chunk2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "ects/Proma/apps/electron/src/main/lib/plugins/tests/ai-indexing-service.test.ts\",\"new_string\":\"  it('builds index and supports incremental reuse', async () => {\\n    const pluginId = 'plugin-ai-index-test'\\n  }\",\"old_string\":\"x\",\"replace_all\":false}"
        })
    );
    let done = format!(
        "data: {}",
        json!({
            "type": "response.output_text.done",
            "text": ""
        })
    );

    let e1 = transformer.transform_sse_line(&chunk1);
    let e2 = transformer.transform_sse_line(&chunk2);
    let e3 = transformer.transform_sse_line(&done);
    let joined = format!("{}{}{}", e1.join(""), e2.join(""), e3.join(""));

    assert!(
        joined.contains("Adding failing deep/standard assertions first"),
        "natural language prefix should stay visible"
    );
    assert!(
        !joined.contains("ai-indexing-service.test.ts")
            && !joined.contains("\"new_string\"")
            && !joined.contains("\"old_string\"")
            && !joined.contains("\"replace_all\""),
        "long leaked edit payload should be suppressed"
    );
}

#[test]
fn text_delta_bound_to_function_call_item_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_suppressed_text",
                "type": "function_call",
                "call_id": "call_suppressed_text",
                "name": "Edit"
            }
        })
    );
    let leaked_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "output_index": 0,
            "item_id": "fc_suppressed_text",
            "delta": "SHOULD_NOT_LEAK"
        })
    );
    let delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "output_index": 0,
            "item_id": "fc_suppressed_text",
            "delta": "{\"file_path\":\"/tmp/safe.ts\"}"
        })
    );
    let done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_suppressed_text",
                "type": "function_call",
                "call_id": "call_suppressed_text",
                "name": "Edit"
            }
        })
    );

    let mut events = transformer.transform_sse_line(&add);
    events.extend(transformer.transform_sse_line(&leaked_text));
    events.extend(transformer.transform_sse_line(&delta));
    events.extend(transformer.transform_sse_line(&done));
    let joined = events.join("");

    assert!(
        joined.contains("\"type\":\"tool_use\""),
        "function call should still be converted to tool_use"
    );
    assert!(
        !joined.contains("SHOULD_NOT_LEAK"),
        "text chunks scoped to function_call items must stay hidden"
    );
    assert!(
        joined.contains("safe.ts"),
        "tool arguments should still flow through the tool channel"
    );
}

#[test]
fn ambiguous_parallel_delta_without_metadata_is_not_attached() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_first",
                "type": "function_call",
                "call_id": "call_first",
                "name": "Edit"
            }
        })
    );
    let add_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "id": "fc_second",
                "type": "function_call",
                "call_id": "call_second",
                "name": "Edit"
            }
        })
    );
    let ambiguous_delta = format!(
        "data: {}",
        json!({
            "type": "response.function_call_arguments.delta",
            "delta": "{\"file_path\":\"/tmp/ambiguous.ts\"}"
        })
    );
    let done_1 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_first",
                "type": "function_call",
                "call_id": "call_first",
                "name": "Edit"
            }
        })
    );
    let done_2 = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {
                "id": "fc_second",
                "type": "function_call",
                "call_id": "call_second",
                "name": "Edit"
            }
        })
    );
    let completed = format!(
        "data: {}",
        json!({
            "type": "response.completed",
            "response": { "status": "completed" }
        })
    );

    let _ = transformer.transform_sse_line(&add_1);
    let _ = transformer.transform_sse_line(&add_2);
    let _ = transformer.transform_sse_line(&ambiguous_delta);
    let mut events = transformer.transform_sse_line(&done_1);
    events.extend(transformer.transform_sse_line(&done_2));
    events.extend(transformer.transform_sse_line(&completed));

    let joined = events.join("");
    assert!(
        joined.contains("\"id\":\"call_first\"") && joined.contains("\"id\":\"call_second\""),
        "both tool calls should still complete"
    );
    assert!(
        !joined.contains("ambiguous.ts"),
        "delta without routing metadata must not stick to the wrong parallel call"
    );
}

#[test]
fn unscoped_text_delta_is_deferred_until_tool_window_closes() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_defer_text",
                "type": "function_call",
                "call_id": "call_defer_text",
                "name": "Edit"
            }
        })
    );
    let unscoped_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "I will summarize once the tool call is done."
        })
    );
    let done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_defer_text",
                "type": "function_call",
                "call_id": "call_defer_text",
                "name": "Edit"
            }
        })
    );

    let _ = transformer.transform_sse_line(&add);
    let during_tool_window = transformer.transform_sse_line(&unscoped_text).join("");
    let after_tool_window = transformer.transform_sse_line(&done).join("");

    assert!(
        !during_tool_window.contains("I will summarize"),
        "unscoped text should be buffered while tool window is still open"
    );
    assert!(
        after_tool_window.contains("\"type\":\"tool_use\"")
            && after_tool_window.contains("\"id\":\"call_defer_text\""),
        "tool use should still be emitted when the call completes"
    );
    assert!(
        after_tool_window.contains("I will summarize once the tool call is done."),
        "buffered unscoped text should flush after tool window closes"
    );
}

#[test]
fn deferred_unscoped_leak_is_suppressed_when_tool_window_closes() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    let add = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_defer_leak",
                "type": "function_call",
                "call_id": "call_defer_leak",
                "name": "Read"
            }
        })
    );
    let leaked = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "Removing obsolete payload branch +#+#+#+#+#+{\"file_path\":\"/tmp/demo.txt\",\"offset\":0,\"limit\":50}"
        })
    );
    let done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": "fc_defer_leak",
                "type": "function_call",
                "call_id": "call_defer_leak",
                "name": "Read"
            }
        })
    );

    let _ = transformer.transform_sse_line(&add);
    let _ = transformer.transform_sse_line(&leaked);
    let joined = transformer.transform_sse_line(&done).join("");

    assert!(
        joined.contains("Removing obsolete payload branch"),
        "safe prefix text should survive deferred flush"
    );
    assert!(
        !joined.contains("\\\"file_path\\\"")
            && !joined.contains("\\\"offset\\\"")
            && !joined.contains("\\\"limit\\\""),
        "leaked read payload should still be suppressed after deferred flush"
    );
}

#[test]
fn suggestion_mode_prompt_is_suppressed_from_visible_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let line = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "[SUGGESTION MODE: Suggest what the user might naturally type next into Claude Code.]\n\nReply with ONLY the suggestion, no quotes or explanation."
        })
    );

    let joined = transformer.transform_sse_line(&line).join("");
    assert!(
        !joined.contains("SUGGESTION MODE"),
        "suggestion-mode prompt should be suppressed from visible output"
    );
    assert!(
        !joined.contains("\"type\":\"text_delta\""),
        "suppressed suggestion prompt should not emit text deltas"
    );
}

#[test]
fn split_suggestion_mode_prompt_across_chunks_is_suppressed() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let chunk1 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "[SUGGESTION MODE: Suggest what the user might naturally type next into Claude Code.]"
        })
    );
    let chunk2 = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "\n\nReply with ONLY the suggestion, no quotes or explanation.\n正常内容"
        })
    );

    let events1 = transformer.transform_sse_line(&chunk1);
    let events2 = transformer.transform_sse_line(&chunk2);
    let joined = format!("{}{}", events1.join(""), events2.join(""));

    assert!(
        !joined.contains("SUGGESTION MODE"),
        "split suggestion-mode prompt chunks should be suppressed"
    );
    assert!(
        joined.contains("正常内容"),
        "text after suggestion-mode prompt should continue streaming"
    );
}
