use serde_json::{json, Value};
use std::collections::HashSet;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::logger::{is_debug_log_enabled, AppLogger};
use crate::models::{
    get_reasoning_effort, AnthropicRequest, ContentBlock, MessageContent, ReasoningEffortMapping,
};
use crate::transform::MessageProcessor;

const CODEX_INSTRUCTIONS: &str = include_str!("../../instructions.txt");
const EMPTY_TOOL_OUTPUT_PLACEHOLDER: &str = "(No output)";
const PROMPT_CACHE_KEY_MAX_CWD_LEN: usize = 64;
const PROMPT_CACHE_KEY_SEP: u8 = 0x1f;
const MAX_TRUSTED_REQUEST_CWD_CHARS: usize = 512;
const MAX_TOOL_DESCRIPTION_CHARS: usize = 240;
const MAX_TOOL_SCHEMA_DESCRIPTION_CHARS: usize = 120;
const DEFAULT_REASONING_SUMMARY_MODE: &str = "auto";
const ENV_REASONING_SUMMARY_MODE: &str = "CODEX_PROXY_REASONING_SUMMARY";
const ENV_INCLUDE_REASONING_ENCRYPTED_CONTENT: &str =
    "CODEX_PROXY_INCLUDE_REASONING_ENCRYPTED_CONTENT";
const ENV_FORCE_STATIC_CODEX_INSTRUCTIONS: &str = "CODEX_PROXY_FORCE_STATIC_INSTRUCTIONS";

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

fn system_looks_like_codex_harness_instructions(system_text: &str) -> bool {
    let lower = system_text.to_ascii_lowercase();
    (lower.contains("you are codex") || lower.contains("codex cli"))
        && (lower.contains("editing constraints")
            || lower.contains("plan tool")
            || lower.contains("presenting your work"))
}

fn should_include_static_codex_instructions(system_text: Option<&str>) -> bool {
    if bool_env_enabled(ENV_FORCE_STATIC_CODEX_INSTRUCTIONS) {
        return true;
    }
    match system_text {
        Some(text) => !system_looks_like_codex_harness_instructions(text),
        None => true,
    }
}

fn is_wrapped_agents_instructions(system_text: &str) -> bool {
    let lower = system_text.to_ascii_lowercase();
    lower.contains("agents.md instructions")
        && lower.contains("<instructions>")
        && lower.contains("</instructions>")
}

fn request_contains_environment_context(request_text_corpus: &str) -> bool {
    request_text_corpus
        .to_ascii_lowercase()
        .contains("<environment_context>")
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
        .trim_matches(|ch: char| matches!(ch, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | ';' | '!' | '?'))
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
    custom_injection_prompt: &str,
    system_text: Option<&str>,
) -> String {
    let mut key_material = Vec::new();
    key_material.extend_from_slice(CODEX_INSTRUCTIONS.as_bytes());
    key_material.push(PROMPT_CACHE_KEY_SEP);
    key_material.extend_from_slice(custom_injection_prompt.trim().as_bytes());
    key_material.push(PROMPT_CACHE_KEY_SEP);
    if let Some(system) = system_text {
        key_material.extend_from_slice(system.trim().as_bytes());
    }
    let key_hash = fnv1a64(&key_material);
    let model_segment = sanitize_cache_key_segment(codex_model, 48);
    let cwd_segment = request_cwd
        .map(|cwd| sanitize_cache_key_segment(cwd, PROMPT_CACHE_KEY_MAX_CWD_LEN))
        .unwrap_or_else(|| "default".to_string());
    format!("codex-proxy:{}:{}:{:016x}", model_segment, cwd_segment, key_hash)
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

fn strip_system_reminder_blocks(text: &str) -> String {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";
    const SKILL_MARKERS: [&str; 7] = [
        "available skills",
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
            sanitized.push_str(block);
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
        )
    }

    pub fn transform_with_options(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        reasoning_mapping: &ReasoningEffortMapping,
        custom_injection_prompt: &str,
        codex_model: &str,
        enable_tool_schema_compaction: bool,
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

        // 构建 input 数组（只包含当前请求上下文，不注入静态模板文件）
        let mut final_input: Vec<Value> = Vec::new();

        let request_text_corpus = collect_request_text_corpus(anthropic_body);
        let request_cwd = extract_request_cwd(&request_text_corpus);
        let mut system_text_for_cache_key = None::<String>;
        let include_static_codex_instructions =
            should_include_static_codex_instructions(Some(&request_text_corpus));

        // 注入 system prompt
        if let Some(system) = &anthropic_body.system {
            let system_text = system.to_string();
            system_text_for_cache_key = Some(system_text.clone());
            log(&format!(
                "📋 [Transform] System prompt: {} chars",
                system_text.len()
            ));

            let system_payload_text = if is_wrapped_agents_instructions(&system_text) {
                system_text
            } else if let Some(cwd) = request_cwd.as_deref() {
                format!(
                    "# AGENTS.md instructions for {}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
                    cwd, system_text
                )
            } else {
                format!(
                    "# AGENTS.md instructions\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
                    system_text
                )
            };
            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": system_payload_text
                }]
            }));

            if request_contains_environment_context(&request_text_corpus) {
                log("📋 [Transform] Skip runtime <environment_context> injection (already present in request text)");
            } else {
                log("📋 [Transform] Skip runtime <environment_context> injection (no trusted request cwd)");
            }
        }

        // 注入提取的 Skills
        if !extracted_skills.is_empty() {
            log(&format!(
                "🎯 [Transform] Injecting {} skill(s)",
                extracted_skills.len()
            ));
            for skill in extracted_skills {
                final_input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": skill
                    }]
                }));
            }
        }

        if !custom_injection_prompt.trim().is_empty() {
            log(&format!(
                "🎯 [Transform] Injecting custom global prompt ({} chars)",
                custom_injection_prompt.len()
            ));
            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": custom_injection_prompt
                }]
            }));
        }

        // 追加对话历史
        final_input.extend(chat_messages);
        if let Some(skill_name) = detect_requested_skill_name(anthropic_body) {
            log(&format!(
                "🎯 [Transform] Skill intent matched, nudging Skill tool: {}",
                skill_name
            ));
            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": format!(
                        "Skill routing hint: the latest user request targets skill `{}`.\nIf this skill is available, call the `Skill` tool first before normal text response.",
                        skill_name
                    )
                }]
            }));
        }
        Self::sanitize_input_for_codex(&mut final_input);
        reconcile_function_call_pairs(&mut final_input);

        // 转换工具
        let transformed_tools = Self::transform_tools(
            anthropic_body.tools.as_ref(),
            log_tx,
            enable_tool_schema_compaction,
        );

        log(&format!(
            "📋 [Transform] Final: {} input items, {} tools",
            final_input.len(),
            transformed_tools.len()
        ));

        // 关键优化：无工具时强制 tool_choice: "none"，避免模型乱吐工具 JSON 到文本
        let tool_choice = if transformed_tools.is_empty() {
            json!("none")
        } else {
            json!("auto")
        };
        let prompt_cache_key = build_prompt_cache_key(
            request_cwd.as_deref(),
            final_codex_model,
            custom_injection_prompt,
            system_text_for_cache_key.as_deref(),
        );
        let reasoning_summary_mode = resolve_reasoning_summary_mode();
        let include_reasoning_encrypted_content = resolve_include_reasoning_encrypted_content();
        log(&format!(
            "🧠 [Transform] reasoning.summary={} include.reasoning.encrypted_content={}",
            reasoning_summary_mode, include_reasoning_encrypted_content
        ));

        let mut body = json!({
            "model": final_codex_model,
            "input": final_input,
            "tools": transformed_tools,
            "tool_choice": tool_choice,
            "parallel_tool_calls": true,
            "reasoning": { "effort": reasoning_effort.as_str(), "summary": reasoning_summary_mode },
            "store": false,
            "stream": true,
            "prompt_cache_key": prompt_cache_key
        });
        if include_static_codex_instructions {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("instructions".to_string(), json!(CODEX_INSTRUCTIONS));
            }
        } else {
            log("📋 [Transform] Skip static instructions injection (already present in system)");
        }
        if include_reasoning_encrypted_content {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("include".to_string(), json!(["reasoning.encrypted_content"]));
            }
        }

        (body, session_id.clone())
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

            if item_type.as_deref() == Some("function_call_output")
                && obj.get("output").and_then(|v| v.as_str()).is_some()
            {
                let output = obj
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if let Some(sanitized_output) = sanitize_function_call_output_for_codex(output) {
                    obj.insert("output".to_string(), json!(sanitized_output));
                } else {
                    obj.insert("output".to_string(), json!(""));
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

        let transformed: Vec<Value> = tools
            .iter()
            .map(|tool| {
// Claude Code 格式: { name, description, input_schema }
                if tool.get("name").is_some() && tool.get("type").is_none() {
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
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

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": compact_tool_description(tool.get("description").and_then(|d| d.as_str())),
                        "strict": false,
                        "parameters": parameters
                    });
                }

                // Anthropic 格式: { type: "tool", name, ... }
                if tool.get("type").and_then(|t| t.as_str()) == Some("tool") {
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
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

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": compact_tool_description(tool.get("description").and_then(|d| d.as_str())),
                        "strict": false,
                        "parameters": parameters
                    });
                }

                // OpenAI 格式: { type: "function", function: {...} }
                if tool.get("type").and_then(|t| t.as_str()) == Some("function") {
                    let func = tool.get("function").unwrap_or(tool);
                    let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
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

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": compact_tool_description(func.get("description").and_then(|d| d.as_str())),
                        "strict": false,
                        "parameters": parameters
                    });
                }

                // 未知格式
                let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
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

                json!({
                    "type": "function",
                    "name": name,
                    "description": compact_tool_description(tool.get("description").and_then(|d| d.as_str())),
                    "strict": false,
                    "parameters": parameters
                })
            })
            .collect();

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
        compact_tool_description, detect_requested_skill_name, extract_request_cwd,
        TransformRequest,
    };
    use crate::models::{AnthropicRequest, ReasoningEffortMapping};
    use serde_json::json;

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
    fn compact_tool_description_truncates_and_normalizes() {
        let description = "First line with    extra spaces.\nSecond line stays in first paragraph.\n\nSecond paragraph should be dropped.";
        let compacted = compact_tool_description(Some(description));
        assert!(
            compacted.contains("First line with extra spaces. Second line stays in first paragraph."),
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
        let transformed = TransformRequest::transform_tools(Some(&tools), None, true);
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

        let transformed = TransformRequest::transform_tools(Some(&tools), None, true);
        let parameters = transformed[0].get("parameters").cloned().unwrap_or_default();
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
            parameters
                .pointer("/required/0")
                .and_then(|v| v.as_str()),
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

        let transformed = TransformRequest::transform_tools(Some(&tools), None, false);
        let parameters = transformed[0].get("parameters").cloned().unwrap_or_default();
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
    fn prompt_cache_key_changes_when_custom_prompt_changes() {
        let request = sample_request();
        let mapping = ReasoningEffortMapping::default();

        let (body_a, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt A", "gpt-5.3-codex");
        let (body_b, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt B", "gpt-5.3-codex");

        assert_ne!(
            body_a.get("prompt_cache_key"),
            body_b.get("prompt_cache_key"),
            "cache key should rotate when static prefix changes"
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

        let first_system_text = input_items
            .first()
            .and_then(|item| item.get("content"))
            .and_then(|v| v.as_array())
            .and_then(|blocks| blocks.first())
            .and_then(|block| block.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        assert!(
            first_system_text.starts_with("# AGENTS.md instructions\n\n<INSTRUCTIONS>\n"),
            "system wrapper should avoid local current_dir path when request cwd is missing"
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

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let first_system_text = input_items
            .first()
            .and_then(|item| item.get("content"))
            .and_then(|v| v.as_array())
            .and_then(|blocks| blocks.first())
            .and_then(|block| block.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        assert!(
            first_system_text.contains("# AGENTS.md instructions for /Users/mr.j"),
            "wrapper should use trusted cwd extracted from request context"
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

        let (body_a, _) =
            TransformRequest::transform(&request_a, None, &mapping, "global prompt", "gpt-5.3-codex");
        let (body_b, _) =
            TransformRequest::transform(&request_b, None, &mapping, "global prompt", "gpt-5.3-codex");

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
        assert_ne!(key_a, key_b, "cache key should change with trusted request cwd");
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
    fn skips_static_instructions_when_system_already_contains_codex_harness_rules() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"hi"}],
            "system": "You are Codex, based on GPT-5.\n## Editing constraints\n## Plan tool\n",
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert!(
            body.get("instructions").is_none(),
            "static instructions should be omitted when equivalent system guidance already exists"
        );
    }

    #[test]
    fn skips_static_instructions_when_messages_already_contain_codex_harness_rules() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":"You are Codex, based on GPT-5.\n## Editing constraints\n## Plan tool\n## Presenting your work"
            }],
            "stream": true
        }))
        .expect("valid anthropic request");
        let mapping = ReasoningEffortMapping::default();

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "global prompt", "gpt-5.3-codex");

        assert!(
            body.get("instructions").is_none(),
            "static instructions should be omitted when equivalent guidance exists in messages"
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
            "runtime environment context should not be appended when already present"
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
        let already_wrapped = "# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>\nhello\n</INSTRUCTIONS>";
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

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let first_system_text = input_items
            .first()
            .and_then(|item| item.get("content"))
            .and_then(|v| v.as_array())
            .and_then(|blocks| blocks.first())
            .and_then(|block| block.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let marker_count = first_system_text.matches("AGENTS.md instructions").count();
        assert_eq!(
            marker_count, 1,
            "existing AGENTS wrapper should not be nested"
        );
        assert_eq!(
            first_system_text, already_wrapped,
            "wrapped system text should be preserved as-is"
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
    fn transform_injects_skill_routing_hint_when_explicit_skill_token_detected() {
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

        let (body, _) =
            TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        let input_items = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let hint_texts: Vec<&str> = input_items
            .iter()
            .filter_map(|item| item.get("content").and_then(|v| v.as_array()))
            .flat_map(|blocks| blocks.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .filter(|text| text.contains("Skill routing hint"))
            .collect();

        assert_eq!(hint_texts.len(), 1, "expected one injected routing hint");
        assert!(
            hint_texts[0].contains("figma-implement-design"),
            "routing hint should include matched skill name"
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
}
