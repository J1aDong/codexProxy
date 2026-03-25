use super::{
    CountTokensMode, PreparedCountTokensRequest, PreparedRequest, RequestEnvelopeHints,
    TransformContext,
};
use crate::models::get_reasoning_effort;
use crate::transform::unified::{
    UnifiedChatRequest, UnifiedContent, UnifiedMessage, UnifiedMessageRole, UnifiedToolChoice,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default)]
pub struct AnthropicAdapter;

#[derive(Clone, Copy, Debug, Default)]
pub struct CodexAdapter;

#[derive(Clone, Copy, Debug, Default)]
pub struct OpenAIChatAdapter;

#[derive(Clone, Copy, Debug, Default)]
pub struct GeminiAdapter;

impl AnthropicAdapter {
    pub fn prepare_messages_request(
        &self,
        unified: &UnifiedChatRequest,
        _ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        anthropic_version: &str,
        route_model: &str,
        _effective_stream: bool,
    ) -> PreparedRequest {
        PreparedRequest {
            url: target_url.to_string(),
            headers: anthropic_headers(api_key, anthropic_version, true),
            body: encode_anthropic_body(unified, route_model),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn prepare_count_tokens_request(
        &self,
        unified: &UnifiedChatRequest,
        _ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        anthropic_version: &str,
        route_model: &str,
    ) -> PreparedCountTokensRequest {
        PreparedCountTokensRequest::native(PreparedRequest {
            url: anthropic_count_tokens_url(target_url),
            headers: anthropic_headers(api_key, anthropic_version, false),
            body: encode_anthropic_body(unified, route_model),
            session_id: Uuid::new_v4().to_string(),
        })
    }
}

impl CodexAdapter {
    pub fn prepare_messages_request(
        &self,
        unified: &UnifiedChatRequest,
        ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        anthropic_version: &str,
        route_model: &str,
        effective_stream: bool,
    ) -> PreparedRequest {
        self.prepare_messages_request_with_hints(
            unified,
            ctx,
            target_url,
            api_key,
            anthropic_version,
            route_model,
            effective_stream,
            &RequestEnvelopeHints::default(),
        )
    }

    pub fn prepare_messages_request_with_hints(
        &self,
        unified: &UnifiedChatRequest,
        ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        anthropic_version: &str,
        route_model: &str,
        effective_stream: bool,
        hints: &RequestEnvelopeHints,
    ) -> PreparedRequest {
        PreparedRequest {
            url: codex_messages_url(target_url),
            headers: codex_headers(api_key, anthropic_version, true),
            body: encode_codex_body(unified, ctx, route_model, effective_stream, hints),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn prepare_count_tokens_request(
        &self,
        unified: &UnifiedChatRequest,
        ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        anthropic_version: &str,
        route_model: &str,
    ) -> PreparedCountTokensRequest {
        self.prepare_count_tokens_request_with_hints(
            unified,
            ctx,
            target_url,
            api_key,
            anthropic_version,
            route_model,
            &RequestEnvelopeHints::default(),
        )
    }

    pub fn prepare_count_tokens_request_with_hints(
        &self,
        unified: &UnifiedChatRequest,
        ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        anthropic_version: &str,
        route_model: &str,
        hints: &RequestEnvelopeHints,
    ) -> PreparedCountTokensRequest {
        PreparedCountTokensRequest::native(PreparedRequest {
            url: codex_count_tokens_url(target_url),
            headers: codex_headers(api_key, anthropic_version, false),
            body: encode_codex_body(unified, ctx, route_model, false, hints),
            session_id: Uuid::new_v4().to_string(),
        })
    }
}

impl OpenAIChatAdapter {
    pub fn prepare_messages_request(
        &self,
        unified: &UnifiedChatRequest,
        ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        _anthropic_version: &str,
        route_model: &str,
        effective_stream: bool,
    ) -> PreparedRequest {
        PreparedRequest {
            url: openai_messages_url(target_url),
            headers: openai_headers(api_key, true),
            body: encode_openai_body(unified, ctx, route_model, effective_stream),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn prepare_count_tokens_request(
        &self,
        _unified: &UnifiedChatRequest,
        _ctx: &TransformContext,
        _target_url: &str,
        _api_key: &str,
        _anthropic_version: &str,
        _route_model: &str,
    ) -> PreparedCountTokensRequest {
        PreparedCountTokensRequest {
            mode: CountTokensMode::Estimate,
            request: None,
        }
    }
}

impl GeminiAdapter {
    pub fn prepare_messages_request(
        &self,
        unified: &UnifiedChatRequest,
        _ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        _anthropic_version: &str,
        route_model: &str,
        _effective_stream: bool,
    ) -> PreparedRequest {
        PreparedRequest {
            url: gemini_messages_url(target_url, route_model),
            headers: gemini_headers(api_key, true),
            body: encode_gemini_body(unified, route_model),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn prepare_count_tokens_request(
        &self,
        unified: &UnifiedChatRequest,
        _ctx: &TransformContext,
        target_url: &str,
        api_key: &str,
        _anthropic_version: &str,
        route_model: &str,
    ) -> PreparedCountTokensRequest {
        PreparedCountTokensRequest::native(PreparedRequest {
            url: gemini_count_tokens_url(target_url, route_model),
            headers: gemini_headers(api_key, false),
            body: encode_gemini_body(unified, route_model),
            session_id: Uuid::new_v4().to_string(),
        })
    }
}

fn encode_anthropic_body(unified: &UnifiedChatRequest, route_model: &str) -> Value {
    let system = system_text(unified);
    let messages: Vec<Value> = unified
        .messages
        .iter()
        .filter_map(|message| match message.role {
            UnifiedMessageRole::System => None,
            UnifiedMessageRole::User => Some(json!({
                "role": "user",
                "content": anthropic_content_blocks(message, false),
            })),
            UnifiedMessageRole::Assistant => Some(json!({
                "role": "assistant",
                "content": anthropic_content_blocks(message, true),
            })),
            UnifiedMessageRole::Tool => Some(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id,
                    "content": message.content_text().unwrap_or_default(),
                }]
            })),
        })
        .collect();

    let mut body = json!({
        "model": route_model,
        "messages": messages,
        "stream": unified.stream,
        "max_tokens": unified.max_tokens,
        "temperature": unified.temperature,
    });

    if let Some(system) = system {
        body["system"] = json!(system);
    }
    if let Some(tools) = encode_anthropic_tools(unified) {
        body["tools"] = json!(tools);
    }
    if let Some(choice) = encode_anthropic_tool_choice(unified) {
        body["tool_choice"] = choice;
    }
    if let Some(reasoning) = encode_anthropic_reasoning(unified) {
        body["thinking"] = reasoning;
    }

    body
}

fn encode_codex_body(
    unified: &UnifiedChatRequest,
    ctx: &TransformContext,
    route_model: &str,
    effective_stream: bool,
    hints: &RequestEnvelopeHints,
) -> Value {
    let (promoted_context, cleaned_messages) = extract_user_scaffolding_to_codex_instructions(unified);
    let applied_system_text = system_text(unified)
        .map(|text| strip_dynamic_system_header_lines(&text))
        .filter(|text| !text.trim().is_empty());
    let applied_instructions =
        merge_instruction_context(applied_system_text.as_deref(), promoted_context.as_deref());
    let mut input = Vec::new();

    for message in &cleaned_messages {
        match message.role {
            UnifiedMessageRole::System => {}
            UnifiedMessageRole::User => {
                input.push(json!({
                    "role": "user",
                    "content": codex_content_blocks(message, false),
                }));
            }
            UnifiedMessageRole::Assistant => {
                let assistant_content = codex_content_blocks(message, true);
                if !assistant_content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": assistant_content,
                    }));
                }
                for call in &message.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.function.name,
                        "arguments": call.function.arguments,
                    }));
                }
            }
            UnifiedMessageRole::Tool => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": message.tool_call_id,
                    "output": message.content_text().unwrap_or_default(),
                }));
            }
        }
    }

    let mut body = json!({
        "model": route_model,
        "input": input,
        "store": false,
        "stream": effective_stream,
        "prompt_cache_key": build_prompt_cache_key(
            route_model,
            hints,
            applied_instructions.as_deref(),
            codex_tools_fingerprint(unified).as_deref(),
        ),
    });

    if let Some(instructions) = applied_instructions {
        body["instructions"] = json!(instructions);
    }
    if let Some(max_tokens) = unified.max_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = unified.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(tools) = encode_codex_tools(unified) {
        body["tools"] = json!(tools);
    }
    if let Some(choice) = encode_codex_tool_choice(unified) {
        body["tool_choice"] = choice;
    }
    if unified.reasoning.as_ref().map(|reasoning| reasoning.enabled).unwrap_or(false) {
        body["reasoning"] = json!({
            "effort": unified
                .reasoning
                .as_ref()
                .and_then(|reasoning| reasoning.effort.clone())
                .unwrap_or_else(|| get_reasoning_effort(&unified.model, &ctx.reasoning_mapping).as_str().to_string()),
            "summary": "detailed",
        });
    }

    body
}

fn encode_openai_body(
    unified: &UnifiedChatRequest,
    ctx: &TransformContext,
    route_model: &str,
    effective_stream: bool,
) -> Value {
    let messages: Vec<Value> = unified
        .messages
        .iter()
        .filter_map(|message| match message.role {
            UnifiedMessageRole::System => message
                .content_text()
                .map(|text| json!({"role":"system","content": text })),
            UnifiedMessageRole::User => Some(json!({
                "role": "user",
                "content": openai_message_content(message),
            })),
            UnifiedMessageRole::Assistant => Some(json!({
                "role": "assistant",
                "content": assistant_text_for_provider(message),
                "tool_calls": if message.tool_calls.is_empty() {
                    Value::Null
                } else {
                    json!(message.tool_calls.iter().map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.function.name,
                                "arguments": call.function.arguments,
                            }
                        })
                    }).collect::<Vec<_>>())
                }
            })),
            UnifiedMessageRole::Tool => Some(json!({
                "role": "tool",
                "tool_call_id": message.tool_call_id,
                "content": message.content_text().unwrap_or_default(),
            })),
        })
        .collect();

    let max_tokens = ctx
        .openai_max_tokens_mapping
        .get_limit(route_model)
        .map(|limit| unified.max_tokens.map(|value| value.min(limit)).unwrap_or(limit))
        .or(unified.max_tokens);

    let mut body = json!({
        "model": route_model,
        "messages": messages,
        "stream": effective_stream,
        "max_tokens": max_tokens,
        "temperature": unified.temperature,
    });

    if let Some(tools) = encode_openai_tools(unified) {
        body["tools"] = json!(tools);
        body["parallel_tool_calls"] = json!(false);
    }
    if let Some(choice) = encode_openai_tool_choice(unified) {
        body["tool_choice"] = choice;
    }

    body
}

fn encode_gemini_body(unified: &UnifiedChatRequest, route_model: &str) -> Value {
    let contents: Vec<Value> = unified
        .messages
        .iter()
        .filter_map(|message| match message.role {
            UnifiedMessageRole::System => None,
            UnifiedMessageRole::User => Some(json!({
                "role": "user",
                "parts": gemini_parts_for_user(message),
            })),
            UnifiedMessageRole::Assistant => Some(json!({
                "role": "model",
                "parts": gemini_parts_for_assistant(message),
            })),
            UnifiedMessageRole::Tool => Some(json!({
                "role": "function",
                "parts": [{
                    "functionResponse": {
                        "name": "tool",
                        "response": {
                            "result": message.content_text().unwrap_or_default(),
                        }
                    }
                }]
            })),
        })
        .collect();

    let mut generation_config = json!({});
    if let Some(config) = generation_config.as_object_mut() {
        if let Some(max_tokens) = unified.max_tokens {
            config.insert("maxOutputTokens".to_string(), json!(max_tokens));
        }
        if let Some(temp) = unified.temperature {
            config.insert("temperature".to_string(), json!(temp));
        }
    }

    let mut body = json!({
        "model": route_model,
        "contents": contents,
        "generationConfig": generation_config,
    });

    if let Some(system) = system_text(unified) {
        body["system_instruction"] = json!({
            "parts": [{ "text": system }]
        });
    }
    if let Some(tools) = encode_gemini_tools(unified) {
        body["tools"] = json!(tools);
    }

    body
}

fn encode_anthropic_tools(unified: &UnifiedChatRequest) -> Option<Vec<Value>> {
    unified.tools.as_ref().map(|tools| {
        tools.iter()
            .map(|tool| {
                json!({
                    "name": tool.function.name,
                    "description": tool.function.description,
                    "input_schema": tool.function.parameters,
                })
            })
            .collect()
    })
}

fn encode_codex_tools(unified: &UnifiedChatRequest) -> Option<Vec<Value>> {
    unified.tools.as_ref().map(|tools| {
        tools.iter()
            .map(|tool| {
                let parameters = normalize_codex_tool_schema(&tool.function.parameters);
                json!({
                    "type": "function",
                    "name": tool.function.name,
                    "description": tool.function.description,
                    "parameters": parameters,
                })
            })
            .collect()
    })
}

fn encode_openai_tools(unified: &UnifiedChatRequest) -> Option<Vec<Value>> {
    unified.tools.as_ref().map(|tools| {
        tools.iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.function.name,
                        "description": tool.function.description,
                        "parameters": tool.function.parameters,
                    }
                })
            })
            .collect()
    })
}

fn encode_gemini_tools(unified: &UnifiedChatRequest) -> Option<Vec<Value>> {
    unified.tools.as_ref().map(|tools| {
        vec![json!({
            "functionDeclarations": tools.iter().map(|tool| {
                json!({
                    "name": tool.function.name,
                    "description": tool.function.description,
                    "parametersJsonSchema": tool.function.parameters,
                })
            }).collect::<Vec<_>>()
        })]
    })
}

fn encode_anthropic_tool_choice(unified: &UnifiedChatRequest) -> Option<Value> {
    match unified.tool_choice.as_ref()? {
        UnifiedToolChoice::Auto => Some(json!({ "type": "auto" })),
        UnifiedToolChoice::None => Some(json!({ "type": "none" })),
        UnifiedToolChoice::Required => Some(json!({ "type": "any" })),
        UnifiedToolChoice::Function { name } => Some(json!({ "type": "tool", "name": name })),
    }
}

fn encode_codex_tool_choice(unified: &UnifiedChatRequest) -> Option<Value> {
    match unified.tool_choice.as_ref()? {
        UnifiedToolChoice::Auto => Some(json!("auto")),
        UnifiedToolChoice::None => Some(json!("none")),
        UnifiedToolChoice::Required => Some(json!("required")),
        UnifiedToolChoice::Function { name } => Some(json!({
            "type": "function",
            "name": name,
        })),
    }
}

fn encode_openai_tool_choice(unified: &UnifiedChatRequest) -> Option<Value> {
    match unified.tool_choice.as_ref()? {
        UnifiedToolChoice::Auto => Some(json!("auto")),
        UnifiedToolChoice::None => Some(json!("none")),
        UnifiedToolChoice::Required => Some(json!("required")),
        UnifiedToolChoice::Function { name } => Some(json!({
            "type": "function",
            "function": { "name": name }
        })),
    }
}

fn encode_anthropic_reasoning(unified: &UnifiedChatRequest) -> Option<Value> {
    let reasoning = unified.reasoning.as_ref()?;
    if !reasoning.enabled {
        return Some(json!({ "type": "disabled" }));
    }

    Some(json!({
        "type": "enabled",
        "budget_tokens": reasoning.max_tokens.unwrap_or(2048),
    }))
}

fn anthropic_content_blocks(message: &UnifiedMessage, include_tool_use: bool) -> Vec<Value> {
    let mut blocks = Vec::new();

    if let Some(thinking) = message.thinking.as_ref() {
        blocks.push(json!({
            "type": "thinking",
            "thinking": thinking.content,
            "signature": thinking.signature,
        }));
    }

    for item in &message.content {
        match item {
            UnifiedContent::Text { text } => blocks.push(json!({
                "type": "text",
                "text": text,
            })),
            UnifiedContent::ImageUrl { url, media_type } => blocks.push(json!({
                "type": "image_url",
                "image_url": url,
                "media_type": media_type,
            })),
        }
    }

    if include_tool_use {
        for call in &message.tool_calls {
            let parsed = serde_json::from_str::<Value>(&call.function.arguments)
                .unwrap_or_else(|_| json!({}));
            blocks.push(json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.function.name,
                "input": parsed,
            }));
        }
    }

    blocks
}

fn codex_content_blocks(message: &UnifiedMessage, is_assistant: bool) -> Vec<Value> {
    let mut blocks = Vec::new();

    if let Some(thinking) = message.thinking.as_ref() {
        blocks.push(json!({
            "type": if is_assistant { "output_text" } else { "input_text" },
            "text": thinking.content,
        }));
    }

    for item in &message.content {
        match item {
            UnifiedContent::Text { text } => blocks.push(json!({
                "type": if is_assistant { "output_text" } else { "input_text" },
                "text": text,
            })),
            UnifiedContent::ImageUrl { url, .. } => blocks.push(json!({
                "type": "input_image",
                "image_url": url,
                "detail": "auto",
            })),
        }
    }

    blocks
}

fn openai_message_content(message: &UnifiedMessage) -> Value {
    let has_image = message
        .content
        .iter()
        .any(|item| matches!(item, UnifiedContent::ImageUrl { .. }));
    if !has_image && message.content.len() == 1 {
        if let Some(UnifiedContent::Text { text }) = message.content.first() {
            return json!(text);
        }
    }

    json!(
        message
            .content
            .iter()
            .map(|item| match item {
                UnifiedContent::Text { text } => json!({
                    "type": "text",
                    "text": text,
                }),
                UnifiedContent::ImageUrl { url, .. } => json!({
                    "type": "image_url",
                    "image_url": { "url": url },
                }),
            })
            .collect::<Vec<_>>()
    )
}

fn gemini_parts_for_user(message: &UnifiedMessage) -> Vec<Value> {
    message
        .content
        .iter()
        .map(|item| match item {
            UnifiedContent::Text { text } => json!({ "text": text }),
            UnifiedContent::ImageUrl { url, media_type } => {
                if url.starts_with("http") {
                    json!({
                        "file_data": {
                            "mime_type": media_type.clone().unwrap_or_else(|| "image/png".to_string()),
                            "file_uri": url,
                        }
                    })
                } else {
                    json!({
                        "inline_data": {
                            "mime_type": media_type.clone().unwrap_or_else(|| "image/png".to_string()),
                            "data": data_tail(url),
                        }
                    })
                }
            }
        })
        .collect()
}

fn gemini_parts_for_assistant(message: &UnifiedMessage) -> Vec<Value> {
    let mut parts = Vec::new();
    if let Some(thinking) = message.thinking.as_ref() {
        parts.push(json!({
            "text": thinking.content,
            "thought": true,
            "thought_signature": thinking.signature,
        }));
    }
    for item in &message.content {
        if let UnifiedContent::Text { text } = item {
            parts.push(json!({ "text": text }));
        }
    }
    for call in &message.tool_calls {
        let args = serde_json::from_str::<Value>(&call.function.arguments)
            .unwrap_or_else(|_| json!({}));
        parts.push(json!({
            "functionCall": {
                "name": call.function.name,
                "args": args,
            }
        }));
    }
    parts
}

fn system_text(unified: &UnifiedChatRequest) -> Option<String> {
    let texts: Vec<String> = unified
        .messages
        .iter()
        .filter(|message| message.role == UnifiedMessageRole::System)
        .filter_map(UnifiedMessage::content_text)
        .filter(|text| !text.trim().is_empty())
        .collect();

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n\n"))
    }
}

fn assistant_text_for_provider(message: &UnifiedMessage) -> String {
    let mut parts = Vec::new();
    if let Some(thinking) = message.thinking.as_ref() {
        parts.push(thinking.content.clone());
    }
    if let Some(text) = message.content_text() {
        parts.push(text);
    }
    parts.join("\n")
}

fn anthropic_headers(api_key: &str, anthropic_version: &str, stream: bool) -> Vec<(String, String)> {
    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };

    vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("x-api-key".to_string(), api_key.to_string()),
        ("Authorization".to_string(), format!("Bearer {}", api_key)),
        (
            "x-anthropic-version".to_string(),
            anthropic_version.to_string(),
        ),
        ("User-Agent".to_string(), "Anthropic-Node/0.3.4".to_string()),
        ("Accept".to_string(), accept.to_string()),
    ]
}

fn codex_headers(api_key: &str, anthropic_version: &str, stream: bool) -> Vec<(String, String)> {
    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };

    vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Authorization".to_string(), format!("Bearer {}", api_key)),
        ("x-api-key".to_string(), api_key.to_string()),
        ("User-Agent".to_string(), "Anthropic-Node/0.3.4".to_string()),
        (
            "x-anthropic-version".to_string(),
            anthropic_version.to_string(),
        ),
        ("originator".to_string(), "codex_cli_rs".to_string()),
        ("Accept".to_string(), accept.to_string()),
    ]
}

fn openai_headers(api_key: &str, stream: bool) -> Vec<(String, String)> {
    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };

    vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Authorization".to_string(), format!("Bearer {}", api_key)),
        ("Accept".to_string(), accept.to_string()),
    ]
}

fn gemini_headers(api_key: &str, stream: bool) -> Vec<(String, String)> {
    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };

    vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("x-goog-api-key".to_string(), api_key.to_string()),
        ("Authorization".to_string(), format!("Bearer {}", api_key)),
        ("Accept".to_string(), accept.to_string()),
    ]
}

fn openai_messages_url(target_url: &str) -> String {
    target_url.to_string()
}

fn codex_messages_url(target_url: &str) -> String {
    target_url.replace("/responses/input_tokens", "/responses")
}

fn codex_count_tokens_url(target_url: &str) -> String {
    if target_url.contains("/responses/input_tokens") {
        target_url.to_string()
    } else {
        target_url.replace("/responses", "/responses/input_tokens")
    }
}

fn anthropic_count_tokens_url(target_url: &str) -> String {
    if target_url.contains("/messages/count_tokens") {
        target_url.to_string()
    } else {
        target_url.replace("/messages", "/messages/count_tokens")
    }
}

fn gemini_messages_url(target_url: &str, _route_model: &str) -> String {
    target_url.to_string()
}

fn gemini_count_tokens_url(target_url: &str, _route_model: &str) -> String {
    target_url
        .replace(":streamGenerateContent?alt=sse", ":countTokens")
        .replace(":streamGenerateContent", ":countTokens")
        .replace(":generateContent", ":countTokens")
}

fn data_tail(url: &str) -> String {
    url.split_once(',')
        .map(|(_, tail)| tail.to_string())
        .unwrap_or_else(|| url.to_string())
}

fn normalize_codex_tool_schema(schema: &Value) -> Value {
    let mut normalized = schema.clone();
    if let Some(obj) = normalized.as_object_mut() {
        if obj.get("type").and_then(|value| value.as_str()) == Some("object")
            && !obj.contains_key("properties")
        {
            obj.insert("properties".to_string(), json!({}));
        }
    }
    normalized
}

fn normalize_text_for_exact_match(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn normalized_contains(haystack: &str, needle: &str) -> bool {
    let normalized_needle = normalize_text_for_exact_match(needle);
    if normalized_needle.is_empty() {
        return false;
    }

    normalize_text_for_exact_match(haystack).contains(&normalized_needle)
}

fn append_instruction_text(base: &str, extra: &str) -> String {
    let trimmed_base = base.trim();
    let trimmed_extra = extra.trim();

    if trimmed_base.is_empty() {
        return trimmed_extra.to_string();
    }
    if trimmed_extra.is_empty() {
        return trimmed_base.to_string();
    }
    if normalized_contains(trimmed_base, trimmed_extra) {
        return trimmed_base.to_string();
    }

    format!("{}\n\n{}", trimmed_base, trimmed_extra)
}

fn merge_instruction_context(first: Option<&str>, second: Option<&str>) -> Option<String> {
    let mut merged = None::<String>;
    for text in [first, second].into_iter().flatten() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        merged = Some(match merged {
            Some(existing) => append_instruction_text(&existing, trimmed),
            None => trimmed.to_string(),
        });
    }
    merged
}

fn strip_dynamic_system_header_lines(text: &str) -> String {
    let filtered = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("x-anthropic-billing-header:")
        })
        .collect::<Vec<_>>()
        .join("\n");
    normalize_text_for_exact_match(&filtered)
}

fn extract_user_scaffolding_to_codex_instructions(
    unified: &UnifiedChatRequest,
) -> (Option<String>, Vec<UnifiedMessage>) {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";

    let mut promoted_parts = Vec::new();
    let mut seen_promoted_parts = HashSet::new();
    let mut cleaned_messages = Vec::new();

    for message in &unified.messages {
        if message.role != UnifiedMessageRole::User {
            cleaned_messages.push(message.clone());
            continue;
        }

        let mut cloned = message.clone();
        let mut new_content = Vec::new();

        for item in &message.content {
            match item {
                UnifiedContent::Text { text } => {
                    let mut remaining = text.clone();

                    while let Some(start_idx) = remaining.find(START) {
                        let after_start = &remaining[start_idx + START.len()..];
                        let Some(end_rel) = after_start.find(END) else {
                            break;
                        };
                        let end_idx = start_idx + START.len() + end_rel + END.len();
                        let block_text =
                            remaining[start_idx + START.len()..start_idx + START.len() + end_rel]
                                .trim();
                        let normalized_block = normalize_text_for_exact_match(block_text);
                        if !normalized_block.is_empty()
                            && seen_promoted_parts.insert(normalized_block.clone())
                        {
                            promoted_parts.push(normalized_block);
                        }
                        remaining =
                            format!("{}{}", &remaining[..start_idx], &remaining[end_idx..]);
                    }

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
                            let normalized_rule_line = normalize_text_for_exact_match(rule_line);
                            if !normalized_rule_line.is_empty()
                                && seen_promoted_parts.insert(normalized_rule_line.clone())
                            {
                                promoted_parts.push(normalized_rule_line);
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
                        new_content.push(UnifiedContent::Text {
                            text: trimmed.to_string(),
                        });
                    }
                }
                other => new_content.push(other.clone()),
            }
        }

        cloned.content = new_content;
        if !cloned.content.is_empty() || !cloned.tool_calls.is_empty() {
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

fn sanitize_cache_key_segment(input: &str, max_len: usize) -> String {
    let mut segment = String::with_capacity(input.len().min(max_len));
    for ch in input.chars() {
        let normalized = if ch.is_ascii_alphanumeric() { ch } else { '_' };
        segment.push(normalized);
        if segment.len() >= max_len {
            break;
        }
    }

    let trimmed = segment.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_cache_material(text: &str) -> String {
    normalize_text_for_exact_match(text)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json_value).collect()),
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut normalized = serde_json::Map::new();
            for key in keys {
                if let Some(child) = map.get(&key) {
                    normalized.insert(key, canonicalize_json_value(child));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

fn fingerprint_json_value(value: &Value) -> String {
    let normalized = canonicalize_json_value(value);
    let bytes = serde_json::to_vec(&normalized).unwrap_or_default();
    format!("{:016x}", fnv1a64(&bytes))
}

fn codex_tools_fingerprint(unified: &UnifiedChatRequest) -> Option<String> {
    let tools = encode_codex_tools(unified)?;
    Some(fingerprint_json_value(&Value::Array(tools)))
}

fn build_prompt_cache_key(
    route_model: &str,
    hints: &RequestEnvelopeHints,
    applied_static_instructions: Option<&str>,
    tools_fingerprint: Option<&str>,
) -> String {
    if let Some(hint) = hints
        .session_hint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let model_segment = sanitize_cache_key_segment(route_model, 48);
        let hint_segment = sanitize_cache_key_segment(hint, 72);
        return format!(
            "codex-proxy:{}:{}:session:{}",
            model_segment,
            hints.request_kind.as_str(),
            hint_segment
        );
    }

    let mut key_material = Vec::new();
    if let Some(instructions) = applied_static_instructions {
        key_material.extend_from_slice(normalize_cache_material(&instructions).as_bytes());
    }
    key_material.push(0x1f);
    if let Some(tools_fingerprint) = tools_fingerprint {
        key_material.extend_from_slice(tools_fingerprint.as_bytes());
    }

    let key_hash = fnv1a64(&key_material);
    let model_segment = sanitize_cache_key_segment(route_model, 48);
    let cwd_segment = hints
        .request_cwd
        .as_deref()
        .map(|cwd| sanitize_cache_key_segment(cwd, 64))
        .unwrap_or_else(|| "default".to_string());

    format!(
        "codex-proxy:{}:{}:{}:{:016x}",
        model_segment,
        hints.request_kind.as_str(),
        cwd_segment,
        key_hash
    )
}
