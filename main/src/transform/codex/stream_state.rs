//! 流式状态机核心（D2）
//!
//! 用 `content_blocks: HashMap<output_index, ContentBlockState>` 单一索引跟踪流式 block，
//! 替换旧的 6 HashMap + 4 级回退查询。处理 Codex Responses 事件，产出 Anthropic SSE 事件字符串。
//!
//! 本模块是 D6 绞杀式迁移的新状态机，尚未接入主路径。接入前 allow(dead_code)。

#![allow(dead_code)]

use std::collections::HashMap;

use serde_json::json;

use super::event::{OutputItem, ResponsesEvent};
use super::inspectors::{background_agent, leak};
use super::reasoning::ActiveReasoning;

/// plan_bridge 标签
const PROPOSED_PLAN_OPEN_TAG: &str = "<proposed_plan>";
const PROPOSED_PLAN_CLOSE_TAG: &str = "</proposed_plan>";

/// 单个 content block 的状态
#[derive(Debug, Clone)]
pub struct ContentBlockState {
    /// block 类型
    pub kind: BlockKind,
    /// 回传引用键（`call_` 前缀），对应 Anthropic `tool_use.id`
    pub call_id: Option<String>,
    /// item 自身标识（`fc_` 前缀）
    pub item_id: Option<String>,
    /// 累积的增量（tool arguments / text / refusal）
    pub buffer: String,
    /// 对外的 content_block index（Anthropic 侧）
    pub block_index: Option<usize>,
    /// tool 名（function_call 用）
    pub tool_name: Option<String>,
    /// 是否已对外发出 content_block_start
    pub started: bool,
    /// 是否已对外发出 content_block_stop
    pub closed: bool,
    /// tool arguments 已通过 input_json_delta 发出的字节数（防止 close 时重复发包）
    pub emitted_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Text,
    Thinking,
    ToolUse,
    Refusal,
}

/// 活跃的 web_search 调用状态
#[derive(Debug, Clone)]
struct WebSearchCall {
    content_block_index: usize,
    output_index: Option<u32>,
    item_id: Option<String>,
    input_closed: bool,
}

/// 新状态机：仅用 output_index 单一索引，无多级回退
pub struct StreamState {
    /// output_index -> block 状态（单一索引，替换旧 6 HashMap）
    content_blocks: HashMap<u32, ContentBlockState>,
    /// item_id -> output_index（仅用于 reasoning 等按 item_id 聚合的场景）
    reasoning_by_item_id: HashMap<String, ActiveReasoning>,
    /// 下一个 Anthropic content_block index
    next_block_index: usize,
    /// 是否已发出 message_start
    sent_message_start: bool,
    /// 当前打开的 text block 的 output_index（Anthropic 同一时刻只开一个 text block）
    open_text_output_index: Option<u32>,
    /// 当前打开的 thinking block 的 output_index
    open_thinking_output_index: Option<u32>,
    /// model 名（用于 message_start）
    model: String,
    /// message id
    message_id: String,
    /// store 是否启用（影响 reasoning 三态机）
    store_enabled: bool,
    /// 累积的 usage
    usage: Option<UsageAccum>,
    /// 活跃的 web_search 调用：server_tool_use_id -> (content_block_index, output_index, item_id)
    /// 用单一映射取代旧的 web_search_call_by_output_index + web_search_call_by_item_id 双 HashMap
    active_web_search: HashMap<String, WebSearchCall>,
    /// output_index / item_id -> server_tool_use_id（用于查找）
    web_search_by_output_index: HashMap<u32, String>,
    web_search_by_item_id: HashMap<String, String>,
    /// 下一个 server_tool_use 序号
    next_server_tool_use_seq: u64,
    /// commentary 阶段：message item 的 phase=="commentary" 时，后续文本重定向到 thinking
    in_commentary_phase: bool,
    /// 是否在本响应中见过 reasoning（用于无显式 phase 时的回退判定）
    had_reasoning: bool,
    /// 是否见过 message output_item.added（用于 commentary 回退判定）
    saw_message_item_added: bool,
    /// markdown_bash 钩子：检测到 ```bash/sh/shell 标记时进入缓冲模式
    in_markdown_bash: bool,
    markdown_bash_buffer: String,
    /// plan_bridge 钩子：捕获 <proposed_plan> body，配置了 plan_file_path 时抑制可见文本
    capturing_proposed_plan: bool,
    proposed_plan_body_buffer: String,
    latest_proposed_plan_body: Option<String>,
    codex_plan_file_path: Option<String>,
    plan_bridge_emitted: bool,
}

#[derive(Debug, Clone, Default)]
struct UsageAccum {
    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    cache_write: u64,
}

impl StreamState {
    pub fn new(model: &str) -> Self {
        Self {
            content_blocks: HashMap::new(),
            reasoning_by_item_id: HashMap::new(),
            next_block_index: 0,
            sent_message_start: false,
            open_text_output_index: None,
            open_thinking_output_index: None,
            model: model.to_string(),
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            store_enabled: true,
            usage: None,
            active_web_search: HashMap::new(),
            web_search_by_output_index: HashMap::new(),
            web_search_by_item_id: HashMap::new(),
            next_server_tool_use_seq: 0,
            in_commentary_phase: false,
            had_reasoning: false,
            saw_message_item_added: false,
            in_markdown_bash: false,
            markdown_bash_buffer: String::new(),
            capturing_proposed_plan: false,
            proposed_plan_body_buffer: String::new(),
            latest_proposed_plan_body: None,
            codex_plan_file_path: None,
            plan_bridge_emitted: false,
        }
    }

    pub fn set_store_enabled(&mut self, enabled: bool) {
        self.store_enabled = enabled;
    }

    /// 配置 plan_bridge 上下文：设置 plan_file_path 启用 proposed_plan 捕获与抑制
    pub fn set_plan_file_path(&mut self, path: Option<String>) {
        self.codex_plan_file_path = path;
    }

    /// 从 ResponseTransformRequestContext 配置上下文（对接旧 ResponseTransformer trait）
    pub fn configure_request_context(&mut self, ctx: &crate::transform::ResponseTransformRequestContext) {
        self.codex_plan_file_path = ctx.codex_plan_file_path.clone();
        // background_agent 相关上下文待 background_agent 钩子实现时接入
    }

    /// 处理一个事件，产出零或多条 Anthropic SSE 字符串
    pub fn handle_event(&mut self, event: ResponsesEvent) -> Vec<String> {
        let mut out = Vec::new();
        match event {
            ResponsesEvent::ResponseCreated { .. } | ResponsesEvent::ResponseInProgress { .. } => {
                self.ensure_message_start(&mut out);
            }
            ResponsesEvent::OutputItemAdded { output_index, item } => {
                self.on_output_item_added(output_index, item, &mut out);
            }
            ResponsesEvent::OutputItemDone { output_index, item } => {
                self.on_output_item_done(output_index, item, &mut out);
            }
            ResponsesEvent::OutputTextDelta { output_index, delta, .. } => {
                self.on_text_delta(output_index, &delta, &mut out);
            }
            ResponsesEvent::OutputTextDone { output_index, text, .. } => {
                self.on_text_done(output_index, &text, &mut out);
            }
            ResponsesEvent::FunctionCallArgumentsDelta { output_index, delta, .. } => {
                self.on_function_args_delta(output_index, &delta, &mut out);
            }
            ResponsesEvent::FunctionCallArgumentsDone { output_index, arguments, item_id, .. } => {
                self.on_function_args_done(output_index, item_id, &arguments, &mut out);
            }
            ResponsesEvent::ReasoningSummaryPartAdded { item_id, output_index, summary_index } => {
                self.on_reasoning_part_added(&item_id, output_index, summary_index, &mut out);
            }
            ResponsesEvent::ReasoningSummaryTextDelta { item_id, output_index, delta, .. } => {
                self.on_reasoning_delta(&item_id, output_index, &delta, &mut out);
            }
            ResponsesEvent::ReasoningSummaryPartDone { item_id, output_index, summary_index } => {
                self.on_reasoning_part_done(&item_id, output_index, summary_index);
            }
            ResponsesEvent::ReasoningSummaryTextDone { item_id, output_index, text, .. } => {
                self.on_reasoning_text_done(&item_id, output_index, &text);
            }
            ResponsesEvent::RefusalDelta { output_index, delta, .. } => {
                self.on_refusal_delta(output_index, &delta, &mut out);
            }
            ResponsesEvent::RefusalDone { output_index, refusal, .. } => {
                self.on_refusal_done(output_index, &refusal, &mut out);
            }
            ResponsesEvent::ContentPartAdded { output_index, part, .. } => {
                // 从 content_part.added 提取文本（替代旧 extract_content_part_text）
                if let crate::transform::codex::event::MessageContent::OutputText { text, .. } = part
                {
                    self.on_text_delta(output_index, &text, &mut out);
                }
            }
            ResponsesEvent::WebSearchCallInProgress { output_index, item_id }
            | ResponsesEvent::WebSearchCallSearching { output_index, item_id } => {
                self.on_web_search_call_start(output_index, item_id, &mut out);
            }
            ResponsesEvent::WebSearchCallCompleted { output_index, item_id, .. } => {
                self.on_web_search_call_complete(output_index, item_id, &mut out);
            }
            ResponsesEvent::ContentPartDone { .. } | ResponsesEvent::Unknown => {
                // 不影响状态机推进
            }
            ResponsesEvent::ResponseCompleted { response } => {
                if let Some(u) = response.usage {
                    self.usage = Some(UsageAccum {
                        input_tokens: u.input_tokens,
                        output_tokens: u.output_tokens,
                        cache_read: u.input_tokens_details.map(|d| d.cached_tokens).unwrap_or(0),
                        cache_write: 0,
                    });
                }
                self.emit_message_stop(&mut out);
            }
            ResponsesEvent::ResponseFailed { .. } | ResponsesEvent::Error { .. } => {
                self.emit_error_and_stop(&mut out);
            }
            ResponsesEvent::ResponseIncomplete { .. } => {
                self.emit_message_stop(&mut out);
            }
        }
        out
    }

    fn ensure_message_start(&mut self, out: &mut Vec<String>) {
        if !self.sent_message_start {
            out.push(format!(
                "event: message_start\ndata: {}\n\n",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": self.message_id,
                        "type": "message",
                        "role": "assistant",
                        "model": self.model,
                        "content": [],
                        "stop_reason": null,
                        "stop_sequence": null,
                        "usage": { "input_tokens": 0, "output_tokens": 0 }
                    }
                })
            ));
            self.sent_message_start = true;
        }
    }

    fn on_output_item_added(&mut self, output_index: u32, item: OutputItem, out: &mut Vec<String>) {
        self.ensure_message_start(out);
        match item {
            OutputItem::Message { phase, .. } => {
                self.saw_message_item_added = true;
                // commentary 阶段：phase=="commentary" 时，后续文本重定向到 thinking
                self.in_commentary_phase = phase.as_deref() == Some("commentary");
            }
            OutputItem::FunctionCall { call_id, name, .. } => {
                let block = ContentBlockState {
                    kind: BlockKind::ToolUse,
                    call_id: Some(call_id),
                    item_id: None,
                    buffer: String::new(),
                    block_index: None,
                    tool_name: Some(name),
                    started: false,
                    closed: false,
                    emitted_len: 0,
                };
                self.content_blocks.insert(output_index, block);
            }
            OutputItem::Reasoning { id, encrypted_content, .. } => {
                self.had_reasoning = true;
                let r = self
                    .reasoning_by_item_id
                    .entry(id.clone())
                    .or_insert_with(ActiveReasoning::new);
                if let Some(enc) = encrypted_content {
                    r.encrypted_content = Some(enc);
                }
                // thinking block 在首个 delta 时开启
                let block = ContentBlockState {
                    kind: BlockKind::Thinking,
                    call_id: None,
                    item_id: Some(id),
                    buffer: String::new(),
                    block_index: None,
                    tool_name: None,
                    started: false,
                    closed: false,
                    emitted_len: 0,
                };
                self.content_blocks.insert(output_index, block);
            }
            OutputItem::FunctionCallOutput { .. } | OutputItem::Unknown => {}
        }
    }

    fn on_output_item_done(&mut self, output_index: u32, item: OutputItem, out: &mut Vec<String>) {
        // 回填 call_id / item_id（delta 事件可能没带）
        if let Some(block) = self.content_blocks.get_mut(&output_index) {
            match &item {
                OutputItem::FunctionCall { call_id, id, name, .. } => {
                    if block.call_id.is_none() {
                        block.call_id = Some(call_id.clone());
                    }
                    if block.item_id.is_none() {
                        block.item_id = Some(id.clone());
                    }
                    if block.tool_name.is_none() {
                        block.tool_name = Some(name.clone());
                    }
                }
                OutputItem::Reasoning { id, encrypted_content, .. } => {
                    if block.item_id.is_none() {
                        block.item_id = Some(id.clone());
                    }
                    if let Some(r) = self.reasoning_by_item_id.get_mut(id) {
                        r.conclude_with(encrypted_content.clone());
                    }
                }
                _ => {}
            }
        }
        // reasoning: 若已 conclude，关闭 thinking block
        if let OutputItem::Reasoning { id, .. } = &item {
            if let Some(r) = self.reasoning_by_item_id.get(id) {
                if r.is_concluded() {
                    self.close_thinking_block(output_index, out);
                }
            }
        }
        // function_call: 若 arguments 已在 done 中给出且未开块，开并关
        if let OutputItem::FunctionCall { arguments, call_id, name, .. } = &item {
            if let Some(block) = self.content_blocks.get_mut(&output_index) {
                if !block.started {
                    // 单 chunk 完成的 tool call
                    block.buffer = arguments.clone();
                    block.call_id = Some(call_id.clone());
                    block.tool_name = Some(name.clone());
                }
            }
            self.close_tool_block(output_index, out);
        }
    }

    fn on_text_delta(&mut self, output_index: u32, delta: &str, out: &mut Vec<String>) {
        self.ensure_message_start(out);
        // leak 钩子：抑制泄漏的 tool 调用标记（如 "assistant to=functions ..."）
        // 这是旧 LeakDetector 的核心护栏之一，作为 side-channel 挂载在文本输出路径
        if leak::starts_with_leaked_tool_marker(delta) {
            return;
        }
        // plan_bridge 钩子：捕获 <proposed_plan> body；配置 plan_file_path 时抑制可见文本
        let filtered = self.observe_proposed_plan_fragment(delta);
        let delta = match filtered {
            Some(d) if !d.is_empty() => d,
            _ => return, // 被抑制或过滤后为空
        };
        // background_agent 钩子：检测 <task-notification> 等生命周期文本，重定向到 thinking_delta
        if let Some(progress_msg) = background_agent::build_task_lifecycle_progress_message(&delta)
        {
            self.emit_thinking_text_delta(output_index, &progress_msg, out);
            self.emit_thinking_text_delta(output_index, "\n", out);
            return;
        }
        // commentary 钩子：commentary 阶段或（见过 reasoning 且未见 message item）时，
        // 文本重定向到 thinking block（旧 CommentaryPhase 行为）
        let redirect_to_thinking = self.in_commentary_phase
            || (self.had_reasoning && !self.saw_message_item_added);
        if redirect_to_thinking {
            self.emit_thinking_text_delta(output_index, &delta, out);
            return;
        }
        // markdown_bash 钩子：检测 ```bash/sh/shell 代码块，缓冲到块结束
        let mb_handled = self.handle_markdown_bash(&delta, out);
        if mb_handled {
            return;
        }
        // 关闭可能打开的 thinking block（text 与 thinking 互斥）
        let need_close = self.open_thinking_output_index.take();
        if let Some(think_idx) = need_close {
            if think_idx != output_index {
                self.close_thinking_block(think_idx, out);
            }
        }
        let block = self
            .content_blocks
            .entry(output_index)
            .or_insert_with(|| ContentBlockState {
                kind: BlockKind::Text,
                call_id: None,
                item_id: None,
                buffer: String::new(),
                block_index: None,
                tool_name: None,
                started: false,
                closed: false,
                emitted_len: 0,
            });
        if !block.started {
            let idx = self.next_block_index;
            self.next_block_index += 1;
            block.block_index = Some(idx);
            block.started = true;
            self.open_text_output_index = Some(output_index);
            out.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({ "type": "content_block_start", "index": idx, "content_block": { "type": "text", "text": "" } })
            ));
        }
        block.buffer.push_str(&delta);
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "text_delta", "text": &delta } })
            ));
        }
    }

    fn on_text_done(&mut self, output_index: u32, _text: &str, out: &mut Vec<String>) {
        self.close_text_block(output_index, out);
    }

    fn on_function_args_delta(&mut self, output_index: u32, delta: &str, out: &mut Vec<String>) {
        // 先关闭可能打开的 text/thinking block（避免与 content_blocks 借用冲突）
        let need_close_text = self.open_text_output_index.take();
        let need_close_thinking = self.open_thinking_output_index.take();
        if let Some(t) = need_close_text {
            if t != output_index {
                self.close_text_block(t, out);
            }
        }
        if let Some(t) = need_close_thinking {
            if t != output_index {
                self.close_thinking_block(t, out);
            }
        }
        // 单一索引定位，无回退查询
        let block = self
            .content_blocks
            .get_mut(&output_index)
            .expect("function_args_delta without output_item.added");
        if !block.started {
            let idx = self.next_block_index;
            self.next_block_index += 1;
            block.block_index = Some(idx);
            block.started = true;
            let tool_name = block.tool_name.clone().unwrap_or_default();
            let tool_id = block.call_id.clone().unwrap_or_else(|| format!("toolu_{}", idx));
            out.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({
                    "type": "content_block_start",
                    "index": idx,
                    "content_block": { "type": "tool_use", "id": tool_id, "name": tool_name, "input": {} }
                })
            ));
        }
        block.buffer.push_str(delta);
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "input_json_delta", "partial_json": delta } })
            ));
            block.emitted_len = block.buffer.len();
        }
    }

    fn on_function_args_done(
        &mut self,
        output_index: u32,
        item_id: Option<String>,
        arguments: &str,
        out: &mut Vec<String>,
    ) {
        if let Some(block) = self.content_blocks.get_mut(&output_index) {
            // done 携带最终完整 arguments：替换 buffer 为权威版本
            // （delta 累积可能与 done 略有出入，以 done 为准）
            if !arguments.is_empty() {
                block.buffer = arguments.to_string();
            } else if block.buffer.is_empty() {
                block.buffer = String::new();
            }
            if block.item_id.is_none() {
                block.item_id = item_id;
            }
        }
        self.close_tool_block(output_index, out);
    }

    fn on_reasoning_part_added(
        &mut self,
        item_id: &str,
        output_index: u32,
        summary_index: u32,
        _out: &mut Vec<String>,
    ) {
        let r = self
            .reasoning_by_item_id
            .entry(item_id.to_string())
            .or_insert_with(ActiveReasoning::new);
        r.activate_part(summary_index);
        // 确保 thinking block 存在
        self.content_blocks
            .entry(output_index)
            .or_insert_with(|| ContentBlockState {
                kind: BlockKind::Thinking,
                call_id: None,
                item_id: Some(item_id.to_string()),
                buffer: String::new(),
                block_index: None,
                tool_name: None,
                started: false,
                closed: false,
                emitted_len: 0,
            });
    }

    fn on_reasoning_delta(
        &mut self,
        item_id: &str,
        output_index: u32,
        delta: &str,
        out: &mut Vec<String>,
    ) {
        self.ensure_message_start(out);
        // 关闭可能打开的 text block
        if let Some(t) = self.open_text_output_index.take() {
            if t != output_index {
                self.close_text_block(t, out);
            }
        }
        let r = self
            .reasoning_by_item_id
            .entry(item_id.to_string())
            .or_insert_with(ActiveReasoning::new);
        r.append_summary_delta(delta);

        let block = self
            .content_blocks
            .entry(output_index)
            .or_insert_with(|| ContentBlockState {
                kind: BlockKind::Thinking,
                call_id: None,
                item_id: Some(item_id.to_string()),
                buffer: String::new(),
                block_index: None,
                tool_name: None,
                started: false,
                closed: false,
                emitted_len: 0,
            });
        if !block.started {
            let idx = self.next_block_index;
            self.next_block_index += 1;
            block.block_index = Some(idx);
            block.started = true;
            self.open_thinking_output_index = Some(output_index);
            out.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({ "type": "content_block_start", "index": idx, "content_block": { "type": "thinking", "thinking": "" } })
            ));
        }
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "thinking_delta", "thinking": delta } })
            ));
        }
    }

    fn on_reasoning_part_done(
        &mut self,
        item_id: &str,
        _output_index: u32,
        summary_index: u32,
    ) {
        if let Some(r) = self.reasoning_by_item_id.get_mut(item_id) {
            r.on_part_done(summary_index, self.store_enabled);
        }
    }

    /// commentary 重定向：把文本作为 thinking_delta 输出到一个 commentary thinking block。
    /// 复用 thinking block 的开启/delta 输出逻辑，item_id 用合成的 commentary 标识。
    fn emit_thinking_text_delta(
        &mut self,
        output_index: u32,
        delta: &str,
        out: &mut Vec<String>,
    ) {
        // 关闭可能打开的 text block（commentary 优先）
        if let Some(t) = self.open_text_output_index.take() {
            if t != output_index {
                self.close_text_block(t, out);
            }
        }
        let block = self
            .content_blocks
            .entry(output_index)
            .or_insert_with(|| ContentBlockState {
                kind: BlockKind::Thinking,
                call_id: None,
                item_id: Some(format!("commentary_{}", output_index)),
                buffer: String::new(),
                block_index: None,
                tool_name: None,
                started: false,
                closed: false,
                emitted_len: 0,
            });
        if !block.started {
            let idx = self.next_block_index;
            self.next_block_index += 1;
            block.block_index = Some(idx);
            block.started = true;
            self.open_thinking_output_index = Some(output_index);
            out.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({ "type": "content_block_start", "index": idx, "content_block": { "type": "thinking", "thinking": "" } })
            ));
        }
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "thinking_delta", "thinking": delta } })
            ));
        }
    }

    fn on_reasoning_text_done(&mut self, _item_id: &str, _output_index: u32, _text: &str) {
        // summary_text 已在 delta 中累积；store=true 时 part.done 已 Concluded
    }

    /// plan_bridge 钩子：检测 <proposed_plan>...</proposed_plan> 标签，捕获 body。
    /// 配置了 codex_plan_file_path 时抑制标签内可见文本，返回 None；
    /// 未配置时原样返回（透传），但仍捕获 body 供后续桥接。
    fn observe_proposed_plan_fragment(&mut self, fragment: &str) -> Option<String> {
        if fragment.is_empty() {
            return Some(String::new());
        }
        let suppress = self.codex_plan_file_path.is_some();
        let mut remaining = fragment.to_string();
        let mut visible = String::new();

        loop {
            if self.capturing_proposed_plan {
                if let Some(end) = remaining.find(PROPOSED_PLAN_CLOSE_TAG) {
                    self.proposed_plan_body_buffer.push_str(&remaining[..end]);
                    self.record_extracted_proposed_plan_body();
                    self.proposed_plan_body_buffer.clear();
                    self.capturing_proposed_plan = false;
                    remaining = remaining[end + PROPOSED_PLAN_CLOSE_TAG.len()..].to_string();
                    continue;
                }
                self.proposed_plan_body_buffer.push_str(&remaining);
                break;
            }

            let Some(start) = remaining.find(PROPOSED_PLAN_OPEN_TAG) else {
                if suppress {
                    // 抑制模式：标签外的文本仍可见
                    visible.push_str(&remaining);
                } else {
                    visible.push_str(&remaining);
                }
                break;
            };
            visible.push_str(&remaining[..start]);
            remaining = remaining[start + PROPOSED_PLAN_OPEN_TAG.len()..].to_string();
            self.capturing_proposed_plan = true;
            self.proposed_plan_body_buffer.clear();
        }

        if suppress {
            // 抑制模式：返回可见部分（标签外文本），标签内已捕获
            Some(visible)
        } else {
            // 未配置 plan_file_path：原样透传，但仍记录 body
            Some(fragment.to_string())
        }
    }

    fn record_extracted_proposed_plan_body(&mut self) {
        let body = self.proposed_plan_body_buffer.trim();
        if body.is_empty() {
            return;
        }
        self.latest_proposed_plan_body = Some(body.to_string());
    }

    /// 在响应完成时尝试 emit plan_bridge：写 plan 文件 + 合成 ExitPlanMode tool_use
    fn maybe_emit_plan_mode_bridge(&mut self, out: &mut Vec<String>) {
        if self.plan_bridge_emitted {
            return;
        }
        if self.latest_proposed_plan_body.is_none() || self.codex_plan_file_path.is_none() {
            return;
        }
        let Some(path) = self.codex_plan_file_path.as_deref() else {
            return;
        };
        let Some(body) = self.latest_proposed_plan_body.as_deref() else {
            return;
        };

        // 写 plan 文件
        let p = std::path::Path::new(path);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(p, body).is_err() {
            return;
        }

        // emit 合成 ExitPlanMode tool_use
        if let Some(t) = self.open_text_output_index.take() {
            self.close_text_block(t, out);
        }
        if let Some(t) = self.open_thinking_output_index.take() {
            self.close_thinking_block(t, out);
        }
        let idx = self.next_block_index;
        self.next_block_index += 1;
        let call_id = format!("plan_bridge_exit_{}", chrono::Utc::now().timestamp_millis());
        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "tool_use", "id": call_id, "name": "ExitPlanMode", "input": {} }
            })
        ));
        out.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "input_json_delta", "partial_json": "{}" } })
        ));
        out.push(format!(
            "event: content_block_stop\ndata: {}\n\n",
            json!({ "type": "content_block_stop", "index": idx })
        ));
        self.plan_bridge_emitted = true;
    }

    /// markdown_bash 钩子：检测 ```bash/sh/shell 代码块，缓冲到块结束，emit 合成 Bash tool_use。
    /// 返回 true 表示已处理（调用方跳过正常文本输出）。
    fn handle_markdown_bash(&mut self, delta: &str, out: &mut Vec<String>) -> bool {
        if self.in_markdown_bash {
            self.markdown_bash_buffer.push_str(delta);
            if self.markdown_bash_buffer.contains("\n```\n")
                || self.markdown_bash_buffer.ends_with("\n```")
                || self.markdown_bash_buffer.ends_with("```")
            {
                self.flush_markdown_bash(out);
            }
            return true;
        }
        if let Some((marker_start, marker_len)) = leak::find_markdown_bash_start(delta) {
            let prefix = &delta[..marker_start];
            let after_marker = &delta[marker_start + marker_len..];
            // prefix 作为正常文本输出（递归调用，但此时 in_markdown_bash 仍为 false）
            if !prefix.is_empty() {
                self.emit_plain_text_fragment(prefix, out);
            }
            self.in_markdown_bash = true;
            self.markdown_bash_buffer.push_str(after_marker);
            if self.markdown_bash_buffer.contains("\n```\n")
                || self.markdown_bash_buffer.ends_with("\n```")
                || self.markdown_bash_buffer.ends_with("```")
            {
                self.flush_markdown_bash(out);
            }
            return true;
        }
        false
    }

    /// flush markdown_bash 缓冲：提取脚本，emit 合成 Bash tool_use，重置状态
    fn flush_markdown_bash(&mut self, out: &mut Vec<String>) {
        if !self.in_markdown_bash {
            return;
        }
        self.in_markdown_bash = false;
        let mut script = std::mem::take(&mut self.markdown_bash_buffer);
        // 去除结束标记
        if let Some(end_idx) = script.find("\n```\n") {
            script.truncate(end_idx);
        } else if let Some(end_idx) = script.find("\n```") {
            script.truncate(end_idx);
        } else if script.ends_with("```\n") {
            script.truncate(script.len() - 4);
        } else if script.ends_with("```") {
            script.truncate(script.len() - 3);
        }
        let script = script.trim().to_string();
        if script.is_empty() {
            return;
        }
        // 关闭可能打开的 text/thinking block
        if let Some(t) = self.open_text_output_index.take() {
            self.close_text_block(t, out);
        }
        if let Some(t) = self.open_thinking_output_index.take() {
            self.close_thinking_block(t, out);
        }
        // emit 合成 Bash tool_use
        let idx = self.next_block_index;
        self.next_block_index += 1;
        let call_id = format!("tool_bash_{}", chrono::Utc::now().timestamp_millis());
        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "tool_use", "id": call_id, "name": "Bash", "input": {} }
            })
        ));
        let arguments = json!({ "command": script }).to_string();
        out.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "input_json_delta", "partial_json": arguments } })
        ));
        out.push(format!(
            "event: content_block_stop\ndata: {}\n\n",
            json!({ "type": "content_block_stop", "index": idx })
        ));
    }

    /// 直接输出纯文本片段（不经 leak/commentary/markdown_bash 钩子），供 markdown_bash 的 prefix 用
    fn emit_plain_text_fragment(&mut self, fragment: &str, out: &mut Vec<String>) {
        if fragment.is_empty() {
            return;
        }
        // 关闭可能打开的 thinking block
        if let Some(t) = self.open_thinking_output_index.take() {
            self.close_thinking_block(t, out);
        }
        let block = self
            .content_blocks
            .entry(0)
            .or_insert_with(|| ContentBlockState {
                kind: BlockKind::Text,
                call_id: None,
                item_id: None,
                buffer: String::new(),
                block_index: None,
                tool_name: None,
                started: false,
                closed: false,
                emitted_len: 0,
            });
        if !block.started {
            let idx = self.next_block_index;
            self.next_block_index += 1;
            block.block_index = Some(idx);
            block.started = true;
            self.open_text_output_index = Some(0);
            out.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({ "type": "content_block_start", "index": idx, "content_block": { "type": "text", "text": "" } })
            ));
        }
        block.buffer.push_str(fragment);
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "text_delta", "text": fragment } })
            ));
        }
    }

    fn on_refusal_delta(&mut self, output_index: u32, delta: &str, out: &mut Vec<String>) {
        self.ensure_message_start(out);
        let block = self
            .content_blocks
            .entry(output_index)
            .or_insert_with(|| ContentBlockState {
                kind: BlockKind::Refusal,
                call_id: None,
                item_id: None,
                buffer: String::new(),
                block_index: None,
                tool_name: None,
                started: false,
                closed: false,
                emitted_len: 0,
            });
        if !block.started {
            let idx = self.next_block_index;
            self.next_block_index += 1;
            block.block_index = Some(idx);
            block.started = true;
            out.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({ "type": "content_block_start", "index": idx, "content_block": { "type": "text", "text": "" } })
            ));
        }
        block.buffer.push_str(delta);
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "text_delta", "text": delta } })
            ));
        }
    }

    fn on_refusal_done(&mut self, output_index: u32, _refusal: &str, out: &mut Vec<String>) {
        self.close_text_block(output_index, out);
    }

    fn on_web_search_call_start(
        &mut self,
        output_index: u32,
        item_id: Option<String>,
        out: &mut Vec<String>,
    ) {
        self.ensure_message_start(out);
        // 已注册则跳过（去重，取代旧 lookup_active_web_search_call_id）
        if self
            .web_search_by_output_index
            .get(&output_index)
            .or_else(|| item_id.as_ref().and_then(|id| self.web_search_by_item_id.get(id)))
            .is_some()
        {
            return;
        }
        // 关闭可能打开的 text/thinking block
        let need_close_text = self.open_text_output_index.take();
        let need_close_thinking = self.open_thinking_output_index.take();
        if let Some(t) = need_close_text {
            self.close_text_block(t, out);
        }
        if let Some(t) = need_close_thinking {
            self.close_thinking_block(t, out);
        }

        let idx = self.next_block_index;
        self.next_block_index += 1;
        self.next_server_tool_use_seq += 1;
        let server_tool_use_id = format!(
            "srvtoolu_{}_{}",
            chrono::Utc::now().timestamp_millis(),
            self.next_server_tool_use_seq
        );

        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
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
        // 空的 tool_json delta（与旧行为一致）
        out.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({
                "type": "content_block_delta",
                "index": idx,
                "delta": { "type": "input_json_delta", "partial_json": "" }
            })
        ));

        self.web_search_by_output_index
            .insert(output_index, server_tool_use_id.clone());
        if let Some(id) = &item_id {
            self.web_search_by_item_id
                .insert(id.clone(), server_tool_use_id.clone());
        }
        self.active_web_search.insert(
            server_tool_use_id,
            WebSearchCall {
                content_block_index: idx,
                output_index: Some(output_index),
                item_id,
                input_closed: false,
            },
        );
    }

    fn on_web_search_call_complete(
        &mut self,
        output_index: u32,
        item_id: Option<String>,
        out: &mut Vec<String>,
    ) {
        let server_id = self
            .web_search_by_output_index
            .get(&output_index)
            .cloned()
            .or_else(|| {
                item_id
                    .as_ref()
                    .and_then(|id| self.web_search_by_item_id.get(id).cloned())
            });
        let Some(server_id) = server_id else {
            return;
        };
        let Some(call) = self.active_web_search.remove(&server_id) else {
            return;
        };
        if !call.input_closed {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": call.content_block_index })
            ));
        }
        self.web_search_by_output_index.remove(&output_index);
        if let Some(id) = &item_id {
            self.web_search_by_item_id.remove(id);
        }
    }

    fn close_text_block(&mut self, output_index: u32, out: &mut Vec<String>) {
        let Some(block) = self.content_blocks.get_mut(&output_index) else {
            return;
        };
        if block.closed || !block.started {
            return;
        }
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        block.closed = true;
        if self.open_text_output_index == Some(output_index) {
            self.open_text_output_index = None;
        }
    }

    fn close_thinking_block(&mut self, output_index: u32, out: &mut Vec<String>) {
        let Some(block) = self.content_blocks.get_mut(&output_index) else {
            return;
        };
        if block.closed || !block.started {
            return;
        }
        if let Some(idx) = block.block_index {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        block.closed = true;
        if self.open_thinking_output_index == Some(output_index) {
            self.open_thinking_output_index = None;
        }
    }

    fn close_tool_block(&mut self, output_index: u32, out: &mut Vec<String>) {
        let Some(block) = self.content_blocks.get_mut(&output_index) else {
            return;
        };
        if block.closed {
            return;
        }
        let idx = block.block_index.unwrap_or_else(|| {
            let i = self.next_block_index;
            self.next_block_index += 1;
            block.block_index = Some(i);
            i
        });
        if !block.started {
            let tool_name = block.tool_name.clone().unwrap_or_default();
            let tool_id = block.call_id.clone().unwrap_or_else(|| format!("toolu_{}", idx));
            out.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                json!({
                    "type": "content_block_start",
                    "index": idx,
                    "content_block": { "type": "tool_use", "id": tool_id, "name": tool_name, "input": {} }
                })
            ));
            block.started = true;
        }
        // 只发尚未通过 input_json_delta 发出的剩余部分，避免重复发包
        // （旧实现用 emitted_arguments_len 解决同样问题；宿主侧会拼接所有 partial_json）
        let emitted_len = block.emitted_len;
        let remaining = &block.buffer[emitted_len..];
        if !remaining.is_empty() {
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({ "type": "content_block_delta", "index": idx, "delta": { "type": "input_json_delta", "partial_json": remaining } })
            ));
            block.emitted_len = block.buffer.len();
        }
        out.push(format!(
            "event: content_block_stop\ndata: {}\n\n",
            json!({ "type": "content_block_stop", "index": idx })
        ));
        block.closed = true;
    }

    fn emit_message_stop(&mut self, out: &mut Vec<String>) {
        // plan_bridge 钩子：响应完成时尝试 emit 合成 ExitPlanMode（写 plan 文件 + tool_use）
        self.maybe_emit_plan_mode_bridge(out);
        // 关闭所有未关 content block
        let indices: Vec<u32> = self.content_blocks.keys().copied().collect();
        for idx in indices {
            if let Some(block) = self.content_blocks.get(&idx) {
                if !block.closed {
                    match block.kind {
                        BlockKind::Text => self.close_text_block(idx, out),
                        BlockKind::Thinking => self.close_thinking_block(idx, out),
                        BlockKind::ToolUse => self.close_tool_block(idx, out),
                        BlockKind::Refusal => self.close_text_block(idx, out),
                    }
                }
            }
        }
        // 关闭所有未关 web_search server_tool_use block
        let web_indices: Vec<usize> = self
            .active_web_search
            .values()
            .filter(|c| !c.input_closed)
            .map(|c| c.content_block_index)
            .collect();
        for idx in web_indices {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        self.active_web_search.clear();
        let (input, output) = match &self.usage {
            Some(u) => (u.input_tokens, u.output_tokens),
            None => (0, 0),
        };
        out.push(format!(
            "event: message_delta\ndata: {}\n\n",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": { "input_tokens": input, "output_tokens": output }
            })
        ));
        out.push("event: message_stop\ndata: {}\n\n".to_string());
    }

    fn emit_error_and_stop(&mut self, out: &mut Vec<String>) {
        out.push(format!(
            "event: error\ndata: {}\n\n",
            json!({ "type": "error", "error": { "type": "upstream_error", "message": "upstream response failed" } })
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::codex::event::parse_sse_event;

    fn drive(state: &mut StreamState, sse_line: &str) -> Vec<String> {
        match parse_sse_event(sse_line) {
            Some(ev) => state.handle_event(ev),
            None => Vec::new(),
        }
    }

    #[test]
    fn text_stream_produces_anthropic_lifecycle() {
        let mut s = StreamState::new("gpt-5");
        let out1 = drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1","status":"in_progress"}}"#);
        assert!(out1.iter().any(|l| l.contains("message_start")));

        let out2 = drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"hello"}"#);
        assert!(out2.iter().any(|l| l.contains("content_block_start") && l.contains("\"type\":\"text\"")));
        assert!(out2.iter().any(|l| l.contains("text_delta") && l.contains("hello")));

        let out3 = drive(&mut s, r#"data: {"type":"response.output_text.done","output_index":0,"content_index":0,"text":"hello"}"#);
        assert!(out3.iter().any(|l| l.contains("content_block_stop")));

        let out4 = drive(&mut s, r#"data: {"type":"response.completed","response":{"id":"r1","status":"completed","output":[]}}"#);
        assert!(out4.iter().any(|l| l.contains("message_stop")));
    }

    #[test]
    fn tool_call_single_index_no_fallback() {
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        // function_call added: 只带 output_index，无 item_id/call_id
        drive(&mut s, r#"data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather","arguments":""}}"#);
        // delta 只带 output_index（是 done arguments 的真实前缀）
        let out = drive(&mut s, r#"data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"city\":"}"#);
        assert!(out.iter().any(|l| l.contains("input_json_delta")));
        // done 给完整 arguments
        let out2 = drive(&mut s, r#"data: {"type":"response.function_call_arguments.done","output_index":1,"arguments":"{\"city\":\"sf\"}"}"#);
        assert!(out2.iter().any(|l| l.contains("content_block_stop")));
        // 验证 tool_use id 用 call_id（对应 Anthropic tool_use.id）
        let start = out.iter().chain(out2.iter()).find(|l| l.contains("content_block_start")).unwrap();
        assert!(start.contains("call_1"));
    }

    #[test]
    fn tool_args_not_double_emitted() {
        // 回归测试：delta 已发过的部分，close 时不应重复发包
        // 宿主侧会拼接所有 input_json_delta，重复发包会导致两个 JSON 串在一起
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        drive(&mut s, r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"tool","arguments":""}}"#);
        // 完整 arguments 在单个 delta 发出
        let delta_out = drive(&mut s, r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"a\":1}"}"#);
        // done 给同样完整的 arguments
        let done_out = drive(&mut s, r#"data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"a\":1}"}"#);
        // 统计 input_json_delta 事件次数
        let delta_count = delta_out.iter().filter(|l| l.contains("input_json_delta")).count();
        let done_count = done_out.iter().filter(|l| l.contains("input_json_delta")).count();
        // delta 发一次；done 时 buffer 已完整（emitted_len == buffer.len()），remaining 为空，不应再发
        assert_eq!(delta_count, 1, "delta should emit one input_json_delta");
        assert_eq!(done_count, 0, "done must NOT re-emit input_json_delta when delta already sent full args; got {}", done_count);
    }

    #[test]
    fn tool_args_done_only_emits_once() {
        // 回归测试：无 delta，只有 done 给完整 arguments 时，只发一次
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        drive(&mut s, r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"tool","arguments":""}}"#);
        let done_out = drive(&mut s, r#"data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"x\":2}"}"#);
        let mut count = 0;
        for line in &done_out {
            if line.contains("input_json_delta") {
                count += 1;
            }
        }
        assert_eq!(count, 1, "done-only tool args should emit exactly one input_json_delta; got {}", count);
    }

    #[test]
    fn reasoning_store_false_waits_for_output_item_done() {
        let mut s = StreamState::new("gpt-5");
        s.set_store_enabled(false);
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        drive(&mut s, r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"r_1","summary":[]}}"#);
        drive(&mut s, r#"data: {"type":"response.reasoning_summary_part.added","item_id":"r_1","output_index":0,"summary_index":0}"#);
        let out_delta = drive(&mut s, r#"data: {"type":"response.reasoning_summary_text.delta","item_id":"r_1","output_index":0,"summary_index":0,"delta":"thinking"}"#);
        assert!(out_delta.iter().any(|l| l.contains("thinking_delta")));
        // part.done: store=false -> CanConclude, 不立即关闭
        drive(&mut s, r#"data: {"type":"response.reasoning_summary_part.done","item_id":"r_1","output_index":0,"summary_index":0}"#);
        // 此时 thinking block 尚未关闭
        let r = s.reasoning_by_item_id.get("r_1").unwrap();
        assert!(r.has_pending());
        assert!(!r.is_concluded());
        // output_item.done 携带 encrypted_content -> Concluded -> 关闭
        let out_done = drive(&mut s, r#"data: {"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"r_1","encrypted_content":"enc","summary":[{"type":"summary_text","text":"thinking"}]}}"#);
        assert!(out_done.iter().any(|l| l.contains("content_block_stop")));
    }

    #[test]
    fn unknown_event_does_not_break_stream() {
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        // 未知事件
        let out = drive(&mut s, r#"data: {"type":"response.some_future_event","output_index":0}"#);
        assert!(out.is_empty());
        // 流仍正常
        let out2 = drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"ok"}"#);
        assert!(out2.iter().any(|l| l.contains("text_delta")));
    }

    #[test]
    fn content_part_added_extracts_text() {
        // content_part.added 携带 output_text 文本时，新状态机应提取为 text_delta
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        let out = drive(&mut s, r#"data: {"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":"hello from part"}}"#);
        assert!(out.iter().any(|l| l.contains("content_block_start") && l.contains("\"type\":\"text\"")));
        assert!(out.iter().any(|l| l.contains("text_delta") && l.contains("hello from part")));
    }

    #[test]
    fn web_search_call_emits_server_tool_use() {
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        let out = drive(&mut s, r#"data: {"type":"response.web_search_call.in_progress","output_index":1,"item_id":"ws_1"}"#);
        assert!(out.iter().any(|l| l.contains("content_block_start") && l.contains("\"type\":\"server_tool_use\"") && l.contains("web_search")));
        // 重复事件去重
        let out2 = drive(&mut s, r#"data: {"type":"response.web_search_call.searching","output_index":1,"item_id":"ws_1"}"#);
        assert!(out2.is_empty(), "duplicate web_search start should be deduped");
        // completed 关闭 block
        let out3 = drive(&mut s, r#"data: {"type":"response.web_search_call.completed","output_index":1,"item_id":"ws_1"}"#);
        assert!(out3.iter().any(|l| l.contains("content_block_stop")));
    }

    #[test]
    fn commentary_phase_redirects_text_to_thinking() {
        // commentary phase: message item 的 phase=="commentary" 时，后续文本应重定向到 thinking block
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        // message item with phase=commentary
        drive(&mut s, r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"m1","role":"assistant","content":[],"phase":"commentary"}}"#);
        let out = drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"internal commentary"}"#);
        let joined = out.join("");
        // 应输出 thinking block，而非 text block
        assert!(joined.contains("\"type\":\"thinking\""), "commentary text should redirect to thinking block");
        assert!(joined.contains("thinking_delta") && joined.contains("internal commentary"));
        assert!(!joined.contains("\"type\":\"text_delta\""), "commentary text should not emit text_delta");
    }

    #[test]
    fn markdown_bash_block_promoted_to_tool_use() {
        // markdown_bash 钩子：```bash 代码块应被提升为合成 Bash tool_use
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        let mut all = String::new();
        // 完整 bash 块在单个 delta
        all.extend(drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"```bash\necho hello\n```"}"#).join("").chars());
        eprintln!("DEBUG MB OUT: [{}]", all);
        // 应 emit Bash tool_use
        assert!(all.contains("\"name\":\"Bash\""), "bash block should be promoted to Bash tool_use; got: {}", all);
        assert!(all.contains("echo hello"), "script should appear; got: {}", all);
        assert!(all.contains("content_block_stop"), "tool_use block should be closed; got: {}", all);
    }

    #[test]
    fn background_agent_lifecycle_redirected_to_thinking() {
        // background_agent 钩子：<task-notification> 文本应重定向到 thinking_delta
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        let out = drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"<task-notification><status>completed</status><summary>done</summary></task-notification>"}"#);
        let joined = out.join("");
        // 应输出 thinking_delta（进度消息），而非 text_delta
        assert!(joined.contains("thinking_delta"), "task-notification should redirect to thinking; got: {}", joined);
        assert!(joined.contains("后台任务已完成"));
        assert!(!joined.contains("\"type\":\"text_delta\""));
    }

    #[test]
    fn plan_bridge_captures_and_emits_exit_plan_mode() {
        // plan_bridge 钩子：配置 plan_file_path 时捕获 <proposed_plan> body，完成后 emit 合成 ExitPlanMode
        let plan_path = std::env::temp_dir().join(format!(
            "codex_proxy_plan_bridge_test_{}.md",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let plan_path_str = plan_path.to_string_lossy().to_string();
        let mut s = StreamState::new("gpt-5");
        s.set_plan_file_path(Some(plan_path_str.clone()));
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        let mut all = String::new();
        // proposed_plan 块
        all.extend(drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"<proposed_plan>\n# Plan\n- step 1\n</proposed_plan>"}"#).join("").chars());
        // 完成
        all.extend(drive(&mut s, r#"data: {"type":"response.completed","response":{"id":"r1","status":"completed","output":[]}}"#).join("").chars());
        // 应 emit ExitPlanMode tool_use
        assert!(all.contains("\"name\":\"ExitPlanMode\""), "plan_bridge should emit synthetic ExitPlanMode; got: {}", all);
        // plan 文件应被写入
        assert!(plan_path.exists(), "plan file should be written");
        let _ = std::fs::remove_file(&plan_path);
    }

    #[test]
    fn leaked_tool_marker_is_suppressed() {
        // leak 钩子：泄漏的 tool 调用标记应被抑制，不输出为 text_delta
        let mut s = StreamState::new("gpt-5");
        drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#);
        let out = drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"assistant to=functions.Write {\"file_path\":\"/tmp/a.ts\"}"}"#);
        let joined = out.join("");
        assert!(!joined.contains("assistant to=functions"), "leaked marker must be suppressed");
        assert!(!joined.contains("\"type\":\"tool_use\""), "leaked text must not promote to tool_use");
    }

    #[test]
    fn mixed_text_and_tool_use_lifecycle() {
        // 完整流：text → tool_use → completion，验证 block 边界与生命周期
        let mut s = StreamState::new("gpt-5");
        let mut all = String::new();
        all.extend(drive(&mut s, r#"data: {"type":"response.created","response":{"id":"r1"}}"#).join("").chars());
        // text block
        all.extend(drive(&mut s, r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"thinking about"}"#).join("").chars());
        all.extend(drive(&mut s, r#"data: {"type":"response.output_text.done","output_index":0,"content_index":0,"text":"thinking about"}"#).join("").chars());
        // tool_use block
        all.extend(drive(&mut s, r#"data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"Read","arguments":""}}"#).join("").chars());
        all.extend(drive(&mut s, r#"data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"path\"}"}"#).join("").chars());
        all.extend(drive(&mut s, r#"data: {"type":"response.function_call_arguments.done","output_index":1,"arguments":"{\"path\":\"/tmp\"}"}"#).join("").chars());
        // completion
        all.extend(drive(&mut s, r#"data: {"type":"response.completed","response":{"id":"r1","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":20}}}"#).join("").chars());
        // 必须 emit message_stop
        assert!(all.contains("message_stop"));
        // tool_use 用 call_id 作为 Anthropic tool_use.id
        assert!(all.contains("call_1"));
        // usage 透传
        assert!(all.contains("\"input_tokens\":10"));
        assert!(all.contains("\"output_tokens\":20"));
    }
}
