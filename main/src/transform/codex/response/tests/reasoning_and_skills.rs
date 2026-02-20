use super::*;
#[test]
fn test_reasoning_effort_opus_default() {
    let mapping = ReasoningEffortMapping::default();
    let effort = get_reasoning_effort("claude-3-opus-20240229", &mapping);
    assert_eq!(effort, ReasoningEffort::Xhigh);
}

#[test]
fn test_reasoning_effort_sonnet_default() {
    let mapping = ReasoningEffortMapping::default();
    let effort = get_reasoning_effort("claude-sonnet-4-20250514", &mapping);
    assert_eq!(effort, ReasoningEffort::Medium);
}

#[test]
fn test_reasoning_effort_haiku_default() {
    let mapping = ReasoningEffortMapping::default();
    let effort = get_reasoning_effort("claude-3-5-haiku-20241022", &mapping);
    assert_eq!(effort, ReasoningEffort::Low);
}

#[test]
fn test_custom_mapping_applied() {
    let mut mapping = ReasoningEffortMapping::default();
    mapping.sonnet = ReasoningEffort::High;

    let effort = get_reasoning_effort("claude-sonnet-4-20250514", &mapping);
    assert_eq!(effort, ReasoningEffort::High);
}

#[test]
fn test_reasoning_effort_as_str() {
    assert_eq!(ReasoningEffort::Xhigh.as_str(), "xhigh");
    assert_eq!(ReasoningEffort::High.as_str(), "high");
    assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
    assert_eq!(ReasoningEffort::Low.as_str(), "low");
}

#[test]
fn test_reasoning_effort_from_str() {
    assert_eq!(ReasoningEffort::from_str("xhigh"), ReasoningEffort::Xhigh);
    assert_eq!(ReasoningEffort::from_str("HIGH"), ReasoningEffort::High);
    assert_eq!(ReasoningEffort::from_str("Medium"), ReasoningEffort::Medium);
    assert_eq!(ReasoningEffort::from_str("low"), ReasoningEffort::Low);
    assert_eq!(
        ReasoningEffort::from_str("invalid"),
        ReasoningEffort::Medium
    ); // default
}

#[test]
fn test_unknown_model_defaults_to_medium() {
    let mapping = ReasoningEffortMapping::default();
    let effort = get_reasoning_effort("gpt-4-turbo", &mapping);
    assert_eq!(effort, ReasoningEffort::Medium);
}

#[test]
fn test_case_insensitive_model_matching() {
    let mapping = ReasoningEffortMapping::default();
    assert_eq!(
        get_reasoning_effort("CLAUDE-3-OPUS", &mapping),
        ReasoningEffort::Xhigh
    );
    assert_eq!(
        get_reasoning_effort("Claude-Sonnet-4", &mapping),
        ReasoningEffort::Medium
    );
    assert_eq!(
        get_reasoning_effort("claude-haiku", &mapping),
        ReasoningEffort::Low
    );
}

#[test]
fn test_transform_response_trait_dispatch() {
    let mut transformer = TransformResponse::new("gpt-5.3-codex");
    let trait_obj: &mut dyn ResponseTransformer = &mut transformer;
    let out = trait_obj.transform_line(r#"data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":20}}}"#);
    assert!(
        out.iter()
            .any(|chunk| chunk.contains("event: message_stop")),
        "trait dispatch should forward to internal transform logic"
    );
    assert!(
        out.iter()
            .any(|chunk| chunk.contains("\"input_tokens\":10")
                && chunk.contains("\"output_tokens\":20")),
        "should include usage statistics in message_delta"
    );
}

// Helper to create a fake tool use block
fn create_tool_use(id: &str, name: &str, input: Value) -> ContentBlock {
    ContentBlock::ToolUse {
        id: Some(id.to_string()),
        name: name.to_string(),
        input,
        signature: None,
    }
}

// Helper to create a fake tool result block
fn create_tool_result(tool_use_id: &str, content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: Some(tool_use_id.to_string()),
        id: Some("result_id".to_string()),
        content: Some(json!(content)),
    }
}

#[test]
fn test_skill_transformation() {
    // Mock messages
    let messages = vec![
        Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_use(
                "call_1",
                "skill",
                json!({
                    "skill": "test-skill",
                    "args": "arg1"
                }),
            )])),
        },
        Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_result(
                "call_1",
                "<command-name>test-skill</command-name>\nBase Path: /tmp\nSome content",
            )])),
        },
    ];

    let (input, skills) = MessageProcessor::transform_messages(&messages, None);

    // Verify skills extracted
    assert_eq!(skills.len(), 1);
    assert!(skills[0].contains("<name>test-skill</name>"));
    assert!(skills[0].contains("Some content"));

    // Verify input structure
    // Find function_call
    let func_call = input
        .iter()
        .find(|v| v["type"] == "function_call")
        .expect("Should have function_call");
    assert_eq!(func_call["name"], "skill");
    let args_str = func_call["arguments"].as_str().unwrap();
    let args: Value = serde_json::from_str(args_str).unwrap();
    assert_eq!(args["command"], "test-skill arg1");

    // Find function_call_output
    let func_out = input
        .iter()
        .find(|v| v["type"] == "function_call_output")
        .expect("Should have function_call_output");
    assert_eq!(func_out["output"], "Skill 'test-skill' loaded.");
}

#[test]
fn test_skill_deduplication() {
    let messages = vec![
        Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_use(
                "call_1",
                "skill",
                json!({"command": "test-skill"}),
            )])),
        },
        Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_result(
                "call_1",
                "<command-name>test-skill</command-name>\nBase Path: /tmp\nContent 1",
            )])),
        },
        Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_use(
                "call_2",
                "skill",
                json!({"command": "test-skill"}),
            )])),
        },
        Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_result(
                "call_2",
                "<command-name>test-skill</command-name>\nBase Path: /tmp\nContent 1",
            )])),
        },
    ];

    let (input, skills) = MessageProcessor::transform_messages(&messages, None);

    // Should only extract once
    assert_eq!(skills.len(), 1);

    // But should have two outputs
    let outputs: Vec<_> = input
        .iter()
        .filter(|v| v["type"] == "function_call_output")
        .collect();
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0]["output"], "Skill 'test-skill' loaded.");
    assert_eq!(outputs[1]["output"], "Skill 'test-skill' loaded.");
}

#[test]
fn test_custom_injection_prompt() {
    // Setup request with skill usage
    let messages = vec![
        Message {
            role: "assistant".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_use(
                "call_1",
                "skill",
                json!({"command": "test-skill"}),
            )])),
        },
        Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![create_tool_result(
                "call_1",
                "<command-name>test-skill</command-name>\nBase Path: /tmp\nContent",
            )])),
        },
    ];

    let request = AnthropicRequest {
        model: Some("claude-3-opus".to_string()),
        messages,
        system: None,
        stream: false,
        tools: None,
        max_tokens: None,
        temperature: None,
        top_p: None,
        top_k: None,
        stop_sequences: None,
    };

    let mapping = ReasoningEffortMapping::default();
    let prompt = "Auto-install dependencies please.";

    let (body, _) = TransformRequest::transform(&request, None, &mapping, prompt, "gpt-5.3-codex");

    let input_arr = body.get("input").unwrap().as_array().unwrap();

    // Find the injected prompt
    // It should be after the skill injection.
    // Input structure: [Template, Skill, Prompt, ...History]
    // Since history starts with assistant, and we inject user messages.

    // Let's look for the prompt text
    let prompt_msg = input_arr.iter().find(|msg| {
        msg["role"] == "user" && msg["content"][0]["text"].as_str().unwrap_or("") == prompt
    });

    assert!(prompt_msg.is_some(), "Should inject custom prompt");
}
