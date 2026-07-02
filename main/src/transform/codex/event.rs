//! Codex Responses API 事件模型（D1）
//!
//! 用 serde tagged enum 严格建模 Codex Responses 的 `OutputItem` union 与流式 SSE 事件 union，
//! 替换旧的 `serde_json::Value` + 字符串嗅探。未知类型用 `#[serde(other)]` 兜底为 `Unknown`。
//!
//! 本模块是纯解析层，无副作用，不接入主路径（D6 绞杀式迁移：先并行存在）。
//! 接入主路径前，类型暂未在 crate 外使用，故 allow(dead_code)。

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================
// OutputItem union（非流式响应体的 output[] 元素）
// ============================================================

/// Codex Responses 的 `output[]` item，按 `type` 判别。
///
/// 注意：`FunctionCall` 同时保留 `call_id`（`call_` 前缀，回传 `function_call_output` 的引用键，
/// 对应 Anthropic `tool_use.id`）与 `id`（`fc_` 前缀，item 自身标识），二者不可合并。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItem {
    /// `type: "message"` —— 文本消息 item
    #[serde(rename = "message")]
    Message {
        id: String,
        role: String,
        #[serde(default)]
        content: Vec<MessageContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        /// `phase: "commentary" | "final_answer"` —— commentary 阶段的文本应重定向到 thinking
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    /// `type: "function_call"` —— 工具调用 item
    #[serde(rename = "function_call")]
    FunctionCall {
        /// item 自身标识（`fc_` 前缀）
        id: String,
        /// 回传引用键（`call_` 前缀），对应 Anthropic `tool_use.id`
        call_id: String,
        name: String,
        /// JSON 字符串形式的参数（output_item.added 时可能缺失，done 时给出）
        #[serde(default)]
        arguments: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    /// `type: "function_call_output"` —— 工具结果回传 item
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        id: String,
        /// 与原 `function_call.call_id` 一致
        call_id: String,
        output: CallOutput,
        status: Option<String>,
    },
    /// `type: "reasoning"` —— 推理 item
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        /// 加密推理上下文（多轮回传用），透传不伪造 Anthropic signature
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
        summary: Vec<SummaryText>,
        content: Option<Vec<MessageContent>>,
    },
    /// 未知 item 类型（兜底，保留原始 type 字符串）
    #[serde(other)]
    Unknown,
}

/// message / reasoning 通用文本内容部分
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageContent {
    #[serde(rename = "output_text")]
    OutputText { text: String, annotations: Option<Vec<Value>> },
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "refusal")]
    Refusal { refusal: String },
    #[serde(other)]
    Unknown,
}

/// reasoning summary 数组元素
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SummaryText {
    #[serde(rename = "summary_text")]
    SummaryText { text: String },
    #[serde(other)]
    Unknown,
}

/// `function_call_output.output` 支持字符串或内容块数组
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CallOutput {
    Text(String),
    Parts(Vec<MessageContent>),
    /// 兜底：未知形态保留原始 JSON
    Raw(Value),
}

// ============================================================
// 流式 SSE 事件 union
// ============================================================

/// Codex Responses 流式 SSE 事件，按 `type` 判别。
///
/// 未知事件用 `Unknown` 兜底，不中断流式转换。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponsesEvent {
    // 状态机事件
    #[serde(rename = "response.created")]
    ResponseCreated { response: ResponseSummary },
    #[serde(rename = "response.in_progress")]
    ResponseInProgress { response: ResponseSummary },
    #[serde(rename = "response.completed")]
    ResponseCompleted { response: ResponseCompletedBody },
    #[serde(rename = "response.failed")]
    ResponseFailed { response: Value, error: Option<ErrorBody> },
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete { response: Value },

    // Output item 事件
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        #[serde(default)]
        output_index: u32,
        item: OutputItem,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        #[serde(default)]
        output_index: u32,
        item: OutputItem,
    },

    // Content part 事件
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
        part: MessageContent,
    },
    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
        part: MessageContent,
    },

    // 文本事件
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
        delta: String,
    },
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
        text: String,
    },

    // Refusal 事件
    #[serde(rename = "response.refusal.delta")]
    RefusalDelta {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
        delta: String,
    },
    #[serde(rename = "response.refusal.done")]
    RefusalDone {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
        refusal: String,
    },

    // Function call 事件
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        #[serde(alias = "arguments")]
        delta: String,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        item_id: Option<String>,
        #[serde(default)]
        output_index: u32,
        arguments: String,
    },

    // Web search call 事件（server tool）
    #[serde(rename = "response.web_search_call.in_progress")]
    WebSearchCallInProgress {
        #[serde(default)]
        output_index: u32,
        item_id: Option<String>,
    },
    #[serde(rename = "response.web_search_call.searching")]
    WebSearchCallSearching {
        #[serde(default)]
        output_index: u32,
        item_id: Option<String>,
    },
    #[serde(rename = "response.web_search_call.completed")]
    WebSearchCallCompleted {
        #[serde(default)]
        output_index: u32,
        item_id: Option<String>,
    },

    // Reasoning 事件
    #[serde(rename = "response.reasoning_summary_part.added")]
    ReasoningSummaryPartAdded {
        item_id: String,
        #[serde(default)]
        output_index: u32,
        summary_index: u32,
    },
    #[serde(rename = "response.reasoning_summary_part.done")]
    ReasoningSummaryPartDone {
        item_id: String,
        #[serde(default)]
        output_index: u32,
        summary_index: u32,
    },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        item_id: String,
        #[serde(default)]
        output_index: u32,
        summary_index: u32,
        delta: String,
    },
    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryTextDone {
        item_id: String,
        #[serde(default)]
        output_index: u32,
        summary_index: u32,
        text: String,
    },

    // 错误事件
    #[serde(rename = "error")]
    Error { message: Option<String>, code: Option<String> },

    /// 未知事件类型（兜底，保留原始 type 字符串）
    #[serde(other)]
    Unknown,
}

/// `response.created` / `response.in_progress` 携带的响应摘要
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseSummary {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// `response.completed` 携带的完整响应体
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseCompletedBody {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub output: Vec<OutputItem>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub input_tokens_details: Option<InputTokenDetails>,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokenDetails>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InputTokenDetails {
    #[serde(default)]
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OutputTokenDetails {
    #[serde(default)]
    pub reasoning_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: Option<String>,
    pub message: Option<String>,
}

// ============================================================
// SSE 帧解析（D1 任务 1.3）
// ============================================================

/// 解析一行（或多行）SSE 为一个事件。
///
/// 输入是上游推送的原始行（可能含 `event:`/`data:` 前缀，或已经是纯 JSON）。
/// data 非 JSON（`[DONE]`/空）时安全返回 `None`。
///
/// 注意：调用方通常逐行调用 `transform_line`，但 SSE 事件可能跨多行（一个 `event:` + 一个 `data:`）。
/// 本函数处理**单行**：若是 `data:` 行则解析其 JSON；若是 `event:` 行或注释行则返回 `None`。
pub fn parse_sse_event(line: &str) -> Option<ResponsesEvent> {
    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
    if trimmed.is_empty() {
        return None;
    }

    // 注释行
    if trimmed.starts_with(':') {
        return None;
    }

    // data: 行
    if let Some(rest) = trimmed.strip_prefix("data:") {
        let payload = rest.trim();
        return parse_data_payload(payload);
    }

    // event: 行 —— 仅声明事件类型，真正载荷在后续 data 行，此处返回 None
    if trimmed.starts_with("event:") {
        return None;
    }

    // 兜底：可能是裸 JSON（无前缀）
    parse_data_payload(trimmed)
}

fn parse_data_payload(payload: &str) -> Option<ResponsesEvent> {
    let payload = payload.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    // 命名归一化：上游可能发下划线变体（如 response.function_call_arguments_delta），
    // 旧代码用 `|` 模式同时兼容。这里把已知的下划线变体归一化为点号变体，让 serde 正确匹配。
    let normalized = normalize_event_type_name(payload);
    // 反序列化；未知类型走 #[serde(other)] -> Unknown，不报错
    match serde_json::from_str::<ResponsesEvent>(&normalized) {
        Ok(event) => Some(event),
        Err(_) => {
            // 解析失败：返回 Unknown 以便上层记录，而不是中断流
            // 但 Unknown 需要原始 type 字符串，这里用 Value 兜底解析 type
            if let Ok(v) = serde_json::from_str::<Value>(&normalized) {
                if v.is_object() {
                    // 已知事件但字段不全时，serde 会失败 —— 归为 Unknown 而非 panic
                    return Some(ResponsesEvent::Unknown);
                }
            }
            None
        }
    }
}

/// 把 payload JSON 里的 `type` 字段已知的下划线变体归一化为点号变体。
///
/// 例如 `response.function_call_arguments_delta` → `response.function_call_arguments.delta`。
/// 只处理已知的下划线变体，其他保持不变。
fn normalize_event_type_name(payload: &str) -> String {
    // 已知的下划线变体 → 点号变体映射
    const VARIANTS: [(&str, &str); 4] = [
        (
            "response.function_call_arguments_delta",
            "response.function_call_arguments.delta",
        ),
        (
            "response.function_call_arguments_done",
            "response.function_call_arguments.done",
        ),
        (
            "response.output_text_delta",
            "response.output_text.delta",
        ),
        (
            "response.output_text_done",
            "response.output_text.done",
        ),
    ];
    let mut result = payload.to_string();
    for (from, to) in VARIANTS {
        // 只替换 "type":"<from>" 形式，避免误伤其他字段
        let needle = format!("\"type\":\"{}\"", from);
        let replacement = format!("\"type\":\"{}\"", to);
        if result.contains(&needle) {
            result = result.replace(&needle, &replacement);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_function_call_item() {
        let json = r#"{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"sf\"}","status":"completed"}"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::FunctionCall { id, call_id, name, arguments, .. } => {
                assert_eq!(id, "fc_1");
                assert_eq!(call_id, "call_1"); // 双 ID 独立保留
                assert_eq!(name, "get_weather");
                assert_eq!(arguments, r#"{"city":"sf"}"#);
            }
            _ => panic!("expected FunctionCall"),
        }
    }

    #[test]
    fn parse_unknown_output_item() {
        let json = r#"{"type":"some_new_item_type","id":"x"}"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        assert_eq!(item, OutputItem::Unknown);
    }

    #[test]
    fn parse_function_call_arguments_delta_event() {
        let json = r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":2,"delta":"{\"ci"}"#;
        let ev = serde_json::from_str::<ResponsesEvent>(json).unwrap();
        match ev {
            ResponsesEvent::FunctionCallArgumentsDelta { output_index, delta, .. } => {
                assert_eq!(output_index, 2);
                assert_eq!(delta, "{\"ci");
            }
            _ => panic!("expected FunctionCallArgumentsDelta"),
        }
    }

    #[test]
    fn parse_unknown_event_falls_back() {
        let json = r#"{"type":"response.some_future_event","output_index":0}"#;
        let ev = serde_json::from_str::<ResponsesEvent>(json).unwrap();
        assert_eq!(ev, ResponsesEvent::Unknown);
    }

    #[test]
    fn parse_data_line() {
        let line = r#"data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"hi"}"#;
        let ev = parse_sse_event(line).unwrap();
        match ev {
            ResponsesEvent::OutputTextDelta { delta, .. } => assert_eq!(delta, "hi"),
            _ => panic!("expected OutputTextDelta"),
        }
    }

    #[test]
    fn parse_event_line_returns_none() {
        assert!(parse_sse_event("event: response.completed").is_none());
    }

    #[test]
    fn parse_done_returns_none() {
        assert!(parse_sse_event("data: [DONE]").is_none());
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_sse_event("").is_none());
        assert!(parse_sse_event(": comment").is_none());
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_sse_event("data: not json").is_none());
    }

    #[test]
    fn call_id_and_item_id_independent() {
        // call_id（call_ 前缀）与 id（fc_ 前缀）必须独立保留，不可合并
        let json = r#"{"type":"function_call","id":"fc_abc","call_id":"call_xyz","name":"n","arguments":""}"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::FunctionCall { id, call_id, .. } => {
                assert_eq!(id, "fc_abc");
                assert_eq!(call_id, "call_xyz");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn function_call_output_uses_call_id() {
        let json = r#"{"type":"function_call_output","id":"fco_1","call_id":"call_xyz","output":"result","status":"completed"}"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::FunctionCallOutput { call_id, .. } => assert_eq!(call_id, "call_xyz"),
            _ => panic!(),
        }
    }

    #[test]
    fn reasoning_carries_encrypted_content() {
        let json = r#"{"type":"reasoning","id":"r_1","encrypted_content":"enc123","summary":[{"type":"summary_text","text":"thinking..."}]}"#;
        let item: OutputItem = serde_json::from_str(json).unwrap();
        match item {
            OutputItem::Reasoning { encrypted_content, summary, .. } => {
                assert_eq!(encrypted_content.as_deref(), Some("enc123"));
                assert_eq!(summary.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn underscore_variant_normalized_to_dot_variant() {
        // 下划线变体应被归一化为点号变体，正确解析为强类型事件
        let line = r#"data: {"type":"response.function_call_arguments_delta","output_index":1,"delta":"{\"a\"}"}"#;
        let ev = parse_sse_event(line).unwrap();
        match ev {
            ResponsesEvent::FunctionCallArgumentsDelta { output_index, .. } => {
                assert_eq!(output_index, 1);
            }
            _ => panic!("expected FunctionCallArgumentsDelta, got {:?}", ev),
        }
    }

    #[test]
    fn underscore_variant_arguments_alias() {
        // 下划线变体可能用 arguments 而非 delta 字段名
        let line = r#"data: {"type":"response.function_call_arguments_delta","output_index":2,"arguments":"{\"b\"}"}"#;
        let ev = parse_sse_event(line).unwrap();
        match ev {
            ResponsesEvent::FunctionCallArgumentsDelta { delta, .. } => {
                assert_eq!(delta, "{\"b\"}");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_web_search_call_events() {
        let ev = parse_sse_event(r#"data: {"type":"response.web_search_call.in_progress","output_index":3,"item_id":"ws_1"}"#).unwrap();
        assert!(matches!(ev, ResponsesEvent::WebSearchCallInProgress { .. }));

        let ev = parse_sse_event(r#"data: {"type":"response.web_search_call.searching","output_index":3,"item_id":"ws_1"}"#).unwrap();
        assert!(matches!(ev, ResponsesEvent::WebSearchCallSearching { .. }));

        let ev = parse_sse_event(r#"data: {"type":"response.web_search_call.completed","output_index":3,"item_id":"ws_1"}"#).unwrap();
        assert!(matches!(ev, ResponsesEvent::WebSearchCallCompleted { .. }));
    }
}
