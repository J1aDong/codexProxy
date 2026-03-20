use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::AnthropicRequest;

use super::{MessageProcessor, ResponseTransformer, TransformBackend, TransformContext};

pub struct OpenAIChatBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextBlockKind {
    Text,
    Thinking,
}

impl OpenAIChatBackend {
    /// Normalize model name for OpenAI API
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

    fn flatten_system_text(system: Option<&crate::models::SystemContent>) -> Option<String> {
        match system {
            Some(crate::models::SystemContent::Text(s)) => {
                let trimmed = s.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            Some(crate::models::SystemContent::Blocks(blocks)) if !blocks.is_empty() => {
                let text = blocks
                    .iter()
                    .filter_map(|block| match block {
                        crate::models::SystemBlock::Text { text } => Some(text.clone()),
                        crate::models::SystemBlock::PlainString(s) => Some(s.clone()),
                        crate::models::SystemBlock::Other(value) => value
                            .get("text")
                            .and_then(|text| text.as_str())
                            .map(|text| text.to_string())
                            .or_else(|| serde_json::to_string(value).ok()),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let trimmed = text.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            _ => None,
        }
    }

    /// Build system message from Anthropic system field and custom injection prompt
    fn build_system_message(
        system: Option<&crate::models::SystemContent>,
        custom_injection_prompt: &str,
    ) -> Option<Value> {
        let system_text = Self::flatten_system_text(system);
        let custom_text = custom_injection_prompt.trim().to_string();

        let merged = match (system_text, custom_text.is_empty()) {
            (Some(system_text), true) => Some(system_text),
            (Some(system_text), false) => Some(format!("{}\n\n{}", system_text, custom_text)),
            (None, false) => Some(custom_text),
            (None, true) => None,
        }?;

        Some(json!({ "role": "system", "content": merged }))
    }
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

    fn resolve_parallel_tool_calls(anthropic_body: &AnthropicRequest) -> bool {
        anthropic_body
            .tool_choice
            .as_ref()
            .and_then(|tool_choice| tool_choice.as_object())
            .and_then(|tool_choice| tool_choice.get("disable_parallel_tool_use"))
            .and_then(|value| value.as_bool())
            .map(|disabled| !disabled)
            .unwrap_or(true)
    }
}

struct OpenAIRequestMapper;

impl OpenAIRequestMapper {
    fn build_body(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        effective_stream: bool,
        requested_model: &str,
    ) -> Value {
        let model = OpenAIChatBackend::normalize_model(requested_model);
        let (transformed_messages, _) =
            MessageProcessor::transform_messages(&anthropic_body.messages, log_tx);

        let mut messages = Vec::new();
        if let Some(system_msg) = OpenAIChatBackend::build_system_message(
            anthropic_body.system.as_ref(),
            &ctx.custom_injection_prompt,
        ) {
            messages.push(system_msg);
        }
        messages.extend(OpenAIChatBackend::build_messages(&transformed_messages));

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
        log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        effective_stream: bool,
        model_override: Option<String>,
    ) -> (Value, String) {
        let session_id = Uuid::new_v4().to_string();

        let requested = model_override
            .or_else(|| anthropic_body.model.clone())
            .unwrap_or_else(|| "gpt-4o".to_string());
        let body = OpenAIRequestMapper::build_body(
            anthropic_body,
            log_tx,
            ctx,
            effective_stream,
            &requested,
        );

        (body, session_id)
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
    block_index: Option<usize>,
    has_started: bool,
}

/// Response transformer for OpenAI Chat Completion SSE to Anthropic SSE
pub struct OpenAIChatResponseTransformer {
    message_id: String,
    model: String,
    allow_visible_thinking: bool,
    content_index: usize,
    open_text_index: Option<usize>,
    open_text_block_kind: Option<TextBlockKind>,
    sent_message_start: bool,
    sent_message_stop: bool,
    tool_calls: Vec<Option<ToolCallState>>,
    saw_tool_call: bool,
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
            content_index: 0,
            open_text_index: None,
            open_text_block_kind: None,
            sent_message_start: false,
            sent_message_stop: false,
            tool_calls: Vec::new(),
            saw_tool_call: false,
            finish_reason: None,
            stop_sequence: None,
            usage: None,
        }
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
        if let Some(Some(state)) = self.tool_calls.get(tool_index) {
            if state.has_started {
                return;
            }
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

    fn close_tool_block(&mut self, tool_index: usize, out: &mut Vec<String>) {
        if let Some(Some(state)) = self.tool_calls.get_mut(tool_index) {
            if let Some(idx) = state.block_index.take() {
                state.has_started = false;
                out.push(format!(
                    "event: content_block_stop\ndata: {}\n\n",
                    json!({ "type": "content_block_stop", "index": idx })
                ));
            }
        }
    }

    fn map_finish_reason(reason: Option<&str>, saw_tool_call: bool) -> &'static str {
        match reason {
            Some("tool_calls") => "tool_use",
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

    fn first_choice<'a>(&self, data: &'a Value) -> Option<&'a Value> {
        data.get("choices")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
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
            name: "unknown".to_string(),
            arguments: String::new(),
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

        if let Some(args_delta) = tool_call_delta
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
        {
            if !args_delta.is_empty() {
                if let Some(Some(state)) = self.tool_calls.get(index) {
                    if let Some(block_idx) = state.block_index {
                        out.push(format!(
                            "event: content_block_delta\ndata: {}\n\n",
                            json!({
                                "type": "content_block_delta",
                                "index": block_idx,
                                "delta": { "type": "input_json_delta", "partial_json": args_delta }
                            })
                        ));
                    }
                }
            }
        }
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

        assert_eq!(
            body.get("metadata"),
            Some(&json!({"trace_id": "req-123", "tenant": "demo"}))
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
        assert_eq!(
            required_body.get("stream_options"),
            Some(&json!({"include_usage": true}))
        );
        assert!(
            required_body
                .get("temperature")
                .and_then(Value::as_f64)
                .map(|value| (value - 0.2).abs() < 1e-6)
                .unwrap_or(false),
            "temperature should be preserved within float tolerance"
        );
        assert!(
            required_body
                .get("top_p")
                .and_then(Value::as_f64)
                .map(|value| (value - 0.9).abs() < 1e-6)
                .unwrap_or(false),
            "top_p should be preserved within float tolerance"
        );
        assert_eq!(required_body.get("stop"), Some(&json!(["</END>"])));

        let mut none_request = required_request;
        none_request.tool_choice = Some(json!("none"));
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
    fn transform_request_appends_custom_injection_prompt_to_system_message() {
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
        assert!(content.contains("Always inspect repo instructions first."));
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
    fn transform_request_creates_system_message_from_custom_injection_prompt_when_missing() {
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
            Some("system")
        );
        assert_eq!(
            messages[0].get("content").and_then(Value::as_str),
            Some("Always inspect repo instructions first.")
        );
        assert_eq!(
            messages[1].get("role").and_then(Value::as_str),
            Some("user")
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
