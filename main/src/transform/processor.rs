use crate::logger::{is_debug_log_enabled, truncate_for_log, AppLogger};
use crate::models::{ContentBlock, ImageSource, ImageUrlValue, Message, MessageContent};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tokio::sync::broadcast;

pub const IMAGE_SYSTEM_HINT: &str = "\n<system_hint>IMAGE PROVIDED. You can see the image above directly. Analyze it as requested. DO NOT ask for file paths.</system_hint>\n";
const MAX_SKILL_CONTENT_CHARS: usize = 4_000;
const MAX_TOTAL_SKILL_CHARS: usize = 12_000;
const SKILL_TRUNCATION_MARKER: &str = "\n[skill content truncated]";

pub struct MessageProcessor;

impl MessageProcessor {
    fn extract_teammate_message_bodies(text: &str) -> Vec<String> {
        let mut bodies = Vec::new();
        let mut rest = text;
        let close_tag = "</teammate-message>";

        while let Some(start) = rest.find("<teammate-message") {
            let after_open = &rest[start..];
            let Some(open_end_rel) = after_open.find('>') else {
                break;
            };
            let body_start = start + open_end_rel + 1;
            let Some(close_rel) = rest[body_start..].find(close_tag) else {
                break;
            };
            let body_end = body_start + close_rel;
            bodies.push(rest[body_start..body_end].trim().to_string());
            rest = &rest[body_end + close_tag.len()..];
        }

        bodies
    }

    fn rewrite_teammate_protocol_text(text: &str) -> Option<String> {
        let trimmed = text.trim();
        if !trimmed.contains("<teammate-message") {
            return None;
        }

        if trimmed.contains("summary=\"Acknowledge shutdown and exit\"")
            && trimmed.contains("shutdown_request")
        {
            return Some(
                "Teammate sent a plain-text shutdown acknowledgment. This is not a valid shutdown_response, so the teammate may still be active. TeamDelete will keep failing until the teammate sends a structured shutdown_response to `team-lead`."
                    .to_string(),
            );
        }

        for body in Self::extract_teammate_message_bodies(trimmed) {
            let Ok(payload) = serde_json::from_str::<Value>(&body) else {
                continue;
            };
            let Some(kind) = payload.get("type").and_then(|value| value.as_str()) else {
                continue;
            };

            if kind == "shutdown_request" {
                let request_id = payload
                    .get("requestId")
                    .or_else(|| payload.get("request_id"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let from = payload
                    .get("from")
                    .and_then(|value| value.as_str())
                    .unwrap_or("team-lead");
                let reason = payload
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");

                return Some(format!(
                    "Structured teammate protocol message from `{from}`: shutdown_request (request_id `{request_id}`). Reason: {reason}\n\nDo NOT reply with plain text. Use SendMessage to send a structured `shutdown_response` to `team-lead` with this exact request_id. If approving shutdown, send `approve: true` and then exit."
                ));
            }

            if kind == "plan_approval_request" {
                let request_id = payload
                    .get("requestId")
                    .or_else(|| payload.get("request_id"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let from = payload
                    .get("from")
                    .and_then(|value| value.as_str())
                    .unwrap_or("team-lead");

                return Some(format!(
                    "Structured teammate protocol message from `{from}`: plan_approval_request (request_id `{request_id}`).\n\nDo NOT reply with plain text. Use SendMessage to send a structured `plan_approval_response` to `{from}` with this exact request_id."
                ));
            }
        }

        None
    }

    fn extract_tool_result_plain_text(content: &Value) -> Option<String> {
        match content {
            Value::String(text) => Some(text.clone()),
            Value::Array(items) => {
                let parts: Vec<String> = items
                    .iter()
                    .filter_map(|item| match item {
                        Value::String(text) => Some(text.clone()),
                        Value::Object(obj) => obj
                            .get("text")
                            .and_then(|value| value.as_str())
                            .map(|text| text.to_string()),
                        _ => None,
                    })
                    .collect();

                (!parts.is_empty()).then(|| parts.join("\n"))
            }
            Value::Object(obj) => obj
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| text.to_string()),
            _ => None,
        }
    }

    fn rewrite_agent_tool_result(content: &Value) -> Option<Value> {
        let text = Self::extract_tool_result_plain_text(content)?;
        let trimmed = text.trim();
        if !trimmed.starts_with("Spawned successfully.") {
            return None;
        }
        if !trimmed.contains("agent_id:") || !trimmed.contains("team_name:") {
            return None;
        }

        let read_field = |label: &str| -> Option<String> {
            trimmed
                .lines()
                .find_map(|line| line.trim().strip_prefix(label))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
        };

        let agent_id = read_field("agent_id:").unwrap_or_default();
        let teammate_name = read_field("name:").unwrap_or_default();
        let team_name = read_field("team_name:").unwrap_or_default();

        if agent_id.is_empty() || team_name.is_empty() {
            return None;
        }

        let rewritten = format!(
            "Spawned successfully.\nagent_id: {agent_id}\nname: {teammate_name}\nteam_name: {team_name}\nThe agent is now running and will receive instructions via mailbox.\n\nImportant: this is a team mailbox agent id, not a TaskOutput task_id. Do not call TaskOutput with this agent_id. Wait for teammate-message or idle_notification updates instead."
        );
        Some(Value::String(rewritten))
    }

    fn rewrite_task_output_tool_result(content: &Value) -> Option<Value> {
        let text = Self::extract_tool_result_plain_text(content)?;
        let trimmed = text.trim();
        let missing_prefix = "No task found with ID:";

        let missing_id = trimmed
            .strip_prefix("<tool_use_error>")
            .and_then(|inner| inner.strip_suffix("</tool_use_error>"))
            .map(str::trim)
            .or(Some(trimmed))
            .and_then(|inner| inner.strip_prefix(missing_prefix))
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        if !missing_id.contains('@') {
            return None;
        }

        let rewritten = format!(
            "TaskOutput cannot query team agent IDs like `{missing_id}`. This is a mailbox-style agent id rather than a TaskOutput task_id. Wait for teammate-message or idle_notification updates instead of polling TaskOutput with this id."
        );
        Some(Value::String(rewritten))
    }

    fn rewrite_tool_result_output(tool_name: Option<&str>, content: &Value) -> Option<Value> {
        let tool_name = tool_name?;

        if tool_name.eq_ignore_ascii_case("Agent") {
            return Self::rewrite_agent_tool_result(content);
        }

        if tool_name.eq_ignore_ascii_case("TaskOutput") {
            return Self::rewrite_task_output_tool_result(content);
        }

        None
    }

    fn normalize_skill_tool_input_for_history(input: &mut Value) {
        let Some(obj) = input.as_object_mut() else {
            return;
        };

        if obj
            .get("skill")
            .and_then(|value| value.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            return;
        }

        let Some(command) = obj
            .get("command")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
        else {
            return;
        };

        let mut parts = command.splitn(2, char::is_whitespace);
        let Some(skill) = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
        else {
            return;
        };
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
    }

    pub fn transform_messages(
        messages: &[Message],
        log_tx: Option<&broadcast::Sender<String>>,
    ) -> (Vec<Value>, Vec<String>) {
        let mut input = Vec::new();
        let mut extracted_skills = Vec::new();
        let mut extracted_skill_keys = std::collections::HashSet::new();
        let mut extracted_skill_chars = 0usize;
        let mut skill_tool_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut tool_name_by_id: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

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

        log(&format!(
            "📝 [Messages] Processing {} messages",
            messages.len()
        ));

        // 第一遍：收集 skill tool ids
        for msg in messages {
            if let Some(MessageContent::Blocks(blocks)) = &msg.content {
                for block in blocks {
                    if let ContentBlock::ToolUse { id, name, .. } = block {
                        if let Some(tool_id) = id {
                            tool_name_by_id.insert(tool_id.clone(), name.clone());
                        }
                        if name.to_lowercase() == "skill" {
                            if let Some(tool_id) = id {
                                skill_tool_ids.insert(tool_id.clone());
                            }
                        }
                    }
                }
            }
        }

        // 第二遍：转换消息
        for (msg_idx, msg) in messages.iter().enumerate() {
            if msg.role == "system" {
                continue;
            }

            if msg.role != "user" && msg.role != "assistant" {
                continue;
            }

            let text_type = if msg.role == "user" {
                "input_text"
            } else {
                "output_text"
            };

            let Some(content) = &msg.content else {
                log(&format!(
                    "📝 [Message #{}] role={}, content=null (skipped)",
                    msg_idx, msg.role
                ));
                continue;
            };

            match content {
                MessageContent::Text(text) => {
                    let text = if msg.role == "user" {
                        Self::rewrite_teammate_protocol_text(text).unwrap_or_else(|| text.clone())
                    } else {
                        text.clone()
                    };
                    log(&format!(
                        "📝 [Message #{}] role={}, type=Text, len={}",
                        msg_idx,
                        msg.role,
                        text.len()
                    ));
                    input.push(json!({
                        "type": "message",
                        "role": msg.role,
                        "content": [{
                            "type": text_type,
                            "text": text
                        }]
                    }));
                }
                MessageContent::Blocks(blocks) => {
                    log(&format!(
                        "📝 [Message #{}] role={}, type=Blocks({})",
                        msg_idx,
                        msg.role,
                        blocks.len()
                    ));

                    let mut current_msg_content = Vec::new();
                    let mut image_hint_added = false;
                    let mut ensure_image_hint = |current_msg_content: &mut Vec<Value>| {
                        if image_hint_added {
                            return;
                        }
                        let already_has_hint = current_msg_content.iter().any(|item| {
                            item.get("type").and_then(|t| t.as_str()) == Some("input_text")
                                && item
                                    .get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|t| t.contains("IMAGE PROVIDED"))
                                    .unwrap_or(false)
                        });
                        if !already_has_hint {
                            current_msg_content.push(json!({
                                "type": "input_text",
                                "text": IMAGE_SYSTEM_HINT
                            }));
                        }
                        image_hint_added = true;
                    };

                    for (block_idx, block) in blocks.iter().enumerate() {
                        match block {
                            ContentBlock::Text { text } => {
                                let text = if msg.role == "user" {
                                    Self::rewrite_teammate_protocol_text(text)
                                        .unwrap_or_else(|| text.clone())
                                } else {
                                    text.clone()
                                };
                                current_msg_content.push(json!({
                                    "type": text_type,
                                    "text": text
                                }));
                            }
                            ContentBlock::Thinking {
                                thinking,
                                signature,
                            } => {
                                current_msg_content.push(json!({
                                    "type": "thinking",
                                    "thinking": thinking,
                                    "signature": signature
                                }));
                            }
                            ContentBlock::Image {
                                source,
                                source_raw,
                                image_url,
                            } => {
                                let mut resolved_url = if let Some(image_url) = image_url {
                                    match image_url {
                                        ImageUrlValue::Str(s) => s.clone(),
                                        ImageUrlValue::ObjUrl { url } => url.clone(),
                                        ImageUrlValue::ObjUri { uri } => uri.clone(),
                                    }
                                } else if let Some(src) = source {
                                    Self::resolve_image_url(src, &log, msg_idx, block_idx)
                                } else {
                                    String::new()
                                };

                                if !resolved_url.is_empty() {
                                    let media_type = source.as_ref().and_then(|s| {
                                        s.media_type.as_deref().or(s.mime_type.as_deref())
                                    });
                                    resolved_url = Self::normalize_image_url(
                                        resolved_url,
                                        media_type,
                                        &log,
                                        msg_idx,
                                        block_idx,
                                    );
                                }

                                if resolved_url.is_empty() {
                                    if let Some(raw) = source_raw {
                                        resolved_url = Self::resolve_image_url_raw(
                                            raw, &log, msg_idx, block_idx,
                                        );
                                    }
                                }

                                if !resolved_url.is_empty() && msg.role == "user" {
                                    ensure_image_hint(&mut current_msg_content);
                                    log(&format!(
                                        "🖼️ [Message #{} Block #{}] Image processed (len={})",
                                        msg_idx,
                                        block_idx,
                                        resolved_url.len()
                                    ));
                                    current_msg_content.push(json!({
                                        "type": "input_image",
                                        "image_url": resolved_url,
                                        "detail": "auto"
                                    }));
                                }
                            }
                            ContentBlock::ImageUrl { image_url } => {
                                let url = match image_url {
                                    ImageUrlValue::Str(s) => s.clone(),
                                    ImageUrlValue::ObjUrl { url } => url.clone(),
                                    ImageUrlValue::ObjUri { uri } => uri.clone(),
                                };
                                let url =
                                    Self::normalize_image_url(url, None, &log, msg_idx, block_idx);
                                if !url.is_empty() && msg.role == "user" {
                                    ensure_image_hint(&mut current_msg_content);
                                    log(&format!(
                                        "🖼️ [Message #{} Block #{}] ImageUrl processed (len={})",
                                        msg_idx,
                                        block_idx,
                                        url.len()
                                    ));
                                    current_msg_content.push(json!({
                                        "type": "input_image",
                                        "image_url": url,
                                        "detail": "auto"
                                    }));
                                }
                            }
                            ContentBlock::InputImage { image_url, url, .. } => {
                                let resolved_url = match image_url {
                                    Some(ImageUrlValue::Str(s)) => s.clone(),
                                    Some(ImageUrlValue::ObjUrl { url }) => url.clone(),
                                    Some(ImageUrlValue::ObjUri { uri }) => uri.clone(),
                                    None => url.clone().unwrap_or_default(),
                                };
                                let resolved_url = Self::normalize_image_url(
                                    resolved_url,
                                    None,
                                    &log,
                                    msg_idx,
                                    block_idx,
                                );
                                if !resolved_url.is_empty() && msg.role == "user" {
                                    ensure_image_hint(&mut current_msg_content);
                                    log(&format!(
                                        "🖼️ [Message #{} Block #{}] InputImage processed (len={})",
                                        msg_idx,
                                        block_idx,
                                        resolved_url.len()
                                    ));
                                    current_msg_content.push(json!({
                                        "type": "input_image",
                                        "image_url": resolved_url,
                                        "detail": "auto"
                                    }));
                                }
                            }
                            ContentBlock::ToolUse {
                                id,
                                name,
                                input: tool_input,
                                signature,
                            } => {
                                if !current_msg_content.is_empty() {
                                    input.push(json!({
                                        "type": "message",
                                        "role": msg.role,
                                        "content": current_msg_content
                                    }));
                                    current_msg_content = Vec::new();
                                }

                                let mut final_tool_input = tool_input.clone();
                                if name.eq_ignore_ascii_case("skill") {
                                    Self::normalize_skill_tool_input_for_history(
                                        &mut final_tool_input,
                                    );
                                }

                                input.push(json!({
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": serde_json::to_string(&final_tool_input).unwrap_or_default(),
                                    "signature": signature
                                }));
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content: tool_content,
                                ..
                            } => {
                                let is_skill = if let Some(tid) = tool_use_id {
                                    skill_tool_ids.contains(tid)
                                } else {
                                    false
                                };

                                let mut override_result_text = None;

                                if is_skill || Self::is_potential_skill_result(tool_content) {
                                    if let Some((s_name, s_content)) =
                                        Self::extract_skill_info(tool_content)
                                    {
                                        let skill_key = Self::build_skill_key(&s_name, &s_content);
                                        if !extracted_skill_keys.contains(&skill_key) {
                                            let remaining_budget = MAX_TOTAL_SKILL_CHARS
                                                .saturating_sub(extracted_skill_chars);
                                            if let Some(skill_formatted) =
                                                Self::build_limited_skill_payload(
                                                    &s_name,
                                                    &s_content,
                                                    remaining_budget,
                                                )
                                            {
                                                extracted_skill_chars +=
                                                    skill_formatted.chars().count();
                                                extracted_skills.push(skill_formatted);
                                                extracted_skill_keys.insert(skill_key);
                                                log(&format!(
                                                    "🎯 Skill extracted: {} (total_chars={})",
                                                    s_name, extracted_skill_chars
                                                ));
                                            } else {
                                                log(&format!(
                                                    "🎯 Skill skipped due budget limit: {}",
                                                    s_name
                                                ));
                                            }
                                        } else {
                                            log(&format!(
                                                "🎯 Skill already extracted (deduped by name+content): {}",
                                                s_name
                                            ));
                                        }
                                        override_result_text =
                                            Some(format!("Skill '{}' loaded.", s_name));
                                    }
                                }

                                if !current_msg_content.is_empty() {
                                    input.push(json!({
                                        "type": "message",
                                        "role": msg.role,
                                        "content": current_msg_content
                                    }));
                                    current_msg_content = Vec::new();
                                }

                                let result_output = if let Some(override_text) =
                                    override_result_text
                                {
                                    Value::String(override_text)
                                } else if let Some(rewritten) = tool_use_id
                                    .as_deref()
                                    .and_then(|tid| tool_name_by_id.get(tid).map(|name| name.as_str()))
                                    .and_then(|tool_name| {
                                        tool_content
                                            .as_ref()
                                            .and_then(|content| {
                                                Self::rewrite_tool_result_output(
                                                    Some(tool_name),
                                                    content,
                                                )
                                            })
                                    })
                                {
                                    rewritten
                                } else if let Some(cv) = tool_content {
                                    Self::transform_tool_result_output(cv, &log, msg_idx, block_idx)
                                } else {
                                    Value::String(String::new())
                                };

                                input.push(json!({
                                    "type": "function_call_output",
                                    "call_id": tool_use_id,
                                    "output": result_output
                                }));
                            }
                            ContentBlock::Document { .. } => {
                                current_msg_content.push(json!({
                                    "type": text_type,
                                    "text": "[document omitted]"
                                }));
                            }
                            ContentBlock::OtherValue(v) => {
                                let text = serde_json::to_string(v)
                                    .unwrap_or_else(|_| "[unknown content]".to_string());
                                current_msg_content.push(json!({
                                    "type": text_type,
                                    "text": text
                                }));
                            }
                        }
                    }

                    if !current_msg_content.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": msg.role,
                            "content": current_msg_content
                        }));
                    }
                }
            }
        }

        (input, extracted_skills)
    }

    fn normalize_image_url<F>(
        url: String,
        _media_type: Option<&str>,
        _log: &F,
        _msg_idx: usize,
        _block_idx: usize,
    ) -> String
    where
        F: Fn(&str),
    {
        url
    }

    fn resolve_image_url<F>(
        source: &ImageSource,
        log: &F,
        msg_idx: usize,
        block_idx: usize,
    ) -> String
    where
        F: Fn(&str),
    {
        if let Some(url) = &source.url {
            let media_type = source
                .media_type
                .as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");
            let normalized =
                Self::normalize_image_url(url.clone(), Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source.url: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(uri) = &source.uri {
            let media_type = source
                .media_type
                .as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");
            let normalized =
                Self::normalize_image_url(uri.clone(), Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source.uri: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(path) = &source.path {
            let media_type = source
                .media_type
                .as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");
            let file_url = if path.starts_with("file://") {
                path.clone()
            } else {
                format!("file://{}", path)
            };
            let normalized =
                Self::normalize_image_url(file_url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source.path: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(data) = &source.data {
            let media_type = source
                .media_type
                .as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");

            log(&format!(
                "🖼️ [Message #{} Block #{}] Image base64: media={}, size={} bytes, prefix={}",
                msg_idx,
                block_idx,
                media_type,
                data.len(),
                truncate_for_log(data, 20)
            ));

            if data.starts_with("data:") {
                return data.clone();
            }
            return format!("data:{};base64,{}", media_type, data);
        }

        log(&format!(
            "🖼️ [Message #{} Block #{}] Image source is empty (no url/uri/data)",
            msg_idx, block_idx
        ));
        String::new()
    }

    fn resolve_image_url_raw<F>(source: &Value, log: &F, msg_idx: usize, block_idx: usize) -> String
    where
        F: Fn(&str),
    {
        let Some(obj) = source.as_object() else {
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw is not object",
                msg_idx, block_idx
            ));
            return String::new();
        };

        let keys = obj.keys().cloned().collect::<Vec<_>>().join(",");
        log(&format!(
            "🖼️ [Message #{} Block #{}] Image source raw keys: {}",
            msg_idx, block_idx, keys
        ));

        let media_type = obj
            .get("media_type")
            .or_else(|| obj.get("mediaType"))
            .or_else(|| obj.get("mime_type"))
            .or_else(|| obj.get("mimeType"))
            .and_then(|v| v.as_str())
            .unwrap_or("image/png");

        let extract_str = |value: &Value| -> Option<String> {
            if let Some(s) = value.as_str() {
                return Some(s.to_string());
            }
            if let Some(obj) = value.as_object() {
                if let Some(s) = obj.get("url").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
                if let Some(s) = obj.get("uri").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
                if let Some(s) = obj.get("data").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
                if let Some(s) = obj.get("base64").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
            }
            None
        };

        if let Some(url) = obj.get("url").and_then(|v| extract_str(v)) {
            let normalized =
                Self::normalize_image_url(url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.url: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(uri) = obj.get("uri").and_then(|v| extract_str(v)) {
            let normalized =
                Self::normalize_image_url(uri, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.uri: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(image_url) = obj.get("image_url").and_then(|v| extract_str(v)) {
            let normalized =
                Self::normalize_image_url(image_url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.image_url: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        let path_value = obj
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                obj.get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                obj.get("filePath")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                obj.get("local_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                obj.get("localPath")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                obj.get("file")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

        if let Some(path) = path_value {
            let file_url = if path.starts_with("file://") {
                path
            } else {
                format!("file://{}", path)
            };
            let normalized =
                Self::normalize_image_url(file_url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.path: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        let data = obj.get("data").and_then(|v| extract_str(v)).or_else(|| {
            obj.get("base64")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

        if let Some(data) = data {
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image raw base64: media={}, size={} bytes, prefix={}",
                msg_idx,
                block_idx,
                media_type,
                data.len(),
                truncate_for_log(&data, 20)
            ));
            if data.starts_with("data:") {
                return data;
            }
            return format!("data:{};base64,{}", media_type, data);
        }

        log(&format!(
            "🖼️ [Message #{} Block #{}] Image source raw is empty",
            msg_idx, block_idx
        ));
        String::new()
    }

    pub fn is_potential_skill_result(content: &Option<Value>) -> bool {
        let Some(content_val) = content else {
            return false;
        };
        let text = match content_val {
            Value::String(s) => s.as_str(),
            Value::Array(arr) => {
                for item in arr {
                    if let Value::Object(obj) = item {
                        if let Some(Value::String(t)) = obj.get("text") {
                            if t.contains("<command-name>") || t.contains("Base Path:") {
                                return true;
                            }
                        }
                    }
                }
                ""
            }
            _ => "",
        };
        text.contains("<command-name>") || text.contains("Base Path:")
    }

    pub fn extract_skill_info(content: &Option<Value>) -> Option<(String, String)> {
        let content_val = content.as_ref()?;
        let full_text = match content_val {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|item| {
                    if let Value::Object(obj) = item {
                        if let Some(Value::String(text)) = obj.get("text") {
                            return Some(text.clone());
                        }
                    }
                    if let Value::String(s) = item {
                        return Some(s.clone());
                    }
                    None
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => content_val.to_string(),
        };

        if !full_text.contains("<command-name>") && !full_text.contains("Base Path:") {
            return None;
        }

        let skill_name = if let Some(start) = full_text.find("<command-name>") {
            let sub = &full_text[start + 14..];
            let end = sub.find("</command-name>")?;
            sub[..end].trim().trim_start_matches('/').to_string()
        } else {
            return None;
        };

        let skill_content = if let Some(path_idx) = full_text.find("Base Path:") {
            let next_line = full_text[path_idx..].find('\n')?;
            full_text[path_idx + next_line..].trim().to_string()
        } else {
            full_text
                .replace(&format!("<command-name>{}</command-name>", skill_name), "")
                .replace(&format!("<command-name>/{}</command-name>", skill_name), "")
                .trim()
                .to_string()
        };

        if skill_name.is_empty() || skill_content.is_empty() {
            return None;
        }

        Some((skill_name, skill_content))
    }

    fn transform_tool_result_output<F>(
        content: &Value,
        log: &F,
        msg_idx: usize,
        block_idx: usize,
    ) -> Value
    where
        F: Fn(&str),
    {
        match content {
            Value::String(text) => Value::String(text.clone()),
            Value::Array(items) => {
                let structured: Vec<Value> = items
                    .iter()
                    .map(|item| Self::transform_tool_result_part(item, log, msg_idx, block_idx))
                    .collect();
                if structured.is_empty() {
                    Value::String(String::new())
                } else {
                    Value::Array(structured)
                }
            }
            other => Value::String(other.to_string()),
        }
    }

    fn transform_tool_result_part<F>(
        item: &Value,
        log: &F,
        msg_idx: usize,
        block_idx: usize,
    ) -> Value
    where
        F: Fn(&str),
    {
        if let Some(image_url) = Self::extract_tool_result_image_url(item, log, msg_idx, block_idx)
        {
            return json!({
                "type": "input_image",
                "image_url": image_url
            });
        }

        if let Some(file_part) = Self::extract_tool_result_file_part(item) {
            return file_part;
        }

        match item {
            Value::String(text) => json!({
                "type": "input_text",
                "text": text
            }),
            Value::Object(obj) => {
                if let Some(text) = obj.get("text").and_then(|value| value.as_str()) {
                    json!({
                        "type": "input_text",
                        "text": text
                    })
                } else {
                    json!({
                        "type": "input_text",
                        "text": serde_json::to_string(item).unwrap_or_else(|_| "[unsupported tool result item]".to_string())
                    })
                }
            }
            other => json!({
                "type": "input_text",
                "text": other.to_string()
            }),
        }
    }

    fn extract_tool_result_image_url<F>(
        item: &Value,
        log: &F,
        msg_idx: usize,
        block_idx: usize,
    ) -> Option<String>
    where
        F: Fn(&str),
    {
        let obj = item.as_object()?;
        let raw_url = obj
            .get("image_url")
            .and_then(Self::extract_image_url_value)
            .or_else(|| {
                obj.get("url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
            })
            .or_else(|| obj.get("source").and_then(Self::extract_image_source_url));

        let normalized = Self::normalize_image_url(raw_url?, None, log, msg_idx, block_idx);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }

    fn extract_image_url_value(value: &Value) -> Option<String> {
        match value {
            Value::String(url) => Some(url.clone()),
            Value::Object(obj) => obj
                .get("url")
                .or_else(|| obj.get("uri"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            _ => None,
        }
    }

    fn extract_image_source_url(value: &Value) -> Option<String> {
        let obj = value.as_object()?;
        obj.get("url")
            .or_else(|| obj.get("uri"))
            .or_else(|| obj.get("path"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
    }

    fn extract_tool_result_file_part(item: &Value) -> Option<Value> {
        let obj = item.as_object()?;
        let source = obj.get("source")?.as_object()?;
        let data = source
            .get("data")
            .or_else(|| source.get("file_data"))
            .and_then(|value| value.as_str())?;
        let media_type = source
            .get("media_type")
            .or_else(|| source.get("mime_type"))
            .and_then(|value| value.as_str())
            .unwrap_or("application/octet-stream");
        let filename = obj
            .get("name")
            .and_then(|value| value.as_str())
            .or_else(|| source.get("filename").and_then(|value| value.as_str()))
            .unwrap_or("data");
        let file_data = if data.starts_with("data:") {
            data.to_string()
        } else {
            format!("data:{};base64,{}", media_type, data)
        };

        Some(json!({
            "type": "input_file",
            "filename": filename,
            "file_data": file_data
        }))
    }

    pub fn convert_to_codex_skill_format(name: &str, content: &str) -> String {
        format!(
            "<skill>\n<name>{}</name>\n<path>unknown</path>\n{}\n</skill>",
            name, content
        )
    }

    fn truncate_skill_content(content: &str, max_chars: usize) -> String {
        if max_chars == 0 {
            return String::new();
        }
        let content = content.trim();
        let char_count = content.chars().count();
        if char_count <= max_chars {
            return content.to_string();
        }

        let marker_chars = SKILL_TRUNCATION_MARKER.chars().count();
        if max_chars <= marker_chars {
            return content.chars().take(max_chars).collect();
        }

        let keep_chars = max_chars - marker_chars;
        let mut truncated: String = content.chars().take(keep_chars).collect();
        truncated.push_str(SKILL_TRUNCATION_MARKER);
        truncated
    }

    fn build_skill_key(name: &str, content: &str) -> String {
        let normalized_name = name.trim().to_ascii_lowercase();
        let normalized_content = content.trim();
        let mut hasher = DefaultHasher::new();
        normalized_content.hash(&mut hasher);
        format!("{}#{}", normalized_name, hasher.finish())
    }

    fn build_limited_skill_payload(
        name: &str,
        content: &str,
        remaining_budget: usize,
    ) -> Option<String> {
        if remaining_budget == 0 {
            return None;
        }

        let wrapper_overhead = Self::convert_to_codex_skill_format(name, "")
            .chars()
            .count();
        if remaining_budget <= wrapper_overhead {
            return None;
        }

        let per_skill_budget = remaining_budget - wrapper_overhead;
        let allowed_content_chars = per_skill_budget.min(MAX_SKILL_CONTENT_CHARS);
        let limited_content = Self::truncate_skill_content(content, allowed_content_chars);
        if limited_content.is_empty() {
            return None;
        }

        Some(Self::convert_to_codex_skill_format(name, &limited_content))
    }
}

#[cfg(test)]
mod tests {
    use super::MessageProcessor;
    use crate::models::{ContentBlock, Message, MessageContent};
    use serde_json::{json, Value};

    #[test]
    fn test_build_skill_key_changes_with_content() {
        let k1 = MessageProcessor::build_skill_key("test-skill", "alpha");
        let k2 = MessageProcessor::build_skill_key("test-skill", "beta");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_skill_payload_is_truncated_when_budget_is_tight() {
        let content = "a".repeat(20_000);
        let payload = MessageProcessor::build_limited_skill_payload("skill-a", &content, 1200)
            .expect("payload should fit with truncation");
        assert!(payload.contains("[skill content truncated]"));
        assert!(payload.chars().count() <= 1200);
    }

    #[test]
    fn test_transform_messages_rewrites_team_agent_spawn_result() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_agent_1".to_string()),
                    name: "Agent".to_string(),
                    input: json!({
                        "name": "agent_vehicle",
                        "team_name": "debug-swarm",
                        "run_in_background": true
                    }),
                    signature: None,
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: Some("call_agent_1".to_string()),
                    id: None,
                    content: Some(Value::Array(vec![json!({
                        "type": "text",
                        "text": "Spawned successfully.\nagent_id: agent_vehicle@debug-swarm\nname: agent_vehicle\nteam_name: debug-swarm\nThe agent is now running and will receive instructions via mailbox."
                    })])),
                }])),
            },
        ];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let rewritten = input
            .iter()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
            .and_then(|item| item.get("output"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        assert!(rewritten.contains("team mailbox agent id"));
        assert!(rewritten.contains("Do not call TaskOutput with this agent_id"));
        assert!(rewritten.contains("agent_vehicle@debug-swarm"));
    }

    #[test]
    fn test_transform_messages_rewrites_task_output_missing_team_agent_error() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_task_output_1".to_string()),
                    name: "TaskOutput".to_string(),
                    input: json!({
                        "task_id": "agent_vehicle@debug-swarm",
                        "block": true,
                        "timeout": 600000
                    }),
                    signature: None,
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: Some("call_task_output_1".to_string()),
                    id: None,
                    content: Some(Value::String(
                        "<tool_use_error>No task found with ID: agent_vehicle@debug-swarm</tool_use_error>"
                            .to_string(),
                    )),
                }])),
            },
        ];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let rewritten = input
            .iter()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
            .and_then(|item| item.get("output"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        assert!(rewritten.contains("TaskOutput cannot query team agent IDs"));
        assert!(rewritten.contains("agent_vehicle@debug-swarm"));
        assert!(!rewritten.contains("<tool_use_error>"));
    }

    #[test]
    fn test_transform_messages_rewrites_teammate_shutdown_request_into_protocol_hint() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(
                "<teammate-message teammate_id=\"team-lead\">\n{\"type\":\"shutdown_request\",\"requestId\":\"shutdown-1773827469809@external-researcher\",\"from\":\"team-lead\",\"reason\":\"User requested the team be disbanded. Stop work and exit the team now.\",\"timestamp\":\"2026-03-18T09:51:09.809Z\"}\n</teammate-message>".to_string(),
            )),
        }];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let rewritten = input
            .first()
            .and_then(|item| item.get("content"))
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        assert!(rewritten.contains("Structured teammate protocol message"));
        assert!(rewritten.contains("shutdown_request"));
        assert!(rewritten.contains("Do NOT reply with plain text"));
        assert!(rewritten.contains("shutdown_response"));
        assert!(rewritten.contains("shutdown-1773827469809@external-researcher"));
    }

    #[test]
    fn test_transform_messages_rewrites_plain_shutdown_acknowledgement_warning() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(
                "<teammate-message teammate_id=\"external-researcher\" color=\"green\" summary=\"Acknowledge shutdown and exit\">\n收到 shutdown_request。已停止工作并退出队伍，不再执行任何任务。\n</teammate-message>\n\n<teammate-message teammate_id=\"external-researcher\" color=\"green\">\n{\"type\":\"idle_notification\",\"from\":\"external-researcher\",\"timestamp\":\"2026-03-18T09:51:21.994Z\",\"idleReason\":\"available\"}\n</teammate-message>".to_string(),
            )),
        }];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let rewritten = input
            .first()
            .and_then(|item| item.get("content"))
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        assert!(rewritten.contains("plain-text shutdown acknowledgment"));
        assert!(rewritten.contains("not a valid shutdown_response"));
        assert!(rewritten.contains("TeamDelete will keep failing"));
    }
}
