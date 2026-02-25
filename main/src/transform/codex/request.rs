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

fn looks_like_high_confidence_tool_json_object(json: &str) -> bool {
    let trimmed = json.trim_start();
    if !trimmed.starts_with('{') {
        return false;
    }

    let is_parallel_tool_json = trimmed.contains("\"tool_uses\"")
        && trimmed.contains("\"recipient_name\"")
        && (trimmed.contains("functions.") || trimmed.contains("multi_tool_use."));
    if is_parallel_tool_json {
        return true;
    }

    let is_single_edit_json = trimmed.contains("\"file_path\"")
        && ((trimmed.contains("\"old_string\"") && trimmed.contains("\"new_string\""))
            || trimmed.contains("\"replace_all\""));
    if is_single_edit_json {
        return true;
    }

    let is_basic_tool_envelope = trimmed.contains("\"recipient_name\"")
        && trimmed.contains("\"parameters\"")
        && (trimmed.contains("\"file_path\"")
            || trimmed.contains("\"pattern\"")
            || trimmed.contains("\"command\""));
    if is_basic_tool_envelope {
        return true;
    }

    let is_exec_command_payload = serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|obj| {
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
        })
        .unwrap_or(false);

    is_exec_command_payload
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

fn find_high_confidence_tool_json_tail_start(text: &str) -> Option<usize> {
    let mut matched_start = None;

    for (idx, ch) in text.char_indices() {
        if ch != '{' {
            continue;
        }

        let candidate = &text[idx..];
        let Some(json_object) = extract_first_json_object_fragment(candidate) else {
            continue;
        };

        if !looks_like_high_confidence_tool_json_object(&json_object) {
            continue;
        }

        let suffix = &candidate[json_object.len()..];
        if suffix.trim().is_empty() {
            matched_start = Some(idx);
        }
    }

    matched_start
}

fn strip_leaked_tool_suffix_from_text(text: &str) -> Option<String> {
    if let Some(marker_pos) = find_leaked_tool_marker_start(text) {
        let head = text[..marker_pos].trim_end();
        return if head.is_empty() {
            None
        } else {
            Some(head.to_string())
        };
    }

    if let Some(json_start) = find_high_confidence_tool_json_tail_start(text) {
        let head = text[..json_start].trim_end();
        return if head.is_empty() {
            None
        } else {
            Some(head.to_string())
        };
    }

    Some(text.to_string())
}

fn strip_system_reminder_blocks(text: &str) -> String {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";

    let mut remaining = text;
    let mut sanitized = String::with_capacity(text.len());

    loop {
        let Some(start_idx) = remaining.find(START) else {
            sanitized.push_str(remaining);
            break;
        };

        sanitized.push_str(&remaining[..start_idx]);
        let after_start = &remaining[start_idx + START.len()..];
        let Some(end_rel) = after_start.find(END) else {
            break;
        };
        remaining = &after_start[end_rel + END.len()..];
    }

    sanitized
}

fn looks_like_suggestion_mode_prompt(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    upper.contains("[SUGGESTION MODE:")
        && upper.contains("REPLY WITH ONLY THE SUGGESTION")
        && upper.contains("WHAT THE USER MIGHT NATURALLY TYPE NEXT")
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

    out
}

fn strip_known_trailing_noise(text: &str) -> String {
    let mut result = text.trim_end().to_string();
    let noise_patterns = [
        "assistantuser",
        "numeroususer",
        "numerusform",
        "天天中彩票user",
        "天天中彩票",
        " +#+#+#+#+#+",
    ];

    for pattern in noise_patterns {
        if result.ends_with(pattern) {
            result.truncate(result.len().saturating_sub(pattern.len()));
            return result.trim_end().to_string();
        }
    }

    result
}

fn sanitize_message_text_for_codex(text: &str) -> Option<String> {
    let without_reminder = strip_system_reminder_blocks(text);
    if without_reminder.trim().is_empty() {
        return None;
    }

    if looks_like_suggestion_mode_prompt(&without_reminder) {
        return None;
    }

    let stripped = strip_leaked_tool_suffix_from_text(&without_reminder)?;
    let collapsed = collapse_adjacent_duplicate_markdown_bold(&stripped);
    let cleaned = strip_known_trailing_noise(&collapsed);
    let final_text = cleaned.trim_end().to_string();

    if final_text.trim().is_empty() {
        None
    } else {
        Some(final_text)
    }
}

fn sanitize_function_call_output_for_codex(output: &str) -> Option<String> {
    let sanitized = strip_system_reminder_blocks(output);
    let trimmed = sanitized.trim_end().to_string();
    if trimmed.trim().is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// 请求转换器 - Anthropic -> Codex
pub struct TransformRequest;

impl TransformRequest {
    pub fn transform(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        reasoning_mapping: &ReasoningEffortMapping,
        custom_injection_prompt: &str,
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

        // 构建 input 数组（只包含当前请求上下文，不注入静态模板文件）
        let mut final_input: Vec<Value> = Vec::new();

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
        }

        if !custom_injection_prompt.trim().is_empty() {
            log(&format!(
                "🎯 [Transform] Injecting custom global prompt ({} chars)",
                custom_injection_prompt.len()
            ));
            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": custom_injection_prompt
                }]
            }));
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

    fn sanitize_input_for_codex(input: &mut Vec<Value>) {
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
                            if let Some(sanitized_text) = sanitize_message_text_for_codex(text) {
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

            if item_type.as_deref() == Some("function_call_output")
                && obj.get("output").and_then(|v| v.as_str()).is_some()
            {
                let output = obj
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if let Some(sanitized_output) = sanitize_function_call_output_for_codex(output) {
                    obj.insert("output".to_string(), json!(sanitized_output));
                } else {
                    obj.insert("output".to_string(), json!(""));
                }
            }
        }

        input.retain(|item| {
            let Some(obj) = item.as_object() else {
                return true;
            };

            match obj.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "message" => obj
                    .get("content")
                    .and_then(|v| v.as_array())
                    .map(|blocks| !blocks.is_empty())
                    .unwrap_or(false),
                "function_call_output" => obj
                    .get("output")
                    .and_then(|v| v.as_str())
                    .map(|text| !text.trim().is_empty())
                    .unwrap_or(true),
                _ => true,
            }
        });
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
            log("🔧 [Tools] No tools provided");
            return Vec::new();
        };

        if tools.is_empty() {
            log("🔧 [Tools] Empty tools array");
            return Vec::new();
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
}
