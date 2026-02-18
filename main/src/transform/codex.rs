use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::logger::{is_debug_log_enabled, AppLogger};
use crate::models::{
    AnthropicRequest, ReasoningEffortMapping, get_reasoning_effort,
};
use super::{TransformBackend, ResponseTransformer, TransformContext, MessageProcessor};

const CODEX_INSTRUCTIONS: &str = include_str!("../instructions.txt");

/// 请求转换器 - Anthropic -> Codex
pub struct TransformRequest;

impl TransformRequest {
    pub fn transform(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        reasoning_mapping: &ReasoningEffortMapping,
        skill_injection_prompt: &str,
        codex_model: &str,
    ) -> (Value, String) {
        let session_id = Uuid::new_v4().to_string();
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());

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
        let final_codex_model = codex_model.trim().is_empty()
            .then(|| "gpt-5.3-codex")
            .unwrap_or(codex_model);

        log(&format!("🤖 [Transform] {} → {} | 🧠 reasoning: {} (from {})", original_model, final_codex_model, reasoning_effort.as_str(), original_model));

        let (chat_messages, extracted_skills) = MessageProcessor::transform_messages(&anthropic_body.messages, log_tx);

        // 构建 input 数组
        let mut final_input: Vec<Value> = vec![Self::build_template_input()];

        // 注入 system prompt
        if let Some(system) = &anthropic_body.system {
            let system_text = system.to_string();
            log(&format!("📋 [Transform] System prompt: {} chars", system_text.len()));

            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": format!("# AGENTS.md instructions for {}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>", cwd, system_text)
                }]
            }));

            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": format!(r#"<environment_context>
  <cwd>{}</cwd>
  <approval_policy>on-request</approval_policy>
  <sandbox_mode>workspace-write</sandbox_mode>
  <network_access>restricted</network_access>
  <shell>{}</shell>
</environment_context>"#, cwd, std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string()))
                }]
            }));
        }

        // 注入提取的 Skills
        if !extracted_skills.is_empty() {
            log(&format!("🎯 [Transform] Injecting {} skill(s)", extracted_skills.len()));
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

            if !skill_injection_prompt.trim().is_empty() {
                log(&format!("🎯 [Transform] Injecting custom skill prompt ({} chars)", skill_injection_prompt.len()));
                final_input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": skill_injection_prompt
                    }]
                }));
            }
        }

        // 追加对话历史
        final_input.extend(chat_messages);
        Self::sanitize_input_for_codex(&mut final_input);

        // 转换工具
        let transformed_tools = Self::transform_tools(anthropic_body.tools.as_ref(), log_tx);

        log(&format!(
            "📋 [Transform] Final: {} input items, {} tools",
            final_input.len(),
            transformed_tools.len()
        ));

        let body = json!({
            "model": final_codex_model,
            "instructions": CODEX_INSTRUCTIONS,
            "input": final_input,
            "tools": transformed_tools,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": { "effort": reasoning_effort.as_str(), "summary": "auto" },
            "store": false,
            "stream": true,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": session_id
        });

        (body, session_id.clone())
    }

    fn sanitize_input_for_codex(input: &mut [Value]) {
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
            }

            if item_type.as_deref() == Some("message") {
                let role = obj
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("assistant")
                    .to_string();
                if let Some(content_blocks) = obj.get_mut("content").and_then(|v| v.as_array_mut()) {
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
                    }
                }
            }
        }
    }

    fn build_template_input() -> Value {
        // 从 codex-request.json 读取完整的模板，与 JavaScript 版本保持一致
        let template_path = std::path::Path::new("codex-request.json");
        if let Ok(template_content) = std::fs::read_to_string(template_path) {
            if let Ok(template) = serde_json::from_str::<Value>(&template_content) {
                if let Some(input) = template.get("input").and_then(|i| i.as_array()) {
                    if let Some(first_input) = input.first() {
                        return first_input.clone();
                    }
                }
            }
        }
        
        // 如果无法读取模板，使用备用值
        json!({
            "type": "message",
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": "# AGENTS.md instructions for /Users/mr.j\n\n<INSTRUCTIONS>\n---\nname: engineer-professional\ndescription: 专业的软件工程师\n---\n</INSTRUCTIONS>"
            }]
        })
    }


    fn transform_tools(
        tools: Option<&Vec<Value>>,
        log_tx: Option<&broadcast::Sender<String>>,
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
            log("🔧 [Tools] No tools provided, using defaults");
            return Self::default_tools();
        };

        if tools.is_empty() {
            log("🔧 [Tools] Empty tools array, using defaults");
            return Self::default_tools();
        }

        log(&format!("🔧 [Tools] Processing {} tools", tools.len()));

        tools
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

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": tool.get("description").and_then(|d| d.as_str()).unwrap_or(""),
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

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": tool.get("description").and_then(|d| d.as_str()).unwrap_or(""),
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

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": func.get("description").and_then(|d| d.as_str()).unwrap_or(""),
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

                json!({
                    "type": "function",
                    "name": name,
                    "description": tool.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                    "strict": false,
                    "parameters": parameters
                })
            })
            .collect()
    }

    fn default_tools() -> Vec<Value> {
        let template_path = std::path::Path::new("codex-request.json");
        if let Ok(template_content) = std::fs::read_to_string(template_path) {
            if let Ok(template) = serde_json::from_str::<Value>(&template_content) {
                if let Some(tools) = template.get("tools").and_then(|t| t.as_array()) {
                    return tools.clone();
                }
            }
        }
        
        vec![json!({
            "type": "function",
            "name": "shell_command",
            "description": "Runs a shell command and returns its output.",
            "strict": false,
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell script to execute"
                    }
                },
                "required": ["command"]
            }
        })]
    }
}

/// 响应转换器 - Codex SSE -> Anthropic SSE
pub struct TransformResponse {
    message_id: String,
    model: String,
    content_index: usize,
    open_text_index: Option<usize>,
    open_tool_index: Option<usize>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    saw_tool_call: bool,
    sent_message_start: bool,
}

impl TransformResponse {
    pub fn new(model: &str) -> Self {
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            content_index: 0,
            open_text_index: None,
            open_tool_index: None,
            tool_call_id: None,
            tool_name: None,
            saw_tool_call: false,
            sent_message_start: false,
        }
    }

    pub fn transform_sse_line(&mut self, line: &str) -> Vec<String> {
        let mut output = Vec::new();

        if !line.starts_with("data: ") {
            return output;
        }

        // 发送 message_start
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
            // 文本输出
            "response.output_text.delta" => {
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

                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                output.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    json!({
                        "type": "content_block_delta",
                        "index": self.open_text_index,
                        "delta": { "type": "text_delta", "text": delta }
                    })
                ));
            }

            // 工具调用开始
            "response.output_item.added" => {
                if let Some(item) = data.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                        self.saw_tool_call = true;

                        // 关闭文本块
                        if let Some(idx) = self.open_text_index.take() {
                            output.push(format!(
                                "event: content_block_stop\ndata: {}\n\n",
                                json!({ "type": "content_block_stop", "index": idx })
                            ));
                        }

                        let call_id = item
                            .get("call_id")
                            .and_then(|c| c.as_str())
                            .unwrap_or("tool_0")
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();

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
                }
            }

            // 工具调用参数
            "response.function_call_arguments.delta" | "response.function_call_arguments_delta" => {
                if self.open_tool_index.is_none() {
                    self.saw_tool_call = true;

                    // 关闭文本块
                    if let Some(idx) = self.open_text_index.take() {
                        output.push(format!(
                            "event: content_block_stop\ndata: {}\n\n",
                            json!({ "type": "content_block_stop", "index": idx })
                        ));
                    }

                    let call_id = self
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| format!("tool_{}", chrono::Utc::now().timestamp_millis()));
                    let name = self.tool_name.clone().unwrap_or_else(|| "unknown".to_string());

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

                let delta = data
                    .get("delta")
                    .or_else(|| data.get("arguments"))
                    .map(|d| {
                        if d.is_string() {
                            d.as_str().unwrap_or("").to_string()
                        } else {
                            serde_json::to_string(d).unwrap_or_default()
                        }
                    })
                    .unwrap_or_default();

                output.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    json!({
                        "type": "content_block_delta",
                        "index": self.open_tool_index,
                        "delta": { "type": "input_json_delta", "partial_json": delta }
                    })
                ));
            }

            // 工具调用完成
            "response.output_item.done" => {
                if let Some(idx) = self.open_tool_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }
                self.tool_call_id = None;
                self.tool_name = None;
            }

            // 响应完成
            "response.completed" => {
                // 关闭所有打开的块
                if let Some(idx) = self.open_text_index.take() {
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

                let stop_reason = if self.saw_tool_call {
                    "tool_use"
                } else {
                    "end_turn"
                };

                // 发送 message_delta
                let usage = data
                    .get("response")
                    .and_then(|r| r.get("usage"))
                    .cloned()
                    .unwrap_or(json!({}));

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

                // 发送 message_stop
                output.push(format!(
                    "event: message_stop\ndata: {}\n\n",
                    json!({ "type": "message_stop", "stop_reason": stop_reason })
                ));
            }

            _ => {}
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

// ─── CodexBackend ──────────────────────────────────────────────────────

/// Codex 后端 —— 将 Anthropic 请求转为 Codex Responses API 格式
pub struct CodexBackend;

impl TransformBackend for CodexBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        model_override: Option<String>,
    ) -> (Value, String) {
        TransformRequest::transform(
            anthropic_body,
            log_tx,
            &ctx.reasoning_mapping,
            &ctx.skill_injection_prompt,
            model_override.as_deref().unwrap_or(&ctx.codex_model),
        )
    }

    fn build_upstream_request(
        &self,
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        session_id: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        client
            .post(target_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("x-api-key", api_key)
            .header("User-Agent", "Anthropic-Node/0.3.4")
            .header("x-anthropic-version", anthropic_version)
            .header("originator", "codex_cli_rs")
            .header("Accept", "text/event-stream")
            .header("conversation_id", session_id)
            .header("session_id", session_id)
            .body(body.to_string())
    }

    fn create_response_transformer(&self, model: &str) -> Box<dyn ResponseTransformer> {
        Box::new(TransformResponse::new(model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    #[test]
    fn test_reasoning_effort_opus_default() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("claude-3-opus-20240229", &mapping);
        assert_eq!(effort, ReasoningEffort::Xhigh);
    }

    #[test]
    fn test_reasoning_effort_sonnet_default() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("claude-sonnet-4-20250514", &mapping);
        assert_eq!(effort, ReasoningEffort::Medium);
    }

    #[test]
    fn test_reasoning_effort_haiku_default() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("claude-3-5-haiku-20241022", &mapping);
        assert_eq!(effort, ReasoningEffort::Low);
    }

    #[test]
    fn test_custom_mapping_applied() {
        let mut mapping = ReasoningEffortMapping::default();
        mapping.sonnet = ReasoningEffort::High;
        
        let effort = get_reasoning_effort("claude-sonnet-4-20250514", &mapping);
        assert_eq!(effort, ReasoningEffort::High);
    }

    #[test]
    fn test_reasoning_effort_as_str() {
        assert_eq!(ReasoningEffort::Xhigh.as_str(), "xhigh");
        assert_eq!(ReasoningEffort::High.as_str(), "high");
        assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
    }

    #[test]
    fn test_reasoning_effort_from_str() {
        assert_eq!(ReasoningEffort::from_str("xhigh"), ReasoningEffort::Xhigh);
        assert_eq!(ReasoningEffort::from_str("HIGH"), ReasoningEffort::High);
        assert_eq!(ReasoningEffort::from_str("Medium"), ReasoningEffort::Medium);
        assert_eq!(ReasoningEffort::from_str("low"), ReasoningEffort::Low);
        assert_eq!(ReasoningEffort::from_str("invalid"), ReasoningEffort::Medium); // default
    }

    #[test]
    fn test_unknown_model_defaults_to_medium() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("gpt-4-turbo", &mapping);
        assert_eq!(effort, ReasoningEffort::Medium);
    }

    #[test]
    fn test_case_insensitive_model_matching() {
        let mapping = ReasoningEffortMapping::default();
        assert_eq!(get_reasoning_effort("CLAUDE-3-OPUS", &mapping), ReasoningEffort::Xhigh);
        assert_eq!(get_reasoning_effort("Claude-Sonnet-4", &mapping), ReasoningEffort::Medium);
        assert_eq!(get_reasoning_effort("claude-haiku", &mapping), ReasoningEffort::Low);
    }

    #[test]
    fn test_transform_response_trait_dispatch() {
        let mut transformer = TransformResponse::new("gpt-5.3-codex");
        let trait_obj: &mut dyn ResponseTransformer = &mut transformer;
        let out = trait_obj.transform_line(r#"data: {"type":"response.completed"}"#);
        assert!(
            out.iter().any(|chunk| chunk.contains("event: message_stop")),
            "trait dispatch should forward to internal transform logic"
        );
    }

    // Helper to create a fake tool use block
    fn create_tool_use(id: &str, name: &str, input: Value) -> ContentBlock {
        ContentBlock::ToolUse {
            id: Some(id.to_string()),
            name: name.to_string(),
            input,
            signature: None,
        }
    }

    // Helper to create a fake tool result block
    fn create_tool_result(tool_use_id: &str, content: &str) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: Some(tool_use_id.to_string()),
            id: Some("result_id".to_string()),
            content: Some(json!(content)),
        }
    }

    #[test]
    fn test_skill_transformation() {
        // Mock messages
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_use("call_1", "skill", json!({
                        "skill": "test-skill",
                        "args": "arg1"
                    }))
                ]))
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_result("call_1", "<command-name>test-skill</command-name>\nBase Path: /tmp\nSome content")
                ]))
            }
        ];

        let (input, skills) = MessageProcessor::transform_messages(&messages, None);

        // Verify skills extracted
        assert_eq!(skills.len(), 1);
        assert!(skills[0].contains("<name>test-skill</name>"));
        assert!(skills[0].contains("Some content"));

        // Verify input structure
        // Find function_call
        let func_call = input.iter().find(|v| v["type"] == "function_call").expect("Should have function_call");
        assert_eq!(func_call["name"], "skill");
        let args_str = func_call["arguments"].as_str().unwrap();
        let args: Value = serde_json::from_str(args_str).unwrap();
        assert_eq!(args["command"], "test-skill arg1");

        // Find function_call_output
        let func_out = input.iter().find(|v| v["type"] == "function_call_output").expect("Should have function_call_output");
        assert_eq!(func_out["output"], "Skill 'test-skill' loaded.");
    }

    #[test]
    fn test_skill_deduplication() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_use("call_1", "skill", json!({"command": "test-skill"}))
                ]))
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_result("call_1", "<command-name>test-skill</command-name>\nBase Path: /tmp\nContent 1")
                ]))
            },
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_use("call_2", "skill", json!({"command": "test-skill"}))
                ]))
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_result("call_2", "<command-name>test-skill</command-name>\nBase Path: /tmp\nContent 1")
                ]))
            }
        ];

        let (input, skills) = MessageProcessor::transform_messages(&messages, None);

        // Should only extract once
        assert_eq!(skills.len(), 1);
        
        // But should have two outputs
        let outputs: Vec<_> = input.iter().filter(|v| v["type"] == "function_call_output").collect();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0]["output"], "Skill 'test-skill' loaded.");
        assert_eq!(outputs[1]["output"], "Skill 'test-skill' loaded.");
    }

    #[test]
    fn test_skill_injection_prompt() {
        // Setup request with skill usage
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_use("call_1", "skill", json!({"command": "test-skill"}))
                ]))
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    create_tool_result("call_1", "<command-name>test-skill</command-name>\nBase Path: /tmp\nContent")
                ]))
            }
        ];
        
        let request = AnthropicRequest {
            model: Some("claude-3-opus".to_string()),
            messages,
            system: None,
            stream: false,
            tools: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let mapping = ReasoningEffortMapping::default();
        let prompt = "Auto-install dependencies please.";
        
        let (body, _) = TransformRequest::transform(&request, None, &mapping, prompt, "gpt-5.3-codex");
        
        let input_arr = body.get("input").unwrap().as_array().unwrap();
        
        // Find the injected prompt
        // It should be after the skill injection.
        // Input structure: [Template, Skill, Prompt, ...History]
        // Since history starts with assistant, and we inject user messages.
        
        // Let's look for the prompt text
        let prompt_msg = input_arr.iter().find(|msg| {
            msg["role"] == "user" && 
            msg["content"][0]["text"].as_str().unwrap_or("") == prompt
        });
        
        assert!(prompt_msg.is_some(), "Should inject custom prompt");
    }

    #[test]
    fn test_codex_input_strips_signature_fields_and_normalizes_thinking_type() {
        let request = AnthropicRequest {
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            messages: vec![
                Message {
                    role: "assistant".to_string(),
                    content: Some(MessageContent::Blocks(vec![
                        ContentBlock::ToolUse {
                            id: Some("call_123".to_string()),
                            name: "WebFetch".to_string(),
                            input: json!({"url": "https://example.com"}),
                            signature: Some("sig_tool_abc".to_string()),
                        },
                    ])),
                },
                Message {
                    role: "assistant".to_string(),
                    content: Some(MessageContent::Blocks(vec![
                        ContentBlock::Thinking {
                            thinking: "internal".to_string(),
                            signature: Some("sig_thinking_abc".to_string()),
                        },
                    ])),
                },
            ],
            system: None,
            stream: true,
            tools: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let mapping = ReasoningEffortMapping::default();
        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");

        let input = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input should be an array");

        let tool_call = input
            .iter()
            .find(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
            .expect("function_call item should exist");
        assert!(
            tool_call.get("signature").is_none(),
            "function_call signature should be stripped for codex"
        );

        let normalized_block = input
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
            .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
            .flat_map(|content| content.iter())
            .find(|block| block.get("type").and_then(|v| v.as_str()) == Some("output_text"))
            .expect("thinking block should be normalized to output_text");
        assert!(
            normalized_block.get("signature").is_none(),
            "normalized block signature should be stripped for codex"
        );
        assert_eq!(
            normalized_block
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "internal",
            "thinking text should be preserved after normalization"
        );
        assert!(
            normalized_block.get("thinking").is_none(),
            "legacy thinking field should be removed after normalization"
        );

        let has_thinking_type = input
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
            .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
            .flat_map(|content| content.iter())
            .any(|block| block.get("type").and_then(|v| v.as_str()) == Some("thinking"));
        assert!(!has_thinking_type, "codex payload must not contain thinking type");

        let has_summary_text_type = input
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
            .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
            .flat_map(|content| content.iter())
            .any(|block| block.get("type").and_then(|v| v.as_str()) == Some("summary_text"));
        assert!(
            !has_summary_text_type,
            "codex message.content should not use summary_text type"
        );
    }

    #[test]
    fn test_codex_input_normalizes_multiple_thinking_blocks() {
        let request = AnthropicRequest {
            model: Some("claude-opus-4-6".to_string()),
            messages: vec![Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    ContentBlock::Thinking {
                        thinking: "first".to_string(),
                        signature: Some("sig_1".to_string()),
                    },
                    ContentBlock::Text {
                        text: "visible".to_string(),
                    },
                    ContentBlock::Thinking {
                        thinking: "second".to_string(),
                        signature: Some("sig_2".to_string()),
                    },
                ])),
            }],
            system: None,
            stream: true,
            tools: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let mapping = ReasoningEffortMapping::default();
        let (body, _) = TransformRequest::transform(&request, None, &mapping, "", "gpt-5.3-codex");
        let input = body
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input should be an array");

        let normalized_texts: Vec<String> = input
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
            .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
            .flat_map(|content| content.iter())
            .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("output_text"))
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();

        assert!(
            normalized_texts.contains(&"first".to_string()),
            "first thinking block should be normalized to output_text"
        );
        assert!(
            normalized_texts.contains(&"second".to_string()),
            "second thinking block should be normalized to output_text"
        );
    }
}
