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

                                let result_text = if let Some(override_text) = override_result_text
                                {
                                    override_text
                                } else if let Some(cv) = tool_content {
                                    match cv {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Array(arr) => arr
                                            .iter()
                                            .filter_map(|item| {
                                                if let serde_json::Value::Object(obj) = item {
                                                    if let Some(serde_json::Value::String(text)) =
                                                        obj.get("text")
                                                    {
                                                        return Some(text.clone());
                                                    }
                                                }
                                                None
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                        _ => cv.to_string(),
                                    }
                                } else {
                                    String::new()
                                };

                                input.push(json!({
                                    "type": "function_call_output",
                                    "call_id": tool_use_id,
                                    "output": result_text
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
}
