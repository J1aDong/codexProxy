//! 基于 StreamState 的 ResponseTransformer 实现（D6 入口切换）
//!
//! 新路径：把上游 SSE 行委托给 `StreamState::handle_event`，取代旧 `TransformResponse`。
//! 旧 `TransformResponse` 保留为回退，生产路径通过 `CodexBackend::create_response_transformer` 切换。

use crate::transform::codex::event::parse_sse_event;
use crate::transform::codex::stream_state::StreamState;
use crate::transform::{ResponseTransformer, ResponseTransformRequestContext};
use serde_json::Value;

pub struct StreamStateBackedTransformer {
    state: StreamState,
    allow_visible_thinking: bool,
}

impl StreamStateBackedTransformer {
    pub fn new(model: &str, allow_visible_thinking: bool) -> Self {
        let state = StreamState::new(model);
        Self {
            state,
            allow_visible_thinking,
        }
    }
}

impl ResponseTransformer for StreamStateBackedTransformer {
    fn transform_line(&mut self, line: &str) -> Vec<String> {
        // 缓冲跨行 SSE 帧（event:/data:），遇到空行或完整 data 帧时解析
        let mut output = Vec::new();
        // 尝试直接解析当前行（data: 前缀）
        if let Some(event) = parse_sse_event(line) {
            if self.allow_visible_thinking {
                output.extend(self.state.handle_event(event));
            } else {
                // 不输出 thinking block：仍处理事件以推进状态机，但过滤 thinking 事件
                let events = self.state.handle_event(event);
                for ev in events {
                    if !ev.contains("\"type\":\"thinking\"") && !ev.contains("thinking_delta") {
                        output.push(ev);
                    }
                }
            }
        }
        output
    }

    fn configure_request_context(&mut self, ctx: &ResponseTransformRequestContext) {
        self.state.configure_request_context(ctx);
    }

    fn take_diagnostics_summary(&mut self) -> Option<Value> {
        None
    }
}
