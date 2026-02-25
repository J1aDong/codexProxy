use crate::logger::AppLogger;
use crate::transform::ResponseTransformer;
use serde_json::{json, Value};
use std::collections::HashMap;

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

#[derive(Clone)]
struct BufferedToolCall {
    order_key: u64,
    arrival_seq: u64,
    output_index: Option<u64>,
    item_id: Option<String>,
    call_id: String,
    name: String,
    arguments_buffer: String,
    done_flag: bool,
}

/// 响应转换器 - Codex SSE -> Anthropic SSE
pub struct TransformResponse {
    message_id: String,
    model: String,
    content_index: usize,
    open_text_index: Option<usize>,
    open_thinking_index: Option<usize>,
    phase: StreamPhase,
    next_tool_order_key: u64,
    next_tool_arrival_seq: u64,
    buffered_tool_calls: Vec<BufferedToolCall>,
    tool_order_by_output_index: HashMap<u64, u64>,
    tool_order_by_item_id: HashMap<String, u64>,
    tool_order_by_call_id: HashMap<String, u64>,
    last_buffered_tool_order: Option<u64>,
    saw_tool_call: bool,
    sent_message_start: bool,
    text_carryover: String,
    pending_tool_text: String,

    // Cross-chunk leak suppression state
    suppressing_cross_chunk_leak: bool,

    // Markdown Base Interception
    in_markdown_bash: bool,
    markdown_bash_buffer: String,

    // Commentary phase: redirect text to thinking blocks instead of text blocks
    in_commentary_phase: bool,
    // Fallback commentary detection: reasoning seen in current response
    had_reasoning_in_response: bool,
    // Track if we've seen a message-type output_item.added (means phase detection is explicit)
    saw_message_item_added: bool,
    // Deduplicate overlapping text between output_text.delta and content_part.added
    last_text_source: Option<TextEventSource>,
    last_text_fragment: String,

    logger: std::sync::Arc<AppLogger>,
}

impl TransformResponse {
    const LEAKED_TOOL_MARKERS: [&'static str; 3] =
        ["assistant to=", "to=functions", "to=multi_tool_use"];

    const MARKDOWN_BASH_MARKERS: [&'static str; 3] = ["```bash", "```sh", "```shell"];

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
            .chain(Self::MARKDOWN_BASH_MARKERS.iter());

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

        trimmed.contains("\"tool_uses\"")
            || trimmed.contains("\"recipient_name\"")
            || trimmed.contains("\"file_path\"")
            || trimmed.contains("\"old_string\"")
            || trimmed.contains("\"new_string\"")
            || trimmed.contains("\"replace_all\"")
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

        Self::looks_like_exec_command_payload_fragment(trimmed)
    }

    fn split_tool_json_prefix_suffix(fragment: &str) -> Option<(String, String, String)> {
        let json_start = Self::find_potential_raw_tool_json_start(fragment)?;
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
                                Self::collapse_adjacent_duplicate_markdown_bold(
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
        let json_start = Self::find_potential_raw_tool_json_start(fragment)?;
        let prefix = fragment[..json_start].to_string();
        let candidate = &fragment[json_start..];
        let json = Self::extract_first_json_object_fragment(candidate)?;

        if !Self::looks_like_contextual_leaked_note_json(&json, context) {
            return None;
        }

        let suffix_start = json_start + json.len();
        let mut suffix = fragment[suffix_start..].to_string();

        // 对上下文泄漏的情况，清理可疑的尾巴噪声
        suffix = Self::strip_suspicious_trailing_noise(&suffix);

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

        if Self::starts_with_leaked_tool_marker(pending_for_tool_parse) {
            if let Some((_, _, suffix)) =
                Self::split_tool_json_prefix_suffix(pending_for_tool_parse)
            {
                self.logger.log_raw(
                    "[Warn] Dropping leaked tool marker + json fragment from visible text",
                );
                if !suffix.is_empty() {
                    self.handle_text_fragment(output, &suffix, true);
                }
                return;
            }

            if !force_flush {
                if let Some(newline_idx) = pending_for_tool_parse.find('\n') {
                    self.logger
                        .log_raw("[Warn] Dropping leaked tool marker fragment from visible text");
                    let suffix = &pending_for_tool_parse[newline_idx + 1..];
                    if !suffix.is_empty() {
                        let cleaned_suffix = Self::strip_suspicious_trailing_noise(suffix);
                        if !cleaned_suffix.is_empty() {
                            self.handle_text_fragment(output, &cleaned_suffix, true);
                        }
                    }
                    return;
                }
                self.pending_tool_text = pending_raw;
                return;
            }

            self.logger
                .log_raw("[Warn] Dropping leaked tool marker fragment from visible text");
            if let Some(newline_idx) = pending_for_tool_parse.find('\n') {
                let suffix = &pending_for_tool_parse[newline_idx + 1..];
                if !suffix.is_empty() {
                    let cleaned_suffix = Self::strip_suspicious_trailing_noise(suffix);
                    if !cleaned_suffix.is_empty() {
                        self.handle_text_fragment(output, &cleaned_suffix, true);
                    }
                }
            }
            return;
        }

        // 检查高置信工具参数泄漏
        if let Some((prefix, _, suffix)) = Self::split_tool_json_prefix_suffix(&pending_raw) {
            self.logger
                .log_raw("[Warn] Dropping raw leaked tool json fragment");
            if !prefix.is_empty() {
                self.emit_plain_text_fragment(output, &prefix);
            }
            if !suffix.is_empty() {
                self.handle_text_fragment(output, &suffix, true);
            }
            return;
        }

        // 检查上下文 note-json 泄漏（新增）
        let context = format!("{}{}", self.text_carryover, &pending_raw);
        if let Some((prefix, _, _suffix, is_cross_chunk)) =
            Self::split_contextual_note_json_prefix_suffix(&pending_raw, &context)
        {
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

        if let Some(raw_json_start) = Self::find_potential_raw_tool_json_start(&pending_raw) {
            let candidate = &pending_raw[raw_json_start..];
            let json_complete = Self::extract_first_json_object_fragment(candidate).is_some();

            if !json_complete {
                if !force_flush {
                    self.pending_tool_text = pending_raw;
                    return;
                }

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
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            content_index: 0,
            open_text_index: None,
            open_thinking_index: None,
            phase: StreamPhase::AwaitingContent,
            next_tool_order_key: 0,
            next_tool_arrival_seq: 0,
            buffered_tool_calls: Vec::new(),
            tool_order_by_output_index: HashMap::new(),
            tool_order_by_item_id: HashMap::new(),
            tool_order_by_call_id: HashMap::new(),
            last_buffered_tool_order: None,
            saw_tool_call: false,
            sent_message_start: false,
            text_carryover: String::new(),
            pending_tool_text: String::new(),
            suppressing_cross_chunk_leak: false,
            in_markdown_bash: false,
            markdown_bash_buffer: String::new(),
            in_commentary_phase: false,
            had_reasoning_in_response: false,
            saw_message_item_added: false,
            last_text_source: None,
            last_text_fragment: String::new(),
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

    fn buffer_tool_call(
        &mut self,
        output_index: Option<u64>,
        item_id: Option<String>,
        call_id: String,
        name: String,
    ) {
        let order_key = self.next_tool_order_key;
        self.next_tool_order_key += 1;
        let arrival_seq = self.next_tool_arrival_seq;
        self.next_tool_arrival_seq += 1;

        if let Some(idx) = output_index {
            self.tool_order_by_output_index.insert(idx, order_key);
        }
        if let Some(ref id) = item_id {
            self.tool_order_by_item_id.insert(id.clone(), order_key);
        }
        self.tool_order_by_call_id
            .insert(call_id.clone(), order_key);
        self.last_buffered_tool_order = Some(order_key);

        self.buffered_tool_calls.push(BufferedToolCall {
            order_key,
            arrival_seq,
            output_index,
            item_id,
            call_id,
            name,
            arguments_buffer: String::new(),
            done_flag: false,
        });
        self.sort_buffered_tools();
        self.saw_tool_call = true;
        self.transition_to(StreamPhase::BufferingToolCalls);
    }

    fn find_buffered_tool_order_from_metadata(
        &self,
        output_index: Option<u64>,
        item_id: Option<&str>,
        call_id: Option<&str>,
    ) -> Option<u64> {
        output_index
            .and_then(|idx| self.tool_order_by_output_index.get(&idx).copied())
            .or_else(|| item_id.and_then(|id| self.tool_order_by_item_id.get(id).copied()))
            .or_else(|| call_id.and_then(|id| self.tool_order_by_call_id.get(id).copied()))
            .or(self.last_buffered_tool_order)
    }

    fn find_buffered_tool_order(&self, data: &Value) -> Option<u64> {
        let output_index = data.get("output_index").and_then(|v| v.as_u64());
        let item_id = data.get("item_id").and_then(|v| v.as_str()).or_else(|| {
            data.get("item")
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str())
        });
        let call_id = data.get("call_id").and_then(|v| v.as_str()).or_else(|| {
            data.get("item")
                .and_then(|v| v.get("call_id"))
                .and_then(|v| v.as_str())
        });
        self.find_buffered_tool_order_from_metadata(output_index, item_id, call_id)
    }

    fn get_buffered_tool_mut(&mut self, order_key: u64) -> Option<&mut BufferedToolCall> {
        self.buffered_tool_calls
            .iter_mut()
            .find(|tool| tool.order_key == order_key)
    }

    fn append_tool_arguments_delta(&mut self, order_key: u64, delta: &str) {
        if delta.is_empty() {
            return;
        }
        if let Some(tool) = self.get_buffered_tool_mut(order_key) {
            tool.arguments_buffer.push_str(delta);
        }
    }

    fn apply_tool_arguments_snapshot(&mut self, order_key: u64, full_arguments: &str) {
        if full_arguments.is_empty() {
            return;
        }
        if let Some(tool) = self.get_buffered_tool_mut(order_key) {
            if full_arguments.starts_with(tool.arguments_buffer.as_str()) {
                let suffix = &full_arguments[tool.arguments_buffer.len()..];
                if !suffix.is_empty() {
                    tool.arguments_buffer.push_str(suffix);
                }
                return;
            }

            if tool.arguments_buffer.starts_with(full_arguments) {
                return;
            }

            tool.arguments_buffer = full_arguments.to_string();
        }
    }

    fn mark_buffered_tool_done(&mut self, order_key: u64) {
        if let Some(tool) = self.get_buffered_tool_mut(order_key) {
            tool.done_flag = true;
        }
    }

    fn cleanup_tool_mappings(&mut self, tool: &BufferedToolCall) {
        if let Some(idx) = tool.output_index {
            self.tool_order_by_output_index.remove(&idx);
        }
        if let Some(ref item_id) = tool.item_id {
            self.tool_order_by_item_id.remove(item_id);
        }
        self.tool_order_by_call_id.remove(&tool.call_id);
        if self.last_buffered_tool_order == Some(tool.order_key) {
            self.last_buffered_tool_order =
                self.buffered_tool_calls.iter().map(|t| t.order_key).max();
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

        if !tool.arguments_buffer.is_empty() {
            self.emit_tool_json_delta(output, idx, tool.arguments_buffer.clone());
        }

        output.push(format!(
            "event: content_block_stop\ndata: {}\n\n",
            json!({ "type": "content_block_stop", "index": idx })
        ));
    }

    fn flush_serialized_tool_calls(&mut self, output: &mut Vec<String>, include_incomplete: bool) {
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
            self.emit_serialized_tool_call(output, &tool);
            self.cleanup_tool_mappings(&tool);
        }
        self.sync_phase_from_runtime();
    }

    fn open_thinking_block_if_needed(&mut self, output: &mut Vec<String>) {
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
        if delta.is_empty() {
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

    fn dedupe_cross_source_fragment(
        &mut self,
        source: TextEventSource,
        fragment: &str,
    ) -> Option<String> {
        if fragment.is_empty() {
            return None;
        }

        let mut deduped = fragment;
        if let Some(last_source) = self.last_text_source {
            if last_source != source {
                if fragment == self.last_text_fragment {
                    return None;
                }
                if !self.last_text_fragment.is_empty()
                    && fragment.starts_with(&self.last_text_fragment)
                {
                    deduped = &fragment[self.last_text_fragment.len()..];
                }
            }
        }

        if deduped.is_empty() {
            return None;
        }

        let deduped_owned = deduped.to_string();
        self.last_text_source = Some(source);
        self.last_text_fragment.clear();
        self.last_text_fragment.push_str(&deduped_owned);
        Some(deduped_owned)
    }

    fn reset_text_dedupe_state(&mut self) {
        self.last_text_source = None;
        self.last_text_fragment.clear();
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
                    let cleaned_remaining = Self::strip_suspicious_trailing_noise(remaining);
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

        if let Some((marker_start, marker_len)) = Self::find_markdown_bash_start(&combined) {
            let prefix_text = &combined[..marker_start];
            let after_marker = &combined[marker_start + marker_len..];

            if emit_plain_text && !self.has_open_tool_block() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
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

        if let Some(marker_start) = Self::find_potential_leaked_tool_marker_start(&combined) {
            let (prefix_text, leaked_fragment) = combined.split_at(marker_start);
            if emit_plain_text && !self.has_open_tool_block() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.process_pending_tool_text(output, !emit_plain_text);
            return;
        }

        // 某些泄漏不带 `assistant to=`/`to=` 前缀，而是直接混入工具参数 JSON。
        // 将裸 JSON 片段送入 pending，按高置信规则分段抑制，仅保留前后安全文本。
        if let Some(raw_json_start) = Self::find_potential_raw_tool_json_start(&combined) {
            let (prefix_text, leaked_fragment) = combined.split_at(raw_json_start);
            if emit_plain_text && !self.has_open_tool_block() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.process_pending_tool_text(output, !emit_plain_text);
            return;
        }

        // 检查上下文 note-json 泄漏
        let context = format!("{}{}", self.text_carryover, &combined);
        if let Some((prefix, _, _suffix, is_cross_chunk)) =
            Self::split_contextual_note_json_prefix_suffix(&combined, &context)
        {
            // 对于上下文泄漏，只保留前缀中的安全部分，完全抑制 JSON 和可疑尾巴
            if is_cross_chunk {
                self.suppressing_cross_chunk_leak = true;
            }
            if emit_plain_text && !self.has_open_tool_block() && !prefix.is_empty() {
                self.emit_plain_text_fragment(output, &prefix);
            }
            // 注意：不处理 suffix，因为它包含可疑的尾巴噪声，应该被完全抑制
            return;
        }

        if !emit_plain_text || self.has_open_tool_block() {
            return;
        }

        let carry_len = Self::leaked_marker_suffix_len(&combined);
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

    fn emit_plain_text_fragment(&mut self, output: &mut Vec<String>, fragment: &str) {
        if fragment.is_empty() {
            return;
        }

        let normalized_fragment = Self::collapse_adjacent_duplicate_markdown_bold(fragment);
        if normalized_fragment.is_empty() {
            return;
        }
        let fragment = normalized_fragment.as_str();

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
            done_flag: true,
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

        if let Some((marker_start, marker_len)) = Self::find_markdown_bash_start(&carryover) {
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

        if let Some(marker_start) = Self::find_potential_leaked_tool_marker_start(&carryover) {
            let (prefix_text, leaked_fragment) = carryover.split_at(marker_start);
            if !self.has_open_tool_block() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.process_pending_tool_text(output, true);
            return;
        }

        // 检查高置信工具参数泄漏
        if let Some((prefix, _, suffix)) = Self::split_tool_json_prefix_suffix(&carryover) {
            self.logger
                .log_raw("[Warn] Dropping raw leaked tool json from carryover");
            if !self.has_open_tool_block() && !prefix.is_empty() {
                self.emit_plain_text_fragment(output, &prefix);
            }
            if !suffix.is_empty() {
                self.handle_text_fragment(output, &suffix, true);
            }
            return;
        }

        // 检查上下文 note-json 泄漏（新增）
        if let Some((prefix, _, suffix, is_cross_chunk)) =
            Self::split_contextual_note_json_prefix_suffix(&carryover, &carryover)
        {
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

        if let Some(raw_json_start) = Self::find_potential_raw_tool_json_start(&carryover) {
            let (prefix_text, leaked_fragment) = carryover.split_at(raw_json_start);
            if !self.has_open_tool_block() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
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
                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                let Some(delta) =
                    self.dedupe_cross_source_fragment(TextEventSource::OutputTextDelta, delta)
                else {
                    return output;
                };
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
                let redirect_to_thinking = self.in_commentary_phase
                    || (self.had_reasoning_in_response && !self.saw_message_item_added);
                if !redirect_to_thinking {
                    self.close_open_thinking_block(&mut output);
                }
                if let Some(fragment) = Self::extract_content_part_text(&data) {
                    if let Some(text) = self
                        .dedupe_cross_source_fragment(TextEventSource::ContentPartAdded, fragment)
                    {
                        self.handle_text_fragment(&mut output, &text, true);
                    }
                }
            }

            // 文本分片结束：如果 pending 里还有疑似工具泄露，立即按边界强制 flush
            "response.output_text.done" => {
                self.flush_text_carryover(&mut output);
                if let Some(done_text) = data.get("text").and_then(|t| t.as_str()) {
                    self.handle_text_fragment(&mut output, done_text, false);
                }
                if !self.pending_tool_text.is_empty() {
                    self.flush_pending_tool_text(&mut output);
                }
                self.reset_text_dedupe_state();
            }

            "response.content_part.done" => {
                self.flush_text_carryover(&mut output);
                if !self.pending_tool_text.is_empty() {
                    self.flush_pending_tool_text(&mut output);
                }
                self.reset_text_dedupe_state();
            }

            // 推理摘要分片：映射为 Anthropic thinking 增量事件，避免长阶段无可见流输出
            "response.reasoning_summary_part.added" => {
                self.had_reasoning_in_response = true;
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                self.open_thinking_block_if_needed(&mut output);
            }

            "response.reasoning_summary_text.delta" => {
                self.had_reasoning_in_response = true;
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

            // 工具调用开始 / 消息项开始 - 严格按照 OpenAI Responses 格式解析
            "response.output_item.added" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                if let Some(item) = data.get("item") {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

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
                            let output_index = data.get("output_index").and_then(|v| v.as_u64());
                            let item_id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            self.buffer_tool_call(output_index, item_id, call_id.clone(), name);

                            if let Some(arguments) = item.get("arguments").and_then(|v| v.as_str())
                            {
                                if !arguments.is_empty() {
                                    if let Some(order_key) = self
                                        .find_buffered_tool_order_from_metadata(
                                            output_index,
                                            item.get("id").and_then(|v| v.as_str()),
                                            Some(call_id.as_str()),
                                        )
                                    {
                                        self.apply_tool_arguments_snapshot(order_key, arguments);
                                    }
                                }
                            }
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
                        _ => {}
                    }
                }
            }

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
                        self.append_tool_arguments_delta(order_key, delta);
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
                        self.apply_tool_arguments_snapshot(order_key, full_arguments);
                    }
                }
            }

            // 工具调用完成 / 消息项完成
            "response.output_item.done" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                self.reset_text_dedupe_state();

                let output_index = data.get("output_index").and_then(|v| v.as_u64());
                let item = data.get("item");
                let item_type = item
                    .and_then(|it| it.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let item_id = item.and_then(|it| it.get("id")).and_then(|v| v.as_str());

                match item_type {
                    "function_call" => {
                        if let Some(full_arguments) = item
                            .and_then(|it| it.get("arguments"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(order_key) = self.find_buffered_tool_order_from_metadata(
                                output_index,
                                item_id,
                                item.and_then(|it| it.get("call_id"))
                                    .and_then(|v| v.as_str()),
                            ) {
                                self.apply_tool_arguments_snapshot(order_key, full_arguments);
                            }
                        }

                        if let Some(order_key) = self.find_buffered_tool_order_from_metadata(
                            output_index,
                            item_id,
                            item.and_then(|it| it.get("call_id"))
                                .and_then(|v| v.as_str()),
                        ) {
                            self.mark_buffered_tool_done(order_key);
                        } else if let Some(order_key) = self.last_buffered_tool_order {
                            // 兼容旧流：item 元数据缺失时回退到最近一个缓冲工具调用
                            self.mark_buffered_tool_done(order_key);
                        }
                        self.flush_serialized_tool_calls(&mut output, false);
                    }
                    "message" => {
                        self.in_commentary_phase = false;
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
                            self.flush_serialized_tool_calls(&mut output, false);
                        }
                    }
                }
            }

            // 响应完成 - 关键：确保完整的事件序列
            "response.completed" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.flush_markdown_bash(&mut output);
                self.reset_text_dedupe_state();

                // 关闭所有打开的块
                self.close_open_text_block(&mut output);
                self.close_open_thinking_block(&mut output);
                self.flush_serialized_tool_calls(&mut output, true);

                // 确定停止原因
                let stop_reason = if self.saw_tool_call {
                    "tool_use"
                } else if data
                    .get("response")
                    .and_then(|r| r.get("status"))
                    .and_then(|s| s.as_str())
                    == Some("incomplete")
                {
                    "max_tokens"
                } else {
                    "end_turn"
                };

                // 提取使用统计
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

                // 发送 message_delta（包含停止原因和使用统计）
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

                // 发送 message_stop（完成流式响应）
                output.push(format!(
                    "event: message_stop\ndata: {}\n\n",
                    json!({ "type": "message_stop" })
                ));
                self.transition_to(StreamPhase::Terminal);
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
}

#[cfg(test)]
mod tests;
