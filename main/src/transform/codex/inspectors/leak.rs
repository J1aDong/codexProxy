//! Leak detection inspectors（D4 治理 side-channel）
//!
//! 从 `response.rs` 剥离的泄漏检测纯函数：识别与抑制上游泄漏的 tool 调用标记、
//! 原始 tool JSON 片段、内部规划泄漏等。本模块只做判定与文本清洗，无状态、无副作用。
//!
//! 迁移自 `TransformResponse` 的 `LeakDetector` facade 及其底层纯函数，语义零变化。

use serde_json::Value;

// ============================================================
// 类型（迁移自 response.rs）
// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawToolJsonRiskTier {
    ReadonlyRecoverable,
    HighRisk,
    Suppressed,
}

impl RawToolJsonRiskTier {
    pub fn as_str(self) -> &'static str {
        match self {
            RawToolJsonRiskTier::ReadonlyRecoverable => "readonly_recoverable",
            RawToolJsonRiskTier::HighRisk => "high_risk",
            RawToolJsonRiskTier::Suppressed => "suppressed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawToolJsonAssessment {
    pub tier: RawToolJsonRiskTier,
    pub score: u8,
    pub reason: &'static str,
}

impl RawToolJsonAssessment {
    pub const fn readonly_recoverable(score: u8, reason: &'static str) -> Self {
        Self {
            tier: RawToolJsonRiskTier::ReadonlyRecoverable,
            score,
            reason,
        }
    }

    pub const fn high_risk(score: u8, reason: &'static str) -> Self {
        Self {
            tier: RawToolJsonRiskTier::HighRisk,
            score,
            reason,
        }
    }

    pub const fn suppressed(score: u8, reason: &'static str) -> Self {
        Self {
            tier: RawToolJsonRiskTier::Suppressed,
            score,
            reason,
        }
    }

    pub fn is_high_risk(self) -> bool {
        matches!(self.tier, RawToolJsonRiskTier::HighRisk)
    }
}

// ============================================================
// 标记常量
// ============================================================

pub const LEAKED_TOOL_MARKERS: [&str; 3] = ["assistant to=", "to=functions", "to=multi_tool_use"];
pub const MARKDOWN_BASH_MARKERS: [&str; 3] = ["```bash", "```sh", "```shell"];

// ============================================================
// 纯函数（迁移自 TransformResponse impl）
// ============================================================

pub fn starts_with_leaked_tool_marker(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("assistant to=")
        || trimmed.starts_with("to=functions")
        || trimmed.starts_with("to=multi_tool_use")
}

pub fn find_markdown_bash_start(line: &str) -> Option<(usize, usize)> {
    for marker in MARKDOWN_BASH_MARKERS {
        if let Some(idx) = line.find(marker) {
            return Some((idx, marker.len()));
        }
    }
    None
}

pub fn find_potential_leaked_tool_marker_start(line: &str) -> Option<usize> {
    LEAKED_TOOL_MARKERS
        .iter()
        .filter_map(|marker| line.find(marker))
        .min()
}

pub fn leaked_marker_suffix_len(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut max_len = 0usize;

    let all_markers = LEAKED_TOOL_MARKERS
        .iter()
        .chain(MARKDOWN_BASH_MARKERS.iter())
        .chain(["<proposed_plan>", "</proposed_plan>"].iter());

    for marker in all_markers {
        let marker_bytes = marker.as_bytes();
        if marker_bytes.len() <= 1 {
            continue;
        }

        let upper = std::cmp::min(bytes.len(), marker_bytes.len() - 1);
        for len in (1..=upper).rev() {
            if bytes.ends_with(&marker_bytes[..len]) {
                max_len = max_len.max(len);
                break;
            }
        }
    }

    max_len
}

pub fn find_potential_raw_tool_json_start(line: &str) -> Option<usize> {
    for (idx, ch) in line.char_indices() {
        if ch != '{' {
            continue;
        }
        if looks_like_potential_raw_tool_json_fragment(&line[idx..]) {
            return Some(idx);
        }
    }
    None
}

pub fn looks_like_potential_raw_tool_json_fragment(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('{') {
        return false;
    }

    let has_task_output_payload = trimmed.contains("\"task_id\"")
        && (trimmed.contains("\"block\"") || trimmed.contains("\"timeout\""));

    trimmed.contains("\"tool_uses\"")
        || trimmed.contains("\"recipient_name\"")
        || trimmed.contains("\"file_path\"")
        || trimmed.contains("\"old_string\"")
        || trimmed.contains("\"new_string\"")
        || trimmed.contains("\"replace_all\"")
        || has_task_output_payload
        || ((trimmed.contains("\"command\"") || trimmed.contains("\"cmd\""))
            && (trimmed.contains("\"description\"")
                || trimmed.contains("\"timeout\"")
                || trimmed.contains("\"yield_time_ms\"")
                || trimmed.contains("\"max_output_tokens\"")
                || trimmed.contains("\"sandbox_permissions\"")))
        || (trimmed.contains("\"pattern\"")
            && (trimmed.contains("\"output_mode\"")
                || trimmed.contains("\"glob\"")
                || trimmed.contains("\"path\"")))
}

pub fn looks_like_contextual_leaked_note_json(fragment: &str, context: &str) -> bool {
    let trimmed = fragment.trim_start();
    if !trimmed.starts_with('{') {
        return false;
    }

    let has_note_field = trimmed.contains("\"note\"") || trimmed.contains("\"notes\"");
    if !has_note_field {
        return false;
    }

    let has_execution_tone = trimmed.contains("running")
        || trimmed.contains("re-running")
        || trimmed.contains("Running")
        || trimmed.contains("Re-running")
        || trimmed.contains("tests")
        || trimmed.contains("fixes")
        || trimmed.contains("now");

    let near_fenced_json = context.contains("```json") || context.ends_with("```json\n");
    let has_suspicious_tail = fragment.contains("numerusform")
        || fragment.contains("assistantuser")
        || fragment.ends_with("user ")
        || fragment.ends_with("user")
        || fragment.contains("天天中彩票");

    let condition_count = [has_execution_tone, near_fenced_json, has_suspicious_tail]
        .iter()
        .filter(|&&x| x)
        .count();

    condition_count >= 2
}

pub fn strip_suspicious_trailing_noise(text: &str) -> String {
    let mut result = text.to_string();

    let noise_patterns = [
        "numerusform",
        "天天中彩票user",
        "天天中彩票",
        "assistantuser",
        " user ",
        " user",
    ];

    for pattern in &noise_patterns {
        if let Some(pos) = result.rfind(pattern) {
            result.truncate(pos);
            break;
        }
    }

    result.trim_end().to_string()
}

pub fn looks_like_contextual_running_prefix(prefix: &str) -> bool {
    let lower = prefix.to_ascii_lowercase();
    lower.contains("**re-running")
        || lower.contains("**running")
        || (lower.contains("running")
            && (lower.contains("verify") || lower.contains("test") || lower.contains("build")))
}

pub fn collapse_adjacent_duplicate_markdown_bold(text: &str) -> String {
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

    collapse_duplicate_bridge_overlap(&out)
}

pub fn collapse_duplicate_bridge_overlap(text: &str) -> String {
    if !text.contains("****") {
        return text.to_string();
    }

    let mut current = text.to_string();
    let mut guard = 0u8;

    while let Some(bridge_pos) = current.find("****") {
        if guard > 8 {
            break;
        }
        guard += 1;

        let left = &current[..bridge_pos];
        let right = &current[bridge_pos + 4..];
        let overlap = longest_suffix_prefix_overlap(left, right);
        if overlap < 16 {
            break;
        }

        let overlap_prefix = &right[..overlap];
        if !overlap_prefix.chars().any(|c| c.is_whitespace()) {
            break;
        }

        current = format!("{}{}", left, &right[overlap..]);
    }

    current
}

pub fn longest_suffix_prefix_overlap(left: &str, right: &str) -> usize {
    let max_possible = std::cmp::min(left.len(), right.len());
    (1..=max_possible)
        .rev()
        .find(|&len| left.ends_with(&right[..len]))
        .unwrap_or(0)
}

pub fn looks_like_exec_command_payload_fragment(line: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<Value>(line.trim()) else {
        return false;
    };
    let Some(obj) = parsed.as_object() else {
        return false;
    };

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
}

pub fn looks_like_task_output_payload_fragment(line: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<Value>(line.trim()) else {
        return false;
    };
    let Some(obj) = parsed.as_object() else {
        return false;
    };

    let has_task_id = obj
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_task_id {
        return false;
    }

    let has_control_fields = obj.contains_key("block") || obj.contains_key("timeout");
    if !has_control_fields {
        return false;
    }

    obj.keys()
        .all(|key| matches!(key.as_str(), "task_id" | "block" | "timeout"))
}

pub fn looks_like_read_payload_fragment(line: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<Value>(line.trim()) else {
        return false;
    };
    let Some(obj) = parsed.as_object() else {
        return false;
    };

    let has_file_path = obj
        .get("file_path")
        .and_then(|value| value.as_str())
        .map(|value| !value.trim().is_empty())
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
}

pub fn strip_known_leak_suffix_noise(text: &str) -> String {
    let trimmed = text.trim_end_matches(char::is_whitespace);
    let noise_patterns = [
        "assistantuser",
        "numeroususer",
        "numerusform",
        "天天中彩票user",
        "天天中彩票",
        " +++",
    ];

    for pattern in noise_patterns {
        if trimmed.ends_with(pattern) {
            let cut = trimmed.len().saturating_sub(pattern.len());
            return trimmed[..cut].to_string();
        }
    }

    text.to_string()
}

pub fn find_internal_planning_leak_start(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let cue_patterns = [
        "need now run ",
        "outside cwd?",
        "tools allowed read/write anywhere",
        "need respond concise",
        "no need reviewers",
        "let's run grep tool",
        "maybe run dart analyze",
    ];

    let mut first_hit: Option<usize> = None;
    let mut hit_count = 0usize;

    for pattern in cue_patterns {
        if let Some(pos) = lower.find(pattern) {
            hit_count += 1;
            first_hit = Some(first_hit.map_or(pos, |current| current.min(pos)));
        }
    }

    if hit_count >= 2 {
        return first_hit;
    }

    None
}

pub fn sanitize_prefix_before_raw_tool_json(prefix: &str) -> String {
    let cleaned = strip_known_leak_suffix_noise(prefix);
    let trimmed_meta = if let Some(cut_pos) = find_internal_planning_leak_start(&cleaned) {
        cleaned[..cut_pos].to_string()
    } else {
        cleaned
    };

    strip_trailing_json_hint_noise(&trimmed_meta)
}

pub fn strip_trailing_json_hint_noise(text: &str) -> String {
    let mut current = text.to_string();
    let mut removed_any = false;
    let marker_variants = ["```json", "####json", "###json", "##json", "#json", "json"];
    loop {
        let trimmed = current.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let lowered = trimmed.to_ascii_lowercase();
        let mut removed = false;
        for marker in marker_variants {
            if lowered.ends_with(marker) {
                let cut = trimmed.len().saturating_sub(marker.len());
                current = trimmed[..cut]
                    .trim_end_matches(|ch: char| {
                        ch.is_whitespace()
                            || matches!(ch, '#' | '`' | '*' | ':' | ';' | '-' | '_' | '.' | '。')
                    })
                    .to_string();
                removed_any = true;
                removed = true;
                break;
            }
        }

        if !removed {
            break;
        }
    }

    if removed_any {
        current
    } else {
        text.to_string()
    }
}

pub fn normalize_recipient_tool_name(recipient_name: &str) -> Option<&str> {
    let trimmed = recipient_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.rsplit('.').next().unwrap_or(trimmed))
}

pub fn is_readonly_tool_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("Read")
        || name.eq_ignore_ascii_case("Grep")
        || name.eq_ignore_ascii_case("Glob")
        || name.eq_ignore_ascii_case("LS")
}

pub fn looks_like_raw_tool_json_fragment(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('{') {
        return false;
    }

    let has_parallel_envelope = trimmed.contains("\"tool_uses\"")
        && trimmed.contains("\"recipient_name\"")
        && (trimmed.contains("functions.") || trimmed.contains("multi_tool_use."));
    if has_parallel_envelope {
        return true;
    }

    let has_edit_payload = trimmed.contains("\"file_path\"")
        && ((trimmed.contains("\"old_string\"") && trimmed.contains("\"new_string\""))
            || trimmed.contains("\"replace_all\""));
    if has_edit_payload {
        return true;
    }

    let has_write_payload = trimmed.contains("\"file_path\"")
        && trimmed.contains("\"content\"")
        && !trimmed.contains("\"old_string\"");
    if has_write_payload {
        return true;
    }

    let has_search_payload = trimmed.contains("\"pattern\"")
        && (trimmed.contains("\"output_mode\"")
            || trimmed.contains("\"path\"")
            || trimmed.contains("\"glob\""));
    if has_search_payload {
        return true;
    }

    let has_basic_tool_call_shape = trimmed.contains("\"recipient_name\"")
        && trimmed.contains("\"parameters\"")
        && (trimmed.contains("\"file_path\"")
            || trimmed.contains("\"pattern\"")
            || trimmed.contains("\"command\""));
    if has_basic_tool_call_shape {
        return true;
    }

    if looks_like_task_output_payload_fragment(trimmed) {
        return true;
    }

    if looks_like_read_payload_fragment(trimmed) {
        return true;
    }

    looks_like_exec_command_payload_fragment(trimmed)
}

pub fn extract_first_json_object_fragment(line: &str) -> Option<String> {
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

pub fn split_tool_json_prefix_suffix(fragment: &str) -> Option<(String, String, String)> {
    let json_start = find_potential_raw_tool_json_start(fragment)?;
    let prefix = fragment[..json_start].to_string();
    let candidate = &fragment[json_start..];
    let json = extract_first_json_object_fragment(candidate)?;
    if !looks_like_raw_tool_json_fragment(&json) {
        return None;
    }
    let suffix_start = json_start + json.len();
    let suffix = fragment[suffix_start..].to_string();
    Some((prefix, json, suffix))
}

pub fn split_contextual_note_json_prefix_suffix(
    fragment: &str,
    context: &str,
) -> Option<(String, String, String, bool)> {
    // 首先尝试查找 ```json 包装的情况
    if let Some(json_start) = context.find("```json") {
        let mut json_content_start = json_start + 7;
        if context.chars().nth(json_content_start) == Some('\n') {
            json_content_start += 1;
        }

        if looks_like_contextual_running_prefix(&context[..json_start]) {
            if let Some(json_end) = context[json_content_start..].find("```") {
                let json_end_pos = json_content_start + json_end;
                let json_content = &context[json_content_start..json_end_pos];

                if json_content.contains("\"note\"") {
                    let after_json_end = json_end_pos + 3;
                    let suffix = &context[after_json_end..];

                    let has_suspicious_tail = suffix.chars().any(|c| {
                        !c.is_ascii() || (c.is_ascii_alphabetic() && suffix.len() > 20)
                    });

                    if has_suspicious_tail {
                        let fragment_json_start =
                            if json_start >= context.len() - fragment.len() {
                                json_start - (context.len() - fragment.len())
                            } else {
                                0
                            };

                        let prefix_in_fragment = if fragment_json_start > 0 {
                            fragment[..fragment_json_start].to_string()
                        } else {
                            String::new()
                        };

                        let prefix_in_fragment =
                            collapse_adjacent_duplicate_markdown_bold(&prefix_in_fragment);

                        return Some((prefix_in_fragment, String::new(), String::new(), false));
                    }
                }
            } else {
                let remaining_content = &context[json_content_start..];
                if remaining_content.contains("\"note\"")
                    || remaining_content.starts_with("{\"note\":")
                {
                    let fragment_json_start = if json_start >= context.len() - fragment.len() {
                        json_start - (context.len() - fragment.len())
                    } else {
                        0
                    };

                    let prefix_in_fragment = if fragment_json_start > 0 {
                        fragment[..fragment_json_start].to_string()
                    } else {
                        String::new()
                    };

                    return Some((prefix_in_fragment, String::new(), String::new(), true));
                }
            }
        }
    }

    // 回退到裸 JSON
    let json_start = find_potential_raw_tool_json_start(fragment)?;
    let prefix = fragment[..json_start].to_string();
    let candidate = &fragment[json_start..];
    let json = extract_first_json_object_fragment(candidate)?;

    if !looks_like_contextual_leaked_note_json(&json, context) {
        return None;
    }

    let suffix_start = json_start + json.len();
    let suffix = strip_suspicious_trailing_noise(&fragment[suffix_start..]);

    Some((prefix, json, suffix, false))
}

pub fn assess_raw_tool_json(raw_json: &str) -> RawToolJsonAssessment {
    let Ok(parsed) = serde_json::from_str::<Value>(raw_json) else {
        return RawToolJsonAssessment::suppressed(20, "json_parse_failed");
    };
    let Some(obj) = parsed.as_object() else {
        return RawToolJsonAssessment::suppressed(25, "json_not_object");
    };

    if let Some(tool_uses) = obj.get("tool_uses").and_then(|v| v.as_array()) {
        if tool_uses.is_empty() {
            return RawToolJsonAssessment::suppressed(55, "tool_uses_empty");
        }

        for tool in tool_uses {
            let recipient = tool.get("recipient_name").and_then(|v| v.as_str());
            let Some(recipient) = recipient else {
                return RawToolJsonAssessment::high_risk(98, "tool_uses_missing_recipient");
            };
            let Some(name) = normalize_recipient_tool_name(recipient) else {
                return RawToolJsonAssessment::high_risk(98, "tool_uses_invalid_recipient");
            };
            if !tool
                .get("parameters")
                .map(|v| v.is_object())
                .unwrap_or(false)
            {
                return RawToolJsonAssessment::high_risk(97, "tool_uses_invalid_parameters");
            }
            if !is_readonly_tool_name(name) {
                return RawToolJsonAssessment::high_risk(95, "tool_uses_non_readonly");
            }
        }

        return RawToolJsonAssessment::readonly_recoverable(93, "tool_uses_all_readonly");
    }

    let has_edit_shape = obj.contains_key("old_string")
        && obj.contains_key("new_string")
        && obj.contains_key("file_path");
    if has_edit_shape {
        return RawToolJsonAssessment::high_risk(90, "edit_payload_shape");
    }

    let has_write_shape = obj.contains_key("content") && obj.contains_key("file_path");
    if has_write_shape {
        return RawToolJsonAssessment::high_risk(88, "write_payload_shape");
    }

    let trimmed = raw_json.trim();
    if looks_like_exec_command_payload_fragment(trimmed) {
        return RawToolJsonAssessment::high_risk(92, "exec_payload_shape");
    }

    if looks_like_task_output_payload_fragment(trimmed) {
        return RawToolJsonAssessment::suppressed(76, "task_output_control_shape");
    }

    if looks_like_read_payload_fragment(trimmed) {
        return RawToolJsonAssessment::suppressed(74, "read_window_shape");
    }

    let has_search_shape = obj.contains_key("pattern")
        && (obj.contains_key("output_mode") || obj.contains_key("path") || obj.contains_key("glob"));
    if has_search_shape {
        return RawToolJsonAssessment::suppressed(73, "search_payload_shape");
    }

    let has_command_only = obj.contains_key("command") || obj.contains_key("cmd");
    if has_command_only {
        return RawToolJsonAssessment::suppressed(60, "command_without_exec_context");
    }

    RawToolJsonAssessment::suppressed(40, "generic_suspicious_json_shape")
}
