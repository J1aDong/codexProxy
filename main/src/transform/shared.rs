use crate::models::{AnthropicRequest, SystemBlock, SystemContent};

pub fn flatten_system_text(system: Option<&SystemContent>) -> Option<String> {
    match system {
        Some(SystemContent::Text(text)) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Some(SystemContent::Blocks(blocks)) if !blocks.is_empty() => {
            let text = blocks
                .iter()
                .filter_map(|block| match block {
                    SystemBlock::Text { text } => Some(text.clone()),
                    SystemBlock::PlainString(text) => Some(text.clone()),
                    SystemBlock::Other(value) => value
                        .get("text")
                        .and_then(|text| text.as_str())
                        .map(|text| text.to_string())
                        .or_else(|| serde_json::to_string(value).ok()),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    }
}

pub fn resolve_parallel_tool_calls(anthropic_body: &AnthropicRequest) -> bool {
    anthropic_body
        .tool_choice
        .as_ref()
        .and_then(|tool_choice| tool_choice.as_object())
        .and_then(|tool_choice| tool_choice.get("disable_parallel_tool_use"))
        .and_then(|value| value.as_bool())
        .map(|disabled| !disabled)
        .unwrap_or(true)
}
