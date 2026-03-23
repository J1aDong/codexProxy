use super::*;

#[test]
fn multi_background_launch_turn_keeps_visible_final_answer_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    <TransformResponse as crate::transform::ResponseTransformer>::configure_request_context(
        &mut transformer,
        &crate::transform::ResponseTransformRequestContext {
            codex_plan_file_path: None,
            contains_background_agent_completion: false,
            historical_background_agent_launch_count: 0,
            terminal_background_agent_completion_count: 0,
        },
    );

    let first_agent_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "call_id": "call_bg_1",
                "name": "Agent"
            }
        })
    );
    transformer.transform_sse_line(&first_agent_added);
    let first_agent_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": "call_bg_1",
                "name": "Agent",
                "arguments": r#"{"description":"Beijing weather","run_in_background":true}"#
            }
        })
    );
    let _ = transformer.transform_sse_line(&first_agent_done);

    let second_agent_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "call_id": "call_bg_2",
                "name": "Agent"
            }
        })
    );
    transformer.transform_sse_line(&second_agent_added);
    let second_agent_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": "call_bg_2",
                "name": "Agent",
                "arguments": r#"{"description":"Guangdong weather","run_in_background":true}"#
            }
        })
    );
    let _ = transformer.transform_sse_line(&second_agent_done);

    let final_answer_item = format!(
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
    transformer.transform_sse_line(&final_answer_item);

    let final_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "看完了。按当前预报，未来 7 天是 2026-03-21 到 2026-03-27。"
        })
    );
    let joined = transformer.transform_sse_line(&final_text).join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "launch round should keep visible final-answer text even when multiple background agents were launched"
    );
    assert!(joined.contains("看完了。按当前预报"));
}

#[test]
fn completion_round_keeps_visible_final_answer_text_before_all_background_handoffs_arrive() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    <TransformResponse as crate::transform::ResponseTransformer>::configure_request_context(
        &mut transformer,
        &crate::transform::ResponseTransformRequestContext {
            codex_plan_file_path: None,
            contains_background_agent_completion: true,
            historical_background_agent_launch_count: 3,
            terminal_background_agent_completion_count: 2,
        },
    );

    let final_answer_item = format!(
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
    transformer.transform_sse_line(&final_answer_item);

    let final_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "补个校正：北京那个 subagent 实际只返回了执行说明。"
        })
    );
    let joined = transformer.transform_sse_line(&final_text).join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "completion round should not suppress visible text while waiting for background-agent handoffs"
    );
    assert!(joined.contains("补个校正"));
}

#[test]
fn completion_round_allows_final_answer_text_after_all_background_handoffs_arrive() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    <TransformResponse as crate::transform::ResponseTransformer>::configure_request_context(
        &mut transformer,
        &crate::transform::ResponseTransformRequestContext {
            codex_plan_file_path: None,
            contains_background_agent_completion: true,
            historical_background_agent_launch_count: 3,
            terminal_background_agent_completion_count: 3,
        },
    );

    let final_answer_item = format!(
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
    transformer.transform_sse_line(&final_answer_item);

    let final_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "3 个都看完了。按北京时间 2026-03-21 的最新预报，未来 7 天是 3月21日—3月27日。"
        })
    );
    let joined = transformer.transform_sse_line(&final_text).join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "completion round should allow visible text only after every launched background agent reached a terminal handoff"
    );
    assert!(joined.contains("3 个都看完了。按北京时间 2026-03-21"));
}

#[test]
fn completion_round_allows_final_answer_text_after_background_handoffs_arrive() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    <TransformResponse as crate::transform::ResponseTransformer>::configure_request_context(
        &mut transformer,
        &crate::transform::ResponseTransformRequestContext {
            codex_plan_file_path: None,
            contains_background_agent_completion: true,
            historical_background_agent_launch_count: 0,
            terminal_background_agent_completion_count: 0,
        },
    );

    let final_answer_item = format!(
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
    transformer.transform_sse_line(&final_answer_item);

    let final_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "3 个都看完了。按北京时间 2026-03-21 的最新预报，未来 7 天是 3月21日—3月27日。"
        })
    );
    let joined = transformer.transform_sse_line(&final_text).join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "completion round should allow visible final-answer text"
    );
    assert!(joined.contains("3 个都看完了。按北京时间 2026-03-21"));
}

#[test]
fn single_background_launch_turn_keeps_visible_final_answer_text() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");

    <TransformResponse as crate::transform::ResponseTransformer>::configure_request_context(
        &mut transformer,
        &crate::transform::ResponseTransformRequestContext {
            codex_plan_file_path: None,
            contains_background_agent_completion: false,
            historical_background_agent_launch_count: 0,
            terminal_background_agent_completion_count: 0,
        },
    );

    let agent_added = format!(
        "data: {}",
        json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "call_id": "call_bg_1",
                "name": "Agent"
            }
        })
    );
    transformer.transform_sse_line(&agent_added);
    let agent_done = format!(
        "data: {}",
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": "call_bg_1",
                "name": "Agent",
                "arguments": r#"{"description":"Beijing weather","run_in_background":true}"#
            }
        })
    );
    let _ = transformer.transform_sse_line(&agent_done);

    let final_answer_item = format!(
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
    transformer.transform_sse_line(&final_answer_item);

    let final_text = format!(
        "data: {}",
        json!({
            "type": "response.output_text.delta",
            "delta": "北京这边先给你一版初步结论。"
        })
    );
    let joined = transformer.transform_sse_line(&final_text).join("");

    assert!(
        joined.contains("\"type\":\"text_delta\""),
        "single background-agent launch should keep visible final-answer text"
    );
    assert!(joined.contains("北京这边先给你一版初步结论。"));
}
