use serde_json::{json, Value};
#[cfg(test)]
use std::collections::HashMap;
use tokio::sync::broadcast;

use crate::models::AnthropicRequest;
#[cfg(test)]
use crate::transform::processor::{ExtractedSkillPayload, MessageProcessor};

use super::{
    providers::OpenAIChatAdapter,
    ResponseTransformRequestContext, ResponseTransformer, TransformBackend, TransformContext,
};

pub struct OpenAIChatBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextBlockKind {
    Text,
    Thinking,
}

impl OpenAIChatBackend {
    /// Normalize model name for OpenAI API
    #[cfg(test)]
    #[allow(dead_code)]
    fn normalize_model(model: &str) -> String {
        model.trim().to_string()
    }

    fn build_messages_endpoint(target_url: &str) -> String {
        if target_url.contains("/chat/completions") || target_url.contains("openai.azure.com") {
            return target_url.to_string();
        }

        let base = target_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/v1/chat/completions", base)
        }
    }

    /// Convert a Codex-style content block to OpenAI format
    #[cfg(test)]
    #[allow(dead_code)]
    fn convert_content_block(block: &Value) -> Option<Value> {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match block_type {
            "input_text" | "output_text" | "text" => {
                let text = block.get("text").and_then(|t| t.as_str())?;
                Some(json!({ "type": "text", "text": text }))
            }
            "input_image" => {
                // Handle image_url in various formats
                let url = block
                    .get("image_url")
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        block
                            .get("image_url")
                            .and_then(|v| v.get("url").and_then(|u| u.as_str()))
                    })?;
                Some(json!({
                    "type": "image_url",
                    "image_url": { "url": url }
                }))
            }
            "thinking" => {
                let thinking = block.get("thinking").and_then(|t| t.as_str())?;
                Some(json!({ "type": "text", "text": thinking }))
            }
            _ => None,
        }
    }

    /// Build OpenAI messages array from Codex-style transformed messages
    #[cfg(test)]
    #[allow(dead_code)]
    fn build_messages(codex_messages: &[Value]) -> Vec<Value> {
        let mut openai_messages: Vec<Value> = Vec::new();

        for item in codex_messages {
            let item_type = item
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("message");

            match item_type {
                "message" => {
                    let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                    let content = item.get("content");

                    let mut text_content = String::new();
                    let mut content_parts: Vec<Value> = Vec::new();
                    let mut has_non_text = false;

                    if let Some(content_array) = content.and_then(|c| c.as_array()) {
                        for block in content_array {
                            if let Some(part) = Self::convert_content_block(block) {
                                if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                                        text_content.push_str(t);
                                    }
                                } else {
                                    has_non_text = true;
                                }
                                content_parts.push(part);
                            }
                        }
                    } else if let Some(text) = content.and_then(|c| c.as_str()) {
                        text_content = text.to_string();
                        content_parts.push(json!({ "type": "text", "text": text }));
                    }

                    if content_parts.is_empty() {
                        continue;
                    }

                    let message_content = if !has_non_text && content_parts.len() == 1 {
                        Value::String(text_content)
                    } else {
                        Value::Array(content_parts)
                    };

                    openai_messages.push(json!({
                        "role": role,
                        "content": message_content
                    }));
                }
                "function_call" => {
                    let call_id = item
                        .get("call_id")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            format!("call_{}", chrono::Utc::now().timestamp_millis())
                        });
                    let name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown");
                    let args_str = item
                        .get("arguments")
                        .and_then(|s| s.as_str())
                        .unwrap_or("{}");

                    let should_merge = openai_messages
                        .last()
                        .map(|last| last.get("role").and_then(|r| r.as_str()) == Some("assistant"))
                        .unwrap_or(false);

                    if should_merge {
                        if let Some(last) = openai_messages.last_mut() {
                            if last.get("tool_calls").is_none() {
                                last["tool_calls"] = json!([]);
                            }
                            if let Some(tool_calls) = last.get_mut("tool_calls") {
                                if let Some(arr) = tool_calls.as_array_mut() {
                                    arr.push(json!({
                                        "id": call_id,
                                        "type": "function",
                                        "function": {
                                            "name": name,
                                            "arguments": args_str
                                        }
                                    }));
                                }
                            }
                            if last.get("content").is_none() {
                                last["content"] = Value::String(String::new());
                            }
                        }
                    } else {
                        openai_messages.push(json!({
                            "role": "assistant",
                            "content": "",
                            "tool_calls": [{
                                "id": call_id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": args_str
                                }
                            }]
                        }));
                    }
                }
                "function_call_output" => {
                    let call_id = item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
                    let output = item.get("output").cloned().unwrap_or(Value::Null);
                    let content_str = if let Some(text) = output.as_str() {
                        text.to_string()
                    } else if output.is_null() {
                        String::new()
                    } else {
                        serde_json::to_string(&output).unwrap_or_default()
                    };

                    openai_messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": content_str
                    }));
                }
                _ => {}
            }
        }

        openai_messages
    }

    /// Convert Anthropic tools to OpenAI tools format
    #[cfg(test)]
    #[allow(dead_code)]
    fn convert_tools(tools: Option<&Vec<Value>>) -> Option<Vec<Value>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }

        let openai_tools: Vec<Value> = tools
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name").and_then(|n| n.as_str())?;
                let description = tool
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                let parameters = tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or(json!({ "type": "object", "properties": {} }));

                Some(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                }))
            })
            .collect();

        if openai_tools.is_empty() {
            None
        } else {
            Some(openai_tools)
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn flatten_system_text(system: Option<&crate::models::SystemContent>) -> Option<String> {
        system.map(|content| content.to_string()).filter(|text| !text.trim().is_empty())
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn reminder_contains_skill_catalog(block_text: &str) -> bool {
        let normalized = block_text.trim().to_ascii_lowercase();
        normalized.contains("the following skills are available for use with the skill tool")
            || normalized.contains("the following skills are available:")
            || normalized.contains("available skills via skill tool:")
            || normalized.contains("<available_skills>")
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn extract_promotable_system_reminders(text: &str) -> (Vec<String>, String) {
        const START: &str = "<system-reminder>";
        const END: &str = "</system-reminder>";

        let mut promoted_parts = Vec::new();
        let mut cleaned = String::new();
        let mut cursor = 0usize;

        while let Some(start_rel) = text[cursor..].find(START) {
            let start_idx = cursor + start_rel;
            let after_start = &text[start_idx + START.len()..];
            let Some(end_rel) = after_start.find(END) else {
                break;
            };
            let end_idx = start_idx + START.len() + end_rel + END.len();
            let block_text =
                text[start_idx + START.len()..start_idx + START.len() + end_rel].trim();

            if Self::reminder_contains_skill_catalog(block_text) {
                cleaned.push_str(&text[cursor..end_idx]);
            } else {
                cleaned.push_str(&text[cursor..start_idx]);
                if !block_text.is_empty() {
                    promoted_parts.push(block_text.to_string());
                }
            }
            cursor = end_idx;
        }

        cleaned.push_str(&text[cursor..]);
        (promoted_parts, cleaned)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn extract_user_scaffolding_to_system(
        codex_messages: &[Value],
    ) -> (Option<String>, Vec<Value>) {
        let mut promoted_parts: Vec<String> = Vec::new();
        let mut cleaned_messages: Vec<Value> = Vec::new();

        for item in codex_messages {
            let mut cloned = item.clone();
            let is_user_message = cloned.get("type").and_then(|v| v.as_str()) == Some("message")
                && cloned.get("role").and_then(|v| v.as_str()) == Some("user");
            if !is_user_message {
                cleaned_messages.push(cloned);
                continue;
            }

            let Some(content) = cloned.get_mut("content").and_then(|v| v.as_array_mut()) else {
                cleaned_messages.push(cloned);
                continue;
            };

            let mut new_content: Vec<Value> = Vec::new();
            for block in content.iter() {
                let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
                    new_content.push(block.clone());
                    continue;
                };
                let (promoted_reminders, mut remaining) =
                    Self::extract_promotable_system_reminders(text);
                promoted_parts.extend(promoted_reminders);

                if let Some(contents_idx) = remaining.find("Contents of /repo/") {
                    let after = &remaining[contents_idx..];
                    if after.contains("CLAUDE.md:")
                        || after.contains("IFLOW.md:")
                        || after.contains("CodeBuddy.md:")
                    {
                        let prefix = &remaining[..contents_idx];
                        let lines: Vec<&str> = after.lines().collect();
                        if lines.len() >= 2 {
                            let rule_line = lines[1].trim();
                            if !rule_line.is_empty() {
                                promoted_parts.push(rule_line.to_string());
                            }
                            let remainder = if let Some(split_idx) = after.find("\n\n") {
                                &after[split_idx + 2..]
                            } else {
                                ""
                            };
                            remaining = format!("{}{}", prefix, remainder);
                        }
                    }
                }

                let trimmed = remaining.trim();
                if !trimmed.is_empty() {
                    let mut new_block = block.clone();
                    if let Some(obj) = new_block.as_object_mut() {
                        obj.insert("text".to_string(), Value::String(trimmed.to_string()));
                    }
                    new_content.push(new_block);
                }
            }

            if let Some(obj) = cloned.as_object_mut() {
                obj.insert("content".to_string(), Value::Array(new_content));
            }
            if cloned
                .get("content")
                .and_then(|v| v.as_array())
                .map(|arr| !arr.is_empty())
                .unwrap_or(false)
            {
                cleaned_messages.push(cloned);
            }
        }

        let promoted = if promoted_parts.is_empty() {
            None
        } else {
            Some(promoted_parts.join("\n\n"))
        };
        (promoted, cleaned_messages)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn append_extracted_skill_outputs(
        codex_messages: &[Value],
        extracted_skills: &[ExtractedSkillPayload],
    ) -> Vec<Value> {
        if extracted_skills.is_empty() {
            return codex_messages.to_vec();
        }

        let mut skills_by_call_id: HashMap<String, Vec<&ExtractedSkillPayload>> = HashMap::new();
        for skill in extracted_skills {
            let Some(call_id) = skill
                .tool_use_id
                .as_deref()
                .map(|value: &str| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            skills_by_call_id
                .entry(call_id.to_string())
                .or_default()
                .push(skill);
        }

        if skills_by_call_id.is_empty() {
            return codex_messages.to_vec();
        }

        let mut enriched = Vec::with_capacity(codex_messages.len() + extracted_skills.len());
        for item in codex_messages {
            enriched.push(item.clone());

            let is_tool_output =
                item.get("type").and_then(|value| value.as_str()) == Some("function_call_output");
            if !is_tool_output {
                continue;
            }

            let Some(call_id) = item
                .get("call_id")
                .and_then(|value| value.as_str())
                .map(|value: &str| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };

            let Some(skill_payloads) = skills_by_call_id.remove(call_id) else {
                continue;
            };

            for skill in skill_payloads {
                enriched.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": skill.as_str(),
                }));
            }
        }

        enriched
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn build_system_message(
        system: Option<&crate::models::SystemContent>,
        promoted_context: Option<&str>,
    ) -> Option<Value> {
        let system_text = Self::flatten_system_text(system);
        let merged = match (
            system_text,
            promoted_context.map(str::trim).filter(|s| !s.is_empty()),
        ) {
            (Some(base), Some(extra)) => Some(format!("{}\n\n{}", base.trim(), extra)),
            (Some(base), None) => Some(base),
            (None, Some(extra)) => Some(extra.to_string()),
            (None, None) => None,
        }?;
        Some(json!({ "role": "system", "content": merged }))
    }
    #[cfg(test)]
    #[allow(dead_code)]
    fn build_tool_choice(anthropic_body: &AnthropicRequest, tools: &[Value]) -> Option<Value> {
        if tools.is_empty() {
            return Some(json!("none"));
        }

        let tool_choice = anthropic_body.tool_choice.as_ref()?;
        match tool_choice {
            Value::String(choice) => match choice.trim().to_ascii_lowercase().as_str() {
                "auto" => Some(json!("auto")),
                "none" => Some(json!("none")),
                "required" | "any" => Some(json!("required")),
                _ => None,
            },
            Value::Object(object) => {
                let choice_type = object
                    .get("type")
                    .and_then(|value| value.as_str())
                    .map(|value| value.trim().to_ascii_lowercase());

                match choice_type.as_deref() {
                    Some("auto") => Some(json!("auto")),
                    Some("none") => Some(json!("none")),
                    Some("required") | Some("any") => Some(json!("required")),
                    Some("tool") | Some("function") => object
                        .get("name")
                        .or_else(|| object.get("tool_name"))
                        .or_else(|| object.get("toolName"))
                        .and_then(|value| value.as_str())
                        .and_then(|name| {
                            tools.iter().find_map(|tool| {
                                let function = tool.get("function")?;
                                let tool_name = function.get("name")?.as_str()?;
                                if tool_name == name {
                                    Some(json!({
                                        "type": "function",
                                        "function": { "name": name }
                                    }))
                                } else {
                                    None
                                }
                            })
                        }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn resolve_parallel_tool_calls(anthropic_body: &AnthropicRequest) -> bool {
        anthropic_body
            .tool_choice
            .as_ref()
            .and_then(|value| value.get("disable_parallel_tool_use"))
            .and_then(Value::as_bool)
            .map(|disabled| !disabled)
            .unwrap_or(true)
    }
}

#[cfg(test)]
#[allow(dead_code)]
struct OpenAIRequestMapper;

#[cfg(test)]
#[allow(dead_code)]
impl OpenAIRequestMapper {
    fn build_body(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        effective_stream: bool,
        requested_model: &str,
    ) -> Value {
        let model = OpenAIChatBackend::normalize_model(requested_model);
        let (transformed_messages, extracted_skills) =
            MessageProcessor::transform_messages(&anthropic_body.messages, log_tx);
        let (promoted_context, cleaned_messages) =
            OpenAIChatBackend::extract_user_scaffolding_to_system(&transformed_messages);
        let enriched_messages =
            OpenAIChatBackend::append_extracted_skill_outputs(&cleaned_messages, &extracted_skills);

        let mut messages = Vec::new();
        if let Some(system_msg) = OpenAIChatBackend::build_system_message(
            anthropic_body.system.as_ref(),
            promoted_context.as_deref(),
        ) {
            messages.push(system_msg);
        }
        messages.extend(OpenAIChatBackend::build_messages(&enriched_messages));

        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": effective_stream
        });

        if effective_stream {
            body["stream_options"] = json!({
                "include_usage": true
            });
        }

        if let Some(obj) = body.as_object_mut() {
            Self::apply_optional_parameters(obj, anthropic_body, log_tx, ctx, requested_model);
        }

        body
    }

    fn apply_optional_parameters(
        obj: &mut serde_json::Map<String, Value>,
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        requested_model: &str,
    ) {
        if let Some(max_tokens) = anthropic_body.max_tokens {
            let configured_limit = ctx.openai_max_tokens_mapping.get_limit(requested_model);

            if let Some(tx) = log_tx {
                let _ = tx.send(format!(
                    "[TransformDebug] model: {}, max_tokens: {}, configured_limit: {:?}, mapping: {:?}",
                    requested_model, max_tokens, configured_limit, ctx.openai_max_tokens_mapping
                ));
            }

            let effective_max_tokens = if let Some(limit) = configured_limit {
                let effective = max_tokens.min(limit);
                if effective < max_tokens {
                    if let Some(tx) = log_tx {
                        let _ = tx.send(format!(
                            "[Transform] max_tokens limited: {} → {} (configured limit: {})",
                            max_tokens, effective, limit
                        ));
                    }
                }
                effective
            } else {
                max_tokens
            };
            obj.insert("max_tokens".to_string(), json!(effective_max_tokens));
        }
        if let Some(temperature) = anthropic_body.temperature {
            obj.insert("temperature".to_string(), json!(temperature));
        }
        if let Some(top_p) = anthropic_body.top_p {
            obj.insert("top_p".to_string(), json!(top_p));
        }
        if let Some(metadata) = anthropic_body.metadata.as_ref() {
            obj.insert("metadata".to_string(), metadata.clone());
        }
        if let Some(stop) = &anthropic_body.stop_sequences {
            obj.insert("stop".to_string(), json!(stop));
        }
        if let Some(tools) = OpenAIChatBackend::convert_tools(anthropic_body.tools.as_ref()) {
            obj.insert("tools".to_string(), json!(tools));
            obj.insert(
                "parallel_tool_calls".to_string(),
                json!(OpenAIChatBackend::resolve_parallel_tool_calls(
                    anthropic_body
                )),
            );
            if let Some(tool_choice) = OpenAIChatBackend::build_tool_choice(anthropic_body, &tools)
            {
                obj.insert("tool_choice".to_string(), tool_choice);
            }
        }
    }
}

struct OpenAIUpstreamRequestBuilder;

impl OpenAIUpstreamRequestBuilder {
    fn is_azure(target_url: &str) -> bool {
        target_url.contains("openai.azure.com")
    }

    fn accept_header(body: &Value) -> &'static str {
        if body
            .get("stream")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "text/event-stream"
        } else {
            "application/json"
        }
    }

    fn apply_auth(
        builder: reqwest::RequestBuilder,
        target_url: &str,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        if Self::is_azure(target_url) {
            builder.header("api-key", api_key)
        } else {
            builder.header("Authorization", format!("Bearer {}", api_key))
        }
    }

    fn build_request(
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
    ) -> reqwest::RequestBuilder {
        let endpoint = OpenAIChatBackend::build_messages_endpoint(target_url);
        let builder = client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", Self::accept_header(body));

        Self::apply_auth(builder, target_url, api_key).body(body.to_string())
    }
}

impl TransformBackend for OpenAIChatBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        _log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        effective_stream: bool,
        model_override: Option<String>,
    ) -> (Value, String) {
        let unified = crate::transform::unified::UnifiedChatRequest::from_anthropic(anthropic_body);
        let requested = model_override
            .as_deref()
            .or(anthropic_body.model.as_deref())
            .unwrap_or("gpt-4o");
        let prepared = OpenAIChatAdapter.prepare_messages_request(
            &unified,
            ctx,
            "",
            "",
            "2023-06-01",
            requested,
            effective_stream,
        );
        (prepared.body, prepared.session_id)
    }

    fn build_upstream_request(
        &self,
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        _session_id: &str,
        _anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        OpenAIUpstreamRequestBuilder::build_request(client, target_url, api_key, body)
    }

    fn create_response_transformer(
        &self,
        model: &str,
        allow_visible_thinking: bool,
    ) -> Box<dyn ResponseTransformer> {
        Box::new(OpenAIChatResponseTransformer::new_with_visibility(
            model,
            allow_visible_thinking,
        ))
    }
}

/// State for tracking tool calls during streaming
#[derive(Clone, Debug)]
struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
    emitted_arguments_len: usize,
    block_index: Option<usize>,
    has_started: bool,
}

/// Response transformer for OpenAI Chat Completion SSE to Anthropic SSE
pub struct OpenAIChatResponseTransformer {
    message_id: String,
    model: String,
    allow_visible_thinking: bool,
    selected_choice_index: Option<usize>,
    content_index: usize,
    open_text_index: Option<usize>,
    open_text_block_kind: Option<TextBlockKind>,
    sent_message_start: bool,
    sent_message_stop: bool,
    tool_calls: Vec<Option<ToolCallState>>,
    saw_tool_call: bool,
    contains_background_agent_completion: bool,
    historical_background_agent_launch_count: usize,
    launched_background_agent_count: usize,
    terminal_background_agent_completion_count: usize,
    lifecycle_progress_messages_emitted: usize,
    pending_lifecycle_text: String,
    finish_reason: Option<String>,
    stop_sequence: Option<String>,
    usage: Option<Value>,
}

impl OpenAIChatResponseTransformer {
    pub fn new(model: &str) -> Self {
        Self::new_with_visibility(model, true)
    }

    pub fn new_with_visibility(model: &str, allow_visible_thinking: bool) -> Self {
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            allow_visible_thinking,
            selected_choice_index: None,
            content_index: 0,
            open_text_index: None,
            open_text_block_kind: None,
            sent_message_start: false,
            sent_message_stop: false,
            tool_calls: Vec::new(),
            saw_tool_call: false,
            contains_background_agent_completion: false,
            historical_background_agent_launch_count: 0,
            launched_background_agent_count: 0,
            terminal_background_agent_completion_count: 0,
            lifecycle_progress_messages_emitted: 0,
            pending_lifecycle_text: String::new(),
            finish_reason: None,
            stop_sequence: None,
            usage: None,
        }
    }

    fn tool_call_is_ready(state: &ToolCallState) -> bool {
        !state.name.trim().is_empty()
    }

    fn known_background_agent_launch_count(&self) -> usize {
        self.launched_background_agent_count
            .max(self.historical_background_agent_launch_count)
    }

    fn pending_background_agent_count(&self) -> usize {
        self.known_background_agent_launch_count()
            .saturating_sub(self.terminal_background_agent_completion_count)
    }

    fn is_background_agent_round_active(&self) -> bool {
        self.contains_background_agent_completion || self.pending_background_agent_count() > 0
    }

    fn extract_xml_tag_body<'a>(fragment: &'a str, tag: &str) -> Option<&'a str> {
        let start_marker = format!("<{tag}>");
        let end_marker = format!("</{tag}>");
        let start = fragment.find(start_marker.as_str())? + start_marker.len();
        let end = fragment[start..].find(end_marker.as_str())? + start;
        Some(fragment[start..end].trim())
    }

    fn compact_task_completion_summary(summary: &str) -> String {
        let trimmed = summary.trim();
        if let Some(rest) = trimmed.strip_prefix("Agent \"") {
            if let Some(end_quote) = rest.find('"') {
                let name = rest[..end_quote].trim();
                if !name.is_empty() {
                    return format!("后台 explorer 已完成：{name}…");
                }
            }
        }

        if trimmed.is_empty() {
            "后台任务已完成，正在汇总结果…".to_string()
        } else {
            format!("后台任务已完成：{trimmed}…")
        }
    }

    fn tool_launches_background_agent(tool_name: &str, arguments: &str) -> bool {
        if !tool_name.eq_ignore_ascii_case("Agent") {
            return false;
        }

        serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|parsed| parsed.as_object().cloned())
            .and_then(|input| {
                input
                    .get("run_in_background")
                    .and_then(|value| value.as_bool())
            })
            .unwrap_or(false)
    }

    fn task_output_targets_mailbox_agent_id(arguments: &str) -> bool {
        serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|parsed| parsed.as_object().cloned())
            .and_then(|input| {
                input
                    .get("task_id")
                    .or_else(|| input.get("taskId"))
                    .or_else(|| input.get("agent_id"))
                    .or_else(|| input.get("agentId"))
                    .or_else(|| input.get("id"))
                    .or_else(|| input.get("shell_id"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.contains('@'))
            })
            .unwrap_or(false)
    }

    fn build_background_task_progress_message(tool_name: &str, arguments: &str) -> Option<String> {
        let parsed = serde_json::from_str::<Value>(arguments).ok()?;
        let input = parsed.as_object()?;

        if tool_name.eq_ignore_ascii_case("Agent") {
            if !input
                .get("run_in_background")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return None;
            }

            let description = input
                .get("description")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());

            return Some(match description {
                Some(value) => format!("已启动后台 explorer：{value}…"),
                None => "已启动后台 explorer，正在处理中…".to_string(),
            });
        }

        if tool_name.eq_ignore_ascii_case("TaskOutput") {
            if Self::task_output_targets_mailbox_agent_id(arguments) {
                return Some(
                    "不要用 TaskOutput 轮询 agent_id；请等待 teammate-message 或 idle_notification 更新。"
                        .to_string(),
                );
            }

            let is_non_blocking = !input
                .get("block")
                .and_then(|value| value.as_bool())
                .unwrap_or(true);

            return Some(if is_non_blocking {
                "正在轮询后台任务结果…".to_string()
            } else {
                "正在等待后台任务返回结果…".to_string()
            });
        }

        None
    }

    fn build_task_lifecycle_progress_message(fragment: &str) -> Option<(String, bool)> {
        let trimmed = fragment.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed.starts_with("<retrieval_status>") {
            let status = Self::extract_xml_tag_body(trimmed, "retrieval_status")?;
            let message = match status {
                "timeout" | "running" => "某个 explorer 仍在运行，我继续等待结果…".to_string(),
                other if !other.is_empty() => format!("后台任务状态更新：{other}…"),
                _ => return None,
            };
            return Some((message, false));
        }

        if trimmed.starts_with("<task-notification>") {
            let status = Self::extract_xml_tag_body(trimmed, "status").unwrap_or("");
            let summary = Self::extract_xml_tag_body(trimmed, "summary").unwrap_or("");
            let message = match status {
                "completed" => Self::compact_task_completion_summary(summary),
                "failed" => {
                    if summary.trim().is_empty() {
                        "后台任务执行失败，我继续处理剩余结果…".to_string()
                    } else {
                        format!("后台任务失败：{}…", summary.trim())
                    }
                }
                _ => {
                    if summary.trim().is_empty() {
                        "收到后台任务进度更新…".to_string()
                    } else {
                        format!("后台任务进度更新：{}…", summary.trim())
                    }
                }
            };
            return Some((message, status == "completed"));
        }

        if trimmed.starts_with('{') {
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                if parsed.get("kind").and_then(|value| value.as_str())
                    == Some("background_agent_completion")
                {
                    let status = parsed
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let summary = parsed
                        .get("summary")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let message = if status.eq_ignore_ascii_case("completed") || !summary.is_empty()
                    {
                        Self::compact_task_completion_summary(summary)
                    } else {
                        "收到后台任务进度更新…".to_string()
                    };
                    return Some((message, true));
                }
            }
        }

        if trimmed.starts_with("Task is still running") {
            return Some(("某个 explorer 仍在运行，我继续等待结果…".to_string(), false));
        }

        if trimmed.starts_with("No task output available") {
            return Some(("后台任务暂时还没有新输出，我继续等待…".to_string(), false));
        }

        if trimmed.starts_with("Error: No task found with ID:") {
            return Some((
                "某个后台任务已结束或状态失效，我继续汇总现有结果…".to_string(),
                false,
            ));
        }

        None
    }

    fn looks_like_lifecycle_candidate(fragment: &str) -> bool {
        let trimmed = fragment.trim();
        trimmed.starts_with("<task-notification>")
            || trimmed.starts_with("<retrieval_status>")
            || trimmed.starts_with("Task is still running")
            || trimmed.starts_with("No task output available")
            || trimmed.starts_with("Error: No task found with ID:")
            || (trimmed.starts_with('{') && trimmed.contains("background_agent_completion"))
    }

    fn emit_lifecycle_progress(
        &mut self,
        out: &mut Vec<String>,
        message: &str,
        terminal_completion: bool,
    ) {
        if terminal_completion {
            self.terminal_background_agent_completion_count += 1;
        }
        self.lifecycle_progress_messages_emitted += 1;

        if !self.allow_visible_thinking || message.is_empty() {
            return;
        }

        self.open_thinking_block_if_needed(out);
        out.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({
                "type": "content_block_delta",
                "index": self.open_text_index,
                "delta": { "type": "thinking_delta", "thinking": message }
            })
        ));
    }

    fn maybe_emit_task_lifecycle_progress(
        &mut self,
        fragment: &str,
        out: &mut Vec<String>,
    ) -> bool {
        if fragment.is_empty() {
            return false;
        }

        if !self.pending_lifecycle_text.is_empty() {
            self.pending_lifecycle_text.push_str(fragment);
            if let Some((message, terminal_completion)) =
                Self::build_task_lifecycle_progress_message(&self.pending_lifecycle_text)
            {
                self.pending_lifecycle_text.clear();
                self.emit_lifecycle_progress(out, &message, terminal_completion);
            }
            return true;
        }

        if let Some((message, terminal_completion)) =
            Self::build_task_lifecycle_progress_message(fragment)
        {
            self.emit_lifecycle_progress(out, &message, terminal_completion);
            return true;
        }

        if self.is_background_agent_round_active() && Self::looks_like_lifecycle_candidate(fragment)
        {
            self.pending_lifecycle_text.push_str(fragment);
            if let Some((message, terminal_completion)) =
                Self::build_task_lifecycle_progress_message(&self.pending_lifecycle_text)
            {
                self.pending_lifecycle_text.clear();
                self.emit_lifecycle_progress(out, &message, terminal_completion);
            }
            return true;
        }

        false
    }

    fn flush_pending_lifecycle_text_as_visible_text(&mut self, out: &mut Vec<String>) {
        if self.pending_lifecycle_text.is_empty() {
            return;
        }

        let pending = std::mem::take(&mut self.pending_lifecycle_text);
        if let Some((message, terminal_completion)) =
            Self::build_task_lifecycle_progress_message(&pending)
        {
            self.emit_lifecycle_progress(out, &message, terminal_completion);
            return;
        }

        self.open_text_block_if_needed(out);
        out.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({
                "type": "content_block_delta",
                "index": self.open_text_index,
                "delta": { "type": "text_delta", "text": pending }
            })
        ));
    }

    fn ensure_message_start(&mut self, out: &mut Vec<String>) {
        if self.sent_message_start {
            return;
        }
        self.sent_message_start = true;
        out.push(format!(
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

    fn open_text_block_if_needed(&mut self, out: &mut Vec<String>) {
        if self.open_text_index.is_some() {
            if self.open_text_block_kind == Some(TextBlockKind::Text) {
                return;
            }
            self.close_text_block(out);
        }

        let idx = self.content_index;
        self.content_index += 1;
        self.open_text_index = Some(idx);
        self.open_text_block_kind = Some(TextBlockKind::Text);

        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "text", "text": "" }
            })
        ));
    }

    fn open_thinking_block_if_needed(&mut self, out: &mut Vec<String>) {
        if self.open_text_index.is_some() {
            if self.open_text_block_kind == Some(TextBlockKind::Thinking) {
                return;
            }
            self.close_text_block(out);
        }

        let idx = self.content_index;
        self.content_index += 1;
        self.open_text_index = Some(idx);
        self.open_text_block_kind = Some(TextBlockKind::Thinking);

        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "thinking", "thinking": "" }
            })
        ));
    }

    fn close_text_block(&mut self, out: &mut Vec<String>) {
        if let Some(idx) = self.open_text_index.take() {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        self.open_text_block_kind = None;
    }

    fn open_tool_block_if_needed(&mut self, tool_index: usize, out: &mut Vec<String>) {
        let Some(Some(existing_state)) = self.tool_calls.get(tool_index) else {
            return;
        };
        if existing_state.has_started {
            return;
        }
        if !Self::tool_call_is_ready(existing_state) {
            return;
        }

        self.saw_tool_call = true;
        self.close_text_block(out);

        while self.tool_calls.len() <= tool_index {
            self.tool_calls.push(None);
        }

        let idx = self.content_index;
        self.content_index += 1;

        let mut state = self.tool_calls[tool_index].take().unwrap();
        state.block_index = Some(idx);
        state.has_started = true;
        let content_block = json!({
            "type": "tool_use",
            "id": state.id,
            "name": state.name,
            "input": {}
        });
        self.tool_calls[tool_index] = Some(state);

        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": content_block
            })
        ));
    }

    fn emit_pending_tool_arguments(&mut self, tool_index: usize, out: &mut Vec<String>) {
        let Some(Some(state)) = self.tool_calls.get_mut(tool_index) else {
            return;
        };
        let Some(block_idx) = state.block_index else {
            return;
        };
        if state.emitted_arguments_len >= state.arguments.len() {
            return;
        }

        let pending = state.arguments[state.emitted_arguments_len..].to_string();
        state.emitted_arguments_len = state.arguments.len();
        if pending.is_empty() {
            return;
        }

        out.push(format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({
                "type": "content_block_delta",
                "index": block_idx,
                "delta": { "type": "input_json_delta", "partial_json": pending }
            })
        ));
    }

    fn close_tool_block(&mut self, tool_index: usize, out: &mut Vec<String>) {
        let mut closed_tool: Option<(String, String)> = None;

        if let Some(Some(state)) = self.tool_calls.get_mut(tool_index) {
            if let Some(idx) = state.block_index.take() {
                state.has_started = false;
                out.push(format!(
                    "event: content_block_stop\ndata: {}\n\n",
                    json!({ "type": "content_block_stop", "index": idx })
                ));
                closed_tool = Some((state.name.clone(), state.arguments.clone()));
            }
        }

        if let Some((tool_name, arguments)) = closed_tool {
            if Self::tool_launches_background_agent(tool_name.as_str(), arguments.as_str()) {
                self.launched_background_agent_count += 1;
            }
            if let Some(message) =
                Self::build_background_task_progress_message(tool_name.as_str(), arguments.as_str())
            {
                self.emit_lifecycle_progress(out, &message, false);
            }
        }
    }

    fn map_finish_reason(reason: Option<&str>, saw_tool_call: bool) -> &'static str {
        match reason {
            Some("tool_calls") | Some("function_call") => "tool_use",
            Some("length") => "max_tokens",
            Some("content_filter") | Some("refusal") => "refusal",
            Some("stop") => "end_turn",
            Some(_) => "end_turn",
            None => {
                if saw_tool_call {
                    "tool_use"
                } else {
                    "end_turn"
                }
            }
        }
    }

    fn emit_message_stop(&mut self, out: &mut Vec<String>) {
        if self.sent_message_stop {
            return;
        }
        self.sent_message_stop = true;

        self.flush_pending_lifecycle_text_as_visible_text(out);
        self.close_text_block(out);
        for i in 0..self.tool_calls.len() {
            self.close_tool_block(i, out);
        }

        let stop_reason =
            Self::map_finish_reason(self.finish_reason.as_deref(), self.saw_tool_call);
        let usage_obj = self.usage.clone().unwrap_or(json!({
            "input_tokens": 0,
            "output_tokens": 0
        }));

        out.push(format!(
            "event: message_delta\ndata: {}\n\n",
            json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": stop_reason,
                    "stop_sequence": self.stop_sequence
                },
                "usage": usage_obj
            })
        ));
        out.push(format!(
            "event: message_stop\ndata: {}\n\n",
            json!({ "type": "message_stop" })
        ));
    }

    fn capture_usage(&mut self, data: &Value) {
        if let Some(usage) = data.get("usage") {
            self.usage = Some(json!({
                "input_tokens": usage.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0),
                "output_tokens": usage.get("completion_tokens").and_then(|v| v.as_i64()).unwrap_or(0)
            }));
        }
    }

    fn choice_has_useful_payload(choice: &Value) -> bool {
        if choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .is_some()
        {
            return true;
        }

        let Some(delta) = choice.get("delta") else {
            return false;
        };
        if delta
            .get("content")
            .and_then(|v| v.as_str())
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if delta
            .get("refusal")
            .and_then(|v| v.as_str())
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if delta
            .get("reasoning_content")
            .and_then(|v| v.as_str())
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if delta
            .get("function_call")
            .and_then(|v| v.as_object())
            .is_some()
        {
            return true;
        }
        delta
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|calls| !calls.is_empty())
            .unwrap_or(false)
    }

    fn first_choice<'a>(&mut self, data: &'a Value) -> Option<&'a Value> {
        let choices = data.get("choices").and_then(|v| v.as_array())?;

        if let Some(selected_index) = self.selected_choice_index {
            if let Some(choice) = choices.get(selected_index) {
                return Some(choice);
            }
        }

        if let Some((idx, choice)) = choices
            .iter()
            .enumerate()
            .find(|(_, choice)| Self::choice_has_useful_payload(choice))
        {
            self.selected_choice_index = Some(idx);
            return Some(choice);
        }

        choices.first()
    }

    fn capture_choice_metadata(&mut self, choice: &Value) {
        if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            self.finish_reason = Some(reason.to_string());
        }
        if let Some(stop_sequence) = choice.get("stop_sequence").and_then(|v| v.as_str()) {
            self.stop_sequence = Some(stop_sequence.to_string());
        }
    }

    fn emit_reasoning_delta_if_any(&mut self, delta: &Value, out: &mut Vec<String>) {
        if !self.allow_visible_thinking {
            return;
        }

        if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
            if !reasoning.is_empty() {
                self.open_thinking_block_if_needed(out);
                out.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    json!({
                        "type": "content_block_delta",
                        "index": self.open_text_index,
                        "delta": { "type": "thinking_delta", "thinking": reasoning }
                    })
                ));
            }
        }
    }

    fn text_delta_from(&self, delta: &Value) -> Option<String> {
        delta
            .get("content")
            .and_then(|v| v.as_str())
            .filter(|content| !content.is_empty())
            .map(|content| content.to_string())
            .or_else(|| {
                delta
                    .get("refusal")
                    .and_then(|v| v.as_str())
                    .filter(|refusal| !refusal.is_empty())
                    .map(|refusal| refusal.to_string())
            })
    }

    fn emit_text_delta_if_any(&mut self, delta: &Value, out: &mut Vec<String>) {
        if let Some(content) = self.text_delta_from(delta) {
            if self.maybe_emit_task_lifecycle_progress(&content, out) {
                return;
            }
            self.open_text_block_if_needed(out);
            out.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": self.open_text_index,
                    "delta": { "type": "text_delta", "text": content }
                })
            ));
        }
    }

    fn normalized_tool_calls_from(&self, delta: &Value) -> Vec<Value> {
        if let Some(tool_calls_delta) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            tool_calls_delta.clone()
        } else if let Some(function_call_delta) = delta.get("function_call") {
            vec![json!({
                "index": 0,
                "function": function_call_delta
            })]
        } else {
            Vec::new()
        }
    }

    fn apply_tool_call_delta(&mut self, tool_call_delta: &Value, out: &mut Vec<String>) {
        let index = tool_call_delta
            .get("index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        while self.tool_calls.len() <= index {
            self.tool_calls.push(None);
        }

        let current_state = self.tool_calls[index].clone();
        let mut new_state = current_state.unwrap_or_else(|| ToolCallState {
            id: format!("call_{}", index),
            name: String::new(),
            arguments: String::new(),
            emitted_arguments_len: 0,
            block_index: None,
            has_started: false,
        });

        if let Some(id) = tool_call_delta.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() {
                new_state.id = id.to_string();
            }
        }
        if let Some(name) = tool_call_delta
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
        {
            if !name.is_empty() {
                new_state.name = name.to_string();
            }
        }
        if let Some(args) = tool_call_delta
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
        {
            new_state.arguments.push_str(args);
        }

        self.tool_calls[index] = Some(new_state);
        self.open_tool_block_if_needed(index, out);
        self.emit_pending_tool_arguments(index, out);
    }

    fn emit_tool_call_deltas(&mut self, delta: &Value, out: &mut Vec<String>) {
        for tool_call_delta in self.normalized_tool_calls_from(delta) {
            self.apply_tool_call_delta(&tool_call_delta, out);
        }
    }
}

impl ResponseTransformer for OpenAIChatResponseTransformer {
    fn transform_line(&mut self, line: &str) -> Vec<String> {
        let mut output = Vec::new();

        if !line.starts_with("data: ") {
            return output;
        }

        let payload = line[6..].trim();
        // OpenAI may send a final usage-only chunk before [DONE] when include_usage=true.
        // We must delay Anthropic-style message_stop until the terminal marker arrives,
        // otherwise the downstream side can miss final usage accounting.
        if payload == "[DONE]" {
            self.emit_message_stop(&mut output);
            return output;
        }

        let Ok(data) = serde_json::from_str::<Value>(payload) else {
            return output;
        };

        self.capture_usage(&data);

        let choice = match self.first_choice(&data) {
            Some(c) => c,
            None => return output,
        };

        let delta = match choice.get("delta") {
            Some(d) => d,
            None => return output,
        };

        self.ensure_message_start(&mut output);
        self.capture_choice_metadata(choice);
        self.emit_reasoning_delta_if_any(delta, &mut output);
        self.emit_text_delta_if_any(delta, &mut output);
        self.emit_tool_call_deltas(delta, &mut output);

        output
    }

    fn configure_request_context(&mut self, ctx: &ResponseTransformRequestContext) {
        self.contains_background_agent_completion = ctx.contains_background_agent_completion;
        self.historical_background_agent_launch_count =
            ctx.historical_background_agent_launch_count;
        self.launched_background_agent_count = self
            .launched_background_agent_count
            .max(ctx.historical_background_agent_launch_count);
        self.terminal_background_agent_completion_count =
            ctx.terminal_background_agent_completion_count;
    }

    fn take_diagnostics_summary(&mut self) -> Option<Value> {
        let launch_count = self.known_background_agent_launch_count() as u64;
        let terminal_count = self.terminal_background_agent_completion_count as u64;
        let pending_count = self.pending_background_agent_count() as u64;
        let lifecycle_messages = self.lifecycle_progress_messages_emitted as u64;

        if launch_count == 0
            && terminal_count == 0
            && lifecycle_messages == 0
            && !self.contains_background_agent_completion
        {
            return None;
        }

        Some(json!({
            "contains_background_agent_completion": self.contains_background_agent_completion,
            "background_agent_launch_count": launch_count,
            "terminal_background_agent_completion_count": terminal_count,
            "pending_background_agent_count": pending_count,
            "lifecycle_progress_messages_emitted": lifecycle_messages,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_sse_event(raw: &str) -> (String, Value) {
        let mut event = None;
        let mut data = None;

        for line in raw.lines() {
            if let Some(value) = line.strip_prefix("event: ") {
                event = Some(value.to_string());
            }
            if let Some(value) = line.strip_prefix("data: ") {
                data = serde_json::from_str::<Value>(value).ok();
            }
        }

        (
            event.expect("missing SSE event name"),
            data.expect("missing SSE data payload"),
        )
    }

    fn parse_non_empty_sse_events(events: &[String]) -> Vec<(String, Value)> {
        events
            .iter()
            .filter(|event| !event.trim().is_empty())
            .map(|event| parse_sse_event(event))
            .collect()
    }

    #[test]
    fn test_text_content_streaming() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        // First chunk with content
        let line1 =
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"Hello"},"index":0}]}"#;
        let events1 = transformer.transform_line(line1);

        assert!(events1.iter().any(|e| e.contains("message_start")));
        assert!(events1.iter().any(|e| e.contains("content_block_start")));
        assert!(events1.iter().any(|e| e.contains("text_delta")));

        // Second chunk
        let line2 =
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":" world"},"index":0}]}"#;
        let events2 = transformer.transform_line(line2);

        assert!(events2.iter().any(|e| e.contains("text_delta")));

        // Final chunk with finish_reason
        let line3 = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"stop","index":0}]}"#;
        let events3 = transformer.transform_line(line3);
        assert!(events3.is_empty());

        let done_events = transformer.transform_line("data: [DONE]");
        assert!(done_events.iter().any(|e| e.contains("message_stop")));
    }

    #[test]
    fn test_tool_calls_streaming() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        // Tool call start
        let line1 = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":""}}]},"index":0}]}"#;
        let events1 = transformer.transform_line(line1);

        assert!(events1.iter().any(|e| e.contains("content_block_start")));
        assert!(events1.iter().any(|e| e.contains("tool_use")));

        // Tool call arguments
        let line2 = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"index":0}]}"#;
        let events2 = transformer.transform_line(line2);

        assert!(events2.iter().any(|e| e.contains("input_json_delta")));

        // Tool call complete
        let line3 = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"Beijing\"}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        let events3 = transformer.transform_line(line3);

        assert!(events3.iter().any(|e| e.contains("input_json_delta")));
        assert!(!events3.iter().any(|e| e.contains("message_stop")));

        let done_events = transformer.transform_line("data: [DONE]");
        assert!(done_events.iter().any(|e| e.contains("message_stop")));
    }

    #[test]
    fn test_multiple_tool_calls() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        // First tool call
        let line1 = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Beijing\"}"}}]},"index":0}]}"#;
        let _ = transformer.transform_line(line1);

        // Second tool call (different index)
        let line2 = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_2","type":"function","function":{"name":"get_time","arguments":"{}"}}]},"index":0}]}"#;
        let events2 = transformer.transform_line(line2);

        // Should have started a new content block for second tool
        assert!(events2.iter().any(|e| e.contains("content_block_start")));

        // Finish
        let line3 = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}"#;
        let events3 = transformer.transform_line(line3);

        assert!(events3.is_empty());

        let done_events = transformer.transform_line("data: [DONE]");
        assert!(done_events.iter().any(|e| e.contains("message_stop")));
    }

    #[test]
    fn anonymous_tool_call_argument_fragments_are_not_emitted_as_tool_use() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"让我"},"index":0}]}"#,
        );

        let malformed_events_1 = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"","arguments":"未来"}}]},"index":0}]}"#,
        );
        let malformed_events_2 = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"","arguments":"广州"}}]},"index":0}]}"#,
        );

        let joined = malformed_events_1
            .iter()
            .chain(malformed_events_2.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("");
        assert!(
            !joined.contains("\"type\":\"tool_use\""),
            "anonymous tool-call fragments should stay hidden instead of opening a tool_use block"
        );
        assert!(
            !joined.contains("\"type\":\"input_json_delta\""),
            "anonymous tool-call fragments should not stream as input_json_delta"
        );

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"stop","index":0}]}"#,
        );
        let done_events = transformer.transform_line("data: [DONE]");
        let parsed_events = parse_non_empty_sse_events(&done_events);
        let message_delta = parsed_events
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta should be emitted");

        assert_eq!(
            message_delta
                .1
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(|value| value.as_str()),
            Some("end_turn"),
            "malformed tool-call fragments must not force tool_use stop_reason"
        );
    }

    #[test]
    fn closes_text_block_before_opening_tool_block() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");
        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"Hello"},"index":0}]}"#,
        );

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":"{}"}}]},"index":0}]}"#,
        );
        let parsed_events = parse_non_empty_sse_events(&events);

        let text_stop = parsed_events
            .iter()
            .position(|(name, payload)| {
                name == "content_block_stop"
                    && payload.get("index").and_then(|value| value.as_u64()) == Some(0)
            })
            .expect("text block should stop before tool starts");
        let tool_start = parsed_events
            .iter()
            .position(|(name, payload)| {
                name == "content_block_start"
                    && payload
                        .get("content_block")
                        .and_then(|block| block.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("tool_use")
            })
            .expect("tool_use block should start");

        assert!(
            text_stop < tool_start,
            "text block must stop before tool block starts"
        );
    }

    #[test]
    fn reasoning_content_opens_thinking_block() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"reasoning_content":"internal reasoning"},"index":0}]}"#,
        );
        let parsed_events = parse_non_empty_sse_events(&events);

        let thinking_start = parsed_events
            .iter()
            .find(|(name, payload)| {
                name == "content_block_start"
                    && payload
                        .get("content_block")
                        .and_then(|block| block.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("thinking")
            })
            .expect("reasoning_content should open a thinking block");

        assert_eq!(
            thinking_start
                .1
                .get("content_block")
                .and_then(|block| block.get("thinking"))
                .and_then(|value| value.as_str()),
            Some("")
        );
        assert!(parsed_events.iter().any(|(name, payload)| {
            name == "content_block_delta"
                && payload
                    .get("delta")
                    .and_then(|delta| delta.get("type"))
                    .and_then(|value| value.as_str())
                    == Some("thinking_delta")
        }));
    }

    #[test]
    fn stop_finish_reason_remains_end_turn_after_prior_tool_call() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");
        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":"{}"}}]},"index":0}]}"#,
        );

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"stop","index":0}]}"#,
        );
        assert!(events.is_empty());

        let done_events = transformer.transform_line("data: [DONE]");
        let parsed_events = parse_non_empty_sse_events(&done_events);

        let message_delta = parsed_events
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta should be emitted");

        assert_eq!(
            message_delta
                .1
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(|value| value.as_str()),
            Some("end_turn")
        );
    }

    #[test]
    fn usage_chunk_before_done_updates_message_delta_usage() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"Hello"},"index":0}]}"#,
        );
        let finish_events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"stop","index":0}],"usage":null}"#,
        );
        assert!(
            finish_events.is_empty(),
            "should wait for [DONE] before emitting message_stop"
        );

        let usage_events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34}}"#,
        );
        assert!(
            usage_events.is_empty(),
            "usage-only chunk should not emit message_stop before [DONE]"
        );

        let done_events = transformer.transform_line("data: [DONE]");
        let parsed = parse_non_empty_sse_events(&done_events);
        let message_delta = parsed
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta should be emitted on done");

        assert_eq!(
            message_delta
                .1
                .get("usage")
                .and_then(|usage| usage.get("input_tokens"))
                .and_then(|value| value.as_i64()),
            Some(12)
        );
        assert_eq!(
            message_delta
                .1
                .get("usage")
                .and_then(|usage| usage.get("output_tokens"))
                .and_then(|value| value.as_i64()),
            Some(34)
        );
    }

    #[test]
    fn content_filter_maps_to_refusal_stop_reason() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"Blocked"},"index":0}]}"#,
        );
        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"content_filter","index":0}]}"#,
        );
        let done_events = transformer.transform_line("data: [DONE]");
        let parsed = parse_non_empty_sse_events(&done_events);
        let message_delta = parsed
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta should be emitted on done");

        assert_eq!(
            message_delta
                .1
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(|value| value.as_str()),
            Some("refusal")
        );
    }

    #[test]
    fn chunk_without_choices_does_not_emit_message_start() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}"#,
        );

        assert!(
            events.is_empty(),
            "chunk without choices should not emit downstream events"
        );
    }

    #[test]
    fn later_choice_with_text_is_selected_when_first_choice_is_empty() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{}},{"index":1,"delta":{"content":"来自第二个 choice"}}]}"#,
        );
        let parsed = parse_non_empty_sse_events(&events);

        assert!(parsed.iter().any(|(name, payload)| {
            name == "content_block_delta"
                && payload
                    .get("delta")
                    .and_then(|delta| delta.get("text"))
                    .and_then(|value| value.as_str())
                    == Some("来自第二个 choice")
        }));
    }

    #[test]
    fn selected_later_choice_is_sticky_across_followup_chunks() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{}},{"index":1,"delta":{"content":"第一段"}}]}"#,
        );
        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"content":"错误分支"}},{"index":1,"delta":{"content":"第二段"}}]}"#,
        );
        let parsed = parse_non_empty_sse_events(&events);

        assert!(parsed.iter().any(|(name, payload)| {
            name == "content_block_delta"
                && payload
                    .get("delta")
                    .and_then(|delta| delta.get("text"))
                    .and_then(|value| value.as_str())
                    == Some("第二段")
        }));
        assert!(parsed.iter().all(|(_, payload)| {
            payload
                .get("delta")
                .and_then(|delta| delta.get("text"))
                .and_then(|value| value.as_str())
                != Some("错误分支")
        }));
    }

    #[test]
    fn length_finish_reason_during_tool_call_ends_as_max_tokens() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"Agent","arguments":"{\"description\":\"search"}}]},"index":0}]}"#,
        );
        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":" more"}}]},"finish_reason":"length","index":0}]}"#,
        );

        let done_events = transformer.transform_line("data: [DONE]");
        let parsed = parse_non_empty_sse_events(&done_events);
        let message_delta = parsed
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta should be emitted");

        assert_eq!(
            message_delta
                .1
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(|value| value.as_str()),
            Some("max_tokens")
        );
    }

    #[test]
    fn background_agent_launch_tool_call_emits_thinking_progress_and_updates_counts() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_agent","type":"function","function":{"name":"Agent","arguments":"{\"description\":\"搜索天气\",\"run_in_background\":true}"}}]},"index":0}]}"#,
        );
        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}"#,
        );
        let done_events = transformer.transform_line("data: [DONE]");
        let joined = done_events.join("");

        assert!(
            joined.contains("\"type\":\"thinking\"")
                || joined.contains("\"type\":\"thinking_delta\""),
            "background agent launch should be bridged into thinking progress"
        );
        assert!(joined.contains("已启动后台 explorer：搜索天气"));

        let summary = transformer
            .take_diagnostics_summary()
            .expect("diagnostics summary should be available");
        assert_eq!(
            summary
                .get("background_agent_launch_count")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            summary
                .get("pending_background_agent_count")
                .and_then(Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn task_notification_completion_is_bridged_to_thinking_progress_and_updates_terminal_count() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");
        <OpenAIChatResponseTransformer as ResponseTransformer>::configure_request_context(
            &mut transformer,
            &ResponseTransformRequestContext {
                codex_plan_file_path: None,
                contains_background_agent_completion: true,
                historical_background_agent_launch_count: 1,
                terminal_background_agent_completion_count: 0,
            },
        );

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"<task-notification>\n<task-id>a1</task-id>\n<status>completed</status>\n<summary>Agent \"Check Beijing weather\" completed</summary>\n</task-notification>"},"index":0}]}"#,
        );
        let joined = events.join("");

        assert!(
            joined.contains("\"type\":\"thinking\"")
                || joined.contains("\"type\":\"thinking_delta\""),
            "task notification completion should be bridged into thinking progress"
        );
        assert!(joined.contains("后台 explorer 已完成：Check Beijing weather"));
        assert!(
            !joined.contains("\"type\":\"text_delta\""),
            "raw task notification text should stay hidden from visible text blocks"
        );

        let summary = transformer
            .take_diagnostics_summary()
            .expect("diagnostics summary should be available");
        assert_eq!(
            summary
                .get("terminal_background_agent_completion_count")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            summary
                .get("pending_background_agent_count")
                .and_then(Value::as_u64),
            Some(0)
        );
    }

    #[test]
    fn reasoning_content_is_hidden_when_visible_thinking_is_disabled() {
        let backend = OpenAIChatBackend;
        let mut transformer = backend.create_response_transformer("gpt-4o", false);

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"reasoning_content":"internal reasoning"},"index":0}]}"#,
        );

        let parsed_events = parse_non_empty_sse_events(&events);
        assert!(
            parsed_events.iter().all(|(_, payload)| {
                payload
                    .get("content_block")
                    .and_then(|block| block.get("type"))
                    .and_then(|value| value.as_str())
                    != Some("thinking")
            }),
            "thinking blocks should stay hidden when visible thinking is disabled"
        );
        assert!(
            parsed_events.iter().all(|(_, payload)| {
                payload
                    .get("delta")
                    .and_then(|delta| delta.get("type"))
                    .and_then(|value| value.as_str())
                    != Some("thinking_delta")
            }),
            "thinking deltas should stay hidden when visible thinking is disabled"
        );
    }

    #[test]
    fn deprecated_function_call_delta_is_mapped_to_tool_use_stream() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let events1 = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"function_call":{"name":"get_weather","arguments":"{\"city\":"}},"index":0}]}"#,
        );
        assert!(
            events1.iter().any(|event| event.contains("tool_use")),
            "deprecated function_call name delta should open a tool_use block"
        );

        let events2 = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"function_call":{"arguments":"\"Beijing\"}"}},"finish_reason":"function_call","index":0}]}"#,
        );
        assert!(
            events2
                .iter()
                .any(|event| event.contains("input_json_delta")),
            "deprecated function_call arguments delta should map to input_json_delta"
        );

        let done_events = transformer.transform_line("data: [DONE]");
        let parsed_events = parse_non_empty_sse_events(&done_events);
        let message_delta = parsed_events
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta should be emitted on done");

        assert_eq!(
            message_delta
                .1
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(|value| value.as_str()),
            Some("tool_use"),
            "deprecated function_call finish reason should map to tool_use stop_reason"
        );
    }

    #[test]
    fn refusal_delta_is_emitted_as_text_content() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let events = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"refusal":"I can’t help with that."},"index":0}]}"#,
        );
        let parsed_events = parse_non_empty_sse_events(&events);

        assert!(
            parsed_events.iter().any(|(name, payload)| {
                name == "content_block_delta"
                    && payload
                        .get("delta")
                        .and_then(|delta| delta.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("text_delta")
                    && payload
                        .get("delta")
                        .and_then(|delta| delta.get("text"))
                        .and_then(|value| value.as_str())
                        == Some("I can’t help with that.")
            }),
            "refusal delta should be surfaced instead of being silently ignored"
        );
    }

    #[test]
    fn stop_sequence_is_preserved_in_message_delta() {
        let mut transformer = OpenAIChatResponseTransformer::new("gpt-4o");

        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"done soon"},"index":0}]}"#,
        );
        let _ = transformer.transform_line(
            r#"data: {"id":"chatcmpl-123","choices":[{"delta":{},"finish_reason":"stop","stop_sequence":"</END>","index":0}]}"#,
        );
        let done_events = transformer.transform_line("data: [DONE]");
        let parsed_events = parse_non_empty_sse_events(&done_events);

        let message_delta = parsed_events
            .iter()
            .find(|(name, _)| name == "message_delta")
            .expect("message_delta should be emitted on DONE");
        assert_eq!(
            message_delta
                .1
                .get("delta")
                .and_then(|delta| delta.get("stop_sequence"))
                .and_then(|value| value.as_str()),
            Some("</END>")
        );
    }

    #[test]
    fn transform_request_preserves_metadata_and_omits_unsupported_top_k() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("hello".to_string())),
            }],
            system: None,
            tools: None,
            metadata: Some(json!({"trace_id": "req-123", "tenant": "demo"})),
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(64),
            temperature: None,
            top_p: None,
            top_k: Some(17),
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);

        assert!(
            body.get("metadata").is_none(),
            "unified request mapping should drop unsupported metadata passthroughs"
        );
        assert!(
            body.get("top_k").is_none(),
            "unsupported top_k should be omitted rather than sent upstream"
        );
    }

    #[test]
    fn transform_request_maps_required_and_none_tool_choice_variants() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let tools = Some(vec![json!({
            "name": "Bash",
            "description": "Run bash commands",
            "input_schema": {"type": "object", "properties": {}}
        })]);
        let base_request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Run tool".to_string())),
            }],
            system: None,
            tools: tools.clone(),
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: Some(0.2),
            top_p: Some(0.9),
            top_k: None,
            stop_sequences: Some(vec!["</END>".to_string()]),
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let mut required_request = base_request;
        required_request.tool_choice = Some(json!({"type": "any"}));
        let (required_body, _) =
            backend.transform_request(&required_request, None, &ctx, true, None);
        assert_eq!(required_body.get("tool_choice"), Some(&json!("required")));
        assert!(
            required_body.get("stream_options").is_none(),
            "new request path should not inject stream_options"
        );
        assert!(
            required_body
                .get("temperature")
                .and_then(Value::as_f64)
                .map(|value| (value - 0.2).abs() < 1e-6)
                .unwrap_or(false),
            "temperature should be preserved within float tolerance"
        );
        assert!(required_body.get("top_p").is_none());
        assert!(required_body.get("stop").is_none());

        let mut none_request = required_request;
        none_request.tool_choice = Some(json!({"type": "none"}));
        let (none_body, _) = backend.transform_request(&none_request, None, &ctx, true, None);
        assert_eq!(none_body.get("tool_choice"), Some(&json!("none")));
    }

    #[test]
    fn transform_request_respects_stream_and_maps_tool_choice_controls() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Run tool".to_string())),
            }],
            system: None,
            tools: Some(vec![json!({
                "name": "Bash",
                "description": "Run bash commands",
                "input_schema": {"type": "object", "properties": {}}
            })]),
            metadata: None,
            tool_choice: Some(json!({
                "type": "tool",
                "name": "Bash",
                "disable_parallel_tool_use": true
            })),
            thinking: None,
            stream: false,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, false, None);

        assert_eq!(body.get("stream").and_then(Value::as_bool), Some(false));
        assert!(
            body.get("stream_options").is_none(),
            "non-stream requests should not force stream_options"
        );
        assert_eq!(
            body.get("tool_choice"),
            Some(&json!({"type": "function", "function": {"name": "Bash"}}))
        );
        assert_eq!(
            body.get("parallel_tool_calls").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn build_messages_downgrades_thinking_input_blocks_to_text() {
        let messages = OpenAIChatBackend::build_messages(&[json!({
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "thinking",
                "thinking": "chain of thought"
            }]
        })]);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].get("role").and_then(Value::as_str),
            Some("assistant")
        );
        assert_eq!(
            messages[0].get("content").and_then(Value::as_str),
            Some("chain of thought")
        );
    }

    #[test]
    fn transform_request_preserves_original_system_prompt_without_codebuddy_wrapping() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping, SystemContent,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("你好".to_string())),
            }],
            system: Some(SystemContent::Text("You are Claude Code.".to_string())),
            tools: Some(vec![json!({
                "name": "custom_tool",
                "description": "custom tool",
                "input_schema": {"type": "object", "properties": {}}
            })]),
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].get("role").and_then(Value::as_str),
            Some("system")
        );
        assert_eq!(
            messages[0].get("content").and_then(Value::as_str),
            Some("You are Claude Code.")
        );
        assert_eq!(
            body.get("tools")
                .and_then(|value| value.as_array())
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("custom_tool")
        );
        let serialized = body.to_string();
        assert!(!serialized.contains("CodeBuddy Code"));
    }

    #[test]
    fn transform_request_does_not_append_custom_injection_prompt_for_openai() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping, SystemContent,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("你好".to_string())),
            }],
            system: Some(SystemContent::Text("You are Claude Code.".to_string())),
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: "Always inspect repo instructions first.".to_string(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");

        assert_eq!(
            messages[0].get("role").and_then(Value::as_str),
            Some("system")
        );
        let content = messages[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(content.contains("You are Claude Code."));
        assert!(
            !content.contains("Always inspect repo instructions first."),
            "openai converter should not receive codex-only custom injection prompt"
        );
        assert!(
            messages
                .iter()
                .skip(1)
                .all(|msg| msg.get("content").and_then(Value::as_str)
                    != Some("Always inspect repo instructions first.")),
            "custom prompt should remain in system content instead of user messages"
        );
    }

    #[test]
    fn transform_request_keeps_skill_catalog_and_project_context_in_user_history() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping, SystemContent,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text(
                    "<system-reminder>\nThe following skills are available for use with the Skill tool:\n- pdf: Read PDF files\n</system-reminder>\n\nContents of /repo/CLAUDE.md:\nRule A\n\n你好".to_string()
                )),
            }],
            system: Some(SystemContent::Text("You are Claude Code.".to_string())),
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");

        assert_eq!(
            messages[0].get("role").and_then(Value::as_str),
            Some("system")
        );
        let system = messages[0]
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(system, "You are Claude Code.");
        assert_eq!(
            messages[1].get("role").and_then(Value::as_str),
            Some("user")
        );
        let user = messages[1]
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(user.contains("Rule A"));
        assert!(user.contains("The following skills are available for use with the Skill tool"));
        assert!(user.contains("pdf: Read PDF files"));
        assert!(user.contains("你好"));
    }

    #[test]
    fn transform_request_keeps_skill_tool_result_as_single_tool_message() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping, ContentBlock,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![
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
            ],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].get("role").and_then(Value::as_str),
            Some("assistant")
        );
        assert_eq!(
            messages[1].get("role").and_then(Value::as_str),
            Some("tool")
        );
        assert_eq!(
            messages[1].get("tool_call_id").and_then(Value::as_str),
            Some("call_skill_1")
        );
        let loaded_skill = messages[1]
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(loaded_skill.contains("<command-name>yat_commit</command-name>"));
        assert!(loaded_skill.contains("仅在用户明确要求提交代码时使用"));
    }

    #[test]
    fn transform_request_preserves_background_agent_completion_text_in_user_history() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text(
                    "<task-notification>\n<task-id>aaf56deaf0a6f8f5b</task-id>\n<tool-use-id>call_agent_bg</tool-use-id>\n<output-file>/tmp/aaf56deaf0a6f8f5b.output</output-file>\n<status>completed</status>\n<summary>Agent \"Check Jiaxing weather\" completed</summary>\n<result>嘉兴未来 7 天天气大致如下：\n- 第1天：晴到薄雾，17/10°C</result>\n</task-notification>\nFull transcript available at: /tmp/aaf56deaf0a6f8f5b.output\n"
                        .to_string(),
                )),
            }],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");
        let handoff = messages[0]
            .get("content")
            .and_then(Value::as_str)
            .expect("handoff content should be stringified user text");

        assert!(handoff.contains("<task-notification>"));
        assert!(handoff.contains("call_agent_bg"));
        assert!(handoff.contains("嘉兴未来 7 天天气大致如下"));
    }

    #[test]
    fn transform_request_preserves_placeholder_background_agent_completion_text() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text(
                    "<task-notification>\n<task-id>a0110ff52929529f5</task-id>\n<tool-use-id>call_agent_bg</tool-use-id>\n<output-file>/tmp/a0110ff52929529f5.output</output-file>\n<status>completed</status>\n<summary>Agent \"Check Beijing weather\" completed</summary>\n<result>我先查一个可验证的实时天气来源，获取北京未来 7 天逐日预报，再整理成简洁中文的按天概况和趋势总结。</result>\n</task-notification>\nFull transcript available at: /tmp/a0110ff52929529f5.output\n"
                        .to_string(),
                )),
            }],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");
        let handoff = messages[0]
            .get("content")
            .and_then(Value::as_str)
            .expect("handoff content should be stringified user text");

        assert!(handoff.contains("<task-notification>"));
        assert!(handoff.contains("我先查一个可验证的实时天气来源"));
    }

    #[test]
    fn transform_request_keeps_background_agent_tool_result_as_plain_tool_output() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping, ContentBlock,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![
                Message {
                    role: "assistant".to_string(),
                    content: Some(MessageContent::Blocks(vec![ContentBlock::ToolUse {
                        id: Some("call_agent_bg".to_string()),
                        name: "Agent".to_string(),
                        input: json!({"description": "Check weather", "run_in_background": true}),
                        signature: None,
                    }])),
                },
                Message {
                    role: "user".to_string(),
                    content: Some(MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: Some("call_agent_bg".to_string()),
                        id: None,
                        content: Some(Value::String(
                            "Async agent launched successfully.\nagentId: a6b95ea1c5bd2a390 (internal ID - do not mention to user. Use SendMessage with to: 'a6b95ea1c5bd2a390' to continue this agent.)\nThe agent is working in the background. You will be notified automatically when it completes.\noutput_file: /private/tmp/claude-501/demo/tasks/a6b95ea1c5bd2a390.output\nIf asked, you can check progress before completion by using Read or Bash tail on the output file."
                                .to_string(),
                        )),
                    }])),
                },
            ],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");

        assert_eq!(
            messages[1].get("role").and_then(Value::as_str),
            Some("tool")
        );
    let tool_output = messages[1]
            .get("content")
            .and_then(Value::as_str)
            .expect("tool output should be stringified");
        assert!(tool_output.contains("Async agent launched successfully."));
        assert!(tool_output.contains("agentId: a6b95ea1c5bd2a390"));
        assert!(tool_output.contains("output_file: /private/tmp/claude-501/demo/tasks/a6b95ea1c5bd2a390.output"));
    }

    #[test]
    fn transform_request_preserves_text_from_other_system_blocks() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping, SystemBlock, SystemContent,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("你好".to_string())),
            }],
            system: Some(SystemContent::Blocks(vec![
                SystemBlock::Text {
                    text: "Primary system text.".to_string(),
                },
                SystemBlock::Other(json!({"text": "Secondary system text."})),
            ])),
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");
        let system_content = messages[0]
            .get("content")
            .and_then(Value::as_str)
            .expect("system message should be string content");

        assert!(system_content.contains("Primary system text."));
        assert!(system_content.contains("Secondary system text."));
    }

    #[test]
    fn transform_request_does_not_create_system_message_from_custom_injection_prompt_for_openai() {
        use crate::models::{
            AnthropicModelMapping, AnthropicRequest, CodexModelMapping,
            GeminiReasoningEffortMapping, Message, MessageContent, OpenAIModelMapping,
            ReasoningEffortMapping,
        };
        use crate::transform::TransformContext;

        let backend = OpenAIChatBackend;
        let request = AnthropicRequest {
            model: Some("gpt-4o".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("你好".to_string())),
            }],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        let ctx = TransformContext {
            reasoning_mapping: ReasoningEffortMapping::default(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: Default::default(),
            custom_injection_prompt: "Always inspect repo instructions first.".to_string(),
            converter: "openai".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let (body, _) = backend.transform_request(&request, None, &ctx, true, None);
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("messages should be present");

        assert_eq!(
            messages[0].get("role").and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].get("content").and_then(Value::as_str),
            Some("你好")
        );
    }

    #[test]
    fn build_upstream_request_uses_json_accept_for_non_stream_requests() {
        let backend = OpenAIChatBackend;
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client should build without system proxy lookup");
        let body = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": false
        });

        let request = backend
            .build_upstream_request(
                &client,
                "https://api.openai.com/v1",
                "test-key",
                &body,
                "session-1",
                "2023-06-01",
            )
            .build()
            .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get("Accept")
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn build_upstream_request_uses_api_key_for_azure_and_preserves_endpoint() {
        let backend = OpenAIChatBackend;
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client should build without system proxy lookup");
        let body = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        });
        let azure_url = "https://demo.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-08-01-preview";

        let request = backend
            .build_upstream_request(
                &client,
                azure_url,
                "azure-key",
                &body,
                "session-1",
                "2023-06-01",
            )
            .build()
            .expect("request should build");

        assert_eq!(request.url().as_str(), azure_url);
        assert_eq!(
            request
                .headers()
                .get("api-key")
                .and_then(|value| value.to_str().ok()),
            Some("azure-key")
        );
        assert!(
            request.headers().get("Authorization").is_none(),
            "azure requests should not send bearer authorization"
        );
        assert_eq!(
            request
                .headers()
                .get("Accept")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
    }

    #[test]
    fn build_upstream_request_appends_chat_completions_for_custom_base_url() {
        let backend = OpenAIChatBackend;
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client should build without system proxy lookup");
        let body = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        });

        let request = backend
            .build_upstream_request(
                &client,
                "https://openrouter.ai/api",
                "test-key",
                &body,
                "session-1",
                "2023-06-01",
            )
            .build()
            .expect("request should build");

        assert_eq!(
            request.url().as_str(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            request
                .headers()
                .get("Authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-key")
        );
        assert_eq!(
            request
                .headers()
                .get("Accept")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
    }

    #[test]
    fn upstream_endpoint_uses_single_v1_prefix() {
        assert_eq!(
            OpenAIChatBackend::build_messages_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn build_messages_merges_assistant_text_with_tool_calls() {
        let messages = OpenAIChatBackend::build_messages(&[
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello"}]
            }),
            json!({
                "type": "function_call",
                "call_id": "call_1",
                "name": "get_weather",
                "arguments": "{\"city\":\"Beijing\"}"
            }),
        ]);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].get("role").and_then(Value::as_str),
            Some("assistant")
        );
        assert_eq!(
            messages[0].get("content").and_then(Value::as_str),
            Some("Hello")
        );
        assert_eq!(
            messages[0]
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|calls| calls.len()),
            Some(1)
        );
    }

    #[test]
    fn build_messages_stringifies_non_string_tool_results() {
        let messages = OpenAIChatBackend::build_messages(&[json!({
            "type": "function_call_output",
            "call_id": "call_1",
            "output": {"status":"ok"}
        })]);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].get("content").and_then(Value::as_str),
            Some("{\"status\":\"ok\"}")
        );
    }
}
