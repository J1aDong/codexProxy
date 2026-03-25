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
