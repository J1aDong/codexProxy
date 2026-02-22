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
        joined.contains("\"type\":\"text_delta\"")
            && joined.contains("Preparing edit. "),
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
        joined.contains("\"type\":\"tool_use\"")
            && joined.contains("\"name\":\"Edit\""),
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
