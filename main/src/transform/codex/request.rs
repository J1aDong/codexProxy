use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::logger::{is_debug_log_enabled, AppLogger};
use crate::models::{get_reasoning_effort, AnthropicRequest, ReasoningEffortMapping};
use crate::transform::MessageProcessor;

const CODEX_INSTRUCTIONS: &str = include_str!("../../instructions.txt");

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

        // 构建 input 数组
        let mut final_input: Vec<Value> = vec![Self::build_template_input()];

        // 注入 system prompt
        if let Some(system) = &anthropic_body.system {
            let system_text = system.to_string();
            log(&format!(
                "📋 [Transform] System prompt: {} chars",
                system_text.len()
            ));

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

            if !skill_injection_prompt.trim().is_empty() {
                log(&format!(
                    "🎯 [Transform] Injecting custom skill prompt ({} chars)",
                    skill_injection_prompt.len()
                ));
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
