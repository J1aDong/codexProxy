use crate::logger::{is_debug_log_enabled, truncate_for_log, AppLogger};
use crate::models::{ContentBlock, ImageSource, ImageUrlValue, Message, MessageContent};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use tokio::sync::broadcast;

pub const IMAGE_SYSTEM_HINT: &str = "\n<system_hint>IMAGE PROVIDED. You can see the image above directly. Analyze it as requested. DO NOT ask for file paths.</system_hint>\n";
const MAX_SKILL_CONTENT_CHARS: usize = 4_000;
const MAX_TOTAL_SKILL_CHARS: usize = 12_000;
const SKILL_TRUNCATION_MARKER: &str = "\n[skill content truncated]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedSkillPayload {
    pub tool_use_id: Option<String>,
    pub name: String,
    pub payload: String,
}

impl ExtractedSkillPayload {
    pub fn as_str(&self) -> &str {
        &self.payload
    }
}

impl Deref for ExtractedSkillPayload {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}

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

    fn extract_xml_tag_body<'a>(fragment: &'a str, tag: &str) -> Option<&'a str> {
        let start_marker = format!("<{tag}>");
        let end_marker = format!("</{tag}>");
        let start = fragment.find(start_marker.as_str())? + start_marker.len();
        let end = fragment[start..].find(end_marker.as_str())? + start;
        Some(fragment[start..end].trim())
    }

    fn extract_background_transcript_path(text: &str) -> Option<String> {
        text.lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("Full transcript available at:")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
            })
    }

    fn looks_like_placeholder_agent_result(result: &str) -> bool {
        let trimmed = result.trim();
        if trimmed.is_empty() {
            return false;
        }

        let normalized = trimmed.replace(char::is_whitespace, "");
        normalized.starts_with("我先")
            || normalized.starts_with("先去")
            || normalized.starts_with("先查")
            || normalized.starts_with("让我先")
            || normalized.starts_with("Letmefirst")
            || normalized.starts_with("I'llfirst")
    }

    fn classify_background_agent_completion_from_transcript(
        output_file: Option<&str>,
        fallback_result: &str,
    ) -> &'static str {
        let Some(path) = output_file.map(str::trim).filter(|value| !value.is_empty()) else {
            return if fallback_result.is_empty() {
                "missing"
            } else if Self::looks_like_placeholder_agent_result(fallback_result) {
                "incomplete_placeholder"
            } else {
                "usable"
            };
        };

        let Ok(raw) = std::fs::read_to_string(path) else {
            return if fallback_result.is_empty() {
                "missing"
            } else if Self::looks_like_placeholder_agent_result(fallback_result) {
                "incomplete_placeholder"
            } else {
                "usable"
            };
        };

        let mut saw_execution = false;
        let mut saw_terminal_answer = false;

        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let payload = trimmed
                .split_once('→')
                .map(|(_, rest)| rest.trim())
                .unwrap_or(trimmed);
            let Ok(entry) = serde_json::from_str::<Value>(payload) else {
                continue;
            };

            let top_level_execution = entry
                .get("type")
                .and_then(|value| value.as_str())
                .map(|value| matches!(value, "function_call" | "function_call_output" | "tool_use" | "tool_result" | "server_tool_use" | "server_tool_result"))
                .unwrap_or(false);
            let nested_execution = entry
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(|value| value.as_array())
                .map(|items| {
                    items.iter().any(|item| {
                        item.get("type")
                            .and_then(|value| value.as_str())
                            .map(|value| {
                                matches!(
                                    value,
                                    "function_call"
                                        | "function_call_output"
                                        | "tool_use"
                                        | "tool_result"
                                        | "server_tool_use"
                                        | "server_tool_result"
                                )
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if top_level_execution || nested_execution {
                saw_execution = true;
            }

            let is_terminal_answer = entry
                .get("type")
                .and_then(|value| value.as_str())
                .map(|value| value == "assistant")
                .unwrap_or(false)
                && entry
                    .get("message")
                    .and_then(|message| message.get("stop_reason"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.eq_ignore_ascii_case("end_turn"))
                    .unwrap_or(false)
                && entry
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items.iter().any(|item| {
                            item.get("type").and_then(|value| value.as_str()) == Some("text")
                                && item
                                    .get("text")
                                    .and_then(|value| value.as_str())
                                    .map(|text| !text.trim().is_empty())
                                    .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);
            if is_terminal_answer {
                saw_terminal_answer = true;
            }
        }

        if saw_execution && saw_terminal_answer {
            return "usable";
        }
        if !saw_execution && saw_terminal_answer {
            return "planning_only";
        }
        if saw_execution {
            return "incomplete";
        }
        if fallback_result.is_empty() {
            "missing"
        } else {
            "planning_only"
        }
    }

    fn build_background_agent_completion_payload(
        source: &str,
        agent_id: Option<&str>,
        task_id: Option<&str>,
        tool_use_id: Option<&str>,
        summary: Option<&str>,
        result: Option<&str>,
        output_file: Option<&str>,
        status: Option<&str>,
    ) -> String {
        let result_text = result.unwrap_or("").trim();
        let result_status = Self::classify_background_agent_completion_from_transcript(
            output_file,
            result_text,
        );

        let mut payload = json!({
            "kind": "background_agent_completion",
            "source": source,
            "result_status": result_status,
        });

        if let Some(agent_id) = agent_id.map(str::trim).filter(|value| !value.is_empty()) {
            payload["agent_id"] = json!(agent_id);
        }
        if let Some(task_id) = task_id.map(str::trim).filter(|value| !value.is_empty()) {
            payload["task_id"] = json!(task_id);
        }
        if let Some(tool_use_id) = tool_use_id.map(str::trim).filter(|value| !value.is_empty()) {
            payload["tool_use_id"] = json!(tool_use_id);
        }
        if let Some(summary) = summary.map(str::trim).filter(|value| !value.is_empty()) {
            payload["summary"] = json!(summary);
        }
        if let Some(status) = status.map(str::trim).filter(|value| !value.is_empty()) {
            payload["status"] = json!(status);
        }
        if !result_text.is_empty() {
            payload["result"] = json!(result_text);
        }
        if let Some(output_file) = output_file.map(str::trim).filter(|value| !value.is_empty()) {
            payload["output_file"] = json!(output_file);
        }
        if result_status != "usable" {
            payload["warning"] = json!("This completion should not be treated as a usable final result.");
        }

        payload.to_string()
    }

    fn rewrite_task_notification_text(text: &str) -> Option<String> {
        let trimmed = text.trim();
        if !trimmed.starts_with("<task-notification>") {
            return None;
        }

        let task_id = Self::extract_xml_tag_body(trimmed, "task-id");
        let tool_use_id = Self::extract_xml_tag_body(trimmed, "tool-use-id");
        let summary = Self::extract_xml_tag_body(trimmed, "summary");
        let result = Self::extract_xml_tag_body(trimmed, "result");
        let status = Self::extract_xml_tag_body(trimmed, "status");
        let output_file = Self::extract_xml_tag_body(trimmed, "output-file")
            .map(|value| value.to_string())
            .or_else(|| Self::extract_background_transcript_path(trimmed));

        let looks_like_agent_completion = summary
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.starts_with("Agent \"") && value.ends_with(" completed"))
            .unwrap_or(false);
        if !looks_like_agent_completion {
            return None;
        }

        Some(Self::build_background_agent_completion_payload(
            "task_notification",
            None,
            task_id,
            tool_use_id,
            summary,
            result,
            output_file.as_deref(),
            status,
        ))
    }

    fn rewrite_teammate_idle_notification_payload(payload: &Value) -> Option<String> {
        let kind = payload.get("type").and_then(|value| value.as_str())?;
        if !kind.eq_ignore_ascii_case("idle_notification") {
            return None;
        }

        let status = payload.get("status").and_then(|value| value.as_str());
        let result = payload.get("result").and_then(|value| value.as_str());
        let output_file = payload.get("output_file").and_then(|value| value.as_str());
        let summary = payload.get("summary").and_then(|value| value.as_str());
        let from = payload
            .get("from")
            .or_else(|| payload.get("agent_id"))
            .and_then(|value| value.as_str());

        let has_handoff_content = status
            .map(|value| value.eq_ignore_ascii_case("completed"))
            .unwrap_or(false)
            || result.map(|value| !value.trim().is_empty()).unwrap_or(false)
            || output_file
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);

        if !has_handoff_content {
            return None;
        }

        Some(Self::build_background_agent_completion_payload(
            "idle_notification",
            from,
            None,
            None,
            summary,
            result,
            output_file,
            status,
        ))
    }

    fn rewrite_teammate_protocol_text(text: &str) -> Option<String> {
        let trimmed = text.trim();
        if let Some(rewritten) = Self::rewrite_task_notification_text(trimmed) {
            return Some(rewritten);
        }
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

            if let Some(rewritten) = Self::rewrite_teammate_idle_notification_payload(&payload) {
                return Some(rewritten);
            }

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
        if trimmed.starts_with("Cannot create agent worktree:") {
            return Some(Value::String(
                "Agent launch failed because the request asked for worktree isolation in a context where worktree creation is unavailable. Ordinary subagent requests do not require worktree; only use worktree when the user explicitly asks for isolated repo work.".to_string(),
            ));
        }

        let is_team_agent = trimmed.starts_with("Spawned successfully.")
            && trimmed.contains("agent_id:")
            && trimmed.contains("team_name:");
        let is_async_agent = trimmed.starts_with("Async agent launched successfully.")
            && trimmed.contains("agentId:");

        if !is_team_agent && !is_async_agent {
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

        let extract_leading_token = |value: Option<String>| -> Option<String> {
            value.and_then(|raw| {
                raw.split_whitespace()
                    .next()
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(|token| token.to_string())
            })
        };

        if is_team_agent {
            let agent_id = read_field("agent_id:").unwrap_or_default();
            let teammate_name = read_field("name:").unwrap_or_default();
            let team_name = read_field("team_name:").unwrap_or_default();

            if agent_id.is_empty() || team_name.is_empty() {
                return None;
            }

            return Some(json!({
                "kind": "background_agent",
                "agent_id": agent_id,
                "name": teammate_name,
                "team_name": team_name,
                "status": "running",
                "poll_with_task_output": false,
                "progress_hint": "Use SendMessage with the agent_id to continue this agent. Do not call TaskOutput with this agent_id; wait for teammate-message or idle_notification updates instead."
            }));
        }

        let agent_id = extract_leading_token(read_field("agentId:")).unwrap_or_default();
        let output_file = read_field("output_file:").unwrap_or_default();

        if agent_id.is_empty() {
            return None;
        }

        Some(json!({
            "kind": "background_agent",
            "agent_id": agent_id,
            "output_file": output_file,
            "status": "running",
            "poll_with_task_output": false,
            "progress_hint": "Use SendMessage with the agent_id to continue this agent, or Read/Bash tail the output_file to inspect progress. Do not call TaskOutput with this agent_id."
        }))
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

    fn is_synthetic_plan_bridge_call_id(call_id: &str) -> bool {
        call_id.trim().starts_with("plan_bridge_exit_")
    }

    fn extract_synthetic_plan_bridge_user_note(content: Option<&Value>) -> Option<String> {
        let text = Self::extract_tool_result_plain_text(content?)?;
        let trimmed = text.trim();
        let marker = "To tell you how to proceed, the user said:";
        let note = trimmed
            .split_once(marker)
            .map(|(_, tail)| tail.trim())
            .filter(|value| !value.is_empty())?;
        Some(note.to_string())
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
    ) -> (Vec<Value>, Vec<ExtractedSkillPayload>) {
        let mut input = Vec::new();
        let mut extracted_skills = Vec::new();
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
                                if id.as_deref().map(Self::is_synthetic_plan_bridge_call_id)
                                    == Some(true)
                                {
                                    log(&format!(
                                        "📝 [Message #{} Block #{}] Skip synthetic plan bridge tool call in history",
                                        msg_idx, block_idx
                                    ));
                                    continue;
                                }
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
                                if tool_use_id
                                    .as_deref()
                                    .map(Self::is_synthetic_plan_bridge_call_id)
                                    == Some(true)
                                {
                                    if let Some(user_note) =
                                        Self::extract_synthetic_plan_bridge_user_note(
                                            tool_content.as_ref(),
                                        )
                                    {
                                        if !current_msg_content.is_empty() {
                                            input.push(json!({
                                                "type": "message",
                                                "role": msg.role,
                                                "content": current_msg_content
                                            }));
                                            current_msg_content = Vec::new();
                                        }
                                        input.push(json!({
                                            "type": "message",
                                            "role": "user",
                                            "content": [{
                                                "type": "input_text",
                                                "text": user_note
                                            }]
                                        }));
                                        log(&format!(
                                            "📝 [Message #{} Block #{}] Promote synthetic plan bridge rejection note back into user history",
                                            msg_idx, block_idx
                                        ));
                                    }
                                    log(&format!(
                                        "📝 [Message #{} Block #{}] Skip synthetic plan bridge tool result in history",
                                        msg_idx, block_idx
                                    ));
                                    continue;
                                }
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
                                            extracted_skills.push(ExtractedSkillPayload {
                                                tool_use_id: tool_use_id.clone(),
                                                name: s_name.clone(),
                                                payload: skill_formatted,
                                            });
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
                                    .and_then(|tid| {
                                        tool_name_by_id.get(tid).map(|name| name.as_str())
                                    })
                                    .and_then(|tool_name| {
                                        tool_content.as_ref().and_then(|content| {
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
                            ContentBlock::Document { name, .. } => {
                                let document_marker = name
                                    .as_deref()
                                    .map(|name| format!("[document omitted: {}]", name))
                                    .unwrap_or_else(|| "[document omitted]".to_string());
                                current_msg_content.push(json!({
                                    "type": text_type,
                                    "text": document_marker
                                }));
                            }
                            ContentBlock::OtherValue(v) => {
                                let text = serde_json::to_string(v)
                                    .map(|json| format!("[unsupported content]{}", json))
                                    .unwrap_or_else(|_| "[unsupported content]".to_string());
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
    use super::{ExtractedSkillPayload, MessageProcessor};
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
    fn test_transform_messages_explicitly_marks_document_and_unknown_blocks() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![
                ContentBlock::Document {
                    source: Some(json!({"type": "text", "media_type": "application/pdf"})),
                    name: Some("spec.pdf".to_string()),
                },
                ContentBlock::OtherValue(json!({"type": "search_result", "query": "foo"})),
            ])),
        }];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let content = input
            .first()
            .and_then(|item| item.get("content"))
            .and_then(|value| value.as_array())
            .cloned()
            .expect("message content should be present");

        assert_eq!(
            content[0].get("text").and_then(Value::as_str),
            Some("[document omitted: spec.pdf]")
        );
        assert_eq!(
            content[1].get("text").and_then(Value::as_str),
            Some("[unsupported content]{\"query\":\"foo\",\"type\":\"search_result\"}")
        );
    }

    #[test]
    fn test_extracted_skill_payload_carries_tool_use_id_and_name() {
        let payload = ExtractedSkillPayload {
            tool_use_id: Some("call_skill_1".to_string()),
            name: "yat_commit".to_string(),
            payload: "<skill><name>yat_commit</name></skill>".to_string(),
        };

        assert_eq!(payload.tool_use_id.as_deref(), Some("call_skill_1"));
        assert_eq!(payload.name, "yat_commit");
        assert!(payload.contains("<name>yat_commit</name>"));
    }

    #[test]
    fn test_transform_messages_preserves_tool_use_id_in_extracted_skill_payload() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_skill_1".to_string()),
                    name: "Skill".to_string(),
                    input: json!({"skill": "yat_commit"}),
                    signature: None,
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: Some("call_skill_1".to_string()),
                    id: None,
                    content: Some(Value::String(
                        "<command-name>yat_commit</command-name>\nBase Path: /tmp\n仅在用户明确要求提交代码时使用".to_string(),
                    )),
                }])),
            },
        ];

        let (_, skills) = MessageProcessor::transform_messages(&messages, None);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].tool_use_id.as_deref(), Some("call_skill_1"));
        assert_eq!(skills[0].name, "yat_commit");
        assert!(skills[0].contains("仅在用户明确要求提交代码时使用"));
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
            .cloned()
            .unwrap_or(Value::Null);

        let output = rewritten.as_object().expect("structured agent launch metadata");
        assert_eq!(output.get("kind").and_then(Value::as_str), Some("background_agent"));
        assert_eq!(
            output.get("agent_id").and_then(Value::as_str),
            Some("agent_vehicle@debug-swarm")
        );
        assert_eq!(output.get("team_name").and_then(Value::as_str), Some("debug-swarm"));
        assert_eq!(output.get("name").and_then(Value::as_str), Some("agent_vehicle"));
        assert_eq!(
            output.get("poll_with_task_output").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn test_transform_messages_rewrites_async_agent_launch_result_into_structured_metadata() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_agent_1".to_string()),
                    name: "Agent".to_string(),
                    input: json!({
                        "name": "weather-beijing",
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
                        "text": "Async agent launched successfully.\nagentId: a6b95ea1c5bd2a390 (internal ID - do not mention to user. Use SendMessage with to: 'a6b95ea1c5bd2a390' to continue this agent.)\nThe agent is working in the background. You will be notified automatically when it completes.\nDo not duplicate this agent's work — avoid working with the same files or topics it is using. Work on non-overlapping tasks, or briefly tell the user what you launched and end your response.\noutput_file: /private/tmp/claude-501/demo/tasks/a6b95ea1c5bd2a390.output\nIf asked, you can check progress before completion by using Read or Bash tail on the output file."
                    })])),
                }])),
            },
        ];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let rewritten = input
            .iter()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
            .and_then(|item| item.get("output"))
            .cloned()
            .unwrap_or(Value::Null);

        let output = rewritten.as_object().expect("structured async agent launch metadata");
        assert_eq!(output.get("kind").and_then(Value::as_str), Some("background_agent"));
        assert_eq!(
            output.get("agent_id").and_then(Value::as_str),
            Some("a6b95ea1c5bd2a390")
        );
        assert_eq!(
            output.get("output_file").and_then(Value::as_str),
            Some("/private/tmp/claude-501/demo/tasks/a6b95ea1c5bd2a390.output")
        );
        assert_eq!(output.get("status").and_then(Value::as_str), Some("running"));
        assert_eq!(
            output.get("poll_with_task_output").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            output
                .get("progress_hint")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "Use SendMessage with the agent_id to continue this agent, or Read/Bash tail the output_file to inspect progress. Do not call TaskOutput with this agent_id."
        );
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
    fn test_transform_messages_rewrites_agent_worktree_failure_into_precise_guidance() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("call_agent_1".to_string()),
                    name: "Agent".to_string(),
                    input: json!({
                        "name": "weather-shanghai",
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
                    content: Some(Value::String(
                        "Cannot create agent worktree: not in a git repository and no WorktreeCreate hooks are configured.".to_string(),
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

        assert!(rewritten.contains("Ordinary subagent requests do not require worktree"));
        assert!(rewritten.contains("explicitly asks for isolated repo work"));
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

    #[test]
    fn test_transform_messages_rewrites_task_notification_completion_into_structured_handoff() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(
                "<task-notification>\n<task-id>aaf56deaf0a6f8f5b</task-id>\n<tool-use-id>call_Pe2KJ3sGtCPm0EL4srTHNw0d</tool-use-id>\n<output-file>/tmp/aaf56deaf0a6f8f5b.output</output-file>\n<status>completed</status>\n<summary>Agent \"Check Jiaxing weather\" completed</summary>\n<result>嘉兴未来 7 天天气大致如下：\n\n- 第1天：晴到薄雾，17/10°C\n- 第2天：晴间多云，21/12°C\n\n整体趋势：先回暖，再降温，后半段在 16–18°C 附近波动。</result>\n</task-notification>\nFull transcript available at: /tmp/aaf56deaf0a6f8f5b.output\n".to_string(),
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

        assert!(rewritten.contains("background_agent_completion"));
        assert!(rewritten.contains(r#""result_status":"usable""#));
        assert!(rewritten.contains("Check Jiaxing weather"));
        assert!(rewritten.contains("嘉兴未来 7 天天气大致如下"));
        assert!(rewritten.contains("/tmp/aaf56deaf0a6f8f5b.output"));
        assert!(!rewritten.contains("<task-notification>"));
        assert!(!rewritten.contains("Full transcript available at:"));
    }

    #[test]
    fn test_transform_messages_marks_placeholder_task_notification_result_as_incomplete_handoff() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(
                "<task-notification>\n<task-id>a0110ff52929529f5</task-id>\n<tool-use-id>call_SoqVpukFkthB6HVTVXr6Qdzn</tool-use-id>\n<output-file>/tmp/a0110ff52929529f5.output</output-file>\n<status>completed</status>\n<summary>Agent \"Check Beijing weather\" completed</summary>\n<result>我先查一个可验证的实时天气来源，获取北京未来 7 天逐日预报，再整理成简洁中文的按天概况和趋势总结。</result>\n</task-notification>\nFull transcript available at: /tmp/a0110ff52929529f5.output\n".to_string(),
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

        assert!(rewritten.contains("background_agent_completion"));
        assert!(rewritten.contains(r#""result_status":"incomplete_placeholder""#));
        assert!(rewritten.contains("Check Beijing weather"));
        assert!(rewritten.contains("我先查一个可验证的实时天气来源"));
        assert!(rewritten.contains("/tmp/a0110ff52929529f5.output"));
        assert!(rewritten.contains("should not be treated as a usable final result"));
        assert!(!rewritten.contains("<task-notification>"));
    }

    #[test]
    fn test_transform_messages_rewrites_teammate_idle_notification_result_into_structured_handoff() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(
                "<teammate-message teammate_id=\"weather-beijing\" color=\"green\">\n{\"type\":\"idle_notification\",\"from\":\"weather-beijing\",\"timestamp\":\"2026-03-18T09:51:21.994Z\",\"status\":\"completed\",\"summary\":\"Check Beijing weather completed\",\"result\":\"- 3月21日：多云，16/5°C\",\"output_file\":\"/tmp/weather-beijing.output\"}\n</teammate-message>".to_string(),
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

        assert!(rewritten.contains("background_agent_completion"));
        assert!(rewritten.contains(r#""result_status":"usable""#));
        assert!(rewritten.contains("weather-beijing"));
        assert!(rewritten.contains("Check Beijing weather completed"));
        assert!(rewritten.contains("- 3月21日：多云，16/5°C"));
        assert!(rewritten.contains("/tmp/weather-beijing.output"));
        assert!(!rewritten.contains("<teammate-message"));
    }

    #[test]
    fn test_transform_messages_marks_task_notification_as_planning_only_from_transcript_structure() {
        let transcript_path = std::env::temp_dir().join(format!(
            "codex_proxy_bg_planning_only_{}.jsonl",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(
            &transcript_path,
            concat!(
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"查北京未来7天的天气变化\"}}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[{\"thinking\":\"请求已发送，正在等待上游开始输出…\\n模型正在处理中…\\n\",\"type\":\"thinking\",\"signature\":\"\"}],\"role\":\"assistant\"}}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[{\"text\":\"先说明一下我会怎么做：我将联网查询北京未来 7 天的最新天气预报。\",\"type\":\"text\"}],\"role\":\"assistant\"}}\n"
            ),
        )
        .expect("transcript should be written");

        let raw = format!(
            "<task-notification>\n<task-id>a2c8dc9dd9190d6ed</task-id>\n<tool-use-id>call_GgkbzQzSSJttuWaYWXFpfJpx</tool-use-id>\n<output-file>{}</output-file>\n<status>completed</status>\n<summary>Agent \"Beijing weather\" completed</summary>\n<result>先说明一下我会怎么做：我将联网查询北京未来 7 天的最新天气预报。</result>\n</task-notification>\nFull transcript available at: {}\n",
            transcript_path.display(),
            transcript_path.display()
        );
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(raw)),
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

        assert!(rewritten.contains("background_agent_completion"));
        assert!(rewritten.contains(r#""result_status":"planning_only""#));
        assert!(rewritten.contains("Beijing weather"));

        let _ = std::fs::remove_file(&transcript_path);
    }

    #[test]
    fn test_transform_messages_uses_transcript_structure_to_mark_task_notification_usable() {
        let transcript_path = std::env::temp_dir().join(format!(
            "codex_proxy_bg_usable_{}.jsonl",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(
            &transcript_path,
            concat!(
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"查浙江嘉兴未来7天的天气变化\"}}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[{\"caller\":{\"type\":\"direct\"},\"id\":\"srvtoolu_1774084400473_1\",\"input\":{\"query\":\"weather: China, Zhejiang, Jiaxing\"},\"name\":\"web_search\",\"type\":\"server_tool_use\"}],\"role\":\"assistant\"}}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[{\"text\":\"以下为浙江嘉兴未来 7 天天气预报：3月21日 17/10°C。\",\"type\":\"text\"}],\"role\":\"assistant\",\"stop_reason\":\"end_turn\"}}\n"
            ),
        )
        .expect("transcript should be written");

        let raw = format!(
            "<task-notification>\n<task-id>a2a3a8606396c56af</task-id>\n<tool-use-id>call_7YJ2OtiBneUjTkAofdnwGsby</tool-use-id>\n<output-file>{}</output-file>\n<status>completed</status>\n<summary>Agent \"Jiaxing weather\" completed</summary>\n<result>我先查一个可验证的实时天气来源，再整理结果。</result>\n</task-notification>\nFull transcript available at: {}\n",
            transcript_path.display(),
            transcript_path.display()
        );
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(raw)),
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

        assert!(rewritten.contains("background_agent_completion"));
        assert!(rewritten.contains(r#""result_status":"usable""#));
        assert!(rewritten.contains("Jiaxing weather"));
        assert!(!rewritten.contains(r#""result_status":"incomplete_placeholder""#));

        let _ = std::fs::remove_file(&transcript_path);
    }

    #[test]
    fn test_transform_messages_does_not_rewrite_background_command_task_notification_as_agent_completion() {
        let raw = "<task-notification>\n<task-id>bxiggtdu6</task-id>\n<tool-use-id>call_VTH8KAQi15QvcdrDBcTgjVWW</tool-use-id>\n<output-file>/tmp/bxiggtdu6.output</output-file>\n<status>completed</status>\n<summary>Background command \"Run the full Codex response test suite after adding multi-background launch text suppression\" completed (exit code 0)</summary>\n</task-notification>\nFull transcript available at: /tmp/bxiggtdu6.output\n";
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Text(raw.to_string())),
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

        assert!(!rewritten.contains("background_agent_completion"));
        assert!(rewritten.contains("Background command \"Run the full Codex response test suite"));
        assert!(rewritten.contains("Full transcript available at: /tmp/bxiggtdu6.output"));
    }

    #[test]
    fn test_transform_messages_skips_synthetic_plan_bridge_history() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: Some("plan_bridge_exit_1773967643650".to_string()),
                    name: "ExitPlanMode".to_string(),
                    input: json!({}),
                    signature: None,
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: Some("plan_bridge_exit_1773967643650".to_string()),
                    id: None,
                    content: Some(Value::String(
                        "The user doesn't want to proceed with this tool use. 线上就是 /app/ 子路径"
                            .to_string(),
                    )),
                }])),
            },
            Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text(
                    "继续分析 /app/ 子路径问题".to_string(),
                )),
            },
        ];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let serialized = serde_json::to_string(&input).expect("input should serialize");

        assert!(
            !serialized.contains("plan_bridge_exit_1773967643650")
                && !serialized.contains("ExitPlanMode"),
            "synthetic plan bridge control history should not be replayed upstream"
        );
        assert!(
            serialized.contains("继续分析 /app/ 子路径问题"),
            "ordinary follow-up user text should remain in upstream history"
        );
    }

    #[test]
    fn test_transform_messages_preserves_user_note_from_synthetic_plan_bridge_rejection() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: Some("plan_bridge_exit_1773967643650".to_string()),
                id: None,
                content: Some(Value::String(
                    "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). To tell you how to proceed, the user said:\n改成模拟表盘".to_string(),
                )),
            }])),
        }];

        let (input, _) = MessageProcessor::transform_messages(&messages, None);
        let content = input
            .first()
            .and_then(|item| item.get("content"))
            .and_then(|value| value.as_array())
            .cloned()
            .expect("message content should be preserved");

        assert_eq!(
            content[0].get("type").and_then(Value::as_str),
            Some("input_text"),
            "rejection note should be promoted to a normal user text message"
        );
        assert_eq!(
            content[0].get("text").and_then(Value::as_str),
            Some("改成模拟表盘"),
            "user note attached to a synthetic ExitPlanMode rejection should remain visible upstream"
        );
    }
}
