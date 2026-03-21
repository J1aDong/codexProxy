use crate::logger::AppLogger;
use crate::transform::{ResponseTransformRequestContext, ResponseTransformer};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextEventSource {
    OutputTextDelta,
    ContentPartAdded,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    AwaitingContent,
    StreamingText,
    StreamingThinking,
    BufferingToolCalls,
    Terminal,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpstreamItemKind {
    Message,
    FunctionCall,
    Reasoning,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextRoutingDecision {
    Emit,
    Suppress,
    DeferUntilToolWindowCloses,
}

#[derive(Clone)]
struct ActiveWebSearchCall {
    output_index: Option<u64>,
    item_id: Option<String>,
    content_block_index: usize,
    input_closed: bool,
}

#[derive(Default)]
struct TransformDiagnostics {
    deferred_unscoped_text_chunks: u64,
    deferred_unscoped_text_flushes: u64,
    detected_proposed_plan_blocks: u64,
    extracted_proposed_plan_body_chars: u64,
    plan_bridge_write_successes: u64,
    plan_bridge_write_failures: u64,
    plan_bridge_exit_plan_mode_emitted: u64,
    dropped_leaked_marker_fragments: u64,
    dropped_raw_tool_json_fragments: u64,
    assessed_raw_tool_json_fragments: u64,
    assessed_raw_tool_json_readonly_recoverable: u64,
    assessed_raw_tool_json_high_risk: u64,
    assessed_raw_tool_json_suppressed: u64,
    dropped_high_risk_raw_tool_json_fragments: u64,
    emitted_high_risk_leak_questions: u64,
    recovered_readonly_leaked_tool_payloads: u64,
    recovered_readonly_leaked_tool_calls: u64,
    dropped_contextual_note_json_fragments: u64,
    dropped_incomplete_tool_json_fragments: u64,
    queued_orphan_tool_argument_updates: u64,
    applied_orphan_tool_argument_updates: u64,
    dropped_orphan_tool_argument_updates_no_hint: u64,
    dropped_orphan_tool_argument_updates_closed_call: u64,
    dropped_pending_tool_argument_updates_closed_call: u64,
    duplicate_active_call_items: u64,
    dropped_reused_closed_call_items: u64,
    binding_conflicts_output_index: u64,
    binding_conflicts_item_id: u64,
    normalized_item_id_mismatches: u64,
    pending_tool_backlog_trimmed: u64,
    dropped_function_args_whitespace_overflow_fragments: u64,
    terminal_invariant_violations: u64,
}

impl TransformDiagnostics {
    fn has_activity(&self) -> bool {
        self.deferred_unscoped_text_chunks > 0
            || self.deferred_unscoped_text_flushes > 0
            || self.detected_proposed_plan_blocks > 0
            || self.extracted_proposed_plan_body_chars > 0
            || self.plan_bridge_write_successes > 0
            || self.plan_bridge_write_failures > 0
            || self.plan_bridge_exit_plan_mode_emitted > 0
            || self.dropped_leaked_marker_fragments > 0
            || self.dropped_raw_tool_json_fragments > 0
            || self.assessed_raw_tool_json_fragments > 0
            || self.assessed_raw_tool_json_readonly_recoverable > 0
            || self.assessed_raw_tool_json_high_risk > 0
            || self.assessed_raw_tool_json_suppressed > 0
            || self.dropped_high_risk_raw_tool_json_fragments > 0
            || self.emitted_high_risk_leak_questions > 0
            || self.recovered_readonly_leaked_tool_payloads > 0
            || self.recovered_readonly_leaked_tool_calls > 0
            || self.dropped_contextual_note_json_fragments > 0
            || self.dropped_incomplete_tool_json_fragments > 0
            || self.queued_orphan_tool_argument_updates > 0
            || self.applied_orphan_tool_argument_updates > 0
            || self.dropped_orphan_tool_argument_updates_no_hint > 0
            || self.dropped_orphan_tool_argument_updates_closed_call > 0
            || self.dropped_pending_tool_argument_updates_closed_call > 0
            || self.duplicate_active_call_items > 0
            || self.dropped_reused_closed_call_items > 0
            || self.binding_conflicts_output_index > 0
            || self.binding_conflicts_item_id > 0
            || self.normalized_item_id_mismatches > 0
            || self.pending_tool_backlog_trimmed > 0
            || self.dropped_function_args_whitespace_overflow_fragments > 0
            || self.terminal_invariant_violations > 0
    }
}

#[derive(Clone, Default, PartialEq, Eq, Hash)]
struct EventPartKey {
    output_index: Option<u64>,
    item_id: Option<String>,
    content_index: Option<u64>,
}

impl EventPartKey {
    fn is_empty(&self) -> bool {
        self.output_index.is_none() && self.item_id.is_none() && self.content_index.is_none()
    }

    fn matches_item(&self, output_index: Option<u64>, item_id: Option<&str>) -> bool {
        let output_match = output_index
            .map(|idx| self.output_index == Some(idx))
            .unwrap_or(false);
        let item_match = item_id
            .map(|id| self.item_id.as_deref() == Some(id))
            .unwrap_or(false);
        output_match || item_match
    }
}

#[derive(Default)]
struct EventMetadata {
    output_index: Option<u64>,
    item_id: Option<String>,
    call_id: Option<String>,
    content_index: Option<u64>,
}

impl EventMetadata {
    fn from_data(data: &Value) -> Self {
        let output_index = data.get("output_index").and_then(|v| v.as_u64());
        let item_id = data
            .get("item_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                data.get("item")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        let call_id = data
            .get("call_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                data.get("item")
                    .and_then(|v| v.get("call_id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        let content_index = data.get("content_index").and_then(|v| v.as_u64());

        Self {
            output_index,
            item_id,
            call_id,
            content_index,
        }
    }

    fn has_routing_hint(&self) -> bool {
        self.output_index.is_some() || self.item_id.is_some() || self.call_id.is_some()
    }

    fn part_key(&self) -> EventPartKey {
        EventPartKey {
            output_index: self.output_index,
            item_id: self.item_id.clone(),
            content_index: self.content_index,
        }
    }
}

#[derive(Clone)]
struct BufferedToolCall {
    order_key: u64,
    arrival_seq: u64,
    output_index: Option<u64>,
    item_id: Option<String>,
    call_id: String,
    name: String,
    arguments_buffer: String,
    consecutive_whitespace_run: usize,
    done_flag: bool,
    start_emitted: bool,
    content_block_index: Option<usize>,
    emitted_arguments_len: usize,
    last_progress_message: Option<String>,
}

enum PendingToolArgumentUpdateKind {
    Delta(String),
    Snapshot(String),
}

struct PendingToolArgumentUpdate {
    output_index: Option<u64>,
    item_id: Option<String>,
    call_id: Option<String>,
    kind: PendingToolArgumentUpdateKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawToolJsonRiskTier {
    ReadonlyRecoverable,
    HighRisk,
    Suppressed,
}

impl RawToolJsonRiskTier {
    fn as_str(self) -> &'static str {
        match self {
            RawToolJsonRiskTier::ReadonlyRecoverable => "readonly_recoverable",
            RawToolJsonRiskTier::HighRisk => "high_risk",
            RawToolJsonRiskTier::Suppressed => "suppressed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RawToolJsonAssessment {
    tier: RawToolJsonRiskTier,
    score: u8,
    reason: &'static str,
}

impl RawToolJsonAssessment {
    const fn readonly_recoverable(score: u8, reason: &'static str) -> Self {
        Self {
            tier: RawToolJsonRiskTier::ReadonlyRecoverable,
            score,
            reason,
        }
    }

    const fn high_risk(score: u8, reason: &'static str) -> Self {
        Self {
            tier: RawToolJsonRiskTier::HighRisk,
            score,
            reason,
        }
    }

    const fn suppressed(score: u8, reason: &'static str) -> Self {
        Self {
            tier: RawToolJsonRiskTier::Suppressed,
            score,
            reason,
        }
    }

    fn is_high_risk(self) -> bool {
        matches!(self.tier, RawToolJsonRiskTier::HighRisk)
    }
}

/// Facade for leak-detection helpers so leak handling can be hardened independently
/// from the core stream state machine.
struct LeakDetector;

impl LeakDetector {
    fn starts_with_leaked_tool_marker(line: &str) -> bool {
        TransformResponse::starts_with_leaked_tool_marker(line)
    }

    fn find_markdown_bash_start(line: &str) -> Option<(usize, usize)> {
        TransformResponse::find_markdown_bash_start(line)
    }

    fn find_potential_leaked_tool_marker_start(line: &str) -> Option<usize> {
        TransformResponse::find_potential_leaked_tool_marker_start(line)
    }

    fn leaked_marker_suffix_len(line: &str) -> usize {
        TransformResponse::leaked_marker_suffix_len(line)
    }

    fn strip_suspicious_trailing_noise(text: &str) -> String {
        TransformResponse::strip_suspicious_trailing_noise(text)
    }

    fn strip_known_leak_suffix_noise(text: &str) -> String {
        TransformResponse::strip_known_leak_suffix_noise(text)
    }

    fn sanitize_prefix_before_raw_tool_json(prefix: &str) -> String {
        TransformResponse::sanitize_prefix_before_raw_tool_json(prefix)
    }

    fn collapse_adjacent_duplicate_markdown_bold(text: &str) -> String {
        TransformResponse::collapse_adjacent_duplicate_markdown_bold(text)
    }

    fn find_potential_raw_tool_json_start(line: &str) -> Option<usize> {
        TransformResponse::find_potential_raw_tool_json_start(line)
    }

    fn split_tool_json_prefix_suffix(fragment: &str) -> Option<(String, String, String)> {
        TransformResponse::split_tool_json_prefix_suffix(fragment)
    }

    fn split_contextual_note_json_prefix_suffix(
        fragment: &str,
        context: &str,
    ) -> Option<(String, String, String, bool)> {
        TransformResponse::split_contextual_note_json_prefix_suffix(fragment, context)
    }

    fn assess_raw_tool_json(raw_json: &str) -> RawToolJsonAssessment {
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
                let Some(name) = TransformResponse::normalize_recipient_tool_name(recipient) else {
                    return RawToolJsonAssessment::high_risk(98, "tool_uses_invalid_recipient");
                };
                if !tool
                    .get("parameters")
                    .map(|v| v.is_object())
                    .unwrap_or(false)
                {
                    return RawToolJsonAssessment::high_risk(97, "tool_uses_invalid_parameters");
                }
                if !TransformResponse::is_readonly_tool_name(name) {
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
        if TransformResponse::looks_like_exec_command_payload_fragment(trimmed) {
            return RawToolJsonAssessment::high_risk(92, "exec_payload_shape");
        }

        if TransformResponse::looks_like_task_output_payload_fragment(trimmed) {
            return RawToolJsonAssessment::suppressed(76, "task_output_control_shape");
        }

        if TransformResponse::looks_like_read_payload_fragment(trimmed) {
            return RawToolJsonAssessment::suppressed(74, "read_window_shape");
        }

        let has_search_shape = obj.contains_key("pattern")
            && (obj.contains_key("output_mode")
                || obj.contains_key("path")
                || obj.contains_key("glob"));
        if has_search_shape {
            return RawToolJsonAssessment::suppressed(73, "search_payload_shape");
        }

        let has_command_only = obj.contains_key("command") || obj.contains_key("cmd");
        if has_command_only {
            return RawToolJsonAssessment::suppressed(60, "command_without_exec_context");
        }

        RawToolJsonAssessment::suppressed(40, "generic_suspicious_json_shape")
    }
}

/// 响应转换器 - Codex SSE -> Anthropic SSE
pub struct TransformResponse {
    message_id: String,
    model: String,
    content_index: usize,
    open_text_index: Option<usize>,
    open_thinking_index: Option<usize>,
    allow_visible_thinking: bool,
    phase: StreamPhase,
    next_tool_order_key: u64,
    next_tool_arrival_seq: u64,
    buffered_tool_calls: Vec<BufferedToolCall>,
    tool_order_by_output_index: HashMap<u64, u64>,
    tool_order_by_item_id: HashMap<String, u64>,
    tool_order_by_call_id: HashMap<String, u64>,
    canonical_item_id_by_output_index: HashMap<u64, String>,
    canonical_output_index_by_item_id: HashMap<String, u64>,
    closed_tool_call_ids: HashSet<String>,
    last_buffered_tool_order: Option<u64>,
    item_kind_by_output_index: HashMap<u64, UpstreamItemKind>,
    item_kind_by_item_id: HashMap<String, UpstreamItemKind>,
    item_kind_by_call_id: HashMap<String, UpstreamItemKind>,
    active_text_parts: HashSet<EventPartKey>,
    text_dedupe_by_part: HashMap<EventPartKey, (TextEventSource, String)>,
    saw_tool_call: bool,
    saw_refusal: bool,
    refusal_text_buffer: String,
    sent_message_start: bool,
    text_carryover: String,
    pending_tool_text: String,
    deferred_unscoped_text: String,
    pending_tool_argument_updates: Vec<PendingToolArgumentUpdate>,
    capturing_proposed_plan: bool,
    proposed_plan_body_buffer: String,
    latest_proposed_plan_body: Option<String>,
    codex_plan_file_path: Option<String>,
    contains_background_agent_completion: bool,
    plan_bridge_emitted: bool,

    // Cross-chunk leak suppression state
    suppressing_cross_chunk_leak: bool,
    suppressing_suggestion_mode_prompt: bool,

    // Markdown Base Interception
    in_markdown_bash: bool,
    markdown_bash_buffer: String,

    // Commentary phase: redirect text to thinking blocks instead of text blocks
    in_commentary_phase: bool,
    // Fallback commentary detection: reasoning seen in current response
    had_reasoning_in_response: bool,
    // Track if we've seen a message-type output_item.added (means phase detection is explicit)
    saw_message_item_added: bool,
    next_server_tool_use_seq: u64,
    active_web_search_calls: HashMap<String, ActiveWebSearchCall>,
    web_search_call_by_output_index: HashMap<u64, String>,
    web_search_call_by_item_id: HashMap<String, String>,
    response_created_announced: bool,
    response_in_progress_announced: bool,
    launched_background_agent_count: usize,
    terminal_background_agent_completion_count: usize,
    suppress_visible_final_answer_text: bool,
    high_risk_leak_question_emitted: bool,
    last_terminal_event: Option<String>,
    diagnostics: TransformDiagnostics,

    logger: std::sync::Arc<AppLogger>,
}

impl TransformResponse {
    const MAX_FUNCTION_ARGS_WHITESPACE_RUN: usize = 64;
    const LEAKED_TOOL_MARKERS: [&'static str; 3] =
        ["assistant to=", "to=functions", "to=multi_tool_use"];

    const MARKDOWN_BASH_MARKERS: [&'static str; 3] = ["```bash", "```sh", "```shell"];
    const SUGGESTION_MODE_START_MARKER: &'static str = "[SUGGESTION MODE:";
    const SUGGESTION_MODE_END_MARKER: &'static str =
        "Reply with ONLY the suggestion, no quotes or explanation.";
    const PROPOSED_PLAN_OPEN_TAG: &'static str = "<proposed_plan>";
    const PROPOSED_PLAN_CLOSE_TAG: &'static str = "</proposed_plan>";

    fn find_potential_leaked_tool_marker_start(line: &str) -> Option<usize> {
        Self::LEAKED_TOOL_MARKERS
            .iter()
            .filter_map(|marker| line.find(marker))
            .min()
    }

    fn leaked_marker_suffix_len(line: &str) -> usize {
        let bytes = line.as_bytes();
        let mut max_len = 0usize;

        let all_markers = Self::LEAKED_TOOL_MARKERS
            .iter()
            .chain(Self::MARKDOWN_BASH_MARKERS.iter())
            .chain([Self::PROPOSED_PLAN_OPEN_TAG, Self::PROPOSED_PLAN_CLOSE_TAG].iter());

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

    fn strip_proposed_plan_wrappers(text: &str) -> String {
        text.to_string()
    }

    fn record_extracted_proposed_plan_body(&mut self) {
        let body = self.proposed_plan_body_buffer.trim();
        if body.is_empty() {
            return;
        }

        self.diagnostics.detected_proposed_plan_blocks += 1;
        self.diagnostics.extracted_proposed_plan_body_chars += body.chars().count() as u64;
        self.latest_proposed_plan_body = Some(body.to_string());
        self.logger.log_raw(&format!(
            "[PlanBridge] Extracted proposed_plan body chars={}\n{}",
            body.chars().count(),
            body
        ));
    }

    fn should_suppress_visible_proposed_plan_text(&self) -> bool {
        self.codex_plan_file_path.is_some()
    }

    fn build_suppressed_proposed_plan_fallback(&self) -> Option<String> {
        if let Some(body) = self.latest_proposed_plan_body.as_deref() {
            return Some(format!(
                "{}\n{}\n{}",
                Self::PROPOSED_PLAN_OPEN_TAG,
                body,
                Self::PROPOSED_PLAN_CLOSE_TAG
            ));
        }

        if self.capturing_proposed_plan && !self.proposed_plan_body_buffer.trim().is_empty() {
            return Some(format!(
                "{}\n{}",
                Self::PROPOSED_PLAN_OPEN_TAG,
                self.proposed_plan_body_buffer.trim()
            ));
        }

        None
    }

    fn write_codex_plan_bridge_file(&mut self) -> bool {
        let Some(plan_file_path) = self.codex_plan_file_path.as_deref() else {
            return false;
        };
        let Some(plan_body) = self.latest_proposed_plan_body.as_deref() else {
            return false;
        };

        let path = std::path::Path::new(plan_file_path);
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.diagnostics.plan_bridge_write_failures += 1;
                self.logger.log_raw(&format!(
                    "[PlanBridge] Failed to create parent dir for {}: {}",
                    plan_file_path, error
                ));
                return false;
            }
        }

        match std::fs::write(path, plan_body) {
            Ok(_) => {
                self.diagnostics.plan_bridge_write_successes += 1;
                self.logger.log_raw(&format!(
                    "[PlanBridge] Wrote plan file to {}",
                    plan_file_path
                ));
                true
            }
            Err(error) => {
                self.diagnostics.plan_bridge_write_failures += 1;
                self.logger.log_raw(&format!(
                    "[PlanBridge] Failed to write plan file {}: {}",
                    plan_file_path, error
                ));
                false
            }
        }
    }

    fn maybe_emit_plan_mode_bridge(&mut self, output: &mut Vec<String>) -> bool {
        if self.plan_bridge_emitted || self.saw_tool_call {
            return false;
        }
        if self.latest_proposed_plan_body.is_none() || self.codex_plan_file_path.is_none() {
            return false;
        }
        if !self.write_codex_plan_bridge_file() {
            return false;
        }

        let synthetic_tool = BufferedToolCall {
            order_key: u64::MAX,
            arrival_seq: u64::MAX,
            output_index: None,
            item_id: None,
            call_id: format!("plan_bridge_exit_{}", chrono::Utc::now().timestamp_millis()),
            name: "ExitPlanMode".to_string(),
            arguments_buffer: "{}".to_string(),
            consecutive_whitespace_run: 0,
            done_flag: true,
            start_emitted: false,
            content_block_index: None,
            emitted_arguments_len: 0,
            last_progress_message: None,
        };

        self.diagnostics.plan_bridge_exit_plan_mode_emitted += 1;
        self.plan_bridge_emitted = true;
        self.saw_tool_call = true;
        self.logger
            .log_raw("[PlanBridge] Emitting synthetic ExitPlanMode tool_use");
        self.emit_serialized_tool_call(output, &synthetic_tool);
        true
    }

    fn observe_proposed_plan_fragment(&mut self, fragment: &str) -> String {
        if fragment.is_empty() {
            return String::new();
        }

        if !self.should_suppress_visible_proposed_plan_text() {
            let mut remaining = fragment.to_string();
            loop {
                if self.capturing_proposed_plan {
                    if let Some(end) = remaining.find(Self::PROPOSED_PLAN_CLOSE_TAG) {
                        self.proposed_plan_body_buffer.push_str(&remaining[..end]);
                        self.record_extracted_proposed_plan_body();
                        self.proposed_plan_body_buffer.clear();
                        self.capturing_proposed_plan = false;
                        remaining =
                            remaining[end + Self::PROPOSED_PLAN_CLOSE_TAG.len()..].to_string();
                        continue;
                    }

                    self.proposed_plan_body_buffer.push_str(&remaining);
                    break;
                }

                let Some(start) = remaining.find(Self::PROPOSED_PLAN_OPEN_TAG) else {
                    break;
                };
                remaining = remaining[start + Self::PROPOSED_PLAN_OPEN_TAG.len()..].to_string();
                self.capturing_proposed_plan = true;
                self.proposed_plan_body_buffer.clear();
            }
            return fragment.to_string();
        }

        let mut remaining = fragment.to_string();
        let mut visible = String::new();
        loop {
            if self.capturing_proposed_plan {
                if let Some(end) = remaining.find(Self::PROPOSED_PLAN_CLOSE_TAG) {
                    self.proposed_plan_body_buffer.push_str(&remaining[..end]);
                    self.record_extracted_proposed_plan_body();
                    self.proposed_plan_body_buffer.clear();
                    self.capturing_proposed_plan = false;
                    remaining = remaining[end + Self::PROPOSED_PLAN_CLOSE_TAG.len()..].to_string();
                    continue;
                }

                self.proposed_plan_body_buffer.push_str(&remaining);
                break;
            }

            let Some(start) = remaining.find(Self::PROPOSED_PLAN_OPEN_TAG) else {
                visible.push_str(&remaining);
                break;
            };
            visible.push_str(&remaining[..start]);
            remaining = remaining[start + Self::PROPOSED_PLAN_OPEN_TAG.len()..].to_string();
            self.capturing_proposed_plan = true;
            self.proposed_plan_body_buffer.clear();
        }

        visible
    }

    fn find_markdown_bash_start(line: &str) -> Option<(usize, usize)> {
        for marker in Self::MARKDOWN_BASH_MARKERS {
            if let Some(idx) = line.find(marker) {
                return Some((idx, marker.len()));
            }
        }
        None
    }

    fn starts_with_leaked_tool_marker(line: &str) -> bool {
        let trimmed = line.trim_start();
        trimmed.starts_with("assistant to=")
            || trimmed.starts_with("to=functions")
            || trimmed.starts_with("to=multi_tool_use")
    }

    fn find_potential_raw_tool_json_start(line: &str) -> Option<usize> {
        for (idx, ch) in line.char_indices() {
            if ch != '{' {
                continue;
            }
            if Self::looks_like_potential_raw_tool_json_fragment(&line[idx..]) {
                return Some(idx);
            }
        }
        None
    }

    fn looks_like_potential_raw_tool_json_fragment(line: &str) -> bool {
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

    fn looks_like_contextual_leaked_note_json(fragment: &str, context: &str) -> bool {
        let trimmed = fragment.trim_start();
        if !trimmed.starts_with('{') {
            return false;
        }

        // 检查是否包含 note 字段且内容是执行提示语气
        let has_note_field = trimmed.contains("\"note\"") || trimmed.contains("\"notes\"");
        if !has_note_field {
            return false;
        }

        // 检查执行提示语气关键词
        let has_execution_tone = trimmed.contains("running")
            || trimmed.contains("re-running")
            || trimmed.contains("Running")
            || trimmed.contains("Re-running")
            || trimmed.contains("tests")
            || trimmed.contains("fixes")
            || trimmed.contains("now");

        // 检查上下文条件
        let near_fenced_json = context.contains("```json") || context.ends_with("```json\n");
        let has_suspicious_tail = fragment.contains("numerusform")
            || fragment.contains("assistantuser")
            || fragment.ends_with("user ")
            || fragment.ends_with("user")
            || fragment.contains("天天中彩票");

        // 至少满足 2 个条件才认为是上下文泄漏
        let condition_count = [has_execution_tone, near_fenced_json, has_suspicious_tail]
            .iter()
            .filter(|&&x| x)
            .count();

        condition_count >= 2
    }

    fn strip_suspicious_trailing_noise(text: &str) -> String {
        let mut result = text.to_string();

        // 移除常见的噪声尾巴
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

    fn looks_like_contextual_running_prefix(prefix: &str) -> bool {
        let lower = prefix.to_ascii_lowercase();
        lower.contains("**re-running")
            || lower.contains("**running")
            || (lower.contains("running")
                && (lower.contains("verify") || lower.contains("test") || lower.contains("build")))
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

        Self::collapse_duplicate_bridge_overlap(&out)
    }

    fn collapse_duplicate_bridge_overlap(text: &str) -> String {
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
            let overlap = Self::longest_suffix_prefix_overlap(left, right);
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

    fn longest_suffix_prefix_overlap(left: &str, right: &str) -> usize {
        let max_len = left.len().min(right.len());
        if max_len == 0 {
            return 0;
        }

        let mut boundaries = Vec::new();
        for (idx, _) in right.char_indices() {
            boundaries.push(idx);
        }
        boundaries.push(right.len());

        for len in boundaries.into_iter().rev() {
            if len == 0 || len > max_len {
                continue;
            }
            if !left.is_char_boundary(left.len().saturating_sub(len)) {
                continue;
            }
            if left[left.len() - len..] == right[..len] {
                return len;
            }
        }

        0
    }

    fn looks_like_exec_command_payload_fragment(line: &str) -> bool {
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

    fn looks_like_task_output_payload_fragment(line: &str) -> bool {
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

    fn looks_like_read_payload_fragment(line: &str) -> bool {
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

    fn strip_known_leak_suffix_noise(text: &str) -> String {
        let trimmed = text.trim_end_matches(char::is_whitespace);
        let noise_patterns = [
            "assistantuser",
            "numeroususer",
            "numerusform",
            "天天中彩票user",
            "天天中彩票",
            " +#+#+#+#+#+",
        ];

        for pattern in noise_patterns {
            if trimmed.ends_with(pattern) {
                let cut = trimmed.len().saturating_sub(pattern.len());
                return trimmed[..cut].to_string();
            }
        }

        text.to_string()
    }

    fn find_internal_planning_leak_start(text: &str) -> Option<usize> {
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

    fn sanitize_prefix_before_raw_tool_json(prefix: &str) -> String {
        let cleaned = LeakDetector::strip_known_leak_suffix_noise(prefix);
        let trimmed_meta = if let Some(cut_pos) = Self::find_internal_planning_leak_start(&cleaned)
        {
            cleaned[..cut_pos].to_string()
        } else {
            cleaned
        };

        Self::strip_trailing_json_hint_noise(&trimmed_meta)
    }

    fn strip_trailing_json_hint_noise(text: &str) -> String {
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
                                || matches!(
                                    ch,
                                    '#' | '`' | '*' | ':' | ';' | '-' | '_' | '.' | '。'
                                )
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

    fn record_raw_tool_json_assessment(&mut self, assessment: RawToolJsonAssessment) {
        self.diagnostics.assessed_raw_tool_json_fragments += 1;
        match assessment.tier {
            RawToolJsonRiskTier::ReadonlyRecoverable => {
                self.diagnostics.assessed_raw_tool_json_readonly_recoverable += 1;
            }
            RawToolJsonRiskTier::HighRisk => {
                self.diagnostics.assessed_raw_tool_json_high_risk += 1;
            }
            RawToolJsonRiskTier::Suppressed => {
                self.diagnostics.assessed_raw_tool_json_suppressed += 1;
            }
        }
        self.logger.log_raw(&format!(
            "[Diag] raw_tool_json_assessment tier={} score={} reason={}",
            assessment.tier.as_str(),
            assessment.score,
            assessment.reason
        ));
    }

    fn maybe_emit_high_risk_leak_question(&mut self, output: &mut Vec<String>) {
        if self.high_risk_leak_question_emitted {
            return;
        }
        self.high_risk_leak_question_emitted = true;
        self.diagnostics.emitted_high_risk_leak_questions += 1;
        self.logger.log_raw(
            "[Warn] Emitting synthetic AskUserQuestion tool_use for high-risk leak confirmation",
        );
        self.emit_high_risk_leak_ask_user_question_tool(output);
    }

    fn emit_high_risk_leak_ask_user_question_tool(&mut self, output: &mut Vec<String>) {
        let seq = self.next_tool_arrival_seq;
        self.next_tool_arrival_seq = self.next_tool_arrival_seq.saturating_add(1);

        let input = json!({
            "questions": [
                {
                    "header": "安全确认",
                    "question": "检测到疑似高风险工具参数泄露，系统已拦截且未自动执行。是否继续由我在安全模式下重试？",
                    "multiSelect": false,
                    "options": [
                        {
                            "label": "继续（推荐）",
                            "description": "仅在安全模式下重试，不会自动执行高风险工具操作。"
                        },
                        {
                            "label": "取消",
                            "description": "保持拦截并结束当前高风险操作。"
                        }
                    ]
                }
            ]
        });

        let tool = BufferedToolCall {
            order_key: 0,
            arrival_seq: seq,
            output_index: None,
            item_id: None,
            call_id: format!("call_high_risk_leak_question_{}", seq),
            name: "AskUserQuestion".to_string(),
            arguments_buffer: input.to_string(),
            consecutive_whitespace_run: 0,
            done_flag: true,
            start_emitted: false,
            content_block_index: None,
            emitted_arguments_len: 0,
            last_progress_message: None,
        };
        self.emit_serialized_tool_call(output, &tool);
    }

    fn normalize_recipient_tool_name(recipient_name: &str) -> Option<&str> {
        let trimmed = recipient_name.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.rsplit('.').next().unwrap_or(trimmed))
    }

    fn is_readonly_tool_name(name: &str) -> bool {
        name.eq_ignore_ascii_case("Read")
            || name.eq_ignore_ascii_case("Grep")
            || name.eq_ignore_ascii_case("Glob")
            || name.eq_ignore_ascii_case("LS")
    }

    fn next_recovered_leak_call_id(&mut self, tool_name: &str) -> String {
        let seq = self.next_tool_arrival_seq;
        self.next_tool_arrival_seq = self.next_tool_arrival_seq.saturating_add(1);
        format!("leak_recovered_{}_{}", tool_name.to_ascii_lowercase(), seq)
    }

    fn try_recover_readonly_tool_uses_from_raw_json(
        &mut self,
        output: &mut Vec<String>,
        raw_json: &str,
    ) -> Option<usize> {
        let parsed = serde_json::from_str::<Value>(raw_json).ok()?;
        let tool_uses = parsed.get("tool_uses")?.as_array()?;
        if tool_uses.is_empty() {
            return None;
        }

        let mut recovered_calls = Vec::with_capacity(tool_uses.len());
        for tool in tool_uses {
            let recipient = tool.get("recipient_name").and_then(|v| v.as_str())?;
            let tool_name = Self::normalize_recipient_tool_name(recipient)?;
            if !Self::is_readonly_tool_name(tool_name) {
                return None;
            }

            let parameters = tool.get("parameters")?;
            if !parameters.is_object() {
                return None;
            }

            let arguments = serde_json::to_string(parameters).ok()?;
            let call_id = self.next_recovered_leak_call_id(tool_name);

            recovered_calls.push(BufferedToolCall {
                order_key: u64::MAX,
                arrival_seq: u64::MAX,
                output_index: None,
                item_id: None,
                call_id,
                name: tool_name.to_string(),
                arguments_buffer: arguments,
                consecutive_whitespace_run: 0,
                done_flag: true,
                start_emitted: false,
                content_block_index: None,
                emitted_arguments_len: 0,
                last_progress_message: None,
            });
        }

        for tool in &recovered_calls {
            self.saw_tool_call = true;
            self.emit_serialized_tool_call(output, tool);
        }
        self.sync_phase_from_runtime();

        Some(recovered_calls.len())
    }

    fn looks_like_raw_tool_json_fragment(line: &str) -> bool {
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

        if Self::looks_like_task_output_payload_fragment(trimmed) {
            return true;
        }

        if Self::looks_like_read_payload_fragment(trimmed) {
            return true;
        }

        Self::looks_like_exec_command_payload_fragment(trimmed)
    }

    fn split_tool_json_prefix_suffix(fragment: &str) -> Option<(String, String, String)> {
        let json_start = LeakDetector::find_potential_raw_tool_json_start(fragment)?;
        let prefix = fragment[..json_start].to_string();
        let candidate = &fragment[json_start..];
        let json = Self::extract_first_json_object_fragment(candidate)?;
        if !Self::looks_like_raw_tool_json_fragment(&json) {
            return None;
        }
        let suffix_start = json_start + json.len();
        let suffix = fragment[suffix_start..].to_string();
        Some((prefix, json, suffix))
    }

    fn split_contextual_note_json_prefix_suffix(
        fragment: &str,
        context: &str,
    ) -> Option<(String, String, String, bool)> {
        // 首先尝试查找 ```json 包装的情况
        if let Some(json_start) = context.find("```json") {
            // 查找 JSON 内容开始位置（跳过 ```json 和可能的换行符）
            let mut json_content_start = json_start + 7; // "```json".len()
            if context.chars().nth(json_content_start) == Some('\n') {
                json_content_start += 1;
            }

            // 检查是否有前缀模式
            if Self::looks_like_contextual_running_prefix(&context[..json_start]) {
                // 查找对应的结束标记 ```
                if let Some(json_end) = context[json_content_start..].find("```") {
                    let json_end_pos = json_content_start + json_end;

                    // 提取 JSON 内容
                    let json_content = &context[json_content_start..json_end_pos];

                    // 检查是否包含 "note" 字段
                    if json_content.contains("\"note\"") {
                        // 检查后缀是否包含可疑内容
                        let after_json_end = json_end_pos + 3; // "```".len()
                        let suffix = &context[after_json_end..];

                        // 可疑尾巴模式：包含非英文字符或看起来像垃圾数据
                        let has_suspicious_tail = suffix.chars().any(|c| {
                            !c.is_ascii() || (c.is_ascii_alphabetic() && suffix.len() > 20)
                        });

                        if has_suspicious_tail {
                            // 计算在原始 fragment 中的位置
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
                                LeakDetector::collapse_adjacent_duplicate_markdown_bold(
                                    &prefix_in_fragment,
                                );

                            // 对于带可疑尾巴的泄漏，完全抑制 JSON 后缀（包含噪声）
                            return Some((prefix_in_fragment, String::new(), String::new(), false));
                        }
                    }
                } else {
                    // 没有找到结束标记，可能是跨块分割的情况
                    // 检查剩余内容是否看起来像 JSON 开始
                    let remaining_content = &context[json_content_start..];
                    if remaining_content.contains("\"note\"")
                        || remaining_content.starts_with("{\"note\":")
                    {
                        // 这很可能是跨块分割的 contextual note-json 泄漏
                        // 计算在原始 fragment 中的位置
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

                        // 对于跨块分割的情况，我们抑制从 JSON 开始到片段结束的所有内容
                        return Some((prefix_in_fragment, String::new(), String::new(), true));
                    }
                }
            }
        }

        // 回退到原来的逻辑处理裸 JSON
        let json_start = LeakDetector::find_potential_raw_tool_json_start(fragment)?;
        let prefix = fragment[..json_start].to_string();
        let candidate = &fragment[json_start..];
        let json = Self::extract_first_json_object_fragment(candidate)?;

        if !Self::looks_like_contextual_leaked_note_json(&json, context) {
            return None;
        }

        let suffix_start = json_start + json.len();
        let mut suffix = fragment[suffix_start..].to_string();

        // 对上下文泄漏的情况，清理可疑的尾巴噪声
        suffix = LeakDetector::strip_suspicious_trailing_noise(&suffix);

        Some((prefix, json, suffix, false))
    }

    fn process_pending_tool_text(&mut self, output: &mut Vec<String>, force_flush: bool) {
        if self.pending_tool_text.trim().is_empty() {
            self.pending_tool_text.clear();
            return;
        }

        let pending_raw = std::mem::take(&mut self.pending_tool_text);
        let trimmed_start_len = pending_raw.len() - pending_raw.trim_start().len();
        let pending_for_tool_parse = &pending_raw[trimmed_start_len..];

        if LeakDetector::starts_with_leaked_tool_marker(pending_for_tool_parse) {
            if let Some((_, raw_json, suffix)) =
                LeakDetector::split_tool_json_prefix_suffix(pending_for_tool_parse)
            {
                self.diagnostics.dropped_leaked_marker_fragments += 1;
                let assessment = LeakDetector::assess_raw_tool_json(&raw_json);
                self.record_raw_tool_json_assessment(assessment);
                if let Some(recovered_calls) =
                    self.try_recover_readonly_tool_uses_from_raw_json(output, &raw_json)
                {
                    self.diagnostics.recovered_readonly_leaked_tool_payloads += 1;
                    self.diagnostics.recovered_readonly_leaked_tool_calls += recovered_calls as u64;
                    self.logger.log_raw(&format!(
                        "[Warn] Recovered leaked marker+json readonly tool_uses payload into synthetic tool_use events count={} score={} reason={}",
                        recovered_calls, assessment.score, assessment.reason
                    ));
                } else if assessment.is_high_risk() {
                    self.diagnostics.dropped_high_risk_raw_tool_json_fragments += 1;
                    self.logger.log_raw(&format!(
                        "[Warn] Dropping high-risk leaked tool marker + json fragment from visible text score={} reason={}",
                        assessment.score, assessment.reason
                    ));
                    self.maybe_emit_high_risk_leak_question(output);
                } else {
                    self.logger.log_raw(&format!(
                        "[Warn] Dropping leaked tool marker + json fragment from visible text score={} reason={}",
                        assessment.score, assessment.reason
                    ));
                }
                if !suffix.is_empty() {
                    self.handle_text_fragment(output, &suffix, true);
                }
                return;
            }

            if !force_flush {
                if let Some(newline_idx) = pending_for_tool_parse.find('\n') {
                    self.diagnostics.dropped_leaked_marker_fragments += 1;
                    self.logger
                        .log_raw("[Warn] Dropping leaked tool marker fragment from visible text");
                    let suffix = &pending_for_tool_parse[newline_idx + 1..];
                    if !suffix.is_empty() {
                        let cleaned_suffix = LeakDetector::strip_suspicious_trailing_noise(suffix);
                        if !cleaned_suffix.is_empty() {
                            // Keep post-marker suffix in pending mode instead of directly emitting.
                            // This avoids leaking chunk-split JSON argument fragments as visible text.
                            self.pending_tool_text = cleaned_suffix;
                            self.process_pending_tool_text(output, false);
                        }
                    }
                    return;
                }
                self.pending_tool_text = pending_raw;
                return;
            }

            self.diagnostics.dropped_leaked_marker_fragments += 1;
            self.logger
                .log_raw("[Warn] Dropping leaked tool marker fragment from visible text");
            if let Some(newline_idx) = pending_for_tool_parse.find('\n') {
                let suffix = &pending_for_tool_parse[newline_idx + 1..];
                if !suffix.is_empty() {
                    let cleaned_suffix = LeakDetector::strip_suspicious_trailing_noise(suffix);
                    if !cleaned_suffix.is_empty() {
                        self.pending_tool_text = cleaned_suffix;
                        self.process_pending_tool_text(output, true);
                    }
                }
            }
            return;
        }

        // 检查高置信工具参数泄漏
        if let Some((prefix, raw_json, suffix)) =
            LeakDetector::split_tool_json_prefix_suffix(&pending_raw)
        {
            self.diagnostics.dropped_raw_tool_json_fragments += 1;
            let assessment = LeakDetector::assess_raw_tool_json(&raw_json);
            self.record_raw_tool_json_assessment(assessment);
            if !prefix.is_empty() {
                let cleaned_prefix = LeakDetector::sanitize_prefix_before_raw_tool_json(&prefix);
                if !cleaned_prefix.is_empty() {
                    self.emit_plain_text_fragment(output, &cleaned_prefix);
                }
            }

            if let Some(recovered_calls) =
                self.try_recover_readonly_tool_uses_from_raw_json(output, &raw_json)
            {
                self.diagnostics.recovered_readonly_leaked_tool_payloads += 1;
                self.diagnostics.recovered_readonly_leaked_tool_calls += recovered_calls as u64;
                self.logger.log_raw(&format!(
                    "[Warn] Recovered leaked readonly tool_uses payload into synthetic tool_use events count={} score={} reason={}",
                    recovered_calls, assessment.score, assessment.reason
                ));
            } else {
                if assessment.is_high_risk() {
                    self.diagnostics.dropped_high_risk_raw_tool_json_fragments += 1;
                    self.logger.log_raw(&format!(
                        "[Warn] Dropping high-risk raw leaked tool json fragment score={} reason={}",
                        assessment.score, assessment.reason
                    ));
                    self.maybe_emit_high_risk_leak_question(output);
                } else {
                    self.logger.log_raw(&format!(
                        "[Warn] Dropping raw leaked tool json fragment score={} reason={}",
                        assessment.score, assessment.reason
                    ));
                }
            }

            if !suffix.is_empty() {
                self.handle_text_fragment(output, &suffix, true);
            }
            return;
        }

        // 检查上下文 note-json 泄漏（新增）
        let context = format!("{}{}", self.text_carryover, &pending_raw);
        if let Some((prefix, _, _suffix, is_cross_chunk)) =
            LeakDetector::split_contextual_note_json_prefix_suffix(&pending_raw, &context)
        {
            self.diagnostics.dropped_contextual_note_json_fragments += 1;
            self.logger
                .log_raw("[Warn] Dropping contextual note-json leak from visible text");
            if is_cross_chunk {
                self.suppressing_cross_chunk_leak = true;
            }
            if !prefix.is_empty() {
                self.emit_plain_text_fragment(output, &prefix);
            }
            return;
        }

        if let Some(raw_json_start) = LeakDetector::find_potential_raw_tool_json_start(&pending_raw)
        {
            let candidate = &pending_raw[raw_json_start..];
            let json_complete = Self::extract_first_json_object_fragment(candidate).is_some();

            if !json_complete {
                if !force_flush {
                    self.pending_tool_text = pending_raw;
                    return;
                }

                self.diagnostics.dropped_incomplete_tool_json_fragments += 1;
                self.logger
                    .log_raw("[Warn] Dropping incomplete potential raw tool json fragment");
                let prefix = &pending_raw[..raw_json_start];
                if !prefix.is_empty() {
                    self.emit_plain_text_fragment(output, prefix);
                }
                return;
            }

            // JSON 已完整但不满足高置信工具参数规则，按普通文本放行。
            self.emit_plain_text_fragment(output, &pending_raw);
            return;
        }

        if !pending_raw.trim().is_empty() {
            self.emit_plain_text_fragment(output, &pending_raw);
        }
    }

    pub fn new(model: &str) -> Self {
        Self::new_with_visible_thinking(model, true)
    }

    pub fn new_with_visible_thinking(model: &str, allow_visible_thinking: bool) -> Self {
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            content_index: 0,
            open_text_index: None,
            open_thinking_index: None,
            allow_visible_thinking,
            phase: StreamPhase::AwaitingContent,
            next_tool_order_key: 0,
            next_tool_arrival_seq: 0,
            buffered_tool_calls: Vec::new(),
            tool_order_by_output_index: HashMap::new(),
            tool_order_by_item_id: HashMap::new(),
            tool_order_by_call_id: HashMap::new(),
            canonical_item_id_by_output_index: HashMap::new(),
            canonical_output_index_by_item_id: HashMap::new(),
            closed_tool_call_ids: HashSet::new(),
            last_buffered_tool_order: None,
            item_kind_by_output_index: HashMap::new(),
            item_kind_by_item_id: HashMap::new(),
            item_kind_by_call_id: HashMap::new(),
            active_text_parts: HashSet::new(),
            text_dedupe_by_part: HashMap::new(),
            saw_tool_call: false,
            saw_refusal: false,
            refusal_text_buffer: String::new(),
            sent_message_start: false,
            text_carryover: String::new(),
            pending_tool_text: String::new(),
            deferred_unscoped_text: String::new(),
            pending_tool_argument_updates: Vec::new(),
            capturing_proposed_plan: false,
            proposed_plan_body_buffer: String::new(),
            latest_proposed_plan_body: None,
            codex_plan_file_path: None,
            contains_background_agent_completion: false,
            plan_bridge_emitted: false,
            suppressing_cross_chunk_leak: false,
            suppressing_suggestion_mode_prompt: false,
            in_markdown_bash: false,
            markdown_bash_buffer: String::new(),
            in_commentary_phase: false,
            had_reasoning_in_response: false,
            saw_message_item_added: false,
            next_server_tool_use_seq: 0,
            active_web_search_calls: HashMap::new(),
            web_search_call_by_output_index: HashMap::new(),
            web_search_call_by_item_id: HashMap::new(),
            response_created_announced: false,
            response_in_progress_announced: false,
            launched_background_agent_count: 0,
            terminal_background_agent_completion_count: 0,
            suppress_visible_final_answer_text: false,
            high_risk_leak_question_emitted: false,
            last_terminal_event: None,
            diagnostics: TransformDiagnostics::default(),
            logger: AppLogger::get().unwrap_or_else(|| {
                // 如果全局 logger 未初始化，创建一个临时的
                AppLogger::init(None)
            }),
        }
    }

    fn close_open_text_block(&mut self, output: &mut Vec<String>) {
        if let Some(idx) = self.open_text_index.take() {
            output.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        self.sync_phase_from_runtime();
    }

    fn close_open_thinking_block(&mut self, output: &mut Vec<String>) {
        if let Some(idx) = self.open_thinking_index.take() {
            output.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        self.sync_phase_from_runtime();
    }

    fn transition_to(&mut self, next: StreamPhase) {
        self.phase = next;
    }

    fn sync_phase_from_runtime(&mut self) {
        if self.phase == StreamPhase::Terminal {
            return;
        }
        if !self.buffered_tool_calls.is_empty() {
            self.phase = StreamPhase::BufferingToolCalls;
        } else if self.open_thinking_index.is_some() {
            self.phase = StreamPhase::StreamingThinking;
        } else if self.open_text_index.is_some() {
            self.phase = StreamPhase::StreamingText;
        } else {
            self.phase = StreamPhase::AwaitingContent;
        }
    }

    fn terminal_invariant_violation_reason(&self) -> Option<&'static str> {
        if self.open_text_index.is_some() {
            return Some("open_text_block");
        }
        if self.open_thinking_index.is_some() {
            return Some("open_thinking_block");
        }
        if !self.buffered_tool_calls.is_empty() {
            return Some("buffered_tool_calls_not_flushed");
        }
        if !self.pending_tool_argument_updates.is_empty() {
            return Some("pending_tool_argument_updates_not_flushed");
        }
        if !self.pending_tool_text.trim().is_empty() {
            return Some("pending_tool_text_not_flushed");
        }
        None
    }

    fn emit_terminal_invariant_violation_error(&mut self, output: &mut Vec<String>, reason: &str) {
        self.diagnostics.terminal_invariant_violations += 1;
        self.logger.log_raw(&format!(
            "[Error] Terminal invariant violation detected before stop emission: {}",
            reason
        ));
        let error_data = json!({
            "error": {
                "message": format!(
                    "Response stream ended with inconsistent block state ({reason}) and was aborted for safety."
                ),
                "code": "terminal_invariant_violation"
            }
        });
        self.emit_error_and_stop(
            output,
            &error_data,
            "Response stream ended with inconsistent block state.",
        );
    }

    fn upstream_item_kind_from_type(item_type: &str) -> UpstreamItemKind {
        match item_type {
            "message" => UpstreamItemKind::Message,
            "function_call" => UpstreamItemKind::FunctionCall,
            "reasoning" | "reasoning_summary" => UpstreamItemKind::Reasoning,
            _ => UpstreamItemKind::Unknown,
        }
    }

    fn register_output_item_kind(
        &mut self,
        output_index: Option<u64>,
        item_id: Option<&str>,
        call_id: Option<&str>,
        item_type: &str,
    ) -> UpstreamItemKind {
        let kind = Self::upstream_item_kind_from_type(item_type);

        if let Some(idx) = output_index {
            self.item_kind_by_output_index.insert(idx, kind);
        }
        if let Some(id) = item_id {
            self.item_kind_by_item_id.insert(id.to_string(), kind);
        }
        if let Some(cid) = call_id {
            self.item_kind_by_call_id.insert(cid.to_string(), kind);
        }

        kind
    }

    fn bind_output_item_identity(&mut self, output_index: Option<u64>, item_id: Option<&str>) {
        let (Some(idx), Some(id)) = (output_index, item_id) else {
            return;
        };

        self.canonical_output_index_by_item_id
            .entry(id.to_string())
            .or_insert(idx);

        if let Some(canonical) = self.canonical_item_id_by_output_index.get(&idx) {
            if canonical != id {
                self.diagnostics.normalized_item_id_mismatches += 1;
                self.logger.log_raw(
                    "[Warn] Normalized mismatched item_id by output_index for stream compatibility",
                );
            }
            return;
        }

        self.canonical_item_id_by_output_index
            .insert(idx, id.to_string());
    }

    fn normalized_event_metadata(&mut self, data: &Value) -> EventMetadata {
        let mut metadata = EventMetadata::from_data(data);

        if metadata.output_index.is_none() {
            if let Some(item_id) = metadata.item_id.as_deref() {
                if let Some(idx) = self.canonical_output_index_by_item_id.get(item_id).copied() {
                    metadata.output_index = Some(idx);
                }
            }
        }

        if let Some(idx) = metadata.output_index {
            if let Some(canonical) = self.canonical_item_id_by_output_index.get(&idx).cloned() {
                if metadata.item_id.as_deref() != Some(canonical.as_str()) {
                    if metadata.item_id.is_some() {
                        self.diagnostics.normalized_item_id_mismatches += 1;
                    }
                    metadata.item_id = Some(canonical);
                }
            } else if let Some(item_id) = metadata.item_id.as_deref() {
                self.bind_output_item_identity(Some(idx), Some(item_id));
            }
        }

        metadata
    }

    fn clear_text_state_for_item(&mut self, output_index: Option<u64>, item_id: Option<&str>) {
        self.active_text_parts
            .retain(|key| !key.matches_item(output_index, item_id));
        self.text_dedupe_by_part
            .retain(|key, _| !key.matches_item(output_index, item_id));
    }

    fn clear_output_item_kind(
        &mut self,
        output_index: Option<u64>,
        item_id: Option<&str>,
        call_id: Option<&str>,
    ) {
        let mut item_ids_to_clear: Vec<String> = Vec::new();

        if let Some(idx) = output_index {
            self.item_kind_by_output_index.remove(&idx);
            if let Some(canonical_item_id) = self.canonical_item_id_by_output_index.remove(&idx) {
                item_ids_to_clear.push(canonical_item_id);
            }
        }
        if let Some(cid) = call_id {
            self.item_kind_by_call_id.remove(cid);
        }
        if let Some(id) = item_id {
            item_ids_to_clear.push(id.to_string());
        }

        item_ids_to_clear.sort_unstable();
        item_ids_to_clear.dedup();
        for id in &item_ids_to_clear {
            self.item_kind_by_item_id.remove(id);
            self.canonical_output_index_by_item_id.remove(id);
        }

        if item_ids_to_clear.is_empty() {
            self.clear_text_state_for_item(output_index, None);
        } else {
            for id in &item_ids_to_clear {
                self.clear_text_state_for_item(output_index, Some(id.as_str()));
            }
        }
    }

    fn lookup_item_kind(&self, metadata: &EventMetadata) -> Option<UpstreamItemKind> {
        metadata
            .call_id
            .as_deref()
            .and_then(|cid| self.item_kind_by_call_id.get(cid).copied())
            .or_else(|| {
                metadata
                    .output_index
                    .and_then(|idx| self.item_kind_by_output_index.get(&idx).copied())
            })
            .or_else(|| {
                metadata
                    .item_id
                    .as_deref()
                    .and_then(|id| self.item_kind_by_item_id.get(id).copied())
            })
    }

    fn register_text_part_if_scoped(&mut self, part_key: &EventPartKey) {
        if part_key.is_empty() {
            return;
        }
        self.active_text_parts.insert(part_key.clone());
    }

    fn finish_text_part(&mut self, part_key: &EventPartKey) -> bool {
        if part_key.is_empty() {
            self.text_dedupe_by_part.remove(part_key);
            return true;
        }

        self.text_dedupe_by_part.remove(part_key);
        self.active_text_parts.remove(part_key)
    }

    fn decide_text_routing(&self, metadata: &EventMetadata) -> TextRoutingDecision {
        match self.lookup_item_kind(metadata) {
            Some(UpstreamItemKind::FunctionCall) | Some(UpstreamItemKind::Reasoning) => {
                TextRoutingDecision::Suppress
            }
            Some(UpstreamItemKind::Message) => {
                // 工具缓冲期间，Message 文本延迟发射，避免被 handle_text_fragment
                // 内部的 has_open_tool_block() 守卫静默丢弃
                if self.has_open_tool_block() {
                    TextRoutingDecision::DeferUntilToolWindowCloses
                } else {
                    TextRoutingDecision::Emit
                }
            }
            Some(UpstreamItemKind::Unknown) | None => {
                if self.has_open_tool_block() {
                    if metadata.has_routing_hint() {
                        TextRoutingDecision::Suppress
                    } else {
                        TextRoutingDecision::DeferUntilToolWindowCloses
                    }
                } else {
                    TextRoutingDecision::Emit
                }
            }
        }
    }

    fn has_open_tool_block(&self) -> bool {
        !self.buffered_tool_calls.is_empty()
    }

    fn sort_buffered_tools(&mut self) {
        self.buffered_tool_calls
            .sort_by(|a, b| match (a.output_index, b.output_index) {
                (Some(left), Some(right)) => left
                    .cmp(&right)
                    .then_with(|| a.arrival_seq.cmp(&b.arrival_seq)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.arrival_seq.cmp(&b.arrival_seq),
            });
    }

    fn upsert_output_index_binding(&mut self, output_index: u64, order_key: u64) -> bool {
        if let Some(existing) = self.tool_order_by_output_index.get(&output_index).copied() {
            if existing != order_key {
                self.diagnostics.binding_conflicts_output_index += 1;
                self.logger
                    .log_raw("[Warn] Ignoring conflicting function_call output_index binding");
                return false;
            }
            return true;
        }
        self.tool_order_by_output_index
            .insert(output_index, order_key);
        true
    }

    fn upsert_item_id_binding(&mut self, item_id: &str, order_key: u64) -> bool {
        if let Some(existing) = self.tool_order_by_item_id.get(item_id).copied() {
            if existing != order_key {
                self.diagnostics.binding_conflicts_item_id += 1;
                self.logger
                    .log_raw("[Warn] Ignoring conflicting function_call item_id binding");
                return false;
            }
            return true;
        }
        self.tool_order_by_item_id
            .insert(item_id.to_string(), order_key);
        true
    }

    fn backfill_tool_metadata_bindings(
        &mut self,
        order_key: u64,
        output_index: Option<u64>,
        item_id: Option<&str>,
    ) {
        if let Some(idx) = output_index {
            if self.upsert_output_index_binding(idx, order_key) {
                if let Some(tool) = self.get_buffered_tool_mut(order_key) {
                    if tool.output_index.is_none() {
                        tool.output_index = Some(idx);
                    }
                }
            }
        }

        if let Some(item_id) = item_id {
            if self.upsert_item_id_binding(item_id, order_key) {
                if let Some(tool) = self.get_buffered_tool_mut(order_key) {
                    if tool.item_id.is_none() {
                        tool.item_id = Some(item_id.to_string());
                    }
                }
            }
        }
    }

    fn buffer_tool_call(
        &mut self,
        output_index: Option<u64>,
        item_id: Option<String>,
        call_id: String,
        name: String,
    ) -> Option<u64> {
        if let Some(existing_order) = self.tool_order_by_call_id.get(&call_id).copied() {
            self.diagnostics.duplicate_active_call_items += 1;
            self.logger.log_raw(
                "[Warn] Duplicate function_call item for active call_id; reusing existing buffered call",
            );
            self.backfill_tool_metadata_bindings(existing_order, output_index, item_id.as_deref());
            return Some(existing_order);
        }

        if self.closed_tool_call_ids.contains(call_id.as_str()) {
            self.diagnostics.dropped_reused_closed_call_items += 1;
            self.logger.log_raw(
                "[Warn] Dropping function_call item because call_id was already closed in this response",
            );
            return None;
        }

        let order_key = self.next_tool_order_key;
        self.next_tool_order_key += 1;
        let arrival_seq = self.next_tool_arrival_seq;
        self.next_tool_arrival_seq += 1;

        let mut resolved_output_index = None;
        if let Some(idx) = output_index {
            if self.upsert_output_index_binding(idx, order_key) {
                resolved_output_index = Some(idx);
            }
        }

        let mut resolved_item_id = None;
        if let Some(id) = item_id {
            if self.upsert_item_id_binding(id.as_str(), order_key) {
                resolved_item_id = Some(id);
            }
        }

        self.tool_order_by_call_id
            .insert(call_id.clone(), order_key);
        self.last_buffered_tool_order = Some(order_key);

        self.buffered_tool_calls.push(BufferedToolCall {
            order_key,
            arrival_seq,
            output_index: resolved_output_index,
            item_id: resolved_item_id,
            call_id,
            name,
            arguments_buffer: String::new(),
            consecutive_whitespace_run: 0,
            done_flag: false,
            start_emitted: false,
            content_block_index: None,
            emitted_arguments_len: 0,
            last_progress_message: None,
        });
        self.sort_buffered_tools();
        self.saw_tool_call = true;
        self.transition_to(StreamPhase::BufferingToolCalls);
        Some(order_key)
    }

    fn find_buffered_tool_order_from_metadata(
        &self,
        output_index: Option<u64>,
        item_id: Option<&str>,
        call_id: Option<&str>,
    ) -> Option<u64> {
        let normalized_output_index = output_index.or_else(|| {
            item_id.and_then(|id| self.canonical_output_index_by_item_id.get(id).copied())
        });

        call_id
            .and_then(|id| self.tool_order_by_call_id.get(id).copied())
            .or_else(|| {
                normalized_output_index
                    .and_then(|idx| self.tool_order_by_output_index.get(&idx).copied())
            })
            .or_else(|| item_id.and_then(|id| self.tool_order_by_item_id.get(id).copied()))
            .or_else(|| {
                if self.buffered_tool_calls.len() == 1 {
                    self.buffered_tool_calls.first().map(|tool| tool.order_key)
                } else {
                    None
                }
            })
    }

    fn find_buffered_tool_order(&mut self, data: &Value) -> Option<u64> {
        let metadata = self.normalized_event_metadata(data);
        self.find_buffered_tool_order_from_metadata(
            metadata.output_index,
            metadata.item_id.as_deref(),
            metadata.call_id.as_deref(),
        )
    }

    fn queue_pending_tool_argument_update(
        &mut self,
        data: &Value,
        kind: PendingToolArgumentUpdateKind,
    ) {
        let metadata = self.normalized_event_metadata(data);
        if !metadata.has_routing_hint() {
            self.diagnostics
                .dropped_orphan_tool_argument_updates_no_hint += 1;
            self.logger
                .log_raw("[Warn] Dropping orphan tool arguments event without routing hints");
            return;
        }

        if let Some(call_id) = metadata.call_id.as_deref() {
            if self.closed_tool_call_ids.contains(call_id) {
                self.diagnostics
                    .dropped_orphan_tool_argument_updates_closed_call += 1;
                self.logger.log_raw(
                    "[Warn] Dropping orphan tool arguments event for already-closed call_id",
                );
                return;
            }
        }

        if self.pending_tool_argument_updates.len() >= 64 {
            self.pending_tool_argument_updates.remove(0);
            self.diagnostics.pending_tool_backlog_trimmed += 1;
            self.logger
                .log_raw("[Warn] Trimming pending tool-argument update backlog");
        }

        self.pending_tool_argument_updates
            .push(PendingToolArgumentUpdate {
                output_index: metadata.output_index,
                item_id: metadata.item_id,
                call_id: metadata.call_id,
                kind,
            });
        self.diagnostics.queued_orphan_tool_argument_updates += 1;
        self.logger
            .log_raw("[Info] Queued orphan tool arguments event waiting for function_call item");
    }

    fn apply_pending_tool_argument_updates(&mut self) -> bool {
        if self.pending_tool_argument_updates.is_empty() {
            return false;
        }

        let mut remaining = Vec::with_capacity(self.pending_tool_argument_updates.len());
        for pending in std::mem::take(&mut self.pending_tool_argument_updates) {
            if let Some(call_id) = pending.call_id.as_deref() {
                if self.closed_tool_call_ids.contains(call_id) {
                    self.diagnostics
                        .dropped_pending_tool_argument_updates_closed_call += 1;
                    self.logger.log_raw(
                        "[Warn] Dropping pending tool-argument update for already-closed call_id",
                    );
                    continue;
                }
            }
            let order_key = self.find_buffered_tool_order_from_metadata(
                pending.output_index,
                pending.item_id.as_deref(),
                pending.call_id.as_deref(),
            );
            if let Some(order_key) = order_key {
                match pending.kind {
                    PendingToolArgumentUpdateKind::Delta(delta) => {
                        if self.append_tool_arguments_delta(order_key, &delta) {
                            self.pending_tool_argument_updates.clear();
                            return true;
                        }
                        self.diagnostics.applied_orphan_tool_argument_updates += 1;
                    }
                    PendingToolArgumentUpdateKind::Snapshot(arguments) => {
                        if self.apply_tool_arguments_snapshot(order_key, &arguments) {
                            self.pending_tool_argument_updates.clear();
                            return true;
                        }
                        self.diagnostics.applied_orphan_tool_argument_updates += 1;
                    }
                }
            } else {
                remaining.push(pending);
            }
        }
        self.pending_tool_argument_updates = remaining;
        false
    }

    fn get_buffered_tool_mut(&mut self, order_key: u64) -> Option<&mut BufferedToolCall> {
        self.buffered_tool_calls
            .iter_mut()
            .find(|tool| tool.order_key == order_key)
    }

    fn is_function_args_whitespace(ch: char) -> bool {
        matches!(ch, ' ' | '\n' | '\r' | '\t')
    }

    fn advance_whitespace_run(mut current_run: usize, text: &str) -> (usize, bool) {
        for ch in text.chars() {
            if Self::is_function_args_whitespace(ch) {
                current_run += 1;
                if current_run > Self::MAX_FUNCTION_ARGS_WHITESPACE_RUN {
                    return (current_run, true);
                }
            } else {
                current_run = 0;
            }
        }
        (current_run, false)
    }

    fn trailing_whitespace_run(text: &str) -> usize {
        text.chars()
            .rev()
            .take_while(|ch| Self::is_function_args_whitespace(*ch))
            .count()
    }

    fn record_function_args_whitespace_overflow(&mut self) {
        self.diagnostics
            .dropped_function_args_whitespace_overflow_fragments += 1;
        self.logger.log_raw(
            "[Warn] Dropping function_call_arguments fragment due to whitespace flood overflow",
        );
    }

    fn append_tool_arguments_delta(&mut self, order_key: u64, delta: &str) -> bool {
        if delta.is_empty() {
            return false;
        }
        if let Some(tool) = self.get_buffered_tool_mut(order_key) {
            let (next_run, overflow) =
                Self::advance_whitespace_run(tool.consecutive_whitespace_run, delta);
            if overflow {
                self.record_function_args_whitespace_overflow();
                return true;
            }
            tool.arguments_buffer.push_str(delta);
            tool.consecutive_whitespace_run = next_run;
        }
        false
    }

    fn apply_tool_arguments_snapshot(&mut self, order_key: u64, full_arguments: &str) -> bool {
        if full_arguments.is_empty() {
            return false;
        }
        if let Some(tool) = self.get_buffered_tool_mut(order_key) {
            if full_arguments.starts_with(tool.arguments_buffer.as_str()) {
                let suffix = &full_arguments[tool.arguments_buffer.len()..];
                if !suffix.is_empty() {
                    let (next_run, overflow) =
                        Self::advance_whitespace_run(tool.consecutive_whitespace_run, suffix);
                    if overflow {
                        self.record_function_args_whitespace_overflow();
                        return true;
                    }
                    tool.arguments_buffer.push_str(suffix);
                    tool.consecutive_whitespace_run = next_run;
                }
                return false;
            }

            if tool.arguments_buffer.starts_with(full_arguments) {
                return false;
            }

            let trailing_run = Self::trailing_whitespace_run(full_arguments);
            if trailing_run > Self::MAX_FUNCTION_ARGS_WHITESPACE_RUN {
                self.record_function_args_whitespace_overflow();
                return true;
            }

            tool.arguments_buffer = full_arguments.to_string();
            tool.consecutive_whitespace_run = trailing_run;
        }
        false
    }

    fn mark_buffered_tool_done(&mut self, order_key: u64) {
        if let Some(tool) = self.get_buffered_tool_mut(order_key) {
            tool.done_flag = true;
        }
    }

    fn cleanup_tool_mappings(&mut self, tool: &BufferedToolCall) {
        if let Some(idx) = tool.output_index {
            if self.tool_order_by_output_index.get(&idx).copied() == Some(tool.order_key) {
                self.tool_order_by_output_index.remove(&idx);
            }
        }
        if let Some(ref item_id) = tool.item_id {
            if self.tool_order_by_item_id.get(item_id).copied() == Some(tool.order_key) {
                self.tool_order_by_item_id.remove(item_id);
            }
        }
        if self.tool_order_by_call_id.get(&tool.call_id).copied() == Some(tool.order_key) {
            self.tool_order_by_call_id.remove(&tool.call_id);
        }
        self.closed_tool_call_ids.insert(tool.call_id.clone());
        if self.last_buffered_tool_order == Some(tool.order_key) {
            self.last_buffered_tool_order =
                self.buffered_tool_calls.iter().map(|t| t.order_key).max();
        }
    }

    fn normalize_skill_tool_arguments(arguments: &str) -> Option<String> {
        let mut parsed = serde_json::from_str::<Value>(arguments).ok()?;
        let obj = parsed.as_object_mut()?;

        let has_skill = obj
            .get("skill")
            .and_then(|value| value.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if has_skill {
            return None;
        }

        let command = obj
            .get("command")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())?;

        let mut parts = command.splitn(2, char::is_whitespace);
        let skill = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())?;
        let args = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        obj.remove("command");
        obj.insert("skill".to_string(), Value::String(skill));
        match args {
            Some(value) => {
                obj.insert("args".to_string(), Value::String(value));
            }
            None => {
                obj.remove("args");
            }
        }

        serde_json::to_string(&parsed).ok()
    }

    fn parse_tool_arguments_object(arguments: &str) -> Option<Map<String, Value>> {
        serde_json::from_str::<Value>(arguments)
            .ok()?
            .as_object()
            .cloned()
    }

    fn first_string_field(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
        keys.iter()
            .filter_map(|key| obj.get(*key))
            .find_map(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }

    fn first_bool_field(obj: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
        keys.iter()
            .filter_map(|key| obj.get(*key))
            .find_map(|value| value.as_bool())
    }

    fn first_number_field(obj: &Map<String, Value>, keys: &[&str]) -> Option<Value> {
        keys.iter()
            .filter_map(|key| obj.get(*key))
            .find(|value| value.is_number())
            .cloned()
    }

    fn shell_escape_single_quotes(value: &str) -> String {
        value.replace('\'', "'\"'\"'")
    }

    fn normalize_bash_tool_arguments(arguments: &str) -> Option<String> {
        let obj = Self::parse_tool_arguments_object(arguments)?;
        let mut normalized = Map::new();
        let mut command = Self::first_string_field(&obj, &["command", "cmd"])?;
        if let Some(workdir) = Self::first_string_field(&obj, &["workdir", "cwd"]) {
            command = format!(
                "cd '{}' && {}",
                Self::shell_escape_single_quotes(&workdir),
                command
            );
        }
        normalized.insert("command".to_string(), Value::String(command));
        if let Some(description) = Self::first_string_field(&obj, &["description", "justification"])
        {
            normalized.insert("description".to_string(), Value::String(description));
        }
        if let Some(timeout) = Self::first_number_field(&obj, &["timeout", "timeout_ms"]) {
            normalized.insert("timeout".to_string(), timeout);
        }
        if let Some(run_in_background) =
            Self::first_bool_field(&obj, &["run_in_background", "background"])
        {
            normalized.insert(
                "run_in_background".to_string(),
                Value::Bool(run_in_background),
            );
        }
        if let Some(disable_sandbox) = Self::first_bool_field(
            &obj,
            &["dangerouslyDisableSandbox", "dangerously_disable_sandbox"],
        ) {
            normalized.insert(
                "dangerouslyDisableSandbox".to_string(),
                Value::Bool(disable_sandbox),
            );
        }
        serde_json::to_string(&Value::Object(normalized)).ok()
    }

    fn normalize_read_tool_arguments(arguments: &str) -> Option<String> {
        let obj = Self::parse_tool_arguments_object(arguments)?;
        let mut normalized = Map::new();
        normalized.insert(
            "file_path".to_string(),
            Value::String(Self::first_string_field(
                &obj,
                &["file_path", "filePath", "path"],
            )?),
        );
        if let Some(offset) = Self::first_number_field(&obj, &["offset"]) {
            normalized.insert("offset".to_string(), offset);
        }
        if let Some(limit) = Self::first_number_field(&obj, &["limit"]) {
            normalized.insert("limit".to_string(), limit);
        }
        if let Some(pages) = Self::first_string_field(&obj, &["pages"]) {
            normalized.insert("pages".to_string(), Value::String(pages));
        }
        serde_json::to_string(&Value::Object(normalized)).ok()
    }

    fn normalize_edit_tool_arguments(arguments: &str) -> Option<String> {
        let obj = Self::parse_tool_arguments_object(arguments)?;
        let mut normalized = Map::new();
        normalized.insert(
            "file_path".to_string(),
            Value::String(Self::first_string_field(
                &obj,
                &["file_path", "filePath", "path"],
            )?),
        );
        normalized.insert(
            "old_string".to_string(),
            Value::String(Self::first_string_field(
                &obj,
                &["old_string", "oldText", "old_text"],
            )?),
        );
        normalized.insert(
            "new_string".to_string(),
            Value::String(Self::first_string_field(
                &obj,
                &["new_string", "newText", "new_text"],
            )?),
        );
        if let Some(replace_all) = Self::first_bool_field(&obj, &["replace_all", "replaceAll"]) {
            normalized.insert("replace_all".to_string(), Value::Bool(replace_all));
        }
        serde_json::to_string(&Value::Object(normalized)).ok()
    }

    fn normalize_write_tool_arguments(arguments: &str) -> Option<String> {
        let obj = Self::parse_tool_arguments_object(arguments)?;
        let mut normalized = Map::new();
        normalized.insert(
            "file_path".to_string(),
            Value::String(Self::first_string_field(
                &obj,
                &["file_path", "filePath", "path"],
            )?),
        );
        normalized.insert(
            "content".to_string(),
            Value::String(Self::first_string_field(
                &obj,
                &["content", "text", "new_string"],
            )?),
        );
        serde_json::to_string(&Value::Object(normalized)).ok()
    }

    fn normalize_task_output_tool_arguments(arguments: &str) -> Option<String> {
        let obj = Self::parse_tool_arguments_object(arguments)?;
        let mut normalized = Map::new();
        normalized.insert(
            "task_id".to_string(),
            Value::String(Self::first_string_field(
                &obj,
                &["task_id", "taskId", "id", "shell_id"],
            )?),
        );
        normalized.insert(
            "block".to_string(),
            Value::Bool(Self::first_bool_field(&obj, &["block"]).unwrap_or(true)),
        );
        normalized.insert(
            "timeout".to_string(),
            Self::first_number_field(&obj, &["timeout", "timeout_ms"])
                .unwrap_or_else(|| json!(30000)),
        );
        serde_json::to_string(&Value::Object(normalized)).ok()
    }

    fn normalize_task_stop_tool_arguments(arguments: &str) -> Option<String> {
        let obj = Self::parse_tool_arguments_object(arguments)?;
        let mut normalized = Map::new();
        if let Some(task_id) =
            Self::first_string_field(&obj, &["task_id", "taskId", "id", "shell_id"])
        {
            normalized.insert("task_id".to_string(), Value::String(task_id));
        }
        if let Some(shell_id) = Self::first_string_field(&obj, &["shell_id"]) {
            normalized.insert("shell_id".to_string(), Value::String(shell_id));
        }
        serde_json::to_string(&Value::Object(normalized)).ok()
    }

    fn normalize_tool_arguments_by_name(tool_name: &str, arguments: &str) -> Option<String> {
        if tool_name.eq_ignore_ascii_case("skill") {
            return Self::normalize_skill_tool_arguments(arguments);
        }
        if tool_name.eq_ignore_ascii_case("bash") {
            return Self::normalize_bash_tool_arguments(arguments);
        }
        if tool_name.eq_ignore_ascii_case("read") {
            return Self::normalize_read_tool_arguments(arguments);
        }
        if tool_name.eq_ignore_ascii_case("edit") {
            return Self::normalize_edit_tool_arguments(arguments);
        }
        if tool_name.eq_ignore_ascii_case("write") {
            return Self::normalize_write_tool_arguments(arguments);
        }
        if tool_name.eq_ignore_ascii_case("taskoutput") {
            return Self::normalize_task_output_tool_arguments(arguments);
        }
        if tool_name.eq_ignore_ascii_case("taskstop") {
            return Self::normalize_task_stop_tool_arguments(arguments);
        }
        None
    }

    fn normalized_tool_arguments(&mut self, tool: &BufferedToolCall) -> String {
        if let Some(normalized) =
            Self::normalize_tool_arguments_by_name(tool.name.as_str(), &tool.arguments_buffer)
        {
            if normalized != tool.arguments_buffer {
                self.logger.log_raw(&format!(
                    "[Info] Normalized {} tool arguments for Claude-compatible schema",
                    tool.name
                ));
                return normalized;
            }
        }

        tool.arguments_buffer.clone()
    }

    fn tool_launches_background_agent(tool_name: &str, arguments: &str) -> bool {
        if !tool_name.eq_ignore_ascii_case("Agent") {
            return false;
        }

        serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|parsed| parsed.as_object().cloned())
            .and_then(|input| input.get("run_in_background").and_then(|value| value.as_bool()))
            .unwrap_or(false)
    }

    fn build_background_task_progress_message(tool_name: &str, arguments: &str) -> Option<String> {
        let parsed = serde_json::from_str::<Value>(arguments).ok()?;
        let input = parsed.as_object()?;

        if tool_name.eq_ignore_ascii_case("Agent") {
            if !input
                .get("run_in_background")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return None;
            }

            let description = input
                .get("description")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());

            return Some(match description {
                Some(value) => format!("已启动后台 explorer：{value}…"),
                None => "已启动后台 explorer，正在处理中…".to_string(),
            });
        }

        if tool_name.eq_ignore_ascii_case("TaskOutput") {
            let is_non_blocking = !input
                .get("block")
                .and_then(|value| value.as_bool())
                .unwrap_or(true);

            return Some(if is_non_blocking {
                "正在轮询后台任务结果…".to_string()
            } else {
                "正在等待后台任务返回结果…".to_string()
            });
        }

        None
    }

    fn emit_background_task_progress(
        &mut self,
        output: &mut Vec<String>,
        tool_name: &str,
        arguments: &str,
    ) {
        let Some(message) = Self::build_background_task_progress_message(tool_name, arguments)
        else {
            return;
        };

        self.open_thinking_block_if_needed(output);
        self.emit_thinking_delta(output, message.as_str());
        self.emit_thinking_delta(output, "\n");
    }

    fn emit_response_lifecycle_progress(&mut self, output: &mut Vec<String>, message: &str) {
        self.open_thinking_block_if_needed(output);
        self.emit_thinking_delta(output, message);
        self.emit_thinking_delta(output, "\n");
    }

    fn emit_ephemeral_thinking_progress(&mut self, output: &mut Vec<String>, message: &str) {
        if !self.allow_visible_thinking || message.is_empty() {
            return;
        }

        self.close_open_text_block(output);
        self.close_open_thinking_block(output);

        let idx = self.content_index;
        self.content_index += 1;

        output.push(format!(
            "event: content_block_start
data: {}

",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "thinking", "thinking": "" }
            })
        ));
        output.push(format!(
            "event: content_block_delta
data: {}

",
            json!({
                "type": "content_block_delta",
                "index": idx,
                "delta": { "type": "thinking_delta", "thinking": message }
            })
        ));
        output.push(format!(
            "event: content_block_delta
data: {}

",
            json!({
                "type": "content_block_delta",
                "index": idx,
                "delta": { "type": "thinking_delta", "thinking": "
            " }
            })
        ));
        output.push(format!(
            "event: content_block_stop
data: {}

",
            json!({ "type": "content_block_stop", "index": idx })
        ));
    }

    fn build_generic_background_task_progress_message(tool_name: &str) -> Option<&'static str> {
        if tool_name.eq_ignore_ascii_case("Agent") {
            Some("正在启动后台 explorer…")
        } else if tool_name.eq_ignore_ascii_case("TaskOutput") {
            Some("正在检查后台任务输出…")
        } else {
            None
        }
    }

    fn maybe_emit_front_buffered_tool_progress(&mut self, output: &mut Vec<String>) {
        let message = {
            let Some(tool) = self.buffered_tool_calls.first_mut() else {
                return;
            };
            if tool.start_emitted {
                return;
            }

            let next_message = Self::build_background_task_progress_message(
                tool.name.as_str(),
                tool.arguments_buffer.as_str(),
            )
            .or_else(|| {
                Self::build_generic_background_task_progress_message(tool.name.as_str())
                    .map(|value| value.to_string())
            });

            match next_message {
                Some(message)
                    if tool.last_progress_message.as_deref() != Some(message.as_str()) =>
                {
                    tool.last_progress_message = Some(message.clone());
                    Some(message)
                }
                _ => None,
            }
        };

        if let Some(message) = message {
            self.emit_ephemeral_thinking_progress(output, message.as_str());
        }
    }

    fn tool_supports_live_argument_stream(name: &str) -> bool {
        !name.eq_ignore_ascii_case("Skill")
    }

    fn contains_any_json_key(arguments: &str, keys: &[&str]) -> bool {
        keys.iter()
            .map(|key| format!("\"{key}\""))
            .any(|needle| arguments.contains(needle.as_str()))
    }

    fn tool_arguments_are_safe_for_live_stream(name: &str, arguments: &str) -> bool {
        if arguments.is_empty() || !Self::tool_supports_live_argument_stream(name) {
            return false;
        }

        if name.eq_ignore_ascii_case("bash") {
            return !Self::contains_any_json_key(
                arguments,
                &[
                    "cmd",
                    "workdir",
                    "cwd",
                    "justification",
                    "background",
                    "dangerously_disable_sandbox",
                ],
            );
        }

        if name.eq_ignore_ascii_case("read") {
            return !Self::contains_any_json_key(arguments, &["filePath", "path"]);
        }

        if name.eq_ignore_ascii_case("edit") {
            return !Self::contains_any_json_key(
                arguments,
                &[
                    "filePath",
                    "path",
                    "oldText",
                    "old_text",
                    "newText",
                    "new_text",
                    "replaceAll",
                ],
            );
        }

        if name.eq_ignore_ascii_case("write") {
            return !Self::contains_any_json_key(arguments, &["filePath", "path", "text"]);
        }

        if name.eq_ignore_ascii_case("taskoutput") || name.eq_ignore_ascii_case("taskstop") {
            return matches!(
                Self::normalize_tool_arguments_by_name(name, arguments),
                Some(normalized) if normalized == arguments
            );
        }

        true
    }

    fn maybe_emit_front_buffered_tool_start(&mut self, output: &mut Vec<String>) {
        let should_emit = self
            .buffered_tool_calls
            .first()
            .map(|tool| !tool.start_emitted)
            .unwrap_or(false);
        if !should_emit {
            return;
        }

        self.maybe_emit_front_buffered_tool_progress(output);
        self.close_open_text_block(output);
        self.close_open_thinking_block(output);

        let idx = self.content_index;
        self.content_index += 1;

        let (call_id, name, initial_delta) = {
            let tool = self
                .buffered_tool_calls
                .first_mut()
                .expect("front buffered tool exists");
            tool.start_emitted = true;
            tool.content_block_index = Some(idx);

            let initial_delta = if Self::tool_arguments_are_safe_for_live_stream(
                tool.name.as_str(),
                tool.arguments_buffer.as_str(),
            ) {
                tool.emitted_arguments_len = tool.arguments_buffer.len();
                tool.arguments_buffer.clone()
            } else {
                tool.emitted_arguments_len = 0;
                String::new()
            };

            (tool.call_id.clone(), tool.name.clone(), initial_delta)
        };

        output.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": {
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": {}
                }
            })
        ));

        if !initial_delta.is_empty() {
            self.emit_tool_json_delta(output, idx, initial_delta);
        }
    }

    fn maybe_emit_front_buffered_tool_argument_delta(&mut self, output: &mut Vec<String>) {
        let pending = {
            let Some(tool) = self.buffered_tool_calls.first_mut() else {
                return;
            };
            if !tool.start_emitted
                || !Self::tool_arguments_are_safe_for_live_stream(
                    tool.name.as_str(),
                    tool.arguments_buffer.as_str(),
                )
            {
                return;
            }
            let Some(idx) = tool.content_block_index else {
                return;
            };
            if tool.arguments_buffer.len() <= tool.emitted_arguments_len {
                return;
            }

            let delta = tool.arguments_buffer[tool.emitted_arguments_len..].to_string();
            tool.emitted_arguments_len = tool.arguments_buffer.len();
            Some((idx, delta))
        };

        if let Some((idx, delta)) = pending {
            self.emit_tool_json_delta(output, idx, delta);
        }
    }

    fn emit_started_tool_call_completion(
        &mut self,
        output: &mut Vec<String>,
        tool: &BufferedToolCall,
    ) {
        let Some(idx) = tool.content_block_index else {
            self.emit_serialized_tool_call(output, tool);
            return;
        };

        let arguments = self.normalized_tool_arguments(tool);
        let suffix = if Self::tool_supports_live_argument_stream(tool.name.as_str()) {
            arguments
                .get(tool.emitted_arguments_len..)
                .unwrap_or("")
                .to_string()
        } else {
            arguments.clone()
        };

        if !suffix.is_empty() {
            self.emit_tool_json_delta(output, idx, suffix);
        }

        output.push(format!(
            "event: content_block_stop\ndata: {}\n\n",
            json!({ "type": "content_block_stop", "index": idx })
        ));

        if Self::tool_launches_background_agent(tool.name.as_str(), arguments.as_str()) {
            self.launched_background_agent_count += 1;
            if self.launched_background_agent_count >= 2
                && !self.contains_background_agent_completion
            {
                self.suppress_visible_final_answer_text = true;
            }
        }

        if Self::build_background_task_progress_message(tool.name.as_str(), arguments.as_str())
            .as_deref()
            != tool.last_progress_message.as_deref()
        {
            self.emit_background_task_progress(output, tool.name.as_str(), arguments.as_str());
        }
    }

    fn emit_serialized_tool_call(&mut self, output: &mut Vec<String>, tool: &BufferedToolCall) {
        self.close_open_text_block(output);
        self.close_open_thinking_block(output);

        let idx = self.content_index;
        self.content_index += 1;

        output.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": {
                    "type": "tool_use",
                    "id": tool.call_id.as_str(),
                    "name": tool.name.as_str(),
                    "input": {}
                }
            })
        ));

        let arguments = self.normalized_tool_arguments(tool);
        if !arguments.is_empty() {
            self.emit_tool_json_delta(output, idx, arguments.clone());
        }

        output.push(format!(
            "event: content_block_stop\ndata: {}\n\n",
            json!({ "type": "content_block_stop", "index": idx })
        ));

        self.emit_background_task_progress(output, tool.name.as_str(), arguments.as_str());
    }

    fn flush_serialized_tool_calls(
        &mut self,
        output: &mut Vec<String>,
        include_incomplete: bool,
    ) -> bool {
        if self.apply_pending_tool_argument_updates() {
            return true;
        }
        loop {
            let should_flush_front = self
                .buffered_tool_calls
                .first()
                .map(|tool| tool.done_flag || include_incomplete)
                .unwrap_or(false);
            if !should_flush_front {
                break;
            }

            let tool = self.buffered_tool_calls.remove(0);
            if tool.start_emitted {
                self.emit_started_tool_call_completion(output, &tool);
            } else {
                self.emit_serialized_tool_call(output, &tool);
            }
            self.cleanup_tool_mappings(&tool);
        }
        if !include_incomplete {
            self.maybe_emit_front_buffered_tool_start(output);
            self.maybe_emit_front_buffered_tool_progress(output);
        }
        self.sync_phase_from_runtime();
        if !self.has_open_tool_block() {
            self.flush_deferred_unscoped_text(output, false);
        }
        false
    }

    fn buffer_deferred_unscoped_text(&mut self, fragment: &str) {
        if fragment.is_empty() {
            return;
        }
        self.diagnostics.deferred_unscoped_text_chunks += 1;
        self.deferred_unscoped_text.push_str(fragment);
    }

    fn flush_deferred_unscoped_text(&mut self, output: &mut Vec<String>, force: bool) {
        if self.deferred_unscoped_text.is_empty() {
            return;
        }
        if self.has_open_tool_block() && !force {
            return;
        }

        let deferred = std::mem::take(&mut self.deferred_unscoped_text);
        self.diagnostics.deferred_unscoped_text_flushes += 1;
        self.logger
            .log_raw("[Info] Flushing deferred unscoped text after tool window");
        self.handle_text_fragment(output, &deferred, true);
    }

    fn open_thinking_block_if_needed(&mut self, output: &mut Vec<String>) {
        if !self.allow_visible_thinking {
            return;
        }

        if self.has_open_tool_block() {
            return;
        }

        if self.open_thinking_index.is_some() {
            return;
        }

        self.close_open_text_block(output);

        let idx = self.content_index;
        self.content_index += 1;
        self.open_thinking_index = Some(idx);
        self.transition_to(StreamPhase::StreamingThinking);
        output.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "thinking", "thinking": "" }
            })
        ));
    }

    fn emit_thinking_delta(&self, output: &mut Vec<String>, delta: &str) {
        if !self.allow_visible_thinking || delta.is_empty() {
            return;
        }

        if let Some(idx) = self.open_thinking_index {
            output.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": idx,
                    "delta": { "type": "thinking_delta", "thinking": delta }
                })
            ));
        }
    }

    fn lookup_active_web_search_call_id(
        &self,
        output_index: Option<u64>,
        item_id: Option<&str>,
    ) -> Option<String> {
        output_index
            .and_then(|idx| self.web_search_call_by_output_index.get(&idx).cloned())
            .or_else(|| item_id.and_then(|id| self.web_search_call_by_item_id.get(id).cloned()))
    }

    fn clear_active_web_search_call(&mut self, server_tool_use_id: &str) {
        if let Some(call) = self.active_web_search_calls.remove(server_tool_use_id) {
            if let Some(output_index) = call.output_index {
                self.web_search_call_by_output_index.remove(&output_index);
            }
            if let Some(item_id) = call.item_id {
                self.web_search_call_by_item_id.remove(&item_id);
            }
        }
    }

    fn register_active_web_search_call(
        &mut self,
        output: &mut Vec<String>,
        output_index: Option<u64>,
        item_id: Option<&str>,
    ) -> String {
        if let Some(existing) = self.lookup_active_web_search_call_id(output_index, item_id) {
            return existing;
        }

        self.close_open_text_block(output);
        self.close_open_thinking_block(output);

        let idx = self.content_index;
        self.content_index += 1;
        self.next_server_tool_use_seq += 1;
        let server_tool_use_id = format!(
            "srvtoolu_{}_{}",
            chrono::Utc::now().timestamp_millis(),
            self.next_server_tool_use_seq
        );

        output.push(format!(
            "event: content_block_start
data: {}

",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": {
                    "type": "server_tool_use",
                    "id": server_tool_use_id,
                    "name": "web_search",
                    "input": {},
                    "caller": { "type": "direct" }
                }
            })
        ));
        self.emit_tool_json_delta(output, idx, String::new());

        let call = ActiveWebSearchCall {
            output_index,
            item_id: item_id.map(|value| value.to_string()),
            content_block_index: idx,
            input_closed: false,
        };

        if let Some(output_index) = output_index {
            self.web_search_call_by_output_index
                .insert(output_index, server_tool_use_id.clone());
        }
        if let Some(item_id) = item_id {
            self.web_search_call_by_item_id
                .insert(item_id.to_string(), server_tool_use_id.clone());
        }
        self.active_web_search_calls
            .insert(server_tool_use_id.clone(), call);

        server_tool_use_id
    }

    fn build_web_search_result_entries(action: &Value) -> Vec<Value> {
        action
            .get("sources")
            .and_then(|value| value.as_array())
            .map(|sources| {
                sources
                    .iter()
                    .filter_map(|source| {
                        let url = source.get("url").and_then(|value| value.as_str())?;
                        let title = source
                            .get("title")
                            .and_then(|value| value.as_str())
                            .unwrap_or(url);
                        let page_age = source
                            .get("page_age")
                            .or_else(|| source.get("pageAge"))
                            .and_then(|value| value.as_str());
                        let encrypted_content = source
                            .get("encrypted_content")
                            .or_else(|| source.get("encryptedContent"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("");

                        Some(json!({
                            "type": "web_search_result",
                            "title": title,
                            "url": url,
                            "encrypted_content": encrypted_content,
                            "page_age": page_age
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn emit_web_search_tool_completion(
        &mut self,
        output: &mut Vec<String>,
        action: Option<&Value>,
        output_index: Option<u64>,
        item_id: Option<&str>,
    ) {
        let server_tool_use_id =
            self.register_active_web_search_call(output, output_index, item_id);

        let (content_block_index, already_closed) =
            match self.active_web_search_calls.get(&server_tool_use_id) {
                Some(call) => (call.content_block_index, call.input_closed),
                None => return,
            };

        if !already_closed {
            let query = action
                .and_then(|value| value.get("query"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if !query.is_empty() {
                self.emit_tool_json_delta(
                    output,
                    content_block_index,
                    json!({ "query": query }).to_string(),
                );
            }
            output.push(format!(
                "event: content_block_stop
data: {}

",
                json!({ "type": "content_block_stop", "index": content_block_index })
            ));
            if let Some(call) = self.active_web_search_calls.get_mut(&server_tool_use_id) {
                call.input_closed = true;
            }
        }

        let results = action
            .map(Self::build_web_search_result_entries)
            .unwrap_or_default();
        if !results.is_empty() {
            let idx = self.content_index;
            self.content_index += 1;
            output.push(format!(
                "event: content_block_start
data: {}

",
                json!({
                    "type": "content_block_start",
                    "index": idx,
                    "content_block": {
                        "type": "web_search_tool_result",
                        "tool_use_id": server_tool_use_id,
                        "content": results
                    }
                })
            ));
            output.push(format!(
                "event: content_block_stop
data: {}

",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }

        self.clear_active_web_search_call(&server_tool_use_id);
    }

    fn close_open_web_search_calls(&mut self, output: &mut Vec<String>) {
        let active_ids: Vec<String> = self.active_web_search_calls.keys().cloned().collect();
        for server_tool_use_id in active_ids {
            let Some(call) = self
                .active_web_search_calls
                .get(&server_tool_use_id)
                .cloned()
            else {
                continue;
            };
            if !call.input_closed {
                output.push(format!(
                    "event: content_block_stop
data: {}

",
                    json!({ "type": "content_block_stop", "index": call.content_block_index })
                ));
            }
            self.clear_active_web_search_call(&server_tool_use_id);
        }
    }

    fn extract_content_part_text<'a>(data: &'a Value) -> Option<&'a str> {
        data.get("part")
            .and_then(|part| {
                let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if part_type == "output_text" || part_type == "text" || part_type.is_empty() {
                    part.get("text")
                        .and_then(|v| v.as_str())
                        .or_else(|| part.get("delta").and_then(|v| v.as_str()))
                } else {
                    None
                }
            })
            .or_else(|| data.get("delta").and_then(|v| v.as_str()))
    }

    fn is_text_content_part(data: &Value) -> bool {
        let Some(part) = data.get("part") else {
            return true;
        };
        let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
        part_type == "output_text" || part_type == "text" || part_type.is_empty()
    }

    fn dedupe_cross_source_fragment(
        &mut self,
        part_key: &EventPartKey,
        source: TextEventSource,
        fragment: &str,
    ) -> Option<String> {
        if fragment.is_empty() {
            return None;
        }

        let state = self
            .text_dedupe_by_part
            .entry(part_key.clone())
            .or_insert_with(|| (source, String::new()));

        let mut deduped = fragment;
        let last_source = state.0;
        let last_fragment = state.1.as_str();
        if !last_fragment.is_empty() {
            if last_source != source {
                if fragment == last_fragment {
                    return None;
                }
                if fragment.starts_with(last_fragment) {
                    deduped = &fragment[last_fragment.len()..];
                }
            }
        }

        if deduped.is_empty() {
            return None;
        }

        state.0 = source;
        state.1.clear();
        state.1.push_str(deduped);
        Some(deduped.to_string())
    }

    fn reset_text_dedupe_state(&mut self) {
        self.text_dedupe_by_part.clear();
        self.active_text_parts.clear();
    }

    fn extract_xml_tag_body<'a>(fragment: &'a str, tag: &str) -> Option<&'a str> {
        let start_marker = format!("<{tag}>");
        let end_marker = format!("</{tag}>");
        let start = fragment.find(start_marker.as_str())? + start_marker.len();
        let end = fragment[start..].find(end_marker.as_str())? + start;
        Some(fragment[start..end].trim())
    }

    fn compact_task_completion_summary(summary: &str) -> String {
        let trimmed = summary.trim();
        if let Some(rest) = trimmed.strip_prefix("Agent \"") {
            if let Some(end_quote) = rest.find('"') {
                let name = rest[..end_quote].trim();
                if !name.is_empty() {
                    return format!("后台 explorer 已完成：{name}…");
                }
            }
        }

        if trimmed.is_empty() {
            "后台任务已完成，正在汇总结果…".to_string()
        } else {
            format!("后台任务已完成：{trimmed}…")
        }
    }

    fn build_task_lifecycle_progress_message(fragment: &str) -> Option<String> {
        let trimmed = fragment.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed.starts_with("<retrieval_status>") {
            let status = Self::extract_xml_tag_body(trimmed, "retrieval_status")?;
            return Some(match status {
                "timeout" | "running" => "某个 explorer 仍在运行，我继续等待结果…".to_string(),
                other if !other.is_empty() => format!("后台任务状态更新：{other}…"),
                _ => return None,
            });
        }

        if trimmed.starts_with("<task-notification>") {
            let status = Self::extract_xml_tag_body(trimmed, "status").unwrap_or("");
            let summary = Self::extract_xml_tag_body(trimmed, "summary").unwrap_or("");
            return Some(match status {
                "completed" => Self::compact_task_completion_summary(summary),
                "failed" => {
                    if summary.trim().is_empty() {
                        "后台任务执行失败，我继续处理剩余结果…".to_string()
                    } else {
                        format!("后台任务失败：{}…", summary.trim())
                    }
                }
                _ => {
                    if summary.trim().is_empty() {
                        "收到后台任务进度更新…".to_string()
                    } else {
                        format!("后台任务进度更新：{}…", summary.trim())
                    }
                }
            });
        }

        if trimmed.starts_with("Task is still running") {
            return Some("某个 explorer 仍在运行，我继续等待结果…".to_string());
        }

        if trimmed.starts_with("No task output available") {
            return Some("后台任务暂时还没有新输出，我继续等待…".to_string());
        }

        if trimmed.starts_with("Error: No task found with ID:") {
            return Some("某个后台任务已结束或状态失效，我继续汇总现有结果…".to_string());
        }

        None
    }

    fn handle_text_fragment(
        &mut self,
        output: &mut Vec<String>,
        fragment: &str,
        emit_plain_text: bool,
    ) {
        if fragment.is_empty() {
            return;
        }

        if self.suppressing_suggestion_mode_prompt {
            if let Some(end_idx) = fragment.find(Self::SUGGESTION_MODE_END_MARKER) {
                let suffix = &fragment[end_idx + Self::SUGGESTION_MODE_END_MARKER.len()..];
                self.suppressing_suggestion_mode_prompt = false;
                self.logger
                    .log_raw("[Info] Suggestion-mode prompt suppression ended");
                if !suffix.is_empty() {
                    self.handle_text_fragment(output, suffix, emit_plain_text);
                }
            }
            return;
        }

        if let Some(start_idx) = fragment.find(Self::SUGGESTION_MODE_START_MARKER) {
            let prefix = &fragment[..start_idx];
            if emit_plain_text && !prefix.is_empty() {
                self.emit_or_defer_plain_text(output, prefix);
            }

            let after_start = &fragment[start_idx..];
            if let Some(end_rel) = after_start.find(Self::SUGGESTION_MODE_END_MARKER) {
                let suffix = &after_start[end_rel + Self::SUGGESTION_MODE_END_MARKER.len()..];
                self.logger
                    .log_raw("[Warn] Dropping suggestion-mode prompt leak from visible text");
                if !suffix.is_empty() {
                    self.handle_text_fragment(output, suffix, emit_plain_text);
                }
            } else {
                self.suppressing_suggestion_mode_prompt = true;
                self.logger
                    .log_raw("[Warn] Dropping suggestion-mode prompt leak from visible text");
            }
            return;
        }

        // 如果我们正在抑制跨块泄漏，检查是否应该继续抑制
        if self.suppressing_cross_chunk_leak {
            // 检查当前片段是否包含泄漏结束标记
            if let Some(end_pos) = fragment.find("```") {
                // 找到结束标记，抑制到结束标记为止
                let remaining = &fragment[end_pos + 3..];
                self.suppressing_cross_chunk_leak = false;
                self.logger
                    .log_raw("[Info] Cross-chunk leak suppression ended");

                // 检查剩余内容是否是可疑的尾巴噪声
                if !remaining.is_empty() {
                    let cleaned_remaining =
                        LeakDetector::strip_suspicious_trailing_noise(remaining);
                    if !cleaned_remaining.is_empty() {
                        self.handle_text_fragment(output, &cleaned_remaining, emit_plain_text);
                    } else {
                        self.logger
                            .log_raw("[Info] Suppressed suspicious tail after cross-chunk leak");
                    }
                }
                return;
            } else {
                // 没有找到结束标记，继续抑制整个片段
                self.logger
                    .log_raw("[Info] Suppressing cross-chunk leak continuation");
                return;
            }
        }

        let combined = if self.text_carryover.is_empty() {
            fragment.to_string()
        } else {
            let mut merged = std::mem::take(&mut self.text_carryover);
            merged.push_str(fragment);
            merged
        };
        let combined = Self::strip_proposed_plan_wrappers(&combined);

        if let Some(message) = Self::build_task_lifecycle_progress_message(&combined) {
            self.logger
                .log_raw("[Info] Bridged background-task lifecycle text into thinking progress");
            self.open_thinking_block_if_needed(output);
            self.emit_thinking_delta(output, message.as_str());
            self.emit_thinking_delta(
                output, "
",
            );
            return;
        }

        if self.in_markdown_bash {
            self.markdown_bash_buffer.push_str(&combined);

            // Check if we hit the closing ```
            if self.markdown_bash_buffer.contains("\n```\n")
                || self.markdown_bash_buffer.ends_with("\n```")
                || !emit_plain_text
            {
                self.flush_markdown_bash(output);
            }
            return;
        }

        if let Some((marker_start, marker_len)) = LeakDetector::find_markdown_bash_start(&combined)
        {
            let prefix_text = &combined[..marker_start];
            let after_marker = &combined[marker_start + marker_len..];

            if emit_plain_text && !prefix_text.is_empty() {
                self.emit_or_defer_plain_text(output, prefix_text);
            }
            self.in_markdown_bash = true;
            self.markdown_bash_buffer.push_str(after_marker);

            if self.markdown_bash_buffer.contains("\n```\n")
                || self.markdown_bash_buffer.ends_with("\n```")
                || !emit_plain_text
            {
                self.flush_markdown_bash(output);
            }
            return;
        }

        // 泄漏工具调用文本可能被拆成多个 chunk。
        // 一旦进入泄漏拼接模式，后续 chunk 持续进入 pending，直到形成可判定边界再处理。
        if !self.pending_tool_text.is_empty() {
            self.pending_tool_text.push_str(&combined);
            self.process_pending_tool_text(output, !emit_plain_text);
            return;
        }

        if let Some(marker_start) = LeakDetector::find_potential_leaked_tool_marker_start(&combined)
        {
            let (prefix_text, leaked_fragment) = combined.split_at(marker_start);
            if emit_plain_text && !prefix_text.is_empty() {
                self.emit_or_defer_plain_text(output, prefix_text);
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.process_pending_tool_text(output, !emit_plain_text);
            return;
        }

        // 某些泄漏不带 `assistant to=`/`to=` 前缀，而是直接混入工具参数 JSON。
        // 将裸 JSON 片段送入 pending，按高置信规则分段抑制，仅保留前后安全文本。
        if let Some(raw_json_start) = LeakDetector::find_potential_raw_tool_json_start(&combined) {
            let (prefix_text, leaked_fragment) = combined.split_at(raw_json_start);
            if emit_plain_text && !prefix_text.is_empty() {
                let cleaned_prefix =
                    LeakDetector::sanitize_prefix_before_raw_tool_json(prefix_text);
                if !cleaned_prefix.is_empty() {
                    self.emit_or_defer_plain_text(output, &cleaned_prefix);
                }
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.process_pending_tool_text(output, !emit_plain_text);
            return;
        }

        // 检查上下文 note-json 泄漏
        let context = format!("{}{}", self.text_carryover, &combined);
        if let Some((prefix, _, _suffix, is_cross_chunk)) =
            LeakDetector::split_contextual_note_json_prefix_suffix(&combined, &context)
        {
            // 对于上下文泄漏，只保留前缀中的安全部分，完全抑制 JSON 和可疑尾巴
            if is_cross_chunk {
                self.suppressing_cross_chunk_leak = true;
            }
            if emit_plain_text && !prefix.is_empty() {
                self.emit_or_defer_plain_text(output, &prefix);
            }
            // 注意：不处理 suffix，因为它包含可疑的尾巴噪声，应该被完全抑制
            return;
        }

        if !emit_plain_text || self.has_open_tool_block() {
            // 工具窗口期间：延迟而非丢弃，保留文本在工具结束后发射
            if emit_plain_text && self.has_open_tool_block() && !combined.is_empty() {
                self.buffer_deferred_unscoped_text(&combined);
            }
            return;
        }

        let carry_len = LeakDetector::leaked_marker_suffix_len(&combined);
        if carry_len == 0 {
            self.emit_plain_text_fragment(output, &combined);
            return;
        }

        let split_at = combined.len() - carry_len;
        let (safe_text, carryover) = combined.split_at(split_at);
        if !safe_text.is_empty() {
            self.emit_plain_text_fragment(output, safe_text);
        }
        self.text_carryover.push_str(carryover);
    }

    /// 工具窗口安全的文本发射：有工具缓冲时延迟，否则直接发射。
    /// 用于替换所有 `emit_plain_text && !has_open_tool_block()` 模式。
    fn emit_or_defer_plain_text(&mut self, output: &mut Vec<String>, fragment: &str) {
        if fragment.is_empty() {
            return;
        }
        if self.has_open_tool_block() {
            self.buffer_deferred_unscoped_text(fragment);
        } else {
            self.emit_plain_text_fragment(output, fragment);
        }
    }

    fn emit_plain_text_fragment(&mut self, output: &mut Vec<String>, fragment: &str) {
        if fragment.is_empty() {
            return;
        }

        let normalized_fragment = LeakDetector::collapse_adjacent_duplicate_markdown_bold(fragment);
        if normalized_fragment.is_empty() {
            return;
        }
        let observed_fragment = self.observe_proposed_plan_fragment(&normalized_fragment);
        if observed_fragment.is_empty() {
            return;
        }
        let fragment = observed_fragment.as_str();

        // Commentary phase: redirect text to thinking blocks
        // Either explicit via phase field, or fallback when reasoning was seen
        // but no message output_item.added arrived (API sometimes omits it)
        if self.in_commentary_phase
            || (self.had_reasoning_in_response && !self.saw_message_item_added)
        {
            self.open_thinking_block_if_needed(output);
            self.emit_thinking_delta(output, fragment);
            return;
        }

        if self.suppress_visible_final_answer_text
            && !self.in_commentary_phase
            && !(self.had_reasoning_in_response && !self.saw_message_item_added)
        {
            self.logger.log_raw(
                "[Info] Suppressing visible final-answer text for multi-background-agent launch turn",
            );
            return;
        }

        self.emit_visible_text_fragment(output, fragment);
    }

    fn emit_visible_text_fragment(&mut self, output: &mut Vec<String>, fragment: &str) {
        if fragment.is_empty() {
            return;
        }

        if self.open_text_index.is_none() {
            let idx = self.content_index;
            self.content_index += 1;
            self.open_text_index = Some(idx);
            output.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({
                    "type": "content_block_start",
                    "index": idx,
                    "content_block": { "type": "text", "text": "" }
                })
            ));
        }

        output.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({
                "type": "content_block_delta",
                "index": self.open_text_index,
                "delta": { "type": "text_delta", "text": fragment }
            })
        ));
        self.transition_to(StreamPhase::StreamingText);
    }

    fn extract_refusal_text<'a>(data: &'a Value) -> Option<&'a str> {
        fn value_to_text(value: &Value) -> Option<&str> {
            value
                .as_str()
                .or_else(|| value.get("text").and_then(|v| v.as_str()))
                .or_else(|| value.get("delta").and_then(|v| v.as_str()))
        }

        data.get("delta")
            .and_then(value_to_text)
            .or_else(|| data.get("refusal").and_then(value_to_text))
            .or_else(|| data.get("text").and_then(|v| v.as_str()))
    }

    fn emit_refusal_delta(&mut self, output: &mut Vec<String>, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.saw_refusal = true;
        self.in_commentary_phase = false;
        self.close_open_thinking_block(output);
        self.emit_visible_text_fragment(output, delta);
        self.refusal_text_buffer.push_str(delta);
    }

    fn emit_refusal_done(&mut self, output: &mut Vec<String>, full_text: &str) {
        if full_text.is_empty() {
            return;
        }
        self.saw_refusal = true;
        self.in_commentary_phase = false;
        self.close_open_thinking_block(output);

        let suffix = if full_text.starts_with(self.refusal_text_buffer.as_str()) {
            &full_text[self.refusal_text_buffer.len()..]
        } else if self.refusal_text_buffer.starts_with(full_text) {
            ""
        } else {
            full_text
        };

        if !suffix.is_empty() {
            self.emit_visible_text_fragment(output, suffix);
        }
        self.refusal_text_buffer = full_text.to_string();
    }

    fn emit_tool_json_delta(&self, output: &mut Vec<String>, idx: usize, delta: String) {
        output.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({
                "type": "content_block_delta",
                "index": idx,
                "delta": { "type": "input_json_delta", "partial_json": delta }
            })
        ));
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

    fn flush_markdown_bash(&mut self, output: &mut Vec<String>) {
        if !self.in_markdown_bash {
            return;
        }
        self.in_markdown_bash = false;

        let mut script = std::mem::take(&mut self.markdown_bash_buffer);
        let mut leftover_text = String::new();

        if let Some(end_idx) = script.find("\n```\n") {
            leftover_text = script[end_idx + 5..].to_string();
            script.truncate(end_idx);
        } else if let Some(end_idx) = script.find("\n```") {
            leftover_text = script[end_idx + 4..].to_string();
            script.truncate(end_idx);
        } else if script.ends_with("```\n") {
            script.truncate(script.len() - 4);
        } else if script.ends_with("```") {
            script.truncate(script.len() - 3);
        }

        let script = script.trim().to_string();

        if script.is_empty() {
            if !leftover_text.is_empty() {
                self.text_carryover.push_str(&leftover_text);
                self.flush_text_carryover(output);
            }
            return;
        }

        self.close_open_text_block(output);

        let call_id = format!("tool_{}", chrono::Utc::now().timestamp_millis());
        let name = "Bash".to_string();
        let arguments = json!({ "command": script }).to_string();
        let synthetic_tool = BufferedToolCall {
            order_key: u64::MAX,
            arrival_seq: u64::MAX,
            output_index: None,
            item_id: None,
            call_id,
            name,
            arguments_buffer: arguments,
            consecutive_whitespace_run: 0,
            done_flag: true,
            start_emitted: false,
            content_block_index: None,
            emitted_arguments_len: 0,
            last_progress_message: None,
        };
        self.saw_tool_call = true;
        self.emit_serialized_tool_call(output, &synthetic_tool);
        self.sync_phase_from_runtime();

        if !leftover_text.is_empty() {
            self.text_carryover.push_str(&leftover_text);
            // Re-eval leftovers to see if another markdown block exists or just emit
            self.flush_text_carryover(output);
        }
    }

    fn flush_text_carryover(&mut self, output: &mut Vec<String>) {
        if self.text_carryover.is_empty() {
            return;
        }

        let carryover = std::mem::take(&mut self.text_carryover);
        let carryover = Self::strip_proposed_plan_wrappers(&carryover);

        if let Some((marker_start, marker_len)) = LeakDetector::find_markdown_bash_start(&carryover)
        {
            let prefix_text = &carryover[..marker_start];
            let after_marker = &carryover[marker_start + marker_len..];

            if !self.has_open_tool_block() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
            }
            self.in_markdown_bash = true;
            self.markdown_bash_buffer.push_str(after_marker);
            self.flush_markdown_bash(output);
            return;
        }

        if let Some(marker_start) =
            LeakDetector::find_potential_leaked_tool_marker_start(&carryover)
        {
            let (prefix_text, leaked_fragment) = carryover.split_at(marker_start);
            if !self.has_open_tool_block() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.process_pending_tool_text(output, true);
            return;
        }

        // 检查高置信工具参数泄漏
        if let Some((prefix, raw_json, suffix)) =
            LeakDetector::split_tool_json_prefix_suffix(&carryover)
        {
            self.diagnostics.dropped_raw_tool_json_fragments += 1;
            let assessment = LeakDetector::assess_raw_tool_json(&raw_json);
            self.record_raw_tool_json_assessment(assessment);
            if !self.has_open_tool_block() && !prefix.is_empty() {
                let cleaned_prefix = LeakDetector::sanitize_prefix_before_raw_tool_json(&prefix);
                if !cleaned_prefix.is_empty() {
                    self.emit_plain_text_fragment(output, &cleaned_prefix);
                }
            }
            if let Some(recovered_calls) =
                self.try_recover_readonly_tool_uses_from_raw_json(output, &raw_json)
            {
                self.diagnostics.recovered_readonly_leaked_tool_payloads += 1;
                self.diagnostics.recovered_readonly_leaked_tool_calls += recovered_calls as u64;
                self.logger.log_raw(&format!(
                    "[Warn] Recovered leaked readonly tool_uses payload from carryover count={} score={} reason={}",
                    recovered_calls, assessment.score, assessment.reason
                ));
            } else {
                if assessment.is_high_risk() {
                    self.diagnostics.dropped_high_risk_raw_tool_json_fragments += 1;
                    self.logger.log_raw(&format!(
                        "[Warn] Dropping high-risk raw leaked tool json from carryover score={} reason={}",
                        assessment.score, assessment.reason
                    ));
                    self.maybe_emit_high_risk_leak_question(output);
                } else {
                    self.logger.log_raw(&format!(
                        "[Warn] Dropping raw leaked tool json from carryover score={} reason={}",
                        assessment.score, assessment.reason
                    ));
                }
            }
            if !suffix.is_empty() {
                self.handle_text_fragment(output, &suffix, true);
            }
            return;
        }

        // 检查上下文 note-json 泄漏（新增）
        if let Some((prefix, _, suffix, is_cross_chunk)) =
            LeakDetector::split_contextual_note_json_prefix_suffix(&carryover, &carryover)
        {
            self.diagnostics.dropped_contextual_note_json_fragments += 1;
            self.logger
                .log_raw("[Warn] Dropping contextual note-json leak from carryover");
            if is_cross_chunk {
                self.suppressing_cross_chunk_leak = true;
            }
            if !self.has_open_tool_block() && !prefix.is_empty() {
                self.emit_plain_text_fragment(output, &prefix);
            }
            if !suffix.is_empty() {
                self.handle_text_fragment(output, &suffix, true);
            }
            return;
        }

        if let Some(raw_json_start) = LeakDetector::find_potential_raw_tool_json_start(&carryover) {
            let (prefix_text, leaked_fragment) = carryover.split_at(raw_json_start);
            if !self.has_open_tool_block() && !prefix_text.is_empty() {
                let cleaned_prefix = LeakDetector::strip_known_leak_suffix_noise(prefix_text);
                if !cleaned_prefix.is_empty() {
                    self.emit_plain_text_fragment(output, &cleaned_prefix);
                }
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.process_pending_tool_text(output, true);
            return;
        }

        if !self.has_open_tool_block() {
            self.emit_plain_text_fragment(output, &carryover);
        }
    }

    fn flush_pending_tool_text(&mut self, output: &mut Vec<String>) {
        self.process_pending_tool_text(output, true);
    }

    fn log_diagnostics_summary(&self, terminal_event: &str) {
        if !self.diagnostics.has_activity() {
            return;
        }

        let summary = self.build_diagnostics_summary(terminal_event);
        self.logger.log_raw(&format!("[DiagJSON] {}", summary));
    }

    fn build_diagnostics_summary(&self, terminal_event: &str) -> Value {
        json!({
            "type": "codex_response_transform_summary",
            "terminal_event": terminal_event,
            "counters": {
                "deferred_unscoped_text_chunks": self.diagnostics.deferred_unscoped_text_chunks,
                "deferred_unscoped_text_flushes": self.diagnostics.deferred_unscoped_text_flushes,
                "detected_proposed_plan_blocks": self.diagnostics.detected_proposed_plan_blocks,
                "extracted_proposed_plan_body_chars": self.diagnostics.extracted_proposed_plan_body_chars,
                "plan_bridge_write_successes": self.diagnostics.plan_bridge_write_successes,
                "plan_bridge_write_failures": self.diagnostics.plan_bridge_write_failures,
                "plan_bridge_exit_plan_mode_emitted": self.diagnostics.plan_bridge_exit_plan_mode_emitted,
                "dropped_leaked_marker_fragments": self.diagnostics.dropped_leaked_marker_fragments,
                "dropped_raw_tool_json_fragments": self.diagnostics.dropped_raw_tool_json_fragments,
                "assessed_raw_tool_json_fragments": self.diagnostics.assessed_raw_tool_json_fragments,
                "assessed_raw_tool_json_readonly_recoverable": self.diagnostics.assessed_raw_tool_json_readonly_recoverable,
                "assessed_raw_tool_json_high_risk": self.diagnostics.assessed_raw_tool_json_high_risk,
                "assessed_raw_tool_json_suppressed": self.diagnostics.assessed_raw_tool_json_suppressed,
                "dropped_high_risk_raw_tool_json_fragments": self.diagnostics.dropped_high_risk_raw_tool_json_fragments,
                "emitted_high_risk_leak_questions": self.diagnostics.emitted_high_risk_leak_questions,
                "recovered_readonly_leaked_tool_payloads": self.diagnostics.recovered_readonly_leaked_tool_payloads,
                "recovered_readonly_leaked_tool_calls": self.diagnostics.recovered_readonly_leaked_tool_calls,
                "dropped_contextual_note_json_fragments": self.diagnostics.dropped_contextual_note_json_fragments,
                "dropped_incomplete_tool_json_fragments": self.diagnostics.dropped_incomplete_tool_json_fragments,
                "queued_orphan_tool_argument_updates": self.diagnostics.queued_orphan_tool_argument_updates,
                "applied_orphan_tool_argument_updates": self.diagnostics.applied_orphan_tool_argument_updates,
                "dropped_orphan_tool_argument_updates_no_hint": self.diagnostics.dropped_orphan_tool_argument_updates_no_hint,
                "dropped_orphan_tool_argument_updates_closed_call": self.diagnostics.dropped_orphan_tool_argument_updates_closed_call,
                "dropped_pending_tool_argument_updates_closed_call": self.diagnostics.dropped_pending_tool_argument_updates_closed_call,
                "duplicate_active_call_items": self.diagnostics.duplicate_active_call_items,
                "dropped_reused_closed_call_items": self.diagnostics.dropped_reused_closed_call_items,
                "binding_conflicts_output_index": self.diagnostics.binding_conflicts_output_index,
                "binding_conflicts_item_id": self.diagnostics.binding_conflicts_item_id,
                "normalized_item_id_mismatches": self.diagnostics.normalized_item_id_mismatches,
                "pending_tool_backlog_trimmed": self.diagnostics.pending_tool_backlog_trimmed,
                "dropped_function_args_whitespace_overflow_fragments": self.diagnostics.dropped_function_args_whitespace_overflow_fragments,
                "terminal_invariant_violations": self.diagnostics.terminal_invariant_violations,
                "pending_orphan_updates": self.pending_tool_argument_updates.len()
            }
        })
    }

    fn map_incomplete_reason_to_stop_reason(
        reason: Option<&str>,
        force_incomplete: bool,
    ) -> &'static str {
        let normalized = reason
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_default();

        if normalized == "pause_turn" {
            return "pause_turn";
        }
        if normalized == "stop_sequence" {
            return "stop_sequence";
        }
        if normalized == "model_context_window_exceeded" || normalized == "context_window_exceeded"
        {
            return "model_context_window_exceeded";
        }
        if normalized == "refusal" || normalized == "content_filter" || normalized == "safety" {
            return "refusal";
        }
        if force_incomplete
            || normalized == "max_output_tokens"
            || normalized == "max_tokens"
            || normalized == "length"
        {
            return "max_tokens";
        }
        "end_turn"
    }

    fn determine_stop_reason(&self, data: &Value, force_incomplete: bool) -> &'static str {
        if self.saw_tool_call {
            return "tool_use";
        }
        if self.saw_refusal {
            return "refusal";
        }

        let response = data.get("response");
        let status = response
            .and_then(|r| r.get("status"))
            .and_then(|s| s.as_str());
        let incomplete_reason = response
            .and_then(|r| r.pointer("/incomplete_details/reason"))
            .and_then(|value| value.as_str())
            .or_else(|| {
                response
                    .and_then(|r| r.get("reason"))
                    .and_then(|value| value.as_str())
            })
            .or_else(|| {
                data.pointer("/incomplete_details/reason")
                    .and_then(|value| value.as_str())
            })
            .or_else(|| data.get("reason").and_then(|value| value.as_str()));

        if status == Some("incomplete") {
            return Self::map_incomplete_reason_to_stop_reason(incomplete_reason, true);
        }

        Self::map_incomplete_reason_to_stop_reason(incomplete_reason, force_incomplete)
    }

    fn emit_terminal_events(
        &mut self,
        output: &mut Vec<String>,
        data: &Value,
        force_incomplete: bool,
    ) {
        if self.phase == StreamPhase::Terminal {
            return;
        }

        self.flush_text_carryover(output);
        self.flush_pending_tool_text(output);
        self.flush_markdown_bash(output);
        self.reset_text_dedupe_state();
        self.suppressing_suggestion_mode_prompt = false;

        self.close_open_text_block(output);
        self.close_open_thinking_block(output);
        self.close_open_web_search_calls(output);
        if self.flush_serialized_tool_calls(output, true) {
            self.emit_function_args_whitespace_overflow_error(output);
            return;
        }
        self.flush_deferred_unscoped_text(output, true);
        self.close_open_text_block(output);
        self.close_open_thinking_block(output);

        // Auto-cleanup orphaned pending tool argument updates at terminal phase
        // This can happen when upstream sends inconsistent tool call data
        if !self.pending_tool_argument_updates.is_empty() {
            let count = self.pending_tool_argument_updates.len();
            self.logger.log_raw(&format!(
                "[Warn] Clearing {} orphaned tool argument updates at terminal phase (upstream inconsistency)",
                count
            ));
            self.pending_tool_argument_updates.clear();
        }

        if let Some(reason) = self.terminal_invariant_violation_reason() {
            self.emit_terminal_invariant_violation_error(output, reason);
            return;
        }

        let should_fallback_proposed_plan = self.codex_plan_file_path.is_some()
            && !self.plan_bridge_emitted
            && !self.saw_tool_call
            && (self.latest_proposed_plan_body.is_some()
                || (self.capturing_proposed_plan
                    && !self.proposed_plan_body_buffer.trim().is_empty()));
        if should_fallback_proposed_plan && !self.maybe_emit_plan_mode_bridge(output) {
            if let Some(fallback_text) = self.build_suppressed_proposed_plan_fallback() {
                self.logger
                    .log_raw("[PlanBridge] Falling back to visible proposed_plan text");
                self.emit_visible_text_fragment(output, fallback_text.as_str());
            }
        }
        let stop_reason = self.determine_stop_reason(data, force_incomplete);

        let usage = data
            .get("response")
            .and_then(|r| r.get("usage"))
            .cloned()
            .unwrap_or_else(|| {
                json!({
                    "input_tokens": 0,
                    "output_tokens": 0
                })
            });

        output.push(format!(
            "event: message_delta\ndata: {}\n\n",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason },
                "usage": {
                    "input_tokens": usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
                    "output_tokens": usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0)
                }
            })
        ));

        output.push(format!(
            "event: message_stop\ndata: {}\n\n",
            json!({ "type": "message_stop" })
        ));
        let terminal_event = if force_incomplete {
            "response.incomplete"
        } else {
            "response.completed"
        };
        self.last_terminal_event = Some(terminal_event.to_string());
        self.log_diagnostics_summary(terminal_event);
        self.transition_to(StreamPhase::Terminal);
    }

    fn extract_upstream_error_message_and_code(data: &Value) -> (Option<String>, Option<String>) {
        let message = data
            .pointer("/response/error/message")
            .or_else(|| data.pointer("/error/message"))
            .or_else(|| data.pointer("/response/error/details/message"))
            .or_else(|| data.pointer("/error/details/message"))
            .or_else(|| data.get("message"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());

        let code = data
            .pointer("/response/error/code")
            .or_else(|| data.pointer("/error/code"))
            .or_else(|| data.get("code"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());

        (message, code)
    }

    fn emit_error_and_stop(
        &mut self,
        output: &mut Vec<String>,
        data: &Value,
        default_message: &str,
    ) {
        if self.phase == StreamPhase::Terminal {
            return;
        }

        self.flush_text_carryover(output);
        self.flush_pending_tool_text(output);
        self.flush_markdown_bash(output);
        self.reset_text_dedupe_state();
        self.suppressing_suggestion_mode_prompt = false;

        self.close_open_text_block(output);
        self.close_open_thinking_block(output);
        self.close_open_web_search_calls(output);
        let _ = self.flush_serialized_tool_calls(output, true);
        self.flush_deferred_unscoped_text(output, true);
        self.close_open_text_block(output);
        self.close_open_thinking_block(output);

        let (message, code) = Self::extract_upstream_error_message_and_code(data);
        let mut error_payload = json!({
            "type": "api_error",
            "message": message.unwrap_or_else(|| default_message.to_string())
        });
        if let Some(code) = code {
            if !code.trim().is_empty() {
                if let Some(obj) = error_payload.as_object_mut() {
                    obj.insert("code".to_string(), Value::String(code));
                }
            }
        }

        output.push(format!(
            "event: error\ndata: {}\n\n",
            json!({
                "type": "error",
                "error": error_payload
            })
        ));
        output.push(format!(
            "event: message_stop\ndata: {}\n\n",
            json!({ "type": "message_stop" })
        ));
        self.last_terminal_event = Some("response.failed_or_error".to_string());
        self.log_diagnostics_summary("response.failed_or_error");
        self.transition_to(StreamPhase::Terminal);
    }

    fn emit_function_args_whitespace_overflow_error(&mut self, output: &mut Vec<String>) {
        let error_data = json!({
            "error": {
                "message": "Function arguments stream contained excessive consecutive whitespace and was aborted for safety.",
                "code": "function_args_whitespace_overflow"
            }
        });
        self.emit_error_and_stop(
            output,
            &error_data,
            "Function arguments stream exceeded whitespace safety threshold.",
        );
    }

    pub fn transform_sse_line(&mut self, line: &str) -> Vec<String> {
        let mut output = Vec::new();

        if !line.starts_with("data: ") {
            return output;
        }

        // 发送 message_start（确保只发送一次）
        if !self.sent_message_start {
            self.sent_message_start = true;
            output.push(format!(
                "event: message_start\ndata: {}\n\n",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": self.message_id,
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": self.model,
                        "stop_reason": null,
                        "usage": { "input_tokens": 0, "output_tokens": 0 }
                    }
                })
            ));
        }

        let Ok(data) = serde_json::from_str::<Value>(&line[6..]) else {
            return output;
        };

        let event_type = data.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            // 纯文本输出 - 严格控制，只在非工具场景下开启文本块
            "response.output_text.delta" => {
                let metadata = self.normalized_event_metadata(&data);
                let part_key = metadata.part_key();
                self.register_text_part_if_scoped(&part_key);
                let routing = self.decide_text_routing(&metadata);

                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                let Some(delta) = self.dedupe_cross_source_fragment(
                    &part_key,
                    TextEventSource::OutputTextDelta,
                    delta,
                ) else {
                    return output;
                };

                if routing == TextRoutingDecision::Suppress {
                    return output;
                }
                if routing == TextRoutingDecision::DeferUntilToolWindowCloses {
                    self.buffer_deferred_unscoped_text(&delta);
                    return output;
                }

                // Only close thinking block if text will NOT be redirected to thinking
                // (avoids unnecessary open/close cycles during commentary)
                let redirect_to_thinking = self.in_commentary_phase
                    || (self.had_reasoning_in_response && !self.saw_message_item_added);
                if !redirect_to_thinking {
                    self.close_open_thinking_block(&mut output);
                }
                self.handle_text_fragment(&mut output, &delta, true);
            }

            // 新版事件：内容分片直接挂在 content_part.added
            "response.content_part.added" => {
                let metadata = self.normalized_event_metadata(&data);
                let part_key = metadata.part_key();
                if let Some(fragment) = Self::extract_content_part_text(&data) {
                    self.register_text_part_if_scoped(&part_key);
                    let routing = self.decide_text_routing(&metadata);
                    if routing == TextRoutingDecision::Suppress {
                        return output;
                    }

                    if let Some(text) = self.dedupe_cross_source_fragment(
                        &part_key,
                        TextEventSource::ContentPartAdded,
                        fragment,
                    ) {
                        if routing == TextRoutingDecision::DeferUntilToolWindowCloses {
                            self.buffer_deferred_unscoped_text(&text);
                            return output;
                        }

                        let redirect_to_thinking = self.in_commentary_phase
                            || (self.had_reasoning_in_response && !self.saw_message_item_added);
                        if !redirect_to_thinking {
                            self.close_open_thinking_block(&mut output);
                        }
                        self.handle_text_fragment(&mut output, &text, true);
                    }
                }
            }

            // 文本分片结束：如果 pending 里还有疑似工具泄露，立即按边界强制 flush
            "response.output_text.done" => {
                let metadata = self.normalized_event_metadata(&data);
                let part_key = metadata.part_key();
                self.flush_text_carryover(&mut output);
                match self.decide_text_routing(&metadata) {
                    TextRoutingDecision::Emit => {
                        if let Some(done_text) = data.get("text").and_then(|t| t.as_str()) {
                            self.handle_text_fragment(&mut output, done_text, false);
                        }
                    }
                    TextRoutingDecision::DeferUntilToolWindowCloses => {
                        if let Some(done_text) = data.get("text").and_then(|t| t.as_str()) {
                            self.buffer_deferred_unscoped_text(done_text);
                        }
                    }
                    TextRoutingDecision::Suppress => {}
                }
                if !self.pending_tool_text.is_empty() {
                    self.flush_pending_tool_text(&mut output);
                }
                self.flush_deferred_unscoped_text(&mut output, false);

                if self.finish_text_part(&part_key) && self.active_text_parts.is_empty() {
                    self.close_open_text_block(&mut output);
                }
            }

            "response.content_part.done" => {
                let metadata = self.normalized_event_metadata(&data);
                let part_key = metadata.part_key();
                self.flush_text_carryover(&mut output);
                if !self.pending_tool_text.is_empty() {
                    self.flush_pending_tool_text(&mut output);
                }
                self.flush_deferred_unscoped_text(&mut output, false);
                if Self::is_text_content_part(&data)
                    && self.finish_text_part(&part_key)
                    && self.active_text_parts.is_empty()
                {
                    self.close_open_text_block(&mut output);
                }
            }

            // 推理摘要分片：映射为 Anthropic thinking 增量事件，避免长阶段无可见流输出
            "response.reasoning_summary_part.added" => {
                self.had_reasoning_in_response = self.allow_visible_thinking;
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                self.open_thinking_block_if_needed(&mut output);
            }

            "response.reasoning_summary_text.delta" => {
                self.had_reasoning_in_response = self.allow_visible_thinking;
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.open_thinking_block_if_needed(&mut output);
                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                self.emit_thinking_delta(&mut output, delta);
            }

            "response.reasoning_summary_text.done" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                if let Some(done_text) = data.get("text").and_then(|t| t.as_str()) {
                    self.open_thinking_block_if_needed(&mut output);
                    self.emit_thinking_delta(&mut output, done_text);
                }
            }

            "response.reasoning_summary_part.done" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
            }

            "response.refusal.delta" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                if let Some(delta) = Self::extract_refusal_text(&data) {
                    self.emit_refusal_delta(&mut output, delta);
                }
            }

            "response.refusal.done" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                if let Some(full_text) = Self::extract_refusal_text(&data) {
                    self.emit_refusal_done(&mut output, full_text);
                }
            }

            "response.created" => {
                if !self.response_created_announced {
                    self.response_created_announced = true;
                    self.emit_response_lifecycle_progress(
                        &mut output,
                        "请求已发送，正在等待上游开始输出…",
                    );
                }
            }

            "response.in_progress" => {
                if !self.response_in_progress_announced {
                    self.response_in_progress_announced = true;
                    self.emit_response_lifecycle_progress(&mut output, "模型正在处理中…");
                }
            }

            // 工具调用开始 / 消息项开始 - 严格按照 OpenAI Responses 格式解析
            "response.output_item.added" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                if let Some(item) = data.get("item") {
                    let metadata = self.normalized_event_metadata(&data);
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    let output_index = metadata.output_index;
                    let item_id = metadata.item_id.as_deref();
                    let call_id = metadata.call_id.as_deref();
                    self.register_output_item_kind(output_index, item_id, call_id, item_type);

                    match item_type {
                        "function_call" => {
                            // 进入工具缓冲状态：先收敛文本/thinking 边界，稍后按顺序串行下发 tool_use
                            self.close_open_text_block(&mut output);
                            self.close_open_thinking_block(&mut output);

                            let call_id = item
                                .get("call_id")
                                .and_then(|c| c.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    format!("tool_{}", chrono::Utc::now().timestamp_millis())
                                });
                            let name = item
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let normalized_item_id = item_id.map(|s| s.to_string());

                            let _ = self.buffer_tool_call(
                                output_index,
                                normalized_item_id.clone(),
                                call_id.clone(),
                                name,
                            );

                            if let Some(arguments) = item.get("arguments").and_then(|v| v.as_str())
                            {
                                if !arguments.is_empty() {
                                    if let Some(order_key) = self
                                        .find_buffered_tool_order_from_metadata(
                                            output_index,
                                            normalized_item_id.as_deref(),
                                            Some(call_id.as_str()),
                                        )
                                    {
                                        if self.apply_tool_arguments_snapshot(order_key, arguments)
                                        {
                                            self.emit_function_args_whitespace_overflow_error(
                                                &mut output,
                                            );
                                            return output;
                                        }
                                    }
                                }
                            }
                            if self.apply_pending_tool_argument_updates() {
                                self.emit_function_args_whitespace_overflow_error(&mut output);
                                return output;
                            }
                            self.maybe_emit_front_buffered_tool_start(&mut output);
                        }
                        "message" => {
                            self.saw_message_item_added = true;
                            // 检测 phase 字段：commentary 阶段的文本重定向为 thinking blocks
                            let phase = item.get("phase").and_then(|p| p.as_str()).unwrap_or("");
                            if phase == "commentary" {
                                self.in_commentary_phase = true;
                                self.logger.log_raw(
                                    "[Info] Commentary phase detected, redirecting text to thinking blocks",
                                );
                            } else {
                                self.in_commentary_phase = false;
                            }
                        }
                        "web_search_call" => {
                            self.register_active_web_search_call(
                                &mut output,
                                output_index,
                                item_id,
                            );
                        }
                        _ => {}
                    }
                }
            }

            "response.web_search_call.in_progress" | "response.web_search_call.searching" => {
                let metadata = self.normalized_event_metadata(&data);
                self.register_active_web_search_call(
                    &mut output,
                    metadata.output_index,
                    metadata.item_id.as_deref(),
                );
            }

            "response.web_search_call.completed" => {}

            // 工具参数增量更新
            "response.function_call_arguments.delta" | "response.function_call_arguments_delta" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);

                let delta = data
                    .get("delta")
                    .or_else(|| data.get("arguments"))
                    .and_then(|d| d.as_str())
                    .unwrap_or("");

                if !delta.is_empty() {
                    if let Some(order_key) = self.find_buffered_tool_order(&data) {
                        if self.append_tool_arguments_delta(order_key, delta) {
                            self.emit_function_args_whitespace_overflow_error(&mut output);
                            return output;
                        }
                        self.maybe_emit_front_buffered_tool_start(&mut output);
                        self.maybe_emit_front_buffered_tool_argument_delta(&mut output);
                    } else {
                        self.queue_pending_tool_argument_update(
                            &data,
                            PendingToolArgumentUpdateKind::Delta(delta.to_string()),
                        );
                    }
                }
            }

            // 参数完成事件（某些流只在 done 里给完整 arguments）
            "response.function_call_arguments.done" | "response.function_call_arguments_done" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                let full_arguments = data.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                if !full_arguments.is_empty() {
                    if let Some(order_key) = self.find_buffered_tool_order(&data) {
                        if self.apply_tool_arguments_snapshot(order_key, full_arguments) {
                            self.emit_function_args_whitespace_overflow_error(&mut output);
                            return output;
                        }
                        self.maybe_emit_front_buffered_tool_start(&mut output);
                        self.maybe_emit_front_buffered_tool_argument_delta(&mut output);
                    } else {
                        self.queue_pending_tool_argument_update(
                            &data,
                            PendingToolArgumentUpdateKind::Snapshot(full_arguments.to_string()),
                        );
                    }
                }
            }

            // 工具调用完成 / 消息项完成
            "response.output_item.done" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                if self.apply_pending_tool_argument_updates() {
                    self.emit_function_args_whitespace_overflow_error(&mut output);
                    return output;
                }

                let metadata = self.normalized_event_metadata(&data);
                let output_index = metadata.output_index;
                let item = data.get("item");
                let item_type = item
                    .and_then(|it| it.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let item_id = metadata.item_id.as_deref();
                let call_id = metadata.call_id.as_deref();

                self.clear_output_item_kind(output_index, item_id, call_id);
                if self.active_text_parts.is_empty() {
                    self.close_open_text_block(&mut output);
                }

                match item_type {
                    "function_call" => {
                        if let Some(full_arguments) = item
                            .and_then(|it| it.get("arguments"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(order_key) = self.find_buffered_tool_order_from_metadata(
                                output_index,
                                item_id,
                                call_id,
                            ) {
                                if self.apply_tool_arguments_snapshot(order_key, full_arguments) {
                                    self.emit_function_args_whitespace_overflow_error(&mut output);
                                    return output;
                                }
                            }
                        }

                        if let Some(order_key) = self.find_buffered_tool_order_from_metadata(
                            output_index,
                            item_id,
                            call_id,
                        ) {
                            self.mark_buffered_tool_done(order_key);
                        } else if self.buffered_tool_calls.len() == 1 {
                            // 兼容旧流：item 元数据缺失时仅在唯一候选时回退，避免并行调用参数串线
                            let order_key = self.buffered_tool_calls[0].order_key;
                            self.mark_buffered_tool_done(order_key);
                        }
                        if self.flush_serialized_tool_calls(&mut output, false) {
                            self.emit_function_args_whitespace_overflow_error(&mut output);
                            return output;
                        }
                    }
                    "message" => {
                        self.in_commentary_phase = false;
                        // Fail-safe fence: once message item is done, force-close any open text
                        // block before subsequent function_call items arrive.
                        self.close_open_text_block(&mut output);
                    }
                    "web_search_call" => {
                        let action = item.and_then(|it| it.get("action"));
                        self.emit_web_search_tool_completion(
                            &mut output,
                            action,
                            output_index,
                            item_id,
                        );
                    }
                    _ => {
                        // 兼容旧流：没有 item 元数据时回退关闭最近缓冲的 tool call
                        if item.is_none() {
                            self.in_commentary_phase = false;
                            if let Some(order_key) = self.find_buffered_tool_order_from_metadata(
                                output_index,
                                item_id,
                                None,
                            ) {
                                self.mark_buffered_tool_done(order_key);
                            }
                            if self.flush_serialized_tool_calls(&mut output, false) {
                                self.emit_function_args_whitespace_overflow_error(&mut output);
                                return output;
                            }
                        }
                    }
                }
            }

            // 响应完成 - 关键：确保完整的事件序列
            "response.completed" => {
                self.emit_terminal_events(&mut output, &data, false);
            }

            // 上游别名事件：与 response.completed 语义等价
            "response.done" => {
                self.emit_terminal_events(&mut output, &data, false);
            }

            // 响应不完整但已终止（例如 max_output_tokens / context limit）
            "response.incomplete" => {
                self.emit_terminal_events(&mut output, &data, true);
            }

            // 上游主动失败：透传 message/code 并补齐终止事件，避免下游悬挂。
            "response.failed" => {
                self.emit_error_and_stop(
                    &mut output,
                    &data,
                    "Upstream returned response.failed and terminated the stream.",
                );
            }

            // 兼容上游显式 error 事件。
            "error" => {
                self.emit_error_and_stop(
                    &mut output,
                    &data,
                    "Upstream returned an error event and terminated the stream.",
                );
            }

            // 忽略其他事件类型（如 response.created, response.in_progress 等）
            _ => {
                self.logger
                    .log(&format!("Ignored event type: {}", event_type));
            }
        }

        output
    }
}

// ─── ResponseTransformer trait impl ────────────────────────────────────

impl ResponseTransformer for TransformResponse {
    fn transform_line(&mut self, line: &str) -> Vec<String> {
        self.transform_sse_line(line)
    }

    fn configure_request_context(&mut self, ctx: &ResponseTransformRequestContext) {
        self.codex_plan_file_path = ctx.codex_plan_file_path.clone();
        self.contains_background_agent_completion = ctx.contains_background_agent_completion;
        self.launched_background_agent_count = self
            .launched_background_agent_count
            .max(ctx.historical_background_agent_launch_count);
        self.terminal_background_agent_completion_count =
            ctx.terminal_background_agent_completion_count;
        self.suppress_visible_final_answer_text = self.launched_background_agent_count >= 2
            && self.terminal_background_agent_completion_count
                < self.launched_background_agent_count;
    }

    fn take_diagnostics_summary(&mut self) -> Option<Value> {
        if !self.diagnostics.has_activity() {
            return None;
        }
        let terminal_event = self
            .last_terminal_event
            .as_deref()
            .unwrap_or("not_terminal");
        Some(self.build_diagnostics_summary(terminal_event))
    }
}

#[cfg(test)]
mod tests;
