use serde_json::{json, Map, Value};
use std::collections::HashSet;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::logger::{is_debug_log_enabled, AppLogger};
use crate::models::{
    get_reasoning_effort, AnthropicRequest, ContentBlock, MessageContent, ReasoningEffortMapping,
};
use crate::transform::MessageProcessor;

const ANTHROPIC_COMPAT_PLAN_MODE_PROMPT: &str = include_str!("plan_mode_prompt.txt");
const EMPTY_TOOL_OUTPUT_PLACEHOLDER: &str = "(No output)";
const PROMPT_CACHE_KEY_MAX_CWD_LEN: usize = 64;
const PROMPT_CACHE_KEY_SEP: u8 = 0x1f;
const MAX_TRUSTED_REQUEST_CWD_CHARS: usize = 512;
const MAX_TOOL_DESCRIPTION_CHARS: usize = 240;
const MAX_TOOL_SCHEMA_DESCRIPTION_CHARS: usize = 120;
const PLAN_MODE_TOOL_BLACKLIST: &[&str] = &[
    "EnterPlanMode",
    "ExitPlanMode",
    "EnterWorktree",
    "ExitWorktree",
];
const DEFAULT_REASONING_SUMMARY_MODE: &str = "auto";
const COMPACT_SUMMARY_SYSTEM_MARKER: &str = "you are a helpful ai assistant tasked with summarizing conversations";
const ENV_REASONING_SUMMARY_MODE: &str = "CODEX_PROXY_REASONING_SUMMARY";
const ENV_INCLUDE_REASONING_ENCRYPTED_CONTENT: &str =
    "CODEX_PROXY_INCLUDE_REASONING_ENCRYPTED_CONTENT";
const NATIVE_RESPONSES_TOOL_TYPES: &[&str] = &[
    "web_search",
    "web_search_preview",
    "code_interpreter",
    "file_search",
    "image_generation",
    "mcp",
    "apply_patch",
    "local_shell",
    "shell",
    "custom",
];
fn bool_env_enabled(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| parse_env_bool(&v))
        .unwrap_or(false)
}

fn parse_env_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn resolve_reasoning_summary_mode() -> String {
    let from_env = std::env::var(ENV_REASONING_SUMMARY_MODE)
        .ok()
        .map(|v| v.trim().to_ascii_lowercase());
    match from_env.as_deref() {
        Some("auto") | Some("detailed") | Some("concise") => from_env.unwrap_or_default(),
        _ => DEFAULT_REASONING_SUMMARY_MODE.to_string(),
    }
}

fn resolve_include_reasoning_encrypted_content() -> bool {
    bool_env_enabled(ENV_INCLUDE_REASONING_ENCRYPTED_CONTENT)
}

fn is_wrapped_agents_instructions(system_text: &str) -> bool {
    let lower = system_text.to_ascii_lowercase();
    lower.contains("agents.md instructions")
        && lower.contains("<instructions>")
        && lower.contains("</instructions>")
}

fn is_compact_summary_request(request: &AnthropicRequest) -> bool {
    request
        .system
        .as_ref()
        .map(|system| {
            system
                .to_string()
                .to_ascii_lowercase()
                .contains(COMPACT_SUMMARY_SYSTEM_MARKER)
        })
        .unwrap_or(false)
}

fn normalized_contains(haystack: &str, needle: &str) -> bool {
    let normalized_needle = normalize_text_for_exact_match(needle);
    if normalized_needle.is_empty() {
        return false;
    }

    normalize_text_for_exact_match(haystack).contains(&normalized_needle)
}

fn append_instruction_text(base: &str, extra: &str) -> String {
    let trimmed_base = base.trim();
    let trimmed_extra = extra.trim();

    if trimmed_base.is_empty() {
        return trimmed_extra.to_string();
    }
    if trimmed_extra.is_empty() {
        return trimmed_base.to_string();
    }
    if normalized_contains(trimmed_base, trimmed_extra) {
        return trimmed_base.to_string();
    }

    format!("{}\n\n{}", trimmed_base, trimmed_extra)
}

fn merge_instruction_context(first: Option<&str>, second: Option<&str>) -> Option<String> {
    let mut merged = None::<String>;
    for text in [first, second].into_iter().flatten() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }

        merged = Some(match merged {
            Some(existing) => append_instruction_text(&existing, trimmed),
            None => trimmed.to_string(),
        });
    }
    merged
}

fn extract_user_scaffolding_to_instructions(messages: &[Value]) -> (Option<String>, Vec<Value>) {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";
    let mut promoted_parts: Vec<String> = Vec::new();
    let mut cleaned_messages: Vec<Value> = Vec::new();

    for item in messages {
        let mut cloned = item.clone();
        let is_user_message = cloned.get("type").and_then(|v| v.as_str()) == Some("message")
            && cloned.get("role").and_then(|v| v.as_str()) == Some("user");
        if !is_user_message {
            cleaned_messages.push(cloned);
            continue;
        }

        let Some(content) = cloned.get_mut("content").and_then(|v| v.as_array_mut()) else {
            cleaned_messages.push(cloned);
            continue;
        };

        let mut new_content: Vec<Value> = Vec::new();
        for block in content.iter() {
            let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
                new_content.push(block.clone());
                continue;
            };
            let mut remaining = text.to_string();

            while let Some(start_idx) = remaining.find(START) {
                let after_start = &remaining[start_idx + START.len()..];
                let Some(end_rel) = after_start.find(END) else {
                    break;
                };
                let end_idx = start_idx + START.len() + end_rel + END.len();
                let block_text = remaining[start_idx + START.len()..start_idx + START.len() + end_rel].trim();
                if !block_text.is_empty() {
                    promoted_parts.push(block_text.to_string());
                }
                remaining = format!("{}{}", &remaining[..start_idx], &remaining[end_idx..]);
            }

            if let Some(contents_idx) = remaining.find("Contents of /repo/") {
                let after = &remaining[contents_idx..];
                if after.contains("CLAUDE.md:") || after.contains("IFLOW.md:") || after.contains("CodeBuddy.md:") {
                    let prefix = &remaining[..contents_idx];
                    let lines: Vec<&str> = after.lines().collect();
                    if lines.len() >= 2 {
                        let rule_line = lines[1].trim();
                        if !rule_line.is_empty() {
                            promoted_parts.push(rule_line.to_string());
                        }
                        let remainder = if let Some(split_idx) = after.find("\n\n") {
                            &after[split_idx + 2..]
                        } else {
                            ""
                        };
                        remaining = format!("{}{}", prefix, remainder);
                    }
                }
            }

            let trimmed = remaining.trim();
            if !trimmed.is_empty() {
                let mut new_block = block.clone();
                if let Some(obj) = new_block.as_object_mut() {
                    obj.insert("text".to_string(), Value::String(trimmed.to_string()));
                }
                new_content.push(new_block);
            }
        }

        if let Some(obj) = cloned.as_object_mut() {
            obj.insert("content".to_string(), Value::Array(new_content));
        }
        if cloned
            .get("content")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false)
        {
            cleaned_messages.push(cloned);
        }
    }

    let promoted = if promoted_parts.is_empty() {
        None
    } else {
        Some(promoted_parts.join("\n\n"))
    };
    (promoted, cleaned_messages)
}

fn request_contains_environment_context(request_text_corpus: &str) -> bool {
    request_text_corpus
        .to_ascii_lowercase()
        .contains("<environment_context>")
}

fn request_metadata_plan_mode_enabled(request: &AnthropicRequest) -> bool {
    request
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("plan_mode"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn request_targets_exit_plan_mode_tool(request: &AnthropicRequest) -> bool {
    let Some(tool_choice) = request
        .tool_choice
        .as_ref()
        .and_then(|value| value.as_object())
    else {
        return false;
    };

    tool_choice
        .get("name")
        .or_else(|| tool_choice.get("tool_name"))
        .or_else(|| tool_choice.get("toolName"))
        .and_then(|value| value.as_str())
        .map(|name| name.eq_ignore_ascii_case("ExitPlanMode"))
        .unwrap_or(false)
}

fn request_targets_blacklisted_plan_mode_tool(request: &AnthropicRequest) -> bool {
    request
        .tool_choice
        .as_ref()
        .and_then(|value| value.as_object())
        .and_then(|tool_choice| {
            tool_choice
                .get("name")
                .or_else(|| tool_choice.get("tool_name"))
                .or_else(|| tool_choice.get("toolName"))
                .and_then(|value| value.as_str())
        })
        .map(is_plan_mode_tool_blacklisted)
        .unwrap_or(false)
}

fn request_contains_recent_plan_mode_reminder(request: &AnthropicRequest) -> bool {
    const PLAN_MODE_REMINDER: &str = "plan mode is active.";

    request
        .messages
        .iter()
        .rev()
        .filter_map(|message| {
            if !message.role.eq_ignore_ascii_case("user") {
                return None;
            }

            let content = message.content.as_ref()?;
            let text = match content {
                MessageContent::Text(text) => text.clone(),
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            };

            (!text.trim().is_empty()).then_some(text)
        })
        .take(1)
        .any(|text| text.to_ascii_lowercase().contains(PLAN_MODE_REMINDER))
}

fn request_contains_plan_approval_signal(
    _request: &AnthropicRequest,
    request_text_corpus: &str,
) -> bool {
    if request_text_corpus
        .to_ascii_lowercase()
        .contains("plan_approval_response")
    {
        return true;
    }

    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestAugmentationMode {
    Agent,
    Passthrough,
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestAugmentationDecision {
    mode: RequestAugmentationMode,
    reasons: Vec<&'static str>,
}

impl RequestAugmentationDecision {
    fn is_agent(&self) -> bool {
        matches!(self.mode, RequestAugmentationMode::Agent)
    }

    fn mode_label(&self) -> &'static str {
        match self.mode {
            RequestAugmentationMode::Agent => "agent",
            RequestAugmentationMode::Passthrough => "passthrough",
            RequestAugmentationMode::Plan => "plan",
        }
    }
}

fn message_contains_agentic_tool_state(message: &crate::models::Message) -> bool {
    let Some(MessageContent::Blocks(blocks)) = message.content.as_ref() else {
        return false;
    };

    blocks.iter().any(|block| {
        matches!(
            block,
            ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. }
        )
    })
}

fn decide_request_augmentation(
    request: &AnthropicRequest,
    request_text_corpus: &str,
) -> RequestAugmentationDecision {
    let mut reasons = Vec::new();

    if request_metadata_plan_mode_enabled(request) {
        reasons.push("plan_mode");
    }
    if request_targets_exit_plan_mode_tool(request) {
        reasons.push("exit_plan_mode_tool");
    }
    if request_contains_plan_approval_signal(request, request_text_corpus) {
        reasons.push("plan_approval_response");
    }
    if request_contains_recent_plan_mode_reminder(request) {
        reasons.push("recent_plan_mode_reminder");
    }

    let has_plan_signal = !reasons.is_empty();

    if request
        .tools
        .as_ref()
        .map(|tools| !tools.is_empty())
        .unwrap_or(false)
    {
        reasons.push("tools");
    }

    if request
        .messages
        .iter()
        .any(message_contains_agentic_tool_state)
    {
        reasons.push("tool_state");
    }

    if request_contains_environment_context(request_text_corpus) {
        reasons.push("environment_context");
    }

    if request
        .system
        .as_ref()
        .map(|system| is_wrapped_agents_instructions(&system.to_string()))
        .unwrap_or(false)
    {
        reasons.push("wrapped_agents_system");
    }

    if reasons.contains(&"environment_context")
        && !reasons.contains(&"tools")
        && !reasons.contains(&"tool_state")
        && !reasons.contains(&"system_codex")
        && !reasons.contains(&"wrapped_agents_system")
    {
        reasons.retain(|reason| *reason != "environment_context");
    }

    let mode = if has_plan_signal {
        RequestAugmentationMode::Plan
    } else if reasons.is_empty() {
        RequestAugmentationMode::Passthrough
    } else {
        RequestAugmentationMode::Agent
    };

    RequestAugmentationDecision { mode, reasons }
}

fn collect_request_text_corpus(request: &AnthropicRequest) -> String {
    let mut parts = Vec::new();

    if let Some(system_text) = request.system.as_ref().map(|s| s.to_string()) {
        let trimmed = system_text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }

    for message in &request.messages {
        let Some(content) = message.content.as_ref() else {
            continue;
        };
        match content {
            MessageContent::Text(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    if let ContentBlock::Text { text } = block {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            parts.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }

    parts.join("\n")
}

fn extract_session_hint_from_user_id(user_id: &str) -> Option<String> {
    let lower = user_id.to_ascii_lowercase();
    if let Some(idx) = lower.find("session_") {
        let tail = &user_id[idx + "session_".len()..];
        let token = tail
            .split(|ch: char| ch.is_whitespace() || ch == ';' || ch == ',' || ch == '"')
            .next()
            .unwrap_or("");
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

fn extract_session_hint_from_metadata(request: &AnthropicRequest) -> Option<String> {
    let metadata = request.metadata.as_ref()?;
    let read_str = |key: &str| {
        metadata
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    };
    if let Some(value) = read_str("session_id") {
        return Some(value);
    }
    if let Some(value) = read_str("conversation_id") {
        return Some(value);
    }
    if let Some(user_id) = metadata.get("user_id").and_then(|value| value.as_str()) {
        return extract_session_hint_from_user_id(user_id);
    }
    None
}

fn extract_request_session_hint(request: &AnthropicRequest) -> Option<String> {
    if let Some(value) = extract_session_hint_from_metadata(request) {
        return Some(value);
    }
    let mut candidates = Vec::new();
    if let Some(system_text) = request.system.as_ref().map(|s| s.to_string()) {
        candidates.push(system_text);
    }
    for message in &request.messages {
        if let Some(content) = message.content.as_ref() {
            match content {
                MessageContent::Text(text) => candidates.push(text.clone()),
                MessageContent::Blocks(blocks) => {
                    for block in blocks {
                        if let ContentBlock::Text { text } = block {
                            candidates.push(text.clone());
                        }
                    }
                }
            }
        }
    }

    for text in candidates {
        let lower = text.to_ascii_lowercase();
        if let Some(idx) = lower.find("session_id") {
            let tail = &text[idx..];
            if let Some(start) = tail.find(':') {
                let after = tail[start + 1..].trim();
                let token = after
                    .split(|ch: char| ch.is_whitespace() || ch == ';' || ch == ',' || ch == '"')
                    .next()
                    .unwrap_or("");
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
        if let Some(idx) = lower.find("conversation_id") {
            let tail = &text[idx..];
            if let Some(start) = tail.find(':') {
                let after = tail[start + 1..].trim();
                let token = after
                    .split(|ch: char| ch.is_whitespace() || ch == ';' || ch == ',' || ch == '"')
                    .next()
                    .unwrap_or("");
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }

    None
}

fn extract_first_tag_content<'a>(text: &'a str, tag_start: &str, tag_end: &str) -> Option<&'a str> {
    let start = text.find(tag_start)?;
    let after_start = &text[start + tag_start.len()..];
    let end_rel = after_start.find(tag_end)?;
    Some(&after_start[..end_rel])
}

fn extract_request_cwd(request_text_corpus: &str) -> Option<String> {
    const ENV_START: &str = "<environment_context>";
    const ENV_END: &str = "</environment_context>";
    const CWD_START: &str = "<cwd>";
    const CWD_END: &str = "</cwd>";

    let mut remaining = request_text_corpus;
    while let Some(env_start_idx) = remaining.find(ENV_START) {
        let after_env_start = &remaining[env_start_idx + ENV_START.len()..];
        let Some(env_end_rel) = after_env_start.find(ENV_END) else {
            break;
        };
        let env_block = &after_env_start[..env_end_rel];
        if let Some(cwd_raw) = extract_first_tag_content(env_block, CWD_START, CWD_END) {
            let cwd = cwd_raw.trim();
            if !cwd.is_empty() && cwd.chars().count() <= MAX_TRUSTED_REQUEST_CWD_CHARS {
                return Some(cwd.to_string());
            }
        }
        remaining = &after_env_start[env_end_rel + ENV_END.len()..];
    }

    None
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn count_normalized_chars(text: &str) -> usize {
    normalize_text_for_exact_match(text).chars().count()
}

fn format_fingerprint(hash: u64) -> String {
    format!("{:016x}", hash)
}

pub(crate) fn normalize_text_for_exact_match(text: &str) -> String {
    normalize_line_endings(text).trim().to_string()
}

fn fingerprint_normalized_text(normalized: &str) -> Option<String> {
    if normalized.is_empty() {
        None
    } else {
        Some(format_fingerprint(fnv1a64(normalized.as_bytes())))
    }
}

fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json_value).collect()),
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut normalized = Map::new();
            for key in keys {
                if let Some(child) = map.get(&key) {
                    normalized.insert(key, canonicalize_json_value(child));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

pub(crate) fn fingerprint_json_value(value: &Value) -> String {
    let normalized = canonicalize_json_value(value);
    let bytes = serde_json::to_vec(&normalized).unwrap_or_default();
    format_fingerprint(fnv1a64(&bytes))
}

#[derive(Debug, Clone)]
struct StaticHeavyPayloadStats {
    wrapped_system_fingerprint: Option<String>,
    wrapped_system_chars: usize,
    custom_prompt_fingerprint: Option<String>,
    custom_prompt_chars: usize,
    tools_fingerprint: Option<String>,
    tools_count: usize,
    tools_bytes: usize,
}

fn build_static_heavy_payload_stats(
    wrapped_system_text: Option<&str>,
    custom_prompt_text: Option<&str>,
    transformed_tools: &[Value],
    transformed_tools_bytes: usize,
) -> StaticHeavyPayloadStats {
    let normalized_wrapped_system = wrapped_system_text.map(normalize_text_for_exact_match);
    let normalized_custom_prompt = custom_prompt_text.map(normalize_text_for_exact_match);
    let tools_fingerprint = (!transformed_tools.is_empty())
        .then(|| fingerprint_json_value(&Value::Array(transformed_tools.to_vec())));

    StaticHeavyPayloadStats {
        wrapped_system_fingerprint: normalized_wrapped_system
            .as_deref()
            .and_then(fingerprint_normalized_text),
        wrapped_system_chars: wrapped_system_text.map(count_normalized_chars).unwrap_or(0),
        custom_prompt_fingerprint: normalized_custom_prompt
            .as_deref()
            .and_then(fingerprint_normalized_text),
        custom_prompt_chars: custom_prompt_text.map(count_normalized_chars).unwrap_or(0),
        tools_fingerprint,
        tools_count: transformed_tools.len(),
        tools_bytes: transformed_tools_bytes,
    }
}

fn format_optional_fingerprint(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

fn has_skill_tool(tools: Option<&Vec<Value>>) -> bool {
    let Some(tools) = tools else {
        return false;
    };
    tools.iter().any(|tool| {
        let direct_name = tool.get("name").and_then(|v| v.as_str());
        let nested_name = tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str());
        direct_name
            .or(nested_name)
            .map(|name| name.eq_ignore_ascii_case("skill"))
            .unwrap_or(false)
    })
}

fn is_skill_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ':'
}

fn normalize_skill_name_token(token: &str) -> Option<String> {
    let trimmed = token
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | ';' | '!' | '?'
            )
        })
        .trim_start_matches('/');
    if trimmed.len() < 2 {
        return None;
    }
    if !trimmed.chars().all(is_skill_identifier_char) {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn extract_available_skill_names_ordered(system_text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for line in system_text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        else {
            continue;
        };
        let candidate = rest.splitn(2, ':').next().unwrap_or_default();
        if candidate.trim_start().starts_with('/') {
            // `/help`-style builtin command bullets are not Skill-tool skills.
            continue;
        }
        if let Some(name) = normalize_skill_name_token(candidate) {
            if seen.insert(name.clone()) {
                names.push(name);
            }
        }
    }
    names
}

fn extract_skill_catalog_entries_ordered(system_text: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for line in system_text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        else {
            continue;
        };
        let candidate = rest.splitn(2, ':').next().unwrap_or_default();
        if candidate.trim_start().starts_with('/') {
            continue;
        }
        let Some(name) = normalize_skill_name_token(candidate) else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let desc = rest
            .splitn(2, ':')
            .nth(1)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());
        if let Some(desc) = desc {
            entries.push(format!("- {}: {}", name, desc));
        } else {
            entries.push(format!("- {}", name));
        }
    }
    entries
}

fn text_contains_skill_catalog_header(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("the following skills are available")
        || lower.contains("skills are available for use with the skill tool")
        || lower.contains("skills available in this session")
        || lower.contains("### available skills")
}

fn extract_available_skill_names_from_request(request: &AnthropicRequest) -> Vec<String> {
    let mut ordered_names = Vec::new();
    let mut seen = HashSet::new();
    let mut push_names = |text: &str| {
        if !text_contains_skill_catalog_header(text) {
            return;
        }
        for name in extract_available_skill_names_ordered(text) {
            if seen.insert(name.clone()) {
                ordered_names.push(name);
            }
        }
    };

    if let Some(system) = request.system.as_ref().map(|s| s.to_string()) {
        push_names(&system);
    }

    for message in &request.messages {
        let Some(content) = message.content.as_ref() else {
            continue;
        };
        match content {
            MessageContent::Text(text) => push_names(text),
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    if let ContentBlock::Text { text } = block {
                        push_names(text);
                    }
                }
            }
        }
    }

    ordered_names
}

fn extract_slash_skill_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        if chars[idx] != '/' {
            idx += 1;
            continue;
        }
        if idx > 0 {
            let prev = chars[idx - 1];
            // Skip URL/path-like slashes (`https://`, `/Users/...`) to avoid false positives.
            if is_skill_identifier_char(prev) || matches!(prev, '/' | ':' | '.') {
                idx += 1;
                continue;
            }
        }
        let mut end = idx + 1;
        while end < chars.len() && is_skill_identifier_char(chars[end]) {
            end += 1;
        }
        if end > idx + 1 {
            if chars.get(end).is_some_and(|ch| matches!(ch, '.' | '/')) {
                idx = end;
                continue;
            }
            let token: String = chars[idx + 1..end].iter().collect();
            if let Some(name) = normalize_skill_name_token(&token) {
                tokens.push(name);
            }
        }
        idx = end;
    }
    tokens
}

fn extract_dollar_skill_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        if chars[idx] != '$' {
            idx += 1;
            continue;
        }
        if idx > 0 {
            let prev = chars[idx - 1];
            if is_skill_identifier_char(prev) {
                idx += 1;
                continue;
            }
        }
        let mut end = idx + 1;
        while end < chars.len() && is_skill_identifier_char(chars[end]) {
            end += 1;
        }
        if end > idx + 1 {
            let token: String = chars[idx + 1..end].iter().collect();
            if let Some(name) = normalize_skill_name_token(&token) {
                tokens.push(name);
            }
        }
        idx = end;
    }
    tokens
}

fn latest_user_text(request: &AnthropicRequest) -> Option<String> {
    for message in request.messages.iter().rev() {
        if !message.role.eq_ignore_ascii_case("user") {
            continue;
        }
        let Some(content) = message.content.as_ref() else {
            continue;
        };
        match content {
            MessageContent::Text(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            MessageContent::Blocks(blocks) => {
                let mut parts: Vec<String> = Vec::new();
                for block in blocks {
                    if let ContentBlock::Text { text } = block {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            parts.push(trimmed.to_string());
                        }
                    }
                }
                if !parts.is_empty() {
                    return Some(parts.join("\n"));
                }
            }
        }
    }
    None
}

fn latest_user_text_block(request: &AnthropicRequest) -> Option<String> {
    for message in request.messages.iter().rev() {
        if !message.role.eq_ignore_ascii_case("user") {
            continue;
        }
        let Some(content) = message.content.as_ref() else {
            continue;
        };
        match content {
            MessageContent::Text(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            MessageContent::Blocks(blocks) => {
                for block in blocks.iter().rev() {
                    if let ContentBlock::Text { text } = block {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn detect_requested_skill_name(request: &AnthropicRequest) -> Option<String> {
    if !has_skill_tool(request.tools.as_ref()) {
        return None;
    }
    let available: HashSet<String> = extract_available_skill_names_from_request(request)
        .into_iter()
        .collect();
    if available.is_empty() {
        return None;
    }
    let latest_text = latest_user_text(request)?;

    for token in extract_slash_skill_tokens(&latest_text) {
        if available.contains(&token) {
            return Some(token);
        }
    }
    for token in extract_dollar_skill_tokens(&latest_text) {
        if available.contains(&token) {
            return Some(token);
        }
    }
    None
}

fn request_explicitly_mentions_worktree(request: &AnthropicRequest) -> bool {
    latest_user_text_block(request)
        .map(|text| {
            let lower = text.to_ascii_lowercase();
            lower.contains("worktree")
                || lower.contains("git worktree")
                || lower.contains("进入 worktree")
                || lower.contains("在 worktree")
                || lower.contains("隔离 worktree")
        })
        .unwrap_or(false)
}

fn strip_agent_worktree_isolation(tool: &mut Value) {
    let Some(obj) = tool.as_object_mut() else {
        return;
    };
    if obj.get("name").and_then(|value| value.as_str()) != Some("Agent") {
        return;
    }
    let Some(parameters) = obj.get_mut("parameters").and_then(|value| value.as_object_mut()) else {
        return;
    };
    if let Some(properties) = parameters
        .get_mut("properties")
        .and_then(|value| value.as_object_mut())
    {
        properties.remove("isolation");
    }
    if let Some(required) = parameters.get_mut("required").and_then(|value| value.as_array_mut()) {
        required.retain(|value| value.as_str() != Some("isolation"));
    }
    if let Some(description) = obj.get_mut("description") {
        if let Some(text) = description.as_str() {
            let clarification = " Ordinary Agent calls do not require worktree isolation; use worktree only when the user explicitly asks for isolated repo work.";
            if !text.contains("do not require worktree isolation") {
                *description = Value::String(format!("{}{}", text.trim_end(), clarification));
            }
        }
    }
}

fn adjust_agent_tool_semantics_for_request(request: &AnthropicRequest, tools: &mut [Value]) {
    if request_explicitly_mentions_worktree(request) {
        return;
    }
    for tool in tools {
        strip_agent_worktree_isolation(tool);
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn sanitize_cache_key_segment(input: &str, max_len: usize) -> String {
    let mut segment = String::with_capacity(input.len().min(max_len));
    for ch in input.chars() {
        let normalized = if ch.is_ascii_alphanumeric() { ch } else { '_' };
        segment.push(normalized);
        if segment.len() >= max_len {
            break;
        }
    }

    let trimmed = segment.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_prompt_cache_key(
    request_cwd: Option<&str>,
    codex_model: &str,
    request_session_hint: Option<&str>,
    applied_custom_injection_prompt: Option<&str>,
    applied_static_instructions: Option<&str>,
    system_text: Option<&str>,
    tools_fingerprint: Option<&str>,
) -> String {
    let hint = request_session_hint
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| sanitize_cache_key_segment(value, 72));
    if let Some(hint) = hint {
        let model_segment = sanitize_cache_key_segment(codex_model, 48);
        return format!("codex-proxy:{}:session:{}", model_segment, hint);
    }
    let mut key_material = Vec::new();
    if let Some(instructions) = applied_static_instructions {
        key_material.extend_from_slice(normalize_text_for_exact_match(instructions).as_bytes());
    }
    key_material.push(PROMPT_CACHE_KEY_SEP);
    if let Some(custom_prompt) = applied_custom_injection_prompt {
        key_material.extend_from_slice(normalize_text_for_exact_match(custom_prompt).as_bytes());
    }
    key_material.push(PROMPT_CACHE_KEY_SEP);
    if let Some(system) = system_text {
        key_material.extend_from_slice(normalize_text_for_exact_match(system).as_bytes());
    }
    key_material.push(PROMPT_CACHE_KEY_SEP);
    if let Some(tools_fingerprint) = tools_fingerprint {
        key_material.extend_from_slice(tools_fingerprint.as_bytes());
    }
    let key_hash = fnv1a64(&key_material);
    let model_segment = sanitize_cache_key_segment(codex_model, 48);
    let cwd_segment = request_cwd
        .map(|cwd| sanitize_cache_key_segment(cwd, PROMPT_CACHE_KEY_MAX_CWD_LEN))
        .unwrap_or_else(|| "default".to_string());
    format!(
        "codex-proxy:{}:{}:{:016x}",
        model_segment, cwd_segment, key_hash
    )
}

fn compact_text_field(value: Option<&str>, max_chars: usize) -> String {
    let Some(raw) = value else {
        return String::new();
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Keep the first paragraph and normalize whitespace to cut verbose boilerplate.
    let first_paragraph = trimmed.split("\n\n").next().unwrap_or(trimmed);
    let normalized = first_paragraph
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized;
    }

    let mut clipped: String = normalized.chars().take(max_chars).collect();
    while clipped.ends_with(|ch: char| ch.is_whitespace()) {
        clipped.pop();
    }
    clipped.push_str("...");
    clipped
}

fn compact_tool_description(description: Option<&str>) -> String {
    compact_text_field(description, MAX_TOOL_DESCRIPTION_CHARS)
}

fn compact_tool_schema_description(description: Option<&str>) -> String {
    compact_text_field(description, MAX_TOOL_SCHEMA_DESCRIPTION_CHARS)
}

fn compact_tool_parameters_schema(value: &mut Value) {
    match value {
        Value::Object(obj) => {
            for key in [
                "title",
                "examples",
                "example",
                "deprecated",
                "readOnly",
                "writeOnly",
                "$comment",
            ] {
                obj.remove(key);
            }

            if let Some(description) = obj.get_mut("description") {
                if let Some(text) = description.as_str() {
                    *description = Value::String(compact_tool_schema_description(Some(text)));
                }
            }

            for child in obj.values_mut() {
                compact_tool_parameters_schema(child);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                compact_tool_parameters_schema(child);
            }
        }
        _ => {}
    }
}

fn sanitize_function_call_name_for_codex(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut last_was_separator = false;

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            normalized.push(ch);
            last_was_separator = false;
        } else if !last_was_separator {
            normalized.push('_');
            last_was_separator = true;
        }
    }

    let trimmed = normalized.trim_matches('_');
    if trimmed.is_empty() {
        "unknown_tool".to_string()
    } else {
        trimmed.to_string()
    }
}

fn find_leaked_tool_marker_start(text: &str) -> Option<usize> {
    const MARKERS: [&str; 4] = [
        "assistant to=",
        "to=functions",
        "to=multi_tool_use.parallel",
        "to=multi_tool_use",
    ];

    MARKERS.iter().filter_map(|marker| text.find(marker)).min()
}

fn looks_like_high_confidence_tool_json_object(json: &str) -> bool {
    let trimmed = json.trim_start();
    if !trimmed.starts_with('{') {
        return false;
    }

    let is_parallel_tool_json = trimmed.contains("\"tool_uses\"")
        && trimmed.contains("\"recipient_name\"")
        && (trimmed.contains("functions.") || trimmed.contains("multi_tool_use."));
    if is_parallel_tool_json {
        return true;
    }

    let is_single_edit_json = trimmed.contains("\"file_path\"")
        && ((trimmed.contains("\"old_string\"") && trimmed.contains("\"new_string\""))
            || trimmed.contains("\"replace_all\""));
    if is_single_edit_json {
        return true;
    }

    let is_basic_tool_envelope = trimmed.contains("\"recipient_name\"")
        && trimmed.contains("\"parameters\"")
        && (trimmed.contains("\"file_path\"")
            || trimmed.contains("\"pattern\"")
            || trimmed.contains("\"command\""));
    if is_basic_tool_envelope {
        return true;
    }

    let is_task_output_payload = serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|obj| {
            let has_task_id = obj
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_task_id {
                return false;
            }

            let has_controls = obj.contains_key("block") || obj.contains_key("timeout");
            if !has_controls {
                return false;
            }

            obj.keys()
                .all(|key| matches!(key.as_str(), "task_id" | "block" | "timeout"))
        })
        .unwrap_or(false);
    if is_task_output_payload {
        return true;
    }

    let is_read_payload = serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|obj| {
            let has_file_path = obj
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_file_path {
                return false;
            }

            let has_window = obj.contains_key("offset") || obj.contains_key("limit");
            if !has_window {
                return false;
            }

            obj.keys()
                .all(|key| matches!(key.as_str(), "file_path" | "offset" | "limit"))
        })
        .unwrap_or(false);
    if is_read_payload {
        return true;
    }

    let is_exec_command_payload = serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|obj| {
            let has_command = obj
                .get("command")
                .or_else(|| obj.get("cmd"))
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_command {
                return false;
            }

            obj.contains_key("description")
                || obj.contains_key("timeout")
                || obj.contains_key("yield_time_ms")
                || obj.contains_key("max_output_tokens")
                || obj.contains_key("sandbox_permissions")
                || obj.contains_key("justification")
                || obj.contains_key("prefix_rule")
                || obj.contains_key("workdir")
                || obj.contains_key("shell")
        })
        .unwrap_or(false);

    is_exec_command_payload
}

fn extract_first_json_object_fragment(line: &str) -> Option<String> {
    let start = line.find('{')?;
    let candidate = &line[start..];
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in candidate.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(candidate[..=idx].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

fn find_high_confidence_tool_json_tail_start(text: &str) -> Option<usize> {
    let mut matched_start = None;

    for (idx, ch) in text.char_indices() {
        if ch != '{' {
            continue;
        }

        let candidate = &text[idx..];
        let Some(json_object) = extract_first_json_object_fragment(candidate) else {
            continue;
        };

        if !looks_like_high_confidence_tool_json_object(&json_object) {
            continue;
        }

        let suffix = &candidate[json_object.len()..];
        if suffix.trim().is_empty() {
            matched_start = Some(idx);
        }
    }

    matched_start
}

fn strip_leaked_tool_suffix_from_text(text: &str) -> Option<String> {
    if let Some(marker_pos) = find_leaked_tool_marker_start(text) {
        let head = text[..marker_pos].trim_end();
        return if head.is_empty() {
            None
        } else {
            Some(head.to_string())
        };
    }

    if let Some(json_start) = find_high_confidence_tool_json_tail_start(text) {
        let head = text[..json_start].trim_end();
        return if head.is_empty() {
            None
        } else {
            Some(head.to_string())
        };
    }

    Some(text.to_string())
}

fn compact_skill_catalog_block(block: &str) -> Option<String> {
    let entries = extract_skill_catalog_entries_ordered(block);
    if entries.is_empty() {
        return None;
    }
    let mut compacted = String::new();
    compacted.push_str(
        "Skill catalog (condensed):
",
    );
    for entry in entries {
        compacted.push_str(&entry);
        compacted.push('\n');
    }
    Some(compacted.trim_end().to_string())
}

fn is_plan_mode_tool_blacklisted(name: &str) -> bool {
    PLAN_MODE_TOOL_BLACKLIST
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

fn is_claude_plan_mode_orchestration_block(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    lower.contains("plan mode is active")
        || lower.contains("## plan file info")
        || lower.contains("## plan workflow")
        || lower.contains("call exitplanmode")
        || lower.contains("launch plan agent")
        || lower.contains("launch up to 3 explore agents")
        || lower.contains("askuserquestion")
        || lower.contains("write tool")
        || lower.contains(".claude/plans/")
        || lower.contains("the only file you are allowed to edit")
}

fn strip_plan_mode_orchestration_system_reminders(text: &str) -> String {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";

    let mut remaining = text;
    let mut sanitized = String::with_capacity(text.len());

    loop {
        let Some(start_idx) = remaining.find(START) else {
            sanitized.push_str(remaining);
            break;
        };

        sanitized.push_str(&remaining[..start_idx]);
        let after_start = &remaining[start_idx + START.len()..];
        let Some(end_rel) = after_start.find(END) else {
            sanitized.push_str(&remaining[start_idx..]);
            break;
        };

        let block = &after_start[..end_rel];
        if is_claude_plan_mode_orchestration_block(block) {
            remaining = &after_start[end_rel + END.len()..];
            continue;
        }

        sanitized.push_str(START);
        if let Some(compacted) = compact_skill_catalog_block(block) {
            sanitized.push_str(&compacted);
        } else {
            sanitized.push_str(block);
        }
        sanitized.push_str(END);
        remaining = &after_start[end_rel + END.len()..];
    }

    sanitized
}

fn sanitize_plan_mode_system_text_for_codex(text: &str) -> String {
    strip_plan_mode_orchestration_system_reminders(text)
        .trim()
        .to_string()
}

fn strip_system_reminder_blocks(text: &str) -> String {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";
    const SKILL_MARKERS: [&str; 10] = [
        "available skills",
        "the following skills are available",
        "skills are available for use with the skill tool",
        "skill catalog (condensed)",
        "### available skills",
        "how to use skills",
        "a skill is a set of local instructions",
        "skill.md",
        "skills available in this session",
        "slash command",
    ];

    let mut remaining = text;
    let mut sanitized = String::with_capacity(text.len());

    loop {
        let Some(start_idx) = remaining.find(START) else {
            sanitized.push_str(remaining);
            break;
        };

        sanitized.push_str(&remaining[..start_idx]);
        let after_start = &remaining[start_idx + START.len()..];
        let Some(end_rel) = after_start.find(END) else {
            // Keep malformed trailing text intact rather than truncating content.
            sanitized.push_str(&remaining[start_idx..]);
            break;
        };
        let block = &after_start[..end_rel];
        let block_lower = block.to_ascii_lowercase();
        let preserve_block = SKILL_MARKERS
            .iter()
            .any(|marker| block_lower.contains(marker));
        if preserve_block {
            sanitized.push_str(START);
            if let Some(compacted) = compact_skill_catalog_block(block) {
                sanitized.push_str(&compacted);
            } else {
                sanitized.push_str(block);
            }
            sanitized.push_str(END);
        };
        remaining = &after_start[end_rel + END.len()..];
    }

    sanitized
}

fn looks_like_suggestion_mode_prompt(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    upper.contains("[SUGGESTION MODE:")
        && upper.contains("REPLY WITH ONLY THE SUGGESTION")
        && upper.contains("WHAT THE USER MIGHT NATURALLY TYPE NEXT")
}

fn collapse_adjacent_duplicate_markdown_bold(text: &str) -> String {
    if !text.contains("****") {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;

    while i < text.len() {
        let rest = &text[i..];
        if let Some(token_start_rel) = rest.find("**") {
            let token_start = i + token_start_rel;
            out.push_str(&text[i..token_start]);

            let token_rest = &text[token_start + 2..];
            if let Some(token_end_rel) = token_rest.find("**") {
                let token_end = token_start + 2 + token_end_rel + 2;
                let token = &text[token_start..token_end];
                out.push_str(token);

                let mut next = token_end;
                while next < text.len() && text[next..].starts_with(token) {
                    next += token.len();
                }
                i = next;
                continue;
            }

            out.push_str(&text[token_start..]);
            return out;
        }

        out.push_str(rest);
        break;
    }

    out
}

fn strip_known_trailing_noise(text: &str) -> String {
    let mut result = text.trim_end().to_string();
    let noise_patterns = [
        "assistantuser",
        "numeroususer",
        "numerusform",
        "天天中彩票user",
        "天天中彩票",
        " +#+#+#+#+#+",
    ];

    for pattern in noise_patterns {
        if result.ends_with(pattern) {
            result.truncate(result.len().saturating_sub(pattern.len()));
            return result.trim_end().to_string();
        }
    }

    result
}

fn strip_dynamic_system_header_lines(text: &str) -> String {
    let mut filtered = Vec::new();
    let mut removed = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("x-anthropic-billing-header:") {
            removed = true;
            continue;
        }
        filtered.push(line);
    }
    if !removed {
        return text.to_string();
    }
    filtered.join("\n")
}

fn sanitize_message_text_for_codex(text: &str) -> Option<String> {
    let without_reminder = strip_system_reminder_blocks(text);
    if without_reminder.trim().is_empty() {
        return None;
    }

    if looks_like_suggestion_mode_prompt(&without_reminder) {
        return None;
    }

    let stripped = strip_leaked_tool_suffix_from_text(&without_reminder)?;
    let collapsed = collapse_adjacent_duplicate_markdown_bold(&stripped);
    let cleaned = strip_known_trailing_noise(&collapsed);
    let final_text = cleaned.trim_end().to_string();

    if final_text.trim().is_empty() {
        None
    } else {
        Some(final_text)
    }
}

fn sanitize_function_call_output_for_codex(output: &str) -> Option<String> {
    let sanitized = strip_system_reminder_blocks(output);
    let trimmed = sanitized.trim_end().to_string();
    if trimmed.trim().is_empty() {
        Some(EMPTY_TOOL_OUTPUT_PLACEHOLDER.to_string())
    } else {
        Some(trimmed)
    }
}

fn normalize_function_call_output_value_for_codex(output: &Value) -> Value {
    if let Some(text) = output.as_str() {
        return sanitize_function_call_output_for_codex(text)
            .map(Value::String)
            .unwrap_or_else(|| Value::String(String::new()));
    }

    if let Some(items) = output.as_array() {
        return Value::Array(items.clone());
    }

    if output.is_object() {
        let compact = serde_json::to_string(output).unwrap_or_else(|_| "{}".to_string());
        let sanitized = sanitize_function_call_output_for_codex(&compact)
            .unwrap_or_else(|| EMPTY_TOOL_OUTPUT_PLACEHOLDER.to_string());
        return Value::Array(vec![json!({
            "type": "input_text",
            "text": sanitized
        })]);
    }

    let fallback = output
        .as_bool()
        .map(|v| v.to_string())
        .or_else(|| output.as_i64().map(|v| v.to_string()))
        .or_else(|| output.as_u64().map(|v| v.to_string()))
        .or_else(|| output.as_f64().map(|v| v.to_string()))
        .unwrap_or_else(|| EMPTY_TOOL_OUTPUT_PLACEHOLDER.to_string());
    Value::String(fallback)
}

fn reconcile_function_call_pairs(input: &mut Vec<Value>) {
    let mut open_calls: HashSet<String> = HashSet::new();
    let mut call_order: Vec<String> = Vec::new();

    for item in input.iter() {
        let Some(item_type) = item.get("type").and_then(|v| v.as_str()) else {
            continue;
        };

        match item_type {
            "function_call" => {
                if let Some(call_id) = item.get("call_id").and_then(|v| v.as_str()) {
                    let normalized = call_id.trim();
                    if !normalized.is_empty() && open_calls.insert(normalized.to_string()) {
                        call_order.push(normalized.to_string());
                    }
                }
            }
            "function_call_output" => {
                if let Some(call_id) = item.get("call_id").and_then(|v| v.as_str()) {
                    open_calls.remove(call_id.trim());
                }
            }
            _ => {}
        }
    }

    for call_id in call_order {
        if !open_calls.contains(call_id.as_str()) {
            continue;
        }

        input.push(json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": EMPTY_TOOL_OUTPUT_PLACEHOLDER
        }));
    }
}

/// 请求转换器 - Anthropic -> Codex
pub struct TransformRequest;

impl TransformRequest {
    pub fn transform(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        reasoning_mapping: &ReasoningEffortMapping,
        custom_injection_prompt: &str,
        codex_model: &str,
    ) -> (Value, String) {
        Self::transform_with_options(
            anthropic_body,
            log_tx,
            reasoning_mapping,
            custom_injection_prompt,
            codex_model,
            true,
            true,
            false,
            anthropic_body.stream,
        )
    }

    pub fn transform_with_options(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        reasoning_mapping: &ReasoningEffortMapping,
        custom_injection_prompt: &str,
        codex_model: &str,
        enable_tool_schema_compaction: bool,
        enable_codex_fast_mode: bool,
        enable_skill_routing_hint: bool,
        _effective_stream: bool,
    ) -> (Value, String) {
        let session_id = Uuid::new_v4().to_string();

        // 获取全局日志记录器
        let logger = AppLogger::get();

        // 辅助函数：同时写入 broadcast 和文件
        let log = |msg: &str| {
            if is_debug_log_enabled() {
                if let Some(tx) = log_tx {
                    let _ = tx.send(msg.to_string());
                }
                if let Some(ref l) = logger {
                    l.log(msg);
                }
            }
        };

        log(&format!("📋 [Transform] Session: {}", &session_id[..8]));

        let original_model = anthropic_body.model.as_deref().unwrap_or("unknown");
        let reasoning_effort = get_reasoning_effort(original_model, reasoning_mapping);
        // 使用用户配置的 codex_model（从前端传入）
        let final_codex_model = codex_model
            .trim()
            .is_empty()
            .then(|| "gpt-5.3-codex")
            .unwrap_or(codex_model);

        log(&format!(
            "🤖 [Transform] {} → {} | 🧠 reasoning: {} (from {})",
            original_model,
            final_codex_model,
            reasoning_effort.as_str(),
            original_model
        ));

        let (chat_messages, extracted_skills) =
            MessageProcessor::transform_messages(&anthropic_body.messages, log_tx);
        let (promoted_context, cleaned_chat_messages) =
            extract_user_scaffolding_to_instructions(&chat_messages);

        // 构建 input 数组（只包含当前请求上下文，不注入静态模板文件）
        let mut final_input: Vec<Value> = Vec::new();

        let request_text_corpus = collect_request_text_corpus(anthropic_body);
        let request_cwd = extract_request_cwd(&request_text_corpus);
        let request_session_hint = extract_request_session_hint(anthropic_body);
        let compact_summary_request = is_compact_summary_request(anthropic_body);
        let augmentation = decide_request_augmentation(anthropic_body, &request_text_corpus);
        let apply_agent_augmentations = augmentation.is_agent();
        let apply_plan_augmentations = matches!(augmentation.mode, RequestAugmentationMode::Plan);
        let augmentation_reasons = if augmentation.reasons.is_empty() {
            "none".to_string()
        } else {
            augmentation.reasons.join(",")
        };
        log(&format!(
            "🧩 [Transform] request_augmentation_mode={} augmentation_reasons=[{}]",
            augmentation.mode_label(),
            augmentation_reasons
        ));
        let trimmed_custom_injection_prompt = custom_injection_prompt.trim();
        let forced_skill_routing = if apply_agent_augmentations && enable_skill_routing_hint {
            detect_requested_skill_name(anthropic_body)
        } else {
            None
        };
        let custom_prompt_applied = (apply_agent_augmentations || apply_plan_augmentations)
            && !trimmed_custom_injection_prompt.is_empty();
        let plan_prompt_applied = apply_plan_augmentations;
        let proxy_instruction_context = merge_instruction_context(
            custom_prompt_applied.then_some(trimmed_custom_injection_prompt),
            plan_prompt_applied.then_some(ANTHROPIC_COMPAT_PLAN_MODE_PROMPT),
        );
        let promoted_instruction_context = merge_instruction_context(
            proxy_instruction_context.as_deref(),
            promoted_context.as_deref(),
        );
        let mut applied_static_instructions = promoted_instruction_context.clone();
        // 归一化 system prompt，并提升到 Responses 顶层 instructions
        if let Some(system) = &anthropic_body.system {
            let mut system_text = system.to_string();
            let sanitized_system_text = strip_dynamic_system_header_lines(&system_text);
            if sanitized_system_text != system_text {
                log("📋 [Transform] Stripped dynamic system header lines");
                system_text = sanitized_system_text;
            }
            if apply_plan_augmentations {
                let sanitized_plan_system_text =
                    sanitize_plan_mode_system_text_for_codex(&system_text);
                if sanitized_plan_system_text != system_text {
                    log("📋 [Transform] Stripped Claude-native plan orchestration from system prompt");
                    system_text = sanitized_plan_system_text;
                }
            }
            log(&format!(
                "📋 [Transform] System prompt: {} chars",
                system_text.len()
            ));
            applied_static_instructions =
                merge_instruction_context(Some(system_text.as_str()), promoted_instruction_context.as_deref());

            if request_contains_environment_context(&request_text_corpus) {
                log("📋 [Transform] Skip runtime <environment_context> injection (already present in request text)");
            } else {
                log("📋 [Transform] Skip runtime <environment_context> injection (no trusted request cwd)");
            }
        } else if let Some(instruction_context) = proxy_instruction_context.as_deref() {
            let context_chars = count_normalized_chars(instruction_context);
            log(&format!(
                "🎯 [Transform] Promoting injected instruction context to top-level instructions ({} chars)",
                context_chars
            ));
            applied_static_instructions = Some(instruction_context.to_string());
        }

        if !extracted_skills.is_empty() {
            log(&format!(
                "🎯 [Transform] Promoting {} extracted skill(s) into instructions",
                extracted_skills.len()
            ));
            let mut seen_skill_keys = std::collections::HashSet::new();
            for skill in &extracted_skills {
                if !seen_skill_keys.insert(skill.dedupe_key()) {
                    log(&format!(
                        "🎯 [Transform] Skip duplicate extracted skill for instructions: {}",
                        skill.name
                    ));
                    continue;
                }
                applied_static_instructions = merge_instruction_context(
                    applied_static_instructions.as_deref(),
                    Some(skill.as_str()),
                );
            }
        }
        // 追加对话历史
        final_input.extend(cleaned_chat_messages);
        Self::sanitize_input_for_codex(&mut final_input);
        reconcile_function_call_pairs(&mut final_input);

        // 转换工具
        let mut transformed_tools = Self::transform_tools(
            anthropic_body.tools.as_ref(),
            log_tx,
            enable_tool_schema_compaction,
            apply_plan_augmentations,
        );
        adjust_agent_tool_semantics_for_request(anthropic_body, &mut transformed_tools);
        if compact_summary_request {
            transformed_tools.clear();
        }
        let transformed_tools_bytes = serde_json::to_vec(&transformed_tools)
            .map(|buf| buf.len())
            .unwrap_or(0);
        let static_heavy_payload_stats = build_static_heavy_payload_stats(
            applied_static_instructions.as_deref(),
            promoted_instruction_context.as_deref(),
            &transformed_tools,
            transformed_tools_bytes,
        );

        log(&format!(
            "📋 [Transform] Final: {} input items, {} tools",
            final_input.len(),
            transformed_tools.len()
        ));

        let tool_choice = (!compact_summary_request).then(|| {
            Self::build_tool_choice(
                anthropic_body,
                &transformed_tools,
                &augmentation,
                forced_skill_routing.as_ref().map(|_| "Skill"),
            )
        }).flatten();
        let parallel_tool_calls = if compact_summary_request {
            false
        } else {
            Self::resolve_parallel_tool_calls(anthropic_body)
        };
        log(&format!(
            "🧰 [Transform] Resolved tool_choice={} parallel_tool_calls={} (tools={})",
            serde_json::to_string(&tool_choice).unwrap_or_else(|_| "\"auto\"".to_string()),
            parallel_tool_calls,
            transformed_tools.len()
        ));
        let prompt_cache_key = build_prompt_cache_key(
            request_cwd.as_deref(),
            final_codex_model,
            request_session_hint.as_deref(),
            None,
            applied_static_instructions.as_deref(),
            None,
            static_heavy_payload_stats.tools_fingerprint.as_deref(),
        );
        let thinking_disabled = anthropic_body.is_thinking_disabled();
        let reasoning_summary_mode = if thinking_disabled || compact_summary_request {
            None
        } else {
            Some(resolve_reasoning_summary_mode())
        };
        let include_reasoning_encrypted_content =
            !thinking_disabled && !compact_summary_request && resolve_include_reasoning_encrypted_content();
        let reasoning_summary_requested = reasoning_summary_mode.is_some();
        log(&format!(
            "🧠 [Transform] thinking_disabled={} reasoning.summary_requested={} reasoning.summary={} include.reasoning.encrypted_content={}",
            thinking_disabled,
            reasoning_summary_requested,
            reasoning_summary_mode.as_deref().unwrap_or("omitted"),
            include_reasoning_encrypted_content
        ));
        log(&format!(
            "🧩 [Transform] custom_prompt_applied={} plan_prompt_applied={}",
            custom_prompt_applied,
            plan_prompt_applied
        ));
        log(&format!(
            "🧾 [Transform] static_heavy wrapped_system_fingerprint={} wrapped_system_chars={} custom_prompt_fingerprint={} custom_prompt_chars={} tools_fingerprint={} tools_count={} tools_bytes={}",
            format_optional_fingerprint(static_heavy_payload_stats.wrapped_system_fingerprint.as_deref()),
            static_heavy_payload_stats.wrapped_system_chars,
            format_optional_fingerprint(static_heavy_payload_stats.custom_prompt_fingerprint.as_deref()),
            static_heavy_payload_stats.custom_prompt_chars,
            format_optional_fingerprint(static_heavy_payload_stats.tools_fingerprint.as_deref()),
            static_heavy_payload_stats.tools_count,
            static_heavy_payload_stats.tools_bytes,
        ));

        let mut reasoning = json!({ "effort": reasoning_effort.as_str() });
        if let Some(summary_mode) = reasoning_summary_mode.as_deref() {
            if let Some(reasoning_obj) = reasoning.as_object_mut() {
                reasoning_obj.insert("summary".to_string(), json!(summary_mode));
            }
        }

        let mut body = json!({
            "model": final_codex_model,
            "input": final_input,
            "reasoning": reasoning,
            "store": false,
            "stream": true,
            "prompt_cache_key": prompt_cache_key
        });
        if !compact_summary_request {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("tools".to_string(), Value::Array(transformed_tools.clone()));
                obj.insert("parallel_tool_calls".to_string(), json!(parallel_tool_calls));
            }
        }
        if let Some(instructions) = applied_static_instructions.as_deref() {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("instructions".to_string(), json!(instructions));
            }
        }
        if let Some(tool_choice) = tool_choice {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("tool_choice".to_string(), tool_choice);
            }
        }
        if enable_codex_fast_mode {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("service_tier".to_string(), json!("priority"));
            }
        }
        if include_reasoning_encrypted_content {
            if let Some(obj) = body.as_object_mut() {
                obj.insert(
                    "include".to_string(),
                    json!(["reasoning.encrypted_content"]),
                );
            }
        }

        (body, session_id.clone())
    }

    fn tool_name(tool: &Value) -> Option<&str> {
        tool.get("name")
            .and_then(|value| value.as_str())
            .or_else(|| {
                tool.get("function")
                    .and_then(|value| value.get("name"))
                    .and_then(|value| value.as_str())
            })
    }

    fn tool_parameters_schema(tool: &Value) -> Option<&Value> {
        tool.get("input_schema").or_else(|| {
            tool.get("function")
                .and_then(|value| value.get("parameters"))
        })
    }

    fn tool_strict(tool: &Value) -> Option<bool> {
        tool.get("function")
            .and_then(|value| value.get("strict"))
            .and_then(|value| value.as_bool())
            .or_else(|| tool.get("strict").and_then(|value| value.as_bool()))
    }

    fn is_native_responses_tool_type(tool_type: &str) -> bool {
        NATIVE_RESPONSES_TOOL_TYPES
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(tool_type))
    }

    fn build_function_tool(
        name: &str,
        description: Option<&str>,
        parameters: Value,
        strict: Option<bool>,
    ) -> Value {
        let mut tool = Map::new();
        tool.insert("type".to_string(), json!("function"));
        tool.insert("name".to_string(), json!(name));
        tool.insert(
            "description".to_string(),
            json!(compact_tool_description(description)),
        );
        if let Some(strict) = strict {
            tool.insert("strict".to_string(), json!(strict));
        }
        tool.insert("parameters".to_string(), parameters);
        Value::Object(tool)
    }

    fn resolve_named_tool_choice(requested_name: &str, transformed_tools: &[Value]) -> Value {
        let requested_name = requested_name.trim();
        if requested_name.is_empty() {
            return json!("auto");
        }

        let wants_native_web_search = requested_name.eq_ignore_ascii_case("WebSearch")
            || requested_name.eq_ignore_ascii_case("web_search")
            || requested_name.eq_ignore_ascii_case("web-search");
        if wants_native_web_search
            && transformed_tools
                .iter()
                .any(|tool| tool.get("type").and_then(|value| value.as_str()) == Some("web_search"))
        {
            return json!({ "type": "web_search" });
        }

        for tool in transformed_tools {
            let tool_type = tool.get("type").and_then(|value| value.as_str());
            match tool_type {
                Some("custom") => {
                    if let Some(name) = tool.get("name").and_then(|value| value.as_str()) {
                        if name.eq_ignore_ascii_case(requested_name) {
                            return json!({ "type": "custom", "name": name });
                        }
                    }
                }
                Some(tool_type) if Self::is_native_responses_tool_type(tool_type) => {
                    if tool_type.eq_ignore_ascii_case(requested_name) {
                        return json!({ "type": tool_type });
                    }
                }
                _ => {
                    if let Some(name) = tool.get("name").and_then(|value| value.as_str()) {
                        if name.eq_ignore_ascii_case(requested_name) {
                            return json!({ "type": "function", "name": name });
                        }
                    }
                }
            }
        }

        if Self::is_native_responses_tool_type(requested_name)
            && !requested_name.eq_ignore_ascii_case("custom")
        {
            return json!({ "type": requested_name });
        }

        json!({ "type": "function", "name": requested_name })
    }

    fn resolve_parallel_tool_calls(anthropic_body: &AnthropicRequest) -> bool {
        anthropic_body
            .tool_choice
            .as_ref()
            .and_then(|tool_choice| tool_choice.as_object())
            .and_then(|tool_choice| tool_choice.get("disable_parallel_tool_use"))
            .and_then(|value| value.as_bool())
            .map(|disabled| !disabled)
            .unwrap_or(true)
    }

    fn build_tool_choice(
        anthropic_body: &AnthropicRequest,
        transformed_tools: &[Value],
        augmentation: &RequestAugmentationDecision,
        forced_tool_name: Option<&str>,
    ) -> Option<Value> {
        if matches!(augmentation.mode, RequestAugmentationMode::Plan)
            && request_targets_blacklisted_plan_mode_tool(anthropic_body)
        {
            return None;
        }

        if transformed_tools.is_empty() {
            return Some(json!("none"));
        }

        if let Some(forced_tool_name) = forced_tool_name {
            if anthropic_body.tool_choice.is_none() {
                return Some(Self::resolve_named_tool_choice(forced_tool_name, transformed_tools));
            }
        }

        let Some(tool_choice) = anthropic_body.tool_choice.as_ref() else {
            return None;
        };

        match tool_choice {
            Value::String(choice) => match choice.trim().to_ascii_lowercase().as_str() {
                "auto" => Some(json!("auto")),
                "none" => Some(json!("none")),
                "required" | "any" => Some(json!("required")),
                _ => None,
            },
            Value::Object(object) => {
                let choice_type = object
                    .get("type")
                    .and_then(|value| value.as_str())
                    .map(|value| value.trim().to_ascii_lowercase());

                match choice_type.as_deref() {
                    Some("auto") => Some(json!("auto")),
                    Some("none") => Some(json!("none")),
                    Some("required") | Some("any") => Some(json!("required")),
                    Some("tool") | Some("function") | Some("custom") => object
                        .get("name")
                        .or_else(|| object.get("tool_name"))
                        .or_else(|| object.get("toolName"))
                        .and_then(|value| value.as_str())
                        .map(|name| Self::resolve_named_tool_choice(name, transformed_tools)),
                    Some(tool_type) if Self::is_native_responses_tool_type(tool_type) => {
                        if tool_type == "custom" {
                            object
                                .get("name")
                                .and_then(|value| value.as_str())
                                .map(|name| json!({ "type": "custom", "name": name }))
                        } else {
                            Some(json!({ "type": tool_type }))
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn is_anthropic_web_search_tool(tool: &Value) -> bool {
        let Some(name) = Self::tool_name(tool) else {
            return false;
        };
        if !name.eq_ignore_ascii_case("WebSearch") {
            return false;
        }

        let Some(schema) = Self::tool_parameters_schema(tool) else {
            return false;
        };

        schema
            .get("properties")
            .and_then(|value| value.get("query"))
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
            == Some("string")
    }

    fn build_native_web_search_tool() -> Value {
        json!({
            "type": "web_search",
            "external_web_access": true
        })
    }

    fn sanitize_input_for_codex(input: &mut Vec<Value>) {
        for item in input.iter_mut() {
            let Some(obj) = item.as_object_mut() else {
                continue;
            };

            let item_type = obj
                .get("type")
                .and_then(|v| v.as_str())
                .map(|value| value.to_string());
            if item_type.as_deref() == Some("function_call") {
                obj.remove("signature");
                if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                    obj.insert(
                        "name".to_string(),
                        json!(sanitize_function_call_name_for_codex(name)),
                    );
                }
            }

            if item_type.as_deref() == Some("message") {
                let role = obj
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("assistant")
                    .to_string();
                if let Some(content_blocks) = obj.get_mut("content").and_then(|v| v.as_array_mut())
                {
                    for block in content_blocks.iter_mut() {
                        let Some(block_obj) = block.as_object_mut() else {
                            continue;
                        };

                        if block_obj.get("type").and_then(|v| v.as_str()) == Some("thinking") {
                            // Codex upstream does not accept `thinking` blocks in message.content.
                            // Normalize to regular text blocks by role.
                            if !block_obj.contains_key("text") {
                                if let Some(thinking_value) = block_obj.remove("thinking") {
                                    block_obj.insert("text".to_string(), thinking_value);
                                }
                            } else {
                                block_obj.remove("thinking");
                            }
                            let normalized_type = if role.eq_ignore_ascii_case("user") {
                                "input_text"
                            } else {
                                "output_text"
                            };
                            block_obj.insert("type".to_string(), json!(normalized_type));
                            block_obj.remove("signature");
                        }

                        let block_type =
                            block_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if (block_type == "input_text"
                            || block_type == "output_text"
                            || block_type == "text")
                            && block_obj.get("text").and_then(|v| v.as_str()).is_some()
                        {
                            let text = block_obj
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            if let Some(sanitized_text) = sanitize_message_text_for_codex(text) {
                                block_obj.insert("text".to_string(), json!(sanitized_text));
                            } else {
                                block_obj.insert("text".to_string(), json!(""));
                            }
                        }
                    }

                    content_blocks.retain(|block| {
                        let Some(block_obj) = block.as_object() else {
                            return true;
                        };
                        let block_type =
                            block_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if block_type == "input_text"
                            || block_type == "output_text"
                            || block_type == "text"
                        {
                            return block_obj
                                .get("text")
                                .and_then(|v| v.as_str())
                                .map(|text| !text.trim().is_empty())
                                .unwrap_or(false);
                        }
                        true
                    });
                }
            }

            if item_type.as_deref() == Some("function_call_output") {
                if let Some(output) = obj.get_mut("output") {
                    let normalized = normalize_function_call_output_value_for_codex(output);
                    *output = normalized;
                }
            }
        }

        input.retain(|item| {
            let Some(obj) = item.as_object() else {
                return true;
            };

            match obj.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "message" => obj
                    .get("content")
                    .and_then(|v| v.as_array())
                    .map(|blocks| !blocks.is_empty())
                    .unwrap_or(false),
                "function_call_output" => obj
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .map(|call_id| !call_id.trim().is_empty())
                    .unwrap_or(false),
                _ => true,
            }
        });
    }

    fn transform_tools(
        tools: Option<&Vec<Value>>,
        log_tx: Option<&broadcast::Sender<String>>,
        enable_tool_schema_compaction: bool,
        apply_plan_mode_blacklist: bool,
    ) -> Vec<Value> {
        // 获取全局日志记录器
        let logger = AppLogger::get();

        // 辅助函数：同时写入 broadcast 和文件
        let log = |msg: &str| {
            if is_debug_log_enabled() {
                if let Some(tx) = log_tx {
                    let _ = tx.send(msg.to_string());
                }
                if let Some(ref l) = logger {
                    l.log(msg);
                }
            }
        };

        let Some(tools) = tools else {
            log("🔧 [Tools] No tools provided");
            return Vec::new();
        };

        if tools.is_empty() {
            log("🔧 [Tools] Empty tools array");
            return Vec::new();
        }

        log(&format!("🔧 [Tools] Processing {} tools", tools.len()));
        let before_bytes = serde_json::to_vec(tools).map(|buf| buf.len()).unwrap_or(0);

        let mut transformed = Vec::with_capacity(tools.len());
        let mut native_web_search_added = false;

        for tool in tools {
            if apply_plan_mode_blacklist {
                if let Some(name) = Self::tool_name(tool) {
                    if is_plan_mode_tool_blacklisted(name) {
                        log(&format!(
                            "🔧 [Tools] {} skipped by plan-mode blacklist",
                            name
                        ));
                        continue;
                    }
                }
            }

            if Self::is_anthropic_web_search_tool(tool) {
                if native_web_search_added {
                    log("🔧 [Tools] WebSearch duplicate skipped after native mapping");
                } else {
                    log("🔧 [Tools] WebSearch (Anthropic official) -> native web_search");
                    transformed.push(Self::build_native_web_search_tool());
                    native_web_search_added = true;
                }
                continue;
            }

            if let Some(tool_type) = tool.get("type").and_then(|value| value.as_str()) {
                if Self::is_native_responses_tool_type(tool_type) {
                    if tool_type == "web_search" {
                        if native_web_search_added {
                            log("🔧 [Tools] web_search duplicate skipped after native mapping");
                        } else {
                            log("🔧 [Tools] web_search (native passthrough)");
                            transformed.push(tool.clone());
                            native_web_search_added = true;
                        }
                    } else {
                        log(&format!("🔧 [Tools] {} (native passthrough)", tool_type));
                        transformed.push(tool.clone());
                    }
                    continue;
                }
            }

            // Claude Code 格式: { name, description, input_schema }
            if tool.get("name").is_some() && tool.get("type").is_none() {
                let name = tool
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                log(&format!("🔧 [Tools] {} (Claude Code format)", name));

                let mut parameters = tool.get("input_schema").cloned().unwrap_or_else(|| {
                    json!({
                        "type": "object",
                        "properties": {}
                    })
                });

                if let Some(obj) = parameters.as_object_mut() {
                    obj.entry("properties").or_insert_with(|| json!({}));
                }
                if enable_tool_schema_compaction {
                    compact_tool_parameters_schema(&mut parameters);
                }

                transformed.push(Self::build_function_tool(
                    name,
                    tool.get("description").and_then(|d| d.as_str()),
                    parameters,
                    Self::tool_strict(tool),
                ));
                continue;
            }

            // Anthropic 格式: { type: "tool", name, ... }
            if tool.get("type").and_then(|t| t.as_str()) == Some("tool") {
                let name = tool
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                log(&format!("🔧 [Tools] {} (Anthropic format)", name));

                let mut parameters = tool.get("input_schema").cloned().unwrap_or_else(|| {
                    json!({
                        "type": "object",
                        "properties": {}
                    })
                });

                if let Some(obj) = parameters.as_object_mut() {
                    obj.entry("properties").or_insert_with(|| json!({}));
                }
                if enable_tool_schema_compaction {
                    compact_tool_parameters_schema(&mut parameters);
                }

                transformed.push(Self::build_function_tool(
                    name,
                    tool.get("description").and_then(|d| d.as_str()),
                    parameters,
                    Self::tool_strict(tool),
                ));
                continue;
            }

            // OpenAI 格式: { type: "function", function: {...} }
            if tool.get("type").and_then(|t| t.as_str()) == Some("function") {
                let func = tool.get("function").unwrap_or(tool);
                let name = func
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                log(&format!("🔧 [Tools] {} (OpenAI format)", name));

                let mut parameters = func.get("parameters").cloned().unwrap_or_else(|| {
                    json!({
                        "type": "object",
                        "properties": {}
                    })
                });

                if let Some(obj) = parameters.as_object_mut() {
                    obj.entry("properties").or_insert_with(|| json!({}));
                }
                if enable_tool_schema_compaction {
                    compact_tool_parameters_schema(&mut parameters);
                }

                transformed.push(Self::build_function_tool(
                    name,
                    func.get("description").and_then(|d| d.as_str()),
                    parameters,
                    Self::tool_strict(tool),
                ));
                continue;
            }

            // 未知格式
            let name = tool
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            log(&format!("🔧 [Tools] {} (unknown format)", name));

            let mut parameters = tool.get("input_schema").cloned().unwrap_or_else(|| {
                json!({
                    "type": "object",
                    "properties": {}
                })
            });

            if let Some(obj) = parameters.as_object_mut() {
                obj.entry("properties").or_insert_with(|| json!({}));
            }
            if enable_tool_schema_compaction {
                compact_tool_parameters_schema(&mut parameters);
            }

            transformed.push(Self::build_function_tool(
                name,
                tool.get("description").and_then(|d| d.as_str()),
                parameters,
                Self::tool_strict(tool),
            ));
        }

        let after_bytes = serde_json::to_vec(&transformed)
            .map(|buf| buf.len())
            .unwrap_or(0);
        log(&format!(
            "🔧 [Tools] bytes_before={} bytes_after={} schema_compaction={}",
            before_bytes, after_bytes, enable_tool_schema_compaction
        ));

        transformed
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_request_text_corpus, compact_tool_description, decide_request_augmentation,
        detect_requested_skill_name, extract_request_cwd, fingerprint_json_value,
        strip_dynamic_system_header_lines, RequestAugmentationMode, TransformRequest,
        ANTHROPIC_COMPAT_PLAN_MODE_PROMPT,
    };
    use crate::models::{AnthropicRequest, ReasoningEffortMapping};
    use crate::transform::MessageProcessor;
    use serde_json::{json, Value};

    fn sample_request() -> AnthropicRequest {
        serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role": "user",
                    "content": [{"type":"text","text":"hello"}]
                }
            ],
            "system": "System prompt",
            "stream": true
        }))
        .expect("valid anthropic request")
    }

    fn sample_passthrough_request() -> AnthropicRequest {
        serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role": "user",
                    "content": [{"type":"text","text":"hello"}]
                }
            ]
        }))
        .expect("valid anthropic request")
    }

    fn input_texts(body: &Value) -> Vec<String> {
        body.get("input")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .map(|text| text.to_string())
            .collect()
    }

    fn instructions_text(body: &Value) -> Option<String> {
        body.get("instructions")
            .and_then(|v| v.as_str())
            .map(|text| text.to_string())
    }

    fn contains_wrapped_agents_user_input(body: &Value) -> bool {
        input_texts(body)
            .iter()
            .any(|text| text.starts_with("# AGENTS.md instructions"))
    }

    fn tool_names(body: &Value) -> Vec<String> {
        body.get("tools")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
            .map(|name| name.to_string())
            .collect()
    }

    fn sample_tool() -> Value {
        json!({
            "name": "Read",
            "description": "Read files",
            "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
        })
    }

    #[test]
    fn transform_does_not_inject_skill_catalog_hint_for_skill_list_queries() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"我当前有哪些技能"}],
            "system": "<system-reminder>\nThe following skills are available:\n- figma-implement-design: Translate Figma nodes\n- pdf: Read PDF files\n- review-pr: Review pull request\n</system-reminder>",
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let catalog_hint_count = input_items
            .iter()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .filter(|text| text.contains("Skill catalog snapshot from current system-reminder."))
            .count();

        assert_eq!(
            catalog_hint_count, 0,
            "skill list queries should not receive forced catalog hint injection"
        );
    }

    #[test]
    fn skill_catalog_system_reminder_is_compacted_for_codex() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content": "<system-reminder>
The following skills are available for use with the Skill tool:
- figma-implement-design: Translate Figma nodes
TRIGGER when: user provides a Figma URL
- pdf: Read PDF files
</system-reminder>

hello"
            }],
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(instructions.contains("figma-implement-design"));
        assert!(instructions.contains("pdf: Read PDF files"));
        assert!(instructions.contains("TRIGGER when: user provides a Figma URL"));
        let joined = input_texts(&body).join("\n");
        assert!(!joined.contains("The following skills are available"));
        assert!(joined.contains("hello"));
    }

    #[test]
    fn strip_dynamic_system_header_lines_removes_billing_header() {
        let input = "x-anthropic-billing-header: cc_version=2.1.72.873; cc_entrypoint=cli; cch=abcd;\nYou are Claude Code.\nKeep this line.";
        let output = strip_dynamic_system_header_lines(input);
        assert_eq!(output, "You are Claude Code.\nKeep this line.");
    }
    #[test]
    fn compact_tool_description_truncates_and_normalizes() {
        let description = "First line with    extra spaces.\nSecond line stays in first paragraph.\n\nSecond paragraph should be dropped.";
        let compacted = compact_tool_description(Some(description));
        assert!(
            compacted
                .contains("First line with extra spaces. Second line stays in first paragraph."),
            "whitespace should be normalized and first paragraph preserved"
        );
        assert!(
            !compacted.contains("Second paragraph should be dropped"),
            "only first paragraph should remain"
        );

        let long = "a".repeat(600);
        let compacted_long = compact_tool_description(Some(&long));
        assert!(
            compacted_long.chars().count() <= 243,
            "long descriptions should be clipped with ellipsis"
        );
        assert!(
            compacted_long.ends_with("..."),
            "clipped descriptions should end with ellipsis"
        );
    }

    #[test]
    fn transform_tools_compacts_verbose_descriptions() {
        let long_description = "Bash tool long description. ".repeat(120);
        let tools = vec![json!({
            "name": "Bash",
            "description": long_description,
            "input_schema": {"type":"object","properties":{"command":{"type":"string"}}}
        })];
        let transformed = TransformRequest::transform_tools(Some(&tools), None, true, false);
        assert_eq!(transformed.len(), 1, "one tool should remain");
        let description = transformed[0]
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            description.chars().count() <= 243,
            "tool description should be compacted for faster request payloads"
        );
    }

    #[test]
    fn transform_tools_compacts_schema_display_fields() {
        let tools = vec![json!({
            "name": "Read",
            "description": "Read file",
            "input_schema": {
                "title": "ReadInput",
                "type": "object",
                "description": "Read helper schema ".repeat(30),
                "deprecated": false,
                "examples": [{"file_path":"/tmp/a.txt"}],
                "properties": {
                    "file_path": {
                        "type": "string",
                        "title": "Path",
                        "readOnly": false,
                        "description": "Absolute path to read ".repeat(20)
                    }
                },
                "required": ["file_path"]
            }
        })];

        let transformed = TransformRequest::transform_tools(Some(&tools), None, true, false);
        let parameters = transformed[0]
            .get("parameters")
            .cloned()
            .unwrap_or_default();
        let parameters_obj = parameters.as_object().expect("schema object");

        assert!(
            !parameters_obj.contains_key("title"),
            "title should be removed from compacted schema"
        );
        assert!(
            !parameters_obj.contains_key("examples"),
            "examples should be removed from compacted schema"
        );
        assert!(
            !parameters_obj.contains_key("deprecated"),
            "deprecated should be removed from compacted schema"
        );
        assert_eq!(
            parameters.pointer("/required/0").and_then(|v| v.as_str()),
            Some("file_path"),
            "required fields should be preserved"
        );
        assert!(
            parameters
                .pointer("/properties/file_path/type")
                .and_then(|v| v.as_str())
                == Some("string"),
            "property types should be preserved"
        );
        assert!(
            parameters
                .pointer("/properties/file_path/description")
                .and_then(|v| v.as_str())
                .map(|text| text.chars().count() <= 123)
                .unwrap_or(false),
            "nested schema description should be compacted"
        );
    }

    #[test]
    fn transform_tools_keeps_schema_when_compaction_disabled() {
        let tools = vec![json!({
            "name": "Read",
            "description": "Read file",
            "input_schema": {
                "title": "ReadInput",
                "type": "object",
                "examples": [{"file_path":"/tmp/a.txt"}],
                "properties": {"file_path": {"type":"string"}}
            }
        })];

        let transformed = TransformRequest::transform_tools(Some(&tools), None, false, false);
        let parameters = transformed[0]
            .get("parameters")
            .cloned()
            .unwrap_or_default();
        let parameters_obj = parameters.as_object().expect("schema object");
        assert!(
            parameters_obj.contains_key("title"),
            "title should be kept when schema compaction disabled"
        );
        assert!(
            parameters_obj.contains_key("examples"),
            "examples should be kept when schema compaction disabled"
        );
    }

    #[test]
    fn transform_tools_maps_official_websearch_to_native_web_search() {
        let tools = vec![json!({
            "name": "WebSearch",
            "description": "Search the web",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "allowed_domains": {"type": "array", "items": {"type": "string"}},
                    "blocked_domains": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["query"]
            }
        })];

        let transformed = TransformRequest::transform_tools(Some(&tools), None, true, false);
        assert_eq!(
            transformed.len(),
            1,
            "web search tool should remain singular after mapping"
        );
        assert_eq!(
            transformed[0].get("type").and_then(|value| value.as_str()),
            Some("web_search")
        );
        assert_eq!(
            transformed[0]
                .get("external_web_access")
                .and_then(|value| value.as_bool()),
            Some(true),
            "native web_search should enable live web access"
        );
        assert!(
            transformed[0].get("name").is_none(),
            "native web_search should not be serialized as a generic function tool"
        );
    }

    #[test]
    fn transform_tools_preserves_explicit_strict_flag() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "Read",
                "description": "Read file",
                "strict": true,
                "parameters": {
                    "type": "object",
                    "properties": {"file_path": {"type": "string"}},
                    "required": ["file_path"]
                }
            }
        })];

        let transformed = TransformRequest::transform_tools(Some(&tools), None, true, false);
        assert_eq!(
            transformed[0]
                .get("strict")
                .and_then(|value| value.as_bool()),
            Some(true),
            "explicit strict flag should be preserved"
        );
    }

    #[test]
    fn transform_tools_omits_strict_when_unspecified() {
        let tools = vec![json!({
            "name": "Read",
            "description": "Read file",
            "input_schema": {
                "type": "object",
                "properties": {"file_path": {"type": "string"}}
            }
        })];

        let transformed = TransformRequest::transform_tools(Some(&tools), None, true, false);
        assert!(
            transformed[0].get("strict").is_none(),
            "strict should be omitted when upstream tool does not specify it"
        );
    }

    #[test]
    fn transform_tools_passthroughs_native_apply_patch_tool() {
        let tools = vec![json!({
            "type": "apply_patch"
        })];

        let transformed = TransformRequest::transform_tools(Some(&tools), None, true, false);
        assert_eq!(
            transformed, tools,
            "native responses tools should pass through unchanged"
        );
    }

    #[test]
    fn transform_tools_strips_agent_worktree_isolation_for_ordinary_subagent_requests() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"用两个subagent搜索下 上海北京的天气"}],
            "tools": [{
                "name": "Agent",
                "description": "Launch a new agent to handle complex tasks autonomously.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "description": {"type": "string"},
                        "isolation": {"type": "string", "enum": ["worktree"]},
                        "prompt": {"type": "string"}
                    },
                    "required": ["description", "prompt"]
                }
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let agent_tool = body
            .get("tools")
            .and_then(|value| value.as_array())
            .and_then(|tools| tools.iter().find(|tool| tool.get("name").and_then(Value::as_str) == Some("Agent")))
            .cloned()
            .expect("agent tool should exist");
        let parameters = agent_tool
            .get("parameters")
            .and_then(Value::as_object)
            .expect("agent parameters");

        assert!(
            parameters
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| !properties.contains_key("isolation"))
                .unwrap_or(false),
            "ordinary subagent requests should not expose worktree isolation in Agent schema"
        );
    }

    #[test]
    fn transform_tools_ignores_system_reminder_worktree_mentions_for_ordinary_subagent_requests() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "<system-reminder>Use when starting feature work that needs isolation from current workspace or before executing implementation plans - creates isolated git worktrees with smart directory selection and safety verification</system-reminder>"
                    },
                    {
                        "type": "text",
                        "text": "用两个subagent搜索下 上海北京的天气"
                    }
                ]
            }],
            "tools": [{
                "name": "Agent",
                "description": "Launch a new agent to handle complex tasks autonomously.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "description": {"type": "string"},
                        "isolation": {"type": "string", "enum": ["worktree"]},
                        "prompt": {"type": "string"}
                    },
                    "required": ["description", "prompt"]
                }
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let agent_tool = body
            .get("tools")
            .and_then(|value| value.as_array())
            .and_then(|tools| tools.iter().find(|tool| tool.get("name").and_then(Value::as_str) == Some("Agent")))
            .cloned()
            .expect("agent tool should exist");
        let parameters = agent_tool
            .get("parameters")
            .and_then(Value::as_object)
            .expect("agent parameters");

        assert!(
            parameters
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| !properties.contains_key("isolation"))
                .unwrap_or(false),
            "system-reminder worktree mentions should not keep Agent isolation visible for ordinary subagent requests"
        );
    }

    #[test]
    fn transform_tools_keeps_agent_worktree_isolation_when_user_explicitly_mentions_worktree() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"请在 worktree 里开一个 subagent 处理这个任务"}],
            "tools": [{
                "name": "Agent",
                "description": "Launch a new agent to handle complex tasks autonomously.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "description": {"type": "string"},
                        "isolation": {"type": "string", "enum": ["worktree"]},
                        "prompt": {"type": "string"}
                    },
                    "required": ["description", "prompt"]
                }
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let agent_tool = body
            .get("tools")
            .and_then(|value| value.as_array())
            .and_then(|tools| tools.iter().find(|tool| tool.get("name").and_then(Value::as_str) == Some("Agent")))
            .cloned()
            .expect("agent tool should exist");
        let parameters = agent_tool
            .get("parameters")
            .and_then(Value::as_object)
            .expect("agent parameters");

        assert!(
            parameters
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| properties.contains_key("isolation"))
                .unwrap_or(false),
            "explicit worktree requests should keep Agent isolation option visible"
        );
    }

    #[test]
    fn plan_mode_filters_worktree_tools_without_removing_agent_tool() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "请先给我 plan"}],
            "metadata": {"plan_mode": true},
            "tools": [
                {
                    "name": "Agent",
                    "description": "Launch a new agent to handle complex tasks autonomously.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "description": {"type": "string"},
                            "prompt": {"type": "string"}
                        },
                        "required": ["description", "prompt"]
                    }
                },
                {
                    "name": "EnterWorktree",
                    "description": "Enter worktree",
                    "input_schema": {"type":"object","properties":{}}
                },
                {
                    "name": "ExitWorktree",
                    "description": "Exit worktree",
                    "input_schema": {"type":"object","properties":{"action":{"type":"string"}}}
                }
            ],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let tool_names = tool_names(&body);

        assert!(tool_names.contains(&"Agent".to_string()));
        assert!(!tool_names.contains(&"EnterWorktree".to_string()));
        assert!(!tool_names.contains(&"ExitWorktree".to_string()));
    }

    #[test]
    fn prompt_cache_key_is_stable_across_requests() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();

        let (body_a, session_a) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");
        let (body_b, session_b) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert_ne!(session_a, session_b, "session id should remain per-request");
        assert_eq!(
            body_a.get("prompt_cache_key"),
            body_b.get("prompt_cache_key"),
            "prompt cache key should be stable for cache hits"
        );
        assert_ne!(
            body_a.get("prompt_cache_key").and_then(|v| v.as_str()),
            Some(session_a.as_str()),
            "prompt cache key should not use random session id"
        );
    }

    #[test]
    fn prompt_cache_key_uses_session_hint_from_metadata_user_id() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role":"user","content":[{"type":"text","text":"hello"}]}],
            "metadata": {"user_id": "user_abc_account__session_123e4567-e89b-12d3-a456-426614174000"},
            "stream": true
        }))
        .expect("request with metadata session hint");
        let mapping = ReasoningEffortMapping::default();

        let (body_a, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt A", "gpt-5.4");
        let (body_b, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt B", "gpt-5.4");

        let key_a = body_a
            .get("prompt_cache_key")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let key_b = body_b
            .get("prompt_cache_key")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        assert_eq!(
            key_a, key_b,
            "metadata session hint should stabilize cache key"
        );
        assert_eq!(
            key_a, "codex-proxy:gpt_5_4:session:123e4567_e89b_12d3_a456_426614174000",
            "cache key should use metadata session hint"
        );
    }

    #[test]
    fn prompt_cache_key_uses_session_hint_when_present() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":[{"type":"text","text":"hello"}]}],
            "system": "System prompt\nsession_id: abc-123",
            "stream": true
        }))
        .expect("request with session hint");
        let mapping = ReasoningEffortMapping::default();

        let (body_a, _) = TransformRequest::transform(
            &request,
            None,
            &mapping,
            "global prompt A",
            "gpt-5.3-codex",
        );
        let (body_b, _) = TransformRequest::transform(
            &request,
            None,
            &mapping,
            "global prompt B",
            "gpt-5.3-codex",
        );

        let key_a = body_a
            .get("prompt_cache_key")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let key_b = body_b
            .get("prompt_cache_key")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        assert_eq!(key_a, key_b, "session hint should stabilize cache key");
        assert_eq!(
            key_a, "codex-proxy:gpt_5_3_codex:session:abc_123",
            "cache key should use sanitized session hint"
        );
    }

    #[test]
    fn prompt_cache_key_changes_when_custom_prompt_changes() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"hello"}],
            "system": "System prompt",
            "tools": [{
                "name": "Read",
                "description": "Read files",
                "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid request");
        let mapping = ReasoningEffortMapping::default();

        let (body_a, _) = TransformRequest::transform(
            &request,
            None,
            &mapping,
            "global prompt A",
            "gpt-5.3-codex",
        );
        let (body_b, _) = TransformRequest::transform(
            &request,
            None,
            &mapping,
            "global prompt B",
            "gpt-5.3-codex",
        );

        assert_ne!(
            body_a.get("prompt_cache_key"),
            body_b.get("prompt_cache_key"),
            "cache key should rotate when active instructions change"
        );
    }

    #[test]
    fn no_local_cwd_injection_when_request_has_no_env_context() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let env_context_count = input_items
            .iter()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .filter(|text| text.contains("<environment_context>"))
            .count();
        assert_eq!(
            env_context_count, 0,
            "runtime environment context should not be injected when request has none"
        );

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("System prompt"),
            "instructions should carry system text when no trusted cwd is available"
        );
    }

    #[test]
    fn uses_request_cwd_when_present_for_agents_wrapper() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":"<environment_context><cwd>/Users/mr.j</cwd><approval_policy>on-request</approval_policy></environment_context>"
            }],
            "system": "System prompt",
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("System prompt"),
            "trusted request cwd should no longer alter the instruction wrapper shape"
        );
    }

    #[test]
    fn prompt_cache_key_uses_default_segment_without_request_cwd() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let key = body
            .get("prompt_cache_key")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            key.contains(":default:"),
            "cache key should use default cwd segment when trusted request cwd is absent"
        );
    }

    #[test]
    fn prompt_cache_key_changes_with_request_cwd() {
        let request_a: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":"<environment_context><cwd>/Users/mr.j/a</cwd><approval_policy>on-request</approval_policy></environment_context>"
            }],
            "system": "System prompt",
            "stream": true
        }))
        .expect("valid request A");
        let request_b: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":"<environment_context><cwd>/Users/mr.j/b</cwd><approval_policy>on-request</approval_policy></environment_context>"
            }],
            "system": "System prompt",
            "stream": true
        }))
        .expect("valid request B");
        let mapping = ReasoningEffortMapping::default();

        let (body_a, _) = TransformRequest::transform(
            &request_a,
            None,
            &mapping,
            "global prompt",
            "gpt-5.3-codex",
        );
        let (body_b, _) = TransformRequest::transform(
            &request_b,
            None,
            &mapping,
            "global prompt",
            "gpt-5.3-codex",
        );

        let key_a = body_a
            .get("prompt_cache_key")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let key_b = body_b
            .get("prompt_cache_key")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        assert_ne!(
            key_a, key_b,
            "cache key should change with trusted request cwd"
        );
        assert!(
            key_a.contains(":Users_mr_j_a:"),
            "cache key should include sanitized trusted cwd segment"
        );
        assert!(
            key_b.contains(":Users_mr_j_b:"),
            "cache key should include sanitized trusted cwd segment"
        );
    }

    #[test]
    fn extract_request_cwd_returns_none_without_environment_context() {
        let corpus = "hello world\nsystem text";
        assert!(
            extract_request_cwd(corpus).is_none(),
            "cwd should only come from explicit environment_context block"
        );
    }

    #[test]
    fn compact_summary_requests_drop_tools_and_visible_reasoning_summary() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "请总结刚才的对话"}],
            "system": "You are Claude Code, Anthropic's official CLI for Claude.\nYou are a helpful AI assistant tasked with summarizing conversations.",
            "tools": [sample_tool()],
            "tool_choice": {"type": "auto"},
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert!(
            body.get("tools").is_none(),
            "compact summary requests should omit tools to avoid tool_use turns"
        );
        assert!(
            body.get("parallel_tool_calls").is_none(),
            "compact summary requests should omit parallel_tool_calls when tools are disabled"
        );
        assert!(
            body.get("tool_choice").is_none(),
            "compact summary requests should not keep tool_choice once tools are removed"
        );
        assert!(
            body.pointer("/reasoning/summary").is_none(),
            "compact summary requests should omit visible reasoning summaries"
        );
        assert_eq!(
            body.pointer("/reasoning/effort").and_then(|v| v.as_str()),
            Some(crate::models::get_reasoning_effort("claude-sonnet-4-5", &mapping).as_str()),
            "compact summary requests should keep reasoning effort mapping"
        );
    }

    #[test]
    fn anthropic_request_parses_top_level_thinking_disabled() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "hello"}],
            "thinking": {
                "type": "disabled",
                "budget_tokens": 0,
                "unexpected": true
            },
            "stream": true
        }))
        .expect("valid anthropic request");

        assert!(
            request.is_thinking_disabled(),
            "top-level thinking.disabled should be preserved by request model"
        );
    }

    #[test]
    fn reasoning_summary_default_is_auto_and_include_omitted() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert_eq!(
            body.pointer("/reasoning/summary").and_then(|v| v.as_str()),
            Some("auto"),
            "default summary mode should be auto for lower token usage"
        );
        assert!(
            body.get("include").is_none(),
            "include should be omitted by default to avoid unnecessary response payload"
        );
    }

    #[test]
    fn thinking_disabled_omits_visible_reasoning_summary() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "hello"}],
            "thinking": {"type": "disabled"},
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert!(
            body.pointer("/reasoning/summary").is_none(),
            "thinking.disabled should omit visible reasoning summary"
        );
        assert_eq!(
            body.pointer("/reasoning/effort").and_then(|v| v.as_str()),
            Some(crate::models::get_reasoning_effort("claude-sonnet-4-5", &mapping).as_str()),
            "thinking.disabled should keep reasoning effort mapping"
        );
        assert!(
            body.get("include").is_none(),
            "thinking.disabled should not request reasoning include payloads"
        );
    }

    #[test]
    fn tool_requests_keep_instruction_semantics_when_thinking_disabled() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "hello"}],
            "tools": [{
                "name": "Read",
                "description": "Read files",
                "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
            }],
            "thinking": {"type": "disabled"},
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();
        let augmentation = decide_request_augmentation(&request, "hello");

        assert_eq!(augmentation.mode, RequestAugmentationMode::Agent);
        assert!(augmentation.reasons.contains(&"tools"));

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("global prompt"),
            "custom prompt should move into top-level instructions"
        );
        assert!(
            !contains_wrapped_agents_user_input(&body),
            "tool-capable requests should no longer synthesize wrapped AGENTS user input"
        );
        assert!(
            body.pointer("/reasoning/summary").is_none(),
            "thinking.disabled should still suppress visible reasoning summary"
        );
    }

    #[test]
    fn tool_fingerprint_is_stable_across_json_key_order_changes() {
        let tool_a = json!({
            "name": "searchDocs",
            "description": "Search docs",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"}
                },
                "required": ["query"]
            }
        });
        let tool_b = json!({
            "description": "Search docs",
            "input_schema": {
                "required": ["query"],
                "properties": {
                    "limit": {"type": "integer"},
                    "query": {"type": "string"}
                },
                "type": "object"
            },
            "name": "searchDocs"
        });

        assert_eq!(
            fingerprint_json_value(&tool_a),
            fingerprint_json_value(&tool_b)
        );
    }

    #[test]
    fn prompt_cache_key_changes_when_tools_change() {
        let request_a: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":[{"type":"text","text":"hello"}]}],
            "system": "System prompt\nsession_id: abc123",
            "tools": [{
                "name": "toolA",
                "description": "A",
                "input_schema": {"type":"object","properties":{"query":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("request a");
        let request_b: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":[{"type":"text","text":"hello"}]}],
            "system": "System prompt\nsession_id: abc123",
            "tools": [{
                "name": "toolB",
                "description": "B",
                "input_schema": {"type":"object","properties":{"query":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("request b");
        let mapping = ReasoningEffortMapping::default();

        let (body_a, _) =
            TransformRequest::transform(&request_a, None, &mapping, "", "gpt-5.3-codex");
        let (body_b, _) =
            TransformRequest::transform(&request_b, None, &mapping, "", "gpt-5.3-codex");

        assert_eq!(
            body_a.get("prompt_cache_key"),
            body_b.get("prompt_cache_key")
        );
    }

    #[test]
    fn prompt_cache_key_ignores_inactive_agent_prefixes_for_passthrough_requests() {
        let request = sample_passthrough_request();
        let mapping = ReasoningEffortMapping::default();

        let (body_a, _) = TransformRequest::transform(
            &request,
            None,
            &mapping,
            "global prompt A",
            "gpt-5.3-codex",
        );
        let (body_b, _) = TransformRequest::transform(
            &request,
            None,
            &mapping,
            "global prompt B",
            "gpt-5.3-codex",
        );

        assert_eq!(
            body_a.get("prompt_cache_key"),
            body_b.get("prompt_cache_key"),
            "passthrough cache key should ignore inactive static prefixes"
        );
    }

    #[test]
    fn title_like_meta_requests_stay_passthrough_without_agent_prefixes() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "根据用户的第一条消息，生成一个简短的对话标题（10字以内）。只输出标题，不要有任何其他内容、标点符号或引号。

用户消息：你好啊"
            }],
            "thinking": {"type": "disabled"},
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();
        let augmentation = decide_request_augmentation(
            &request,
            "根据用户的第一条消息，生成一个简短的对话标题（10字以内）。只输出标题，不要有任何其他内容、标点符号或引号。

用户消息：你好啊",
        );

        assert_eq!(augmentation.mode, RequestAugmentationMode::Passthrough);

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");
        let instructions = instructions_text(&body);
        assert!(instructions.is_none());
        let texts = input_texts(&body);
        assert_eq!(
            texts.len(),
            1,
            "meta request should keep a single original user text item"
        );
        assert!(
            texts[0].contains("用户消息：你好啊"),
            "original request text should pass through untouched"
        );
    }

    #[test]
    fn codex_fast_mode_defaults_to_priority_service_tier() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert_eq!(
            body.get("service_tier").and_then(|v| v.as_str()),
            Some("priority"),
            "fast mode should map to priority processing by default"
        );
    }

    #[test]
    fn codex_fast_mode_can_be_disabled_per_request() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();
        let (body, _) = TransformRequest::transform_with_options(
            &request,
            None,
            &mapping,
            "global prompt",
            "gpt-5.3-codex",
            true,
            false,
            false,
            true,
        );

        assert!(
            body.get("service_tier").is_none(),
            "fast mode disabled should omit service_tier"
        );
    }

    #[test]
    fn codex_request_upstream_transport_remains_streaming_for_non_stream_clients() {
        let request = sample_passthrough_request();
        let mapping = ReasoningEffortMapping::default();
        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        assert_eq!(
            body.get("stream").and_then(|v| v.as_bool()),
            Some(true),
            "codex upstream transport should stay streaming even for non-stream clients"
        );
    }

    #[test]
    fn codex_request_upstream_transport_stays_streaming_for_stream_clients() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();
        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        assert_eq!(
            body.get("stream").and_then(|v| v.as_bool()),
            Some(true),
            "codex upstream transport should keep stream=true requests streaming"
        );
    }

    #[test]
    fn plain_text_requests_use_passthrough_augmentation_and_skip_agent_prefixes() {
        let request = sample_passthrough_request();
        let mapping = ReasoningEffortMapping::default();
        let augmentation = decide_request_augmentation(&request, "hello");

        assert_eq!(augmentation.mode, RequestAugmentationMode::Passthrough);
        assert!(
            augmentation.reasons.is_empty(),
            "plain text request should not have agent reasons"
        );

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert!(
            body.get("instructions").is_none(),
            "passthrough requests should not carry static codex instructions"
        );
        let texts = input_texts(&body);
        assert_eq!(texts, vec!["hello".to_string()]);
        assert!(
            !texts
                .iter()
                .any(|text| text.contains("After emitting the <proposed_plan> block")),
            "passthrough requests must not receive plan-mode prompt injection"
        );
    }

    #[test]
    fn system_requests_stay_passthrough_and_use_instructions() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();
        let augmentation = decide_request_augmentation(
            &request,
            "System prompt
hello",
        );

        assert_eq!(augmentation.mode, RequestAugmentationMode::Passthrough);
        assert!(
            !augmentation.reasons.contains(&"system"),
            "plain system prompts should not force agent augmentation"
        );

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("System prompt"),
            "instructions should preserve the original system prompt"
        );
        assert!(
            !contains_wrapped_agents_user_input(&body),
            "system requests should no longer synthesize wrapped AGENTS user input"
        );
        let texts = input_texts(&body);
        assert!(
            !texts.iter().any(|text| text == "global prompt"),
            "custom global prompt should remain out of standalone user messages when passthrough"
        );
        assert!(
            !texts
                .iter()
                .any(|text| text.contains("After emitting the <proposed_plan> block")),
            "ordinary system requests must not receive plan-mode prompt injection"
        );
    }

    #[test]
    fn agent_requests_without_system_wrap_custom_prompt_without_touching_default_instructions() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "call_1",
                    "name": "skill",
                    "input": { "command": "test-skill" }
                }]
            }, {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call_1",
                    "content": "<command-name>test-skill</command-name>\nBase Path: /tmp\nContent"
                }]
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("global prompt"),
            "agent requests without system should move custom prompt into top-level instructions"
        );

        let texts = input_texts(&body);
        assert!(
            !texts.iter().any(|text| text == "global prompt"),
            "custom prompt should not appear as a standalone user message"
        );

        assert!(
            !contains_wrapped_agents_user_input(&body),
            "agent requests without system should no longer keep custom prompt in wrapped user input"
        );
    }

    #[test]
    fn plan_mode_requests_use_plan_augmentation() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "请先给我 plan，不要直接执行"
            }],
            "system": "<system-reminder>Skill catalog (condensed):\n- test-driven-development: Use when implementing any feature or bugfix, before writing implementation code\n</system-reminder>",
            "metadata": {
                "plan_mode": true
            },
            "tools": [{
                "name": "Read",
                "description": "Read files",
                "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
            }],
            "tool_choice": {
                "type": "tool",
                "name": "ExitPlanMode"
            },
            "stream": true
        }))
        .expect("valid anthropic request");
        let request_text_corpus = collect_request_text_corpus(&request);
        let augmentation = decide_request_augmentation(&request, &request_text_corpus);

        assert_eq!(augmentation.mode, RequestAugmentationMode::Plan);
        assert!(
            augmentation.reasons.contains(&"plan_mode"),
            "plan requests should record the detected plan reason"
        );

        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");
        let texts = input_texts(&body);
        let instructions = instructions_text(&body).expect("instructions should be present");

        assert!(
            instructions.contains("Emit exactly one <proposed_plan> block in the turn."),
            "plan requests should inject the anthropic-compatible plan-mode prompt"
        );
        assert!(
            instructions.contains("<proposed_plan>") && instructions.contains("</proposed_plan>"),
            "plan requests should explicitly instruct the model to emit a proposed_plan block"
        );
        assert!(
            instructions.contains(
                "After the <proposed_plan> block, call ExitPlanMode to request approval."
            ),
            "plan requests should instruct the model to use ExitPlanMode after emitting the plan"
        );
        assert!(
            instructions.contains("global prompt"),
            "plan-mode requests should also carry the custom prompt in instructions"
        );
        assert_eq!(
            texts.iter()
                .filter(|text| text.trim() == ANTHROPIC_COMPAT_PLAN_MODE_PROMPT.trim())
                .count(),
            0,
            "plan-mode prompt should be merged into top-level instructions, not sent as a standalone user message"
        );
        assert_eq!(
            body.get("tool_choice"),
            None,
            "blacklisted Claude orchestration tool choices should fall back to upstream default behavior"
        );
    }

    #[test]
    fn plan_mode_system_prompt_strips_claude_plan_orchestration() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "在下载文件夹中写一个时钟的html页面，告诉我你的方案"
            }],
            "system": "<system-reminder>\nThe following skills are available for use with the Skill tool:\n- test-driven-development: Use when implementing any feature or bugfix, before writing implementation code\n</system-reminder>\n\n<system-reminder>\nPlan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits.\n\n## Plan File Info:\nNo plan file exists yet. You should create your plan at /Users/mr.j/.claude/plans/warm-cooking-token.md using the Write tool.\n\n## Plan Workflow\nLaunch Plan agent(s) to design the implementation.\nAt the very end of your turn, call ExitPlanMode.\n</system-reminder>",
            "metadata": {
                "plan_mode": true
            },
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");
        let instructions = instructions_text(&body).expect("instructions should be present");

        assert!(
            instructions.contains("Skill catalog (condensed)"),
            "skill catalog reminder should still be preserved after plan-mode cleaning"
        );
        assert!(
            !instructions.contains("Plan File Info")
                && !instructions.contains("warm-cooking-token.md")
                && !instructions.contains("Launch Plan agent(s)"),
            "claude-native plan orchestration text should be stripped before forwarding to codex"
        );
        assert!(
            instructions.contains("Emit exactly one <proposed_plan> block in the turn."),
            "codex-native plan instructions should still be injected"
        );
    }

    #[test]
    fn plan_mode_tool_blacklist_filters_only_orchestration_tools() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "请先给我 plan"
            }],
            "metadata": {
                "plan_mode": true
            },
            "tools": [
                {
                    "name": "Read",
                    "description": "Read files",
                    "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
                },
                {
                    "name": "EnterPlanMode",
                    "description": "Enter plan mode",
                    "input_schema": {"type":"object","properties":{}}
                },
                {
                    "name": "ExitPlanMode",
                    "description": "Exit plan mode",
                    "input_schema": {"type":"object","properties":{}}
                },
                {
                    "name": "EnterWorktree",
                    "description": "Enter worktree",
                    "input_schema": {"type":"object","properties":{}}
                },
                {
                    "name": "ExitWorktree",
                    "description": "Exit worktree",
                    "input_schema": {"type":"object","properties":{"action":{"type":"string"}}}
                },
                {
                    "name": "AskUserQuestion",
                    "description": "Ask user question",
                    "input_schema": {"type":"object","properties":{"questions":{"type":"array"}}}
                }
            ],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let tool_names = tool_names(&body);

        assert!(
            tool_names.contains(&"Read".to_string())
                && tool_names.contains(&"AskUserQuestion".to_string()),
            "ordinary non-blacklisted tools should still be forwarded in plan mode"
        );
        assert!(
            !tool_names.contains(&"EnterPlanMode".to_string())
                && !tool_names.contains(&"ExitPlanMode".to_string())
                && !tool_names.contains(&"EnterWorktree".to_string())
                && !tool_names.contains(&"ExitWorktree".to_string()),
            "claude-native orchestration tools should be removed by the plan-mode blacklist"
        );
    }

    #[test]
    fn non_plan_requests_keep_worktree_tools_available() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "退出当前 worktree"
            }],
            "tools": [
                {
                    "name": "ExitWorktree",
                    "description": "Exit worktree",
                    "input_schema": {"type":"object","properties":{"action":{"type":"string"}}}
                },
                {
                    "name": "Read",
                    "description": "Read files",
                    "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
                }
            ],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let tool_names = tool_names(&body);

        assert!(
            tool_names.contains(&"ExitWorktree".to_string())
                && tool_names.contains(&"Read".to_string()),
            "worktree tools should remain available outside plan-mode requests"
        );
    }

    #[test]
    fn plan_mode_blacklisted_tool_choice_does_not_force_none() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "请先给我 plan"
            }],
            "metadata": {
                "plan_mode": true
            },
            "tools": [
                {
                    "name": "Read",
                    "description": "Read files",
                    "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
                },
                {
                    "name": "ExitPlanMode",
                    "description": "Exit plan mode",
                    "input_schema": {"type":"object","properties":{}}
                }
            ],
            "tool_choice": {
                "type": "tool",
                "name": "ExitPlanMode"
            },
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        assert_eq!(
            body.get("tool_choice"),
            None,
            "blacklisted plan-mode tool choices should not lock codex upstream into tool_choice none"
        );
        assert!(
            tool_names(&body).contains(&"Read".to_string()),
            "safe tools should still remain available after tool-choice fallback"
        );
    }

    #[test]
    fn plan_approval_signal_requests_use_plan_augmentation() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "请处理这个 plan_approval_response，再等我确认"
            }],
            "system": "",
            "stream": true
        }))
        .expect("valid anthropic request");
        let request_text_corpus = collect_request_text_corpus(&request);
        let augmentation = decide_request_augmentation(&request, &request_text_corpus);

        assert_eq!(augmentation.mode, RequestAugmentationMode::Plan);
        assert!(
            augmentation.reasons.contains(&"plan_approval_response"),
            "plan approval signal should keep the request on the plan path"
        );

        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");
        let texts = input_texts(&body);
        let instructions = instructions_text(&body).expect("instructions should be present");

        assert!(
            instructions.contains("Emit exactly one <proposed_plan> block in the turn."),
            "plan approval signal should still inject the anthropic-compatible plan prompt"
        );
        assert!(
            instructions.contains("<proposed_plan>") && instructions.contains("</proposed_plan>"),
            "plan approval signal should preserve the proposed_plan requirement"
        );
        assert!(
            instructions.contains("Do not ask the user to type approval in normal text."),
            "plan approval signal should discourage plain-text approval asks"
        );
        assert!(
            instructions.contains("global prompt"),
            "plan approval requests should not drop the custom prompt"
        );
        assert_eq!(
            texts.iter()
                .filter(|text| text.trim() == ANTHROPIC_COMPAT_PLAN_MODE_PROMPT.trim())
                .count(),
            0,
            "plan-mode prompt should stay in top-level instructions instead of input messages"
        );
    }

    #[test]
    fn tool_schema_mentions_plan_approval_response_do_not_force_plan_mode() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role": "user",
                "content": "你好啊啊啊啊啊啊"
            }],
            "system": "You are Claude Code.",
            "tools": [{
                "name": "SendMessage",
                "description": "When a teammate is in plan mode, send a plan_approval_response after approval.",
                "input_schema": {
                    "type":"object",
                    "properties":{
                        "message":{
                            "type":"object",
                            "properties":{
                                "type":{"const":"plan_approval_response"},
                                "request_id":{"type":"string"},
                                "approve":{"type":"boolean"}
                            }
                        }
                    }
                }
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let request_text_corpus = collect_request_text_corpus(&request);
        let augmentation = decide_request_augmentation(&request, &request_text_corpus);

        assert_eq!(
            augmentation.mode,
            RequestAugmentationMode::Agent,
            "tool schema text alone should not force plan mode"
        );
        assert!(
            !augmentation.reasons.contains(&"plan_approval_response"),
            "tool schema mentions should not be treated as plan approval signals"
        );

        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");
        let texts = input_texts(&body);
        let instructions = instructions_text(&body).expect("instructions should be present");

        assert!(
            !texts
                .iter()
                .any(|text| text.contains("After emitting the <proposed_plan> block")),
            "non-plan requests should not inject the plan prompt"
        );
        assert!(
            instructions.contains("global prompt"),
            "ordinary agent requests should still carry the custom prompt in instructions"
        );
    }

    #[test]
    fn recent_plan_mode_reminder_keeps_request_on_plan_path() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        "<system-reminder>\nPlan mode is active. The user indicated that they do not want you to execute yet.\n</system-reminder>",
                        "测试plan"
                    ]
                }
            ],
            "system": "You are Claude Code.",
            "stream": true
        }))
        .expect("valid anthropic request");
        let request_text_corpus = collect_request_text_corpus(&request);
        let augmentation = decide_request_augmentation(&request, &request_text_corpus);

        assert_eq!(augmentation.mode, RequestAugmentationMode::Plan);
        assert!(
            augmentation.reasons.contains(&"recent_plan_mode_reminder"),
            "latest user message carrying the official plan reminder should keep the request on the plan path"
        );
    }

    #[test]
    fn previous_user_plan_mode_reminder_does_not_keep_next_request_in_plan_mode() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        "<system-reminder>\nPlan mode is active. The user indicated that they do not want you to execute yet.\n</system-reminder>",
                        "你好啊"
                    ]
                },
                {
                    "role": "assistant",
                    "content": "先给你一个计划。"
                },
                {
                    "role": "user",
                    "content": "现在正常聊一句"
                }
            ],
            "system": "You are Claude Code.",
            "stream": true
        }))
        .expect("valid anthropic request");
        let request_text_corpus = collect_request_text_corpus(&request);
        let augmentation = decide_request_augmentation(&request, &request_text_corpus);

        assert_eq!(
            augmentation.mode,
            RequestAugmentationMode::Passthrough,
            "a prior user turn carrying the plan reminder should not force the next plain request into agent or plan mode"
        );
        assert!(
            !augmentation.reasons.contains(&"recent_plan_mode_reminder"),
            "only the latest user message should count as a current plan-mode reminder"
        );
    }

    #[test]
    fn includes_static_instructions_even_when_request_contains_codex_harness_rules() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":"You are Codex, based on GPT-5.\n## Editing constraints\n## Plan tool\n## Presenting your work"
            }],
            "system": "You are Codex, based on GPT-5.\n## Editing constraints\n## Presenting your work",
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("You are Codex, based on GPT-5."),
            "system-only requests with codex rules should preserve system text in instructions"
        );
    }

    #[test]
    fn skips_runtime_environment_context_when_system_already_provides_it() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"hi"}],
            "system": "<environment_context><cwd>/tmp</cwd><approval_policy>on-request</approval_policy></environment_context>",
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("<environment_context>"),
            "system-provided environment context should stay in top-level instructions"
        );
    }

    #[test]
    fn skips_runtime_environment_context_when_messages_already_provide_it() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":"<environment_context><cwd>/tmp</cwd><approval_policy>on-request</approval_policy></environment_context>"
            }],
            "system": "System prompt",
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let env_context_count = input_items
            .iter()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .filter(|text| text.contains("<environment_context>"))
            .count();

        assert_eq!(
            env_context_count, 1,
            "runtime environment context should not be duplicated when already present in messages"
        );
    }

    #[test]
    fn preserves_existing_agents_wrapper_without_double_wrapping() {
        let already_wrapped =
            "# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>\nhello\n</INSTRUCTIONS>";
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"hi"}],
            "system": already_wrapped,
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();
        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");

        let marker_count = instructions.matches("AGENTS.md instructions").count();
        assert_eq!(
            marker_count, 1,
            "existing AGENTS wrapper should not be nested"
        );
        assert!(
            instructions.contains("hello"),
            "wrapped system text should preserve the original instructions"
        );
        assert!(
            instructions.contains("global prompt"),
            "wrapped agent systems should still merge active custom prompts into instructions"
        );
    }

    #[test]
    fn detect_requested_skill_name_prefers_explicit_slash_skill() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"请用 /figma-implement-design 做一个页面"}],
            "system": "<system-reminder>\nThe following skills are available:\n- figma-implement-design: Translate Figma nodes\n- pdf: Read PDF files\n</system-reminder>",
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");

        assert_eq!(
            detect_requested_skill_name(&request).as_deref(),
            Some("figma-implement-design")
        );
    }

    #[test]
    fn detect_requested_skill_name_requires_skill_tool_presence() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"请用 /figma-implement-design 做一个页面"}],
            "system": "<system-reminder>\nThe following skills are available:\n- figma-implement-design: Translate Figma nodes\n</system-reminder>",
            "tools": [{
                "name": "Read",
                "description": "Read files",
                "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");

        assert!(
            detect_requested_skill_name(&request).is_none(),
            "should not inject skill hint when Skill tool is unavailable"
        );
    }

    #[test]
    fn transform_routes_explicit_skill_token_via_structured_tool_choice() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"请使用 /figma-implement-design 处理这个节点"}],
            "system": "<system-reminder>\nThe following skills are available:\n- figma-implement-design: Translate Figma nodes\n- pdf: Read PDF files\n</system-reminder>",
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}, "args":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform_with_options(
            &request,
            None,
            &mapping,
            "",
            "gpt-5.3-codex",
            true,
            true,
            true,
            true,
        );

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let hint_text_count = input_items
            .iter()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .filter(|text| text.contains("Skill routing hint"))
            .count();

        assert_eq!(hint_text_count, 0, "structured routing should avoid synthetic skill hint messages");
        assert_eq!(
            body.get("tool_choice"),
            Some(&json!({"type":"function","name":"Skill"})),
            "explicit skill requests should be routed via tool_choice=function(Skill)"
        );
    }

    #[test]
    fn transform_does_not_inject_skill_routing_hint_by_default() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"请使用 /figma-implement-design 处理这个节点"}],
            "system": "<system-reminder>\nThe following skills are available:\n- figma-implement-design: Translate Figma nodes\n</system-reminder>",
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let hint_text_count = input_items
            .iter()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .filter(|text| text.contains("Skill routing hint"))
            .count();

        assert_eq!(
            hint_text_count, 0,
            "skill routing hint should be disabled by default for passthrough parity"
        );
    }

    #[test]
    fn transform_moves_extracted_skill_payloads_into_instructions() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_skill_1",
                        "name": "Skill",
                        "input": {"skill": "figma-implement-design"}
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_skill_1",
                        "content": [
                            {"text": "<command-name>figma-implement-design</command-name>", "type": "text"},
                            {"text": "Base Path: /tmp/figma", "type": "text"},
                            {"text": "Use the figma skill for exact design implementation.", "type": "text"}
                        ]
                    }]
                }
            ],
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (messages_for_codex, extracted_skills) =
            MessageProcessor::transform_messages(&request.messages, None);
        assert!(
            !messages_for_codex.is_empty(),
            "skill extraction fixture should still produce codex input messages"
        );
        assert!(
            !extracted_skills.is_empty(),
            "skill extraction fixture should produce extracted skill payloads"
        );

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(
            instructions.contains("<skill>") && instructions.contains("<name>figma-implement-design</name>"),
            "extracted skill payloads should move into instructions"
        );
        let input_text = input_texts(&body).join("\n");
        assert!(
            !input_text.contains("<skill>"),
            "extracted skill payloads should no longer be injected into input messages"
        );
    }

    #[test]
    fn transform_dedupes_duplicate_extracted_skills_when_merging_instructions() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_skill_1",
                        "name": "Skill",
                        "input": {"skill": "yat_commit"}
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_skill_1",
                        "content": [
                            {"text": "<command-name>yat_commit</command-name>", "type": "text"},
                            {"text": "Base Path: /tmp/yat", "type": "text"},
                            {"text": "仅在用户明确要求提交代码时使用", "type": "text"}
                        ]
                    }]
                },
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_skill_2",
                        "name": "Skill",
                        "input": {"skill": "yat_commit"}
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_skill_2",
                        "content": [
                            {"text": "<command-name>yat_commit</command-name>", "type": "text"},
                            {"text": "Base Path: /tmp/yat", "type": "text"},
                            {"text": "仅在用户明确要求提交代码时使用", "type": "text"}
                        ]
                    }]
                }
            ],
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let instructions = instructions_text(&body).expect("instructions should be present");

        assert_eq!(
            instructions.matches("<name>yat_commit</name>").count(),
            1,
            "duplicate extracted skill payloads should be deduped when merged into instructions"
        );
    }

    #[test]
    fn detect_requested_skill_name_supports_dollar_prefixed_skill() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"请用 $figma-implement-design 完成这个任务"}],
            "system": "<system-reminder>\nThe following skills are available:\n- figma-implement-design: Translate Figma nodes\n- pdf: Read PDF files\n</system-reminder>",
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");

        assert_eq!(
            detect_requested_skill_name(&request).as_deref(),
            Some("figma-implement-design")
        );
    }

    #[test]
    fn detect_requested_skill_name_ignores_path_like_slashes() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"请看 /Users/mr.j/.agents/skills/platform/SKILL.md"}],
            "system": "<system-reminder>\nThe following skills are available:\n- platform: Internal platform helper\n- pdf: Read PDF files\n</system-reminder>",
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");

        assert!(
            detect_requested_skill_name(&request).is_none(),
            "file path slashes should not be interpreted as explicit skill invocation"
        );
    }

    #[test]
    fn detect_requested_skill_name_uses_catalog_from_message_system_reminder() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role":"user",
                    "content":[
                        {
                            "type":"text",
                            "text":"<system-reminder>\nThe following skills are available for use with the Skill tool:\n- figma-implement-design: Translate Figma nodes\n- pdf: Read PDF files\n</system-reminder>"
                        }
                    ]
                },
                {
                    "role":"user",
                    "content":"请用 /figma-implement-design 处理这个节点"
                }
            ],
            "tools": [{
                "name": "Skill",
                "description": "Execute skill",
                "input_schema": {"type":"object","properties":{"skill":{"type":"string"}}}
            }],
            "stream": true
        }))
        .expect("valid anthropic request");

        assert_eq!(
            detect_requested_skill_name(&request).as_deref(),
            Some("figma-implement-design"),
            "skill detection should work even when catalog comes from message reminder"
        );
    }

    #[test]
    fn transform_moves_skill_catalog_system_reminder_from_messages_into_instructions() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":[
                    {
                        "type":"text",
                        "text":"<system-reminder>\nThe following skills are available for use with the Skill tool:\n- figma-implement-design: Translate Figma nodes\n- pdf: Read PDF files\n</system-reminder>\n\n你好"
                    }
                ]
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        let instructions = instructions_text(&body).expect("instructions should be present");
        assert!(instructions.contains("figma-implement-design"));
        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let joined = input_items
            .iter()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined.contains("The following skills are available"));
        assert!(joined.contains("你好"));
    }
}
