use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::models::AnthropicRequest;

use super::{
    providers::GeminiAdapter, ResponseTransformer, TransformBackend, TransformContext,
};

pub struct GeminiBackend;

struct GeminiUpstreamRequestBuilder;

impl GeminiUpstreamRequestBuilder {
    fn uses_cli_style_headers(target_url: &str) -> bool {
        let lower = target_url.to_ascii_lowercase();
        !(lower.contains("generativelanguage.googleapis.com")
            || lower.contains("googleapis.com"))
    }

    fn build_endpoint(target_url: &str, model: &str) -> String {
        if target_url.contains(":streamGenerateContent") {
            target_url.to_string()
        } else if target_url.contains("{model}") {
            target_url.replace("{model}", model)
        } else {
            let base = target_url.trim_end_matches('/');
            format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", base, model)
        }
    }

    fn apply_auth_headers(
        builder: reqwest::RequestBuilder,
        target_url: &str,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        let builder = builder
            .header("x-goog-api-key", api_key)
            .header("Authorization", format!("Bearer {}", api_key));

        if Self::uses_cli_style_headers(target_url) {
            builder.header("x-goog-api-client", "GeminiCLI/1.0")
        } else {
            builder
        }
    }

    fn build_request(
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
    ) -> reqwest::RequestBuilder {
        let model = body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gemini-3-pro-preview");
        let endpoint = Self::build_endpoint(target_url, model);

        let mut upstream_body = body.clone();
        if let Some(obj) = upstream_body.as_object_mut() {
            obj.remove("model");
        }

        let builder = client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");
        Self::apply_auth_headers(builder, target_url, api_key).body(upstream_body.to_string())
    }
}

impl TransformBackend for GeminiBackend {
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
            .unwrap_or("gemini-3-pro-preview");
        let prepared = GeminiAdapter.prepare_messages_request(
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
        GeminiUpstreamRequestBuilder::build_request(client, target_url, api_key, body)
    }

    fn create_response_transformer(
        &self,
        model: &str,
        _allow_visible_thinking: bool,
    ) -> Box<dyn ResponseTransformer> {
        Box::new(GeminiResponseTransformer::new(model))
    }
}

pub struct GeminiResponseTransformer {
    message_id: String,
    model: String,
    content_index: usize,
    open_text_index: Option<usize>,
    open_text_block_kind: Option<TextBlockKind>,
    open_tool_index: Option<usize>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    saw_tool_call: bool,
    sent_message_start: bool,
    sent_message_stop: bool,
    thought_signature: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextBlockKind {
    Text,
    Thinking,
}

impl GeminiResponseTransformer {
    pub fn new(model: &str) -> Self {
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            content_index: 0,
            open_text_index: None,
            open_text_block_kind: None,
            open_tool_index: None,
            tool_call_id: None,
            tool_name: None,
            saw_tool_call: false,
            sent_message_start: false,
            sent_message_stop: false,
            thought_signature: None,
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

    fn close_text_block(&mut self, out: &mut Vec<String>) {
        if let Some(idx) = self.open_text_index.take() {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        self.open_text_block_kind = None;
    }

    fn open_tool_block_if_needed(&mut self, out: &mut Vec<String>) {
        if self.open_tool_index.is_some() {
            return;
        }
        self.saw_tool_call = true;
        self.close_text_block(out);

        let call_id = self
            .tool_call_id
            .clone()
            .unwrap_or_else(|| format!("tool_{}", chrono::Utc::now().timestamp_millis()));
        let tool_name = self
            .tool_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let idx = self.content_index;
        self.content_index += 1;
        self.open_tool_index = Some(idx);

        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": {
                    "type": "tool_use",
                    "id": call_id,
                    "name": tool_name,
                    "input": {}
                }
            })
        ));
    }

    fn close_tool_block(&mut self, out: &mut Vec<String>) {
        if let Some(idx) = self.open_tool_index.take() {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
    }

    fn open_thinking_block_if_needed(&mut self, out: &mut Vec<String>, signature: Option<&str>) {
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
                "content_block": {
                    "type": "thinking",
                    "thinking": "",
                    "signature": signature
                }
            })
        ));
    }

    fn emit_message_stop(&mut self, out: &mut Vec<String>, stop_reason: &str) {
        if self.sent_message_stop {
            return;
        }
        self.sent_message_stop = true;

        self.close_text_block(out);
        self.close_tool_block(out);

        out.push(format!(
            "event: message_delta\ndata: {}\n\n",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            })
        ));
        out.push(format!(
            "event: message_stop\ndata: {}\n\n",
            json!({ "type": "message_stop", "stop_reason": stop_reason })
        ));
    }

    fn extract_thinking_from_candidates(data: &Value) -> Vec<String> {
        data.get("candidates")
            .and_then(|v| v.as_array())
            .map(|candidates| {
                candidates
                    .iter()
                    .flat_map(|candidate| {
                        candidate
                            .get("content")
                            .and_then(|v| v.get("parts"))
                            .and_then(|v| v.as_array())
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(|part| {
                                        if part
                                            .get("thought")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(false)
                                        {
                                            part.get("text")
                                                .and_then(|t| t.as_str())
                                                .map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn extract_thought_signature(data: &Value) -> Option<String> {
        let candidates = data.get("candidates")?.as_array()?;
        for candidate in candidates {
            // Check top-level of candidate first
            if let Some(sig) = candidate.get("thoughtSignature").and_then(|v| v.as_str()) {
                return Some(sig.to_string());
            }
            // Check in parts (sometimes it's inside a part)
            if let Some(parts) = candidate
                .get("content")
                .and_then(|v| v.get("parts"))
                .and_then(|v| v.as_array())
            {
                for part in parts {
                    if let Some(sig) = part.get("thoughtSignature").and_then(|v| v.as_str()) {
                        return Some(sig.to_string());
                    }
                }
            }
        }
        None
    }

    fn extract_tool_call(data: &Value) -> Option<(String, Value)> {
        let candidates = data.get("candidates")?.as_array()?;
        for candidate in candidates {
            let parts = candidate
                .get("content")
                .and_then(|v| v.get("parts"))
                .and_then(|v| v.as_array())?;

            for part in parts {
                if let Some(function_call) = part.get("functionCall") {
                    let name = function_call
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let args = function_call
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    return Some((name, args));
                }
            }
        }
        None
    }

    fn has_finish_reason(data: &Value) -> bool {
        data.get("candidates")
            .and_then(|v| v.as_array())
            .map(|candidates| {
                candidates.iter().any(|candidate| {
                    candidate
                        .get("finishReason")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }
}

impl ResponseTransformer for GeminiResponseTransformer {
    fn transform_line(&mut self, line: &str) -> Vec<String> {
        let mut output = Vec::new();

        if !line.starts_with("data: ") {
            return output;
        }

        self.ensure_message_start(&mut output);

        let payload = line[6..].trim();
        if payload == "[DONE]" {
            let stop_reason = if self.saw_tool_call {
                "tool_use"
            } else {
                "end_turn"
            };
            self.emit_message_stop(&mut output, stop_reason);
            return output;
        }

        let Ok(parsed_data) = serde_json::from_str::<Value>(payload) else {
            return output;
        };
        let data = parsed_data.get("response").cloned().unwrap_or(parsed_data);

        // 1. Extract thought signature if present
        if let Some(sig) = Self::extract_thought_signature(&data) {
            self.thought_signature = Some(sig);
        }

        // 2. Process Thinking/Thought
        let sig = self.thought_signature.clone();
        for thinking in Self::extract_thinking_from_candidates(&data) {
            if thinking.is_empty() {
                continue;
            }
            self.open_thinking_block_if_needed(&mut output, sig.as_deref());
            output.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": self.open_text_index,
                    "delta": { "type": "thinking_delta", "thinking": thinking }
                })
            ));
        }

        // 3. Process normal text
        data.get("candidates")
            .and_then(|v| v.as_array())
            .map(|candidates| {
                for candidate in candidates {
                    if let Some(parts) = candidate
                        .get("content")
                        .and_then(|v| v.get("parts"))
                        .and_then(|v| v.as_array())
                    {
                        for part in parts {
                            // Only process text that is NOT thought
                            if part
                                .get("thought")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                continue;
                            }
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                if text.is_empty() {
                                    continue;
                                }
                                self.open_text_block_if_needed(&mut output);
                                output.push(format!(
                                    "event: content_block_delta\ndata: {}\n\n",
                                    json!({
                                        "type": "content_block_delta",
                                        "index": self.open_text_index,
                                        "delta": { "type": "text_delta", "text": text }
                                    })
                                ));
                            }
                        }
                    }
                }
            });

        // 4. Process tool calls

        if let Some((tool_name, args)) = Self::extract_tool_call(&data) {
            self.tool_name = Some(tool_name);
            self.tool_call_id = Some(format!("tool_{}", chrono::Utc::now().timestamp_millis()));
            self.open_tool_block_if_needed(&mut output);

            let partial_json = if args.is_string() {
                args.as_str().unwrap_or("").to_string()
            } else {
                serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
            };
            output.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": self.open_tool_index,
                    "delta": { "type": "input_json_delta", "partial_json": partial_json }
                })
            ));
            self.close_tool_block(&mut output);
            self.tool_call_id = None;
            self.tool_name = None;
        }

        if Self::has_finish_reason(&data) {
            let stop_reason = if self.saw_tool_call {
                "tool_use"
            } else {
                "end_turn"
            };
            self.emit_message_stop(&mut output, stop_reason);
        }

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

    #[test]
    fn request_mapper_merges_top_level_and_message_system_into_system_instruction() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "gemini-2.0-flash",
            "system": "top-level system",
            "messages": [
                {
                    "role": "system",
                    "content": [{"type": "text", "text": "message system"}]
                },
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "hello"}]
                }
            ],
            "stream": true
        }))
        .expect("valid anthropic request");

        let unified = crate::transform::UnifiedChatRequest::from_anthropic(&request);
        let ctx = crate::transform::TransformContext {
            reasoning_mapping: crate::models::ReasoningEffortMapping::default(),
            codex_model_mapping: crate::models::CodexModelMapping::default(),
            anthropic_model_mapping: crate::models::AnthropicModelMapping::default(),
            openai_model_mapping: crate::models::OpenAIModelMapping::default(),
            openai_max_tokens_mapping: crate::models::OpenAIMaxTokensMapping::default(),
            custom_injection_prompt: String::new(),
            converter: "gemini".to_string(),
            codex_model: String::new(),
            gemini_reasoning_effort: crate::models::GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let prepared = crate::transform::GeminiAdapter.prepare_messages_request(
            &unified,
            &ctx,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse",
            "test-key",
            "2023-06-01",
            "gemini-2.0-flash",
            true,
        );
        let body = prepared.body;
        let parts = body
            .get("system_instruction")
            .and_then(|value| value.get("parts"))
            .and_then(|value| value.as_array())
            .expect("system instruction parts");

        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].get("text").and_then(Value::as_str), Some("top-level system"));
    }

    #[test]
    fn upstream_builder_uses_official_google_headers_for_google_endpoint() {
        let client = reqwest::Client::new();
        let body = json!({"model": "gemini-2.0-flash", "contents": [], "system_instruction": null});

        let request = GeminiUpstreamRequestBuilder::build_request(
            &client,
            "https://generativelanguage.googleapis.com",
            "test-key",
            &body,
        )
        .build()
        .expect("request should build");

        assert_eq!(
            request.url().as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            request.headers().get("x-goog-api-key").and_then(|value| value.to_str().ok()),
            Some("test-key")
        );
        assert!(
            request.headers().get("x-goog-api-client").is_none(),
            "official google endpoint should not add Gemini CLI client header"
        );
    }

    #[test]
    fn upstream_builder_uses_cli_header_for_non_google_endpoint() {
        let client = reqwest::Client::new();
        let body = json!({"model": "gemini-2.0-flash", "contents": [], "system_instruction": null});

        let request = GeminiUpstreamRequestBuilder::build_request(
            &client,
            "https://gemini-cli.example.com",
            "test-key",
            &body,
        )
        .build()
        .expect("request should build");

        assert_eq!(
            request.headers().get("x-goog-api-client").and_then(|value| value.to_str().ok()),
            Some("GeminiCLI/1.0")
        );
    }

    #[test]
    fn wrapped_response_and_usage_variants_emit_stable_message_stop() {
        let mut transformer = GeminiResponseTransformer::new("gemini-test");
        let events = transformer.transform_line(
            r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"pong"}]},"finishReason":"STOP"}],"usage":{"promptTokenCount":3,"candidatesTokenCount":5,"totalTokenCount":8}}}"#,
        );
        let parsed_events: Vec<(String, Value)> =
            events.iter().map(|event| parse_sse_event(event)).collect();

        assert!(
            parsed_events.iter().any(|(name, payload)| {
                name == "content_block_delta"
                    && payload
                        .get("delta")
                        .and_then(|delta| delta.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("text_delta")
            }),
            "wrapped response should still emit text deltas"
        );
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
}
