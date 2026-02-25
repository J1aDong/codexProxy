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
