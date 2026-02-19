use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::logger::{is_debug_log_enabled, AppLogger};
use crate::models::{
    AnthropicRequest, ReasoningEffortMapping, get_reasoning_effort,
};
use super::{TransformBackend, ResponseTransformer, TransformContext, MessageProcessor};

const CODEX_INSTRUCTIONS: &str = include_str!("../instructions.txt");

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

fn strip_leaked_tool_suffix_from_text(text: &str) -> Option<String> {
    let Some(marker_pos) = find_leaked_tool_marker_start(text) else {
        return Some(text.to_string());
    };

    let head = text[..marker_pos].trim_end();
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

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

        // 关键优化：无工具时强制 tool_choice: "none"，避免模型乱吐工具 JSON 到文本
        let tool_choice = if transformed_tools.is_empty() {
            json!("none")
        } else {
            json!("auto")
        };

        let body = json!({
            "model": final_codex_model,
            "instructions": CODEX_INSTRUCTIONS,
            "input": final_input,
            "tools": transformed_tools,
            "tool_choice": tool_choice,
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

                        let block_type = block_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if (block_type == "input_text" || block_type == "output_text" || block_type == "text")
                            && block_obj.get("text").and_then(|v| v.as_str()).is_some()
                        {
                            let text = block_obj
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            if let Some(sanitized_text) = strip_leaked_tool_suffix_from_text(text) {
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
                        let block_type = block_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if block_type == "input_text" || block_type == "output_text" || block_type == "text" {
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
    pending_tool_text: String,
    accumulated_tool_args: String,
    logger: std::sync::Arc<AppLogger>,
}

impl TransformResponse {
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

    fn contains_potential_leaked_tool_marker(line: &str) -> bool {
        line.contains("assistant to=")
            || line.contains("to=functions")
            || line.contains("to=multi_tool_use")
    }

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

    fn parse_leaked_tool_line(line: &str) -> Option<(String, String, String)> {
        let target = Self::extract_leaked_tool_target(line)?;
        let arguments = if let Some(json_start) = line.find('{') {
            let candidate = line[json_start..].trim();
            if serde_json::from_str::<Value>(candidate).is_ok() {
                candidate.to_string()
            } else {
                "{}".to_string()
            }
        } else {
            "{}".to_string()
        };
        let name = target.to_string();

        let call_id = format!("tool_{}", chrono::Utc::now().timestamp_millis());
        Some((call_id, name, arguments))
    }

    fn flush_pending_tool_text(&mut self, output: &mut Vec<String>) {
        if self.pending_tool_text.trim().is_empty() {
            self.pending_tool_text.clear();
            return;
        }

        let pending_raw = std::mem::take(&mut self.pending_tool_text);
        let pending_for_tool_parse = pending_raw.trim();

        // 检查是否是泄漏的工具调用
        if let Some((call_id, name, arguments)) = Self::parse_leaked_tool_line(pending_for_tool_parse) {
            // 关闭文本块（如果有）
            self.close_open_text_block(output);

            // 开启工具块
            self.open_tool_block_if_needed(output, call_id, name);

            // 发送工具参数（即使为空也要发送空对象）
            if !arguments.is_empty() && arguments != "{}" {
                self.emit_tool_json_delta(output, arguments);
            } else {
                // 如果没有参数或参数为空对象，发送空对象
                self.emit_tool_json_delta(output, "{}".to_string());
            }

            // 关闭工具块
            if let Some(idx) = self.open_tool_index.take() {
                output.push(format!(
                    "event: content_block_stop\ndata: {}\n\n",
                    json!({ "type": "content_block_stop", "index": idx })
                ));
            }
            self.tool_call_id = None;
            self.tool_name = None;
            return;
        }

        // 如果不是工具调用，作为普通文本处理
        if !pending_raw.trim().is_empty() {
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
                    "delta": { "type": "text_delta", "text": pending_raw }
                })
            ));
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
                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");

                // 泄漏工具调用文本可能被拆成多个 chunk。
                // 一旦进入泄漏拼接模式，后续 chunk 持续进入 pending，直到遇到换行/收尾再统一解析。
                if !self.pending_tool_text.is_empty() || Self::contains_potential_leaked_tool_marker(delta) {
                    self.pending_tool_text.push_str(delta);
                    if delta.contains('\n') {
                        self.flush_pending_tool_text(&mut output);
                    }
                    return output;
                }

                // 纯文本处理 - 只在没有工具块开启时处理
                if self.open_tool_index.is_none() {
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
                            "delta": { "type": "text_delta", "text": delta }
                        })
                    ));
                }
            }

            // 工具调用开始 - 严格按照 OpenAI Responses 格式解析
            "response.output_item.added" => {
                self.flush_pending_tool_text(&mut output);
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
                self.flush_pending_tool_text(&mut output);
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
                self.flush_pending_tool_text(&mut output);

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

                // 确定停止原因
                let stop_reason = if self.saw_tool_call {
                    "tool_use"
                } else if data.get("response").and_then(|r| r.get("status")).and_then(|s| s.as_str()) == Some("incomplete") {
                    "max_tokens"
                } else {
                    "end_turn"
                };

                // 提取使用统计
                let usage = data
                    .get("response")
                    .and_then(|r| r.get("usage"))
                    .cloned()
                    .unwrap_or_else(|| json!({
                        "input_tokens": 0,
                        "output_tokens": 0
                    }));

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
                self.logger.log(&format!("Ignored event type: {}", event_type));
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
        let out = trait_obj.transform_line(r#"data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":20}}}"#);
        assert!(
            out.iter().any(|chunk| chunk.contains("event: message_stop")),
            "trait dispatch should forward to internal transform logic"
        );
        assert!(
            out.iter().any(|chunk| chunk.contains("\"input_tokens\":10") && chunk.contains("\"output_tokens\":20")),
            "should include usage statistics in message_delta"
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

    #[test]
    fn test_leaked_tool_text_is_promoted_to_tool_use() {
        let mut transformer = TransformResponse::new("gpt-5.3-codex");
        let line = format!(
            "data: {}",
            json!({
                "type": "response.output_text.delta",
                "delta": "assistant to=multi_tool_use.parallel {\"tool_uses\":[]}\n"
            })
        );

        let events = transformer.transform_sse_line(&line);
        let joined = events.join("");
        assert!(
            joined.contains("\"content_block\":{\"id\":\"tool_")
                && joined.contains("\"type\":\"tool_use\""),
            "leaked tool text should be converted into a tool_use block"
        );
        assert!(
            joined.contains("\"name\":\"multi_tool_use.parallel\""),
            "tool_use name should preserve leaked tool target for client-side routing"
        );
    }

    #[test]
    fn test_leaked_parallel_tool_line_without_json_is_still_promoted() {
        let mut transformer = TransformResponse::new("gpt-5.3-codex");
        let line = format!(
            "data: {}",
            json!({
                "type": "response.output_text.delta",
                "delta": "assistant to=multi_tool_use.parallel մեկնաբանություն\n"
            })
        );

        let events = transformer.transform_sse_line(&line);
        let joined = events.join("");
        assert!(
            joined.contains("\"type\":\"tool_use\"")
                && joined.contains("\"name\":\"multi_tool_use.parallel\""),
            "parallel leaked tool line should be promoted even when json payload is missing"
        );
        assert!(
            joined.contains("\"partial_json\":\"{}\""),
            "missing leaked json payload should fall back to empty object arguments"
        );
        assert!(
            !joined.contains("մեկնաբանություն"),
            "leaked tool line suffix should not appear in visible text output"
        );
    }

    #[test]
    fn test_leaked_functions_tool_line_without_assistant_prefix_is_promoted() {
        let mut transformer = TransformResponse::new("gpt-5.3-codex");
        let line = format!(
            "data: {}",
            json!({
                "type": "response.output_text.delta",
                "delta": "numerusform to=functions.Bash {\"command\":\"pwd\",\"description\":\"Check cwd\"}\n"
            })
        );

        let events = transformer.transform_sse_line(&line);
        let joined = events.join("");
        assert!(
            joined.contains("\"type\":\"tool_use\"")
                && joined.contains("\"name\":\"functions.Bash\""),
            "functions leak should be promoted even without assistant prefix"
        );
        assert!(
            joined.contains("\\\"command\\\":\\\"pwd\\\"")
                && joined.contains("\\\"description\\\":\\\"Check cwd\\\""),
            "valid leaked json payload should be forwarded as tool arguments"
        );
        assert!(
            !joined.contains("numerusform"),
            "prefix leak text should not appear in visible assistant output"
        );
    }

    #[test]
    fn test_leaked_functions_tool_line_split_across_chunks_is_promoted() {
        let mut transformer = TransformResponse::new("gpt-5.3-codex");
        let line_1 = format!(
            "data: {}",
            json!({
                "type": "response.output_text.delta",
                "delta": "to=functions.Read "
            })
        );
        let line_2 = format!(
            "data: {}",
            json!({
                "type": "response.output_text.delta",
                "delta": "{\"file_path\":\"/tmp/a.txt\"}\n"
            })
        );

        let events_1 = transformer.transform_sse_line(&line_1);
        let events_2 = transformer.transform_sse_line(&line_2);
        let joined = format!("{}{}", events_1.join(""), events_2.join(""));

        assert!(
            joined.contains("\"type\":\"tool_use\"")
                && joined.contains("\"name\":\"functions.Read\""),
            "split leaked functions line should still be promoted to tool_use"
        );
        assert!(
            joined.contains("\\\"file_path\\\":\\\"/tmp/a.txt\\\""),
            "split leaked json payload should be forwarded as tool arguments"
        );
        assert!(
            !joined.contains("\"type\":\"text_delta\""),
            "split leaked line should not fall through to text output"
        );
    }

    #[test]
    fn test_plain_text_is_not_misclassified_as_tool_use() {
        let mut transformer = TransformResponse::new("gpt-5.3-codex");
        let line = format!(
            "data: {}",
            json!({
                "type": "response.output_text.delta",
                "delta": "你好\n"
            })
        );

        let events = transformer.transform_sse_line(&line);
        let joined = events.join("");
        let has_text_block_payload = joined.contains("\"content_block\":{\"text\":\"\",\"type\":\"text\"}")
            || joined.contains("\"content_block\":{\"type\":\"text\",\"text\":\"\"}");
        assert!(
            joined.contains("\"content_block_start\"") && has_text_block_payload,
            "plain text should open a text block"
        );
        assert!(
            joined.contains("\"type\":\"text_delta\"") && joined.contains("\"text\":\"你好\\n\""),
            "plain text should emit text_delta and preserve newline"
        );
        assert!(
            !joined.contains("\"type\":\"tool_use\""),
            "plain text must not be promoted to tool_use"
        );
    }

    #[test]
    fn test_plain_text_preserves_markdown_line_breaks() {
        let mut transformer = TransformResponse::new("gpt-5.3-codex");
        let line = format!(
            "data: {}",
            json!({
                "type": "response.output_text.delta",
                "delta": "## Rust 入门\n\n1. 语法基础\n2. 核心机制\n"
            })
        );

        let events = transformer.transform_sse_line(&line);
        let joined = events.join("");
        assert!(
            joined.contains("\"text\":\"## Rust 入门\\n\\n1. 语法基础\\n2. 核心机制\\n\""),
            "markdown text should keep line breaks to avoid collapsed layout"
        );
    }

    #[test]
    fn test_codex_input_sanitizes_function_call_name() {
        let request = AnthropicRequest {
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            messages: vec![Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_abc".to_string()),
                    name: "functions.exec_command".to_string(),
                    input: json!({"cmd": "echo hi"}),
                    signature: None,
                }])),
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

        let tool_call = input
            .iter()
            .find(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
            .expect("function_call item should exist");
        assert_eq!(
            tool_call.get("name").and_then(|v| v.as_str()),
            Some("functions_exec_command"),
            "function_call name should be sanitized to codex-accepted pattern"
        );
    }

    #[test]
    fn test_codex_input_all_function_call_names_match_pattern() {
        let request = AnthropicRequest {
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            messages: vec![Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![
                    ContentBlock::ToolUse {
                        id: Some("call_1".to_string()),
                        name: "functions.exec_command".to_string(),
                        input: json!({"cmd": "echo hi"}),
                        signature: None,
                    },
                    ContentBlock::ToolUse {
                        id: Some("call_2".to_string()),
                        name: "multi_tool_use.parallel".to_string(),
                        input: json!({"tool_uses": []}),
                        signature: None,
                    },
                    ContentBlock::ToolUse {
                        id: Some("call_3".to_string()),
                        name: "Valid_Name-01".to_string(),
                        input: json!({"ok": true}),
                        signature: None,
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

        let call_names: Vec<String> = input
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
            .filter_map(|item| item.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();

        assert_eq!(call_names.len(), 3, "expected all tool_use blocks to become function_call");
        for name in call_names {
            assert!(
                !name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'),
                "function_call name '{}' must match ^[a-zA-Z0-9_-]+$",
                name
            );
        }
    }

    #[test]
    fn test_codex_input_strips_leaked_tool_suffix_from_message_text() {
        let request = AnthropicRequest {
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            messages: vec![Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::Text {
                    text: "先起草 design。 to=functions.Write {\"file_path\":\"/tmp/design.md\"}".to_string(),
                }])),
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

        let texts: Vec<String> = input
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("message"))
            .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
            .flat_map(|content| content.iter())
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();

        assert!(
            texts.iter().any(|text| text.contains("先起草 design。")),
            "normal prefix text should be preserved"
        );
        assert!(
            texts.iter().all(|text| !text.contains("to=functions.Write")),
            "leaked tool marker should be stripped from outbound message text"
        );
    }
}
