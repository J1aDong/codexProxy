use crate::logger::AppLogger;
use crate::transform::ResponseTransformer;
use serde_json::{json, Value};

/// 响应转换器 - Codex SSE -> Anthropic SSE
pub struct TransformResponse {
    message_id: String,
    model: String,
    content_index: usize,
    open_text_index: Option<usize>,
    open_thinking_index: Option<usize>,
    open_tool_index: Option<usize>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    saw_tool_call: bool,
    sent_message_start: bool,
    text_carryover: String,
    pending_tool_text: String,
    accumulated_tool_args: String,
    logger: std::sync::Arc<AppLogger>,
}

impl TransformResponse {
    const LEAKED_TOOL_MARKERS: [&'static str; 3] =
        ["assistant to=", "to=functions", "to=multi_tool_use"];

    // 兼容不同工具命名来源（Anthropic tool 名称 / Codex 泄露文本名称）。
    // 优先走显式映射，避免语义漂移；未命中时再回退到 `functions.` 前缀剥离。
    fn leaked_tool_name_compat_alias(name: &str) -> Option<&'static str> {
        match name {
            "functions.Write" => Some("Write"),
            "functions.Edit" => Some("Edit"),
            "functions.Read" => Some("Read"),
            "functions.Bash" => Some("Bash"),
            "functions.Grep" => Some("Grep"),
            "functions.Glob" => Some("Glob"),
            "functions.Task" => Some("Task"),
            "functions.WebSearch" => Some("WebSearch"),
            "functions.WebFetch" => Some("WebFetch"),
            "functions.TodoRead" => Some("TodoRead"),
            "functions.TodoWrite" => Some("TodoWrite"),
            "functions.apply_patch" => Some("apply_patch"),
            _ => None,
        }
    }

    fn normalize_leaked_tool_name(target: &str) -> String {
        if let Some(mapped) = Self::leaked_tool_name_compat_alias(target) {
            return mapped.to_string();
        }

        target
            .strip_prefix("functions.")
            .filter(|name| !name.is_empty())
            .unwrap_or(target)
            .to_string()
    }

    fn extract_leaked_tool_target(line: &str) -> Option<String> {
        // Prefer explicit assistant leak marker, but also tolerate bare `to=...` leaks.
        let candidate = if let Some(start) = line.find("assistant to=") {
            let offset = start + "assistant to=".len();
            line.get(offset..)?.trim()
        } else if let Some(start) = line.find("to=") {
            let offset = start + "to=".len();
            line.get(offset..)?.trim()
        } else {
            return None;
        };

        let target: String = candidate
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '.' || *ch == '-')
            .collect();

        if target.is_empty() {
            return None;
        }

        if target == "multi_tool_use.parallel" || target.starts_with("functions.") {
            Some(target)
        } else {
            None
        }
    }

    fn find_potential_leaked_tool_marker_start(line: &str) -> Option<usize> {
        Self::LEAKED_TOOL_MARKERS
            .iter()
            .filter_map(|marker| line.find(marker))
            .min()
    }

    fn leaked_marker_suffix_len(line: &str) -> usize {
        let bytes = line.as_bytes();
        let mut max_len = 0usize;

        for marker in Self::LEAKED_TOOL_MARKERS {
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

    fn starts_with_leaked_tool_marker(line: &str) -> bool {
        let trimmed = line.trim_start();
        trimmed.starts_with("assistant to=")
            || trimmed.starts_with("to=functions")
            || trimmed.starts_with("to=multi_tool_use")
    }

    pub fn new(model: &str) -> Self {
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            content_index: 0,
            open_text_index: None,
            open_thinking_index: None,
            open_tool_index: None,
            tool_call_id: None,
            tool_name: None,
            saw_tool_call: false,
            sent_message_start: false,
            text_carryover: String::new(),
            pending_tool_text: String::new(),
            accumulated_tool_args: String::new(),
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
    }

    fn close_open_thinking_block(&mut self, output: &mut Vec<String>) {
        if let Some(idx) = self.open_thinking_index.take() {
            output.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
    }

    fn open_thinking_block_if_needed(&mut self, output: &mut Vec<String>) {
        if self.open_tool_index.is_some() {
            return;
        }

        if self.open_thinking_index.is_some() {
            return;
        }

        self.close_open_text_block(output);

        let idx = self.content_index;
        self.content_index += 1;
        self.open_thinking_index = Some(idx);
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

    fn handle_text_fragment(
        &mut self,
        output: &mut Vec<String>,
        fragment: &str,
        emit_plain_text: bool,
    ) {
        if fragment.is_empty() {
            return;
        }

        let combined = if self.text_carryover.is_empty() {
            fragment.to_string()
        } else {
            let mut merged = std::mem::take(&mut self.text_carryover);
            merged.push_str(fragment);
            merged
        };

        // 泄漏工具调用文本可能被拆成多个 chunk。
        // 一旦进入泄漏拼接模式，后续 chunk 持续进入 pending，直到遇到换行/收尾再统一解析。
        if !self.pending_tool_text.is_empty() {
            self.pending_tool_text.push_str(&combined);
            if combined.contains('\n') {
                self.flush_pending_tool_text(output);
            }
            return;
        }

        if let Some(marker_start) = Self::find_potential_leaked_tool_marker_start(&combined) {
            let (prefix_text, leaked_fragment) = combined.split_at(marker_start);
            if emit_plain_text && self.open_tool_index.is_none() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
            }
            self.pending_tool_text.push_str(leaked_fragment);
            if leaked_fragment.contains('\n') || !emit_plain_text {
                self.flush_pending_tool_text(output);
            }
            return;
        }

        if !emit_plain_text || self.open_tool_index.is_some() {
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
    }

    fn open_tool_block_if_needed(
        &mut self,
        output: &mut Vec<String>,
        call_id: String,
        name: String,
    ) {
        self.saw_tool_call = true;
        self.close_open_text_block(output);

        if self.open_tool_index.is_some() {
            return;
        }

        self.tool_call_id = Some(call_id.clone());
        self.tool_name = Some(name.clone());

        let idx = self.content_index;
        self.content_index += 1;
        self.open_tool_index = Some(idx);

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
    }

    fn emit_tool_json_delta(&self, output: &mut Vec<String>, delta: String) {
        if let Some(idx) = self.open_tool_index {
            output.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": idx,
                    "delta": { "type": "input_json_delta", "partial_json": delta }
                })
            ));
        }
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

    fn parse_leaked_parallel_tool_calls(line: &str) -> Option<Vec<(String, String)>> {
        let payload_fragment = Self::extract_first_json_object_fragment(line)?;
        let payload = serde_json::from_str::<Value>(&payload_fragment).ok()?;
        let tool_uses = payload.get("tool_uses")?.as_array()?;

        let mut calls = Vec::new();
        for tool_use in tool_uses {
            let Some(recipient_name) = tool_use.get("recipient_name").and_then(|v| v.as_str())
            else {
                continue;
            };

            let name = Self::normalize_leaked_tool_name(recipient_name);
            let parameters = tool_use
                .get("parameters")
                .cloned()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| json!({}));
            let arguments = serde_json::to_string(&parameters).unwrap_or_else(|_| "{}".to_string());
            calls.push((name, arguments));
        }

        Some(calls)
    }

    fn parse_leaked_tool_line(line: &str) -> Option<Vec<(String, String)>> {
        let target = Self::extract_leaked_tool_target(line)?;

        if target == "multi_tool_use.parallel" {
            return Self::parse_leaked_parallel_tool_calls(line);
        }

        let arguments = if line.contains('{') {
            let candidate = Self::extract_first_json_object_fragment(line)?;
            if serde_json::from_str::<Value>(&candidate).is_ok() {
                candidate
            } else {
                return None;
            }
        } else {
            "{}".to_string()
        };

        let name = Self::normalize_leaked_tool_name(&target);
        Some(vec![(name, arguments)])
    }

    fn emit_leaked_tool_calls(&mut self, output: &mut Vec<String>, calls: Vec<(String, String)>) {
        for (idx, (name, arguments)) in calls.into_iter().enumerate() {
            let call_id = format!("tool_{}_{}", chrono::Utc::now().timestamp_millis(), idx);
            self.open_tool_block_if_needed(output, call_id, name);
            self.emit_tool_json_delta(output, arguments);

            if let Some(block_idx) = self.open_tool_index.take() {
                output.push(format!(
                    "event: content_block_stop\ndata: {}\n\n",
                    json!({ "type": "content_block_stop", "index": block_idx })
                ));
            }
            self.tool_call_id = None;
            self.tool_name = None;
        }
    }

    fn flush_text_carryover(&mut self, output: &mut Vec<String>) {
        if self.text_carryover.is_empty() {
            return;
        }

        let carryover = std::mem::take(&mut self.text_carryover);
        if let Some(marker_start) = Self::find_potential_leaked_tool_marker_start(&carryover) {
            let (prefix_text, leaked_fragment) = carryover.split_at(marker_start);
            if self.open_tool_index.is_none() && !prefix_text.is_empty() {
                self.emit_plain_text_fragment(output, prefix_text);
            }
            self.pending_tool_text.push_str(leaked_fragment);
            self.flush_pending_tool_text(output);
            return;
        }

        if self.open_tool_index.is_none() {
            self.emit_plain_text_fragment(output, &carryover);
        }
    }

    fn flush_pending_tool_text(&mut self, output: &mut Vec<String>) {
        if self.pending_tool_text.trim().is_empty() {
            self.pending_tool_text.clear();
            return;
        }

        let pending_raw = std::mem::take(&mut self.pending_tool_text);
        let pending_for_tool_parse = pending_raw.trim();

        // 检查是否是泄漏的工具调用
        if let Some(calls) = Self::parse_leaked_tool_line(pending_for_tool_parse) {
            if calls.is_empty() {
                self.logger.log_raw(
                    "[Warn] Dropping leaked multi_tool_use.parallel with no valid tool_uses",
                );
                return;
            }
            // 关闭文本块（如果有）
            self.close_open_text_block(output);
            self.emit_leaked_tool_calls(output, calls);
            return;
        }

        // 疑似泄漏但解析失败时，不回落到可见文本，避免把 tool 片段/乱码暴露给客户端。
        if Self::starts_with_leaked_tool_marker(pending_for_tool_parse) {
            self.logger.log_raw(
                "[Warn] Dropping unparsable leaked tool marker fragment from visible text",
            );
            return;
        }

        // 如果不是工具调用，作为普通文本处理
        if !pending_raw.trim().is_empty() {
            self.emit_plain_text_fragment(output, &pending_raw);
        }
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
                self.close_open_thinking_block(&mut output);
                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                self.handle_text_fragment(&mut output, delta, true);
            }

            // 新版事件：内容分片直接挂在 content_part.added
            "response.content_part.added" => {
                self.close_open_thinking_block(&mut output);
                if let Some(text) = Self::extract_content_part_text(&data) {
                    self.handle_text_fragment(&mut output, text, true);
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
            }

            "response.content_part.done" => {
                self.flush_text_carryover(&mut output);
                if !self.pending_tool_text.is_empty() {
                    self.flush_pending_tool_text(&mut output);
                }
            }

            // 推理摘要分片：映射为 Anthropic thinking 增量事件，避免长阶段无可见流输出
            "response.reasoning_summary_part.added" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                self.open_thinking_block_if_needed(&mut output);
            }

            "response.reasoning_summary_text.delta" => {
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

            // 工具调用开始 - 严格按照 OpenAI Responses 格式解析
            "response.output_item.added" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                if let Some(item) = data.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                        // 关闭文本块（如果有）
                        self.close_open_text_block(&mut output);

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

                        self.open_tool_block_if_needed(&mut output, call_id, name);
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

                if !delta.is_empty() && self.open_tool_index.is_some() {
                    self.accumulated_tool_args.push_str(delta);
                    output.push(format!(
                        "event: content_block_delta\ndata: {}\n\n",
                        json!({
                            "type": "content_block_delta",
                            "index": self.open_tool_index,
                            "delta": { "type": "input_json_delta", "partial_json": delta }
                        })
                    ));
                }
            }

            // 工具调用完成
            "response.output_item.done" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);
                self.close_open_thinking_block(&mut output);
                if let Some(idx) = self.open_tool_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }
                self.tool_call_id = None;
                self.tool_name = None;
                self.accumulated_tool_args.clear();
            }

            // 响应完成 - 关键：确保完整的事件序列
            "response.completed" => {
                self.flush_text_carryover(&mut output);
                self.flush_pending_tool_text(&mut output);

                // 关闭所有打开的块
                if let Some(idx) = self.open_text_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }
                if let Some(idx) = self.open_thinking_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }
                if let Some(idx) = self.open_tool_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }

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
