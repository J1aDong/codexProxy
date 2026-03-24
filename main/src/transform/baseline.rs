use serde_json::{json, Value};
use uuid::Uuid;

use super::ResponseTransformer;
use crate::models::AnthropicRequest;

pub struct IdentityAnthropicRequestMapper;

impl IdentityAnthropicRequestMapper {
    pub fn map_request(
        anthropic_body: &AnthropicRequest,
        model_override: Option<String>,
    ) -> (Value, String) {
        let session_id = Uuid::new_v4().to_string();
        let mut body = serde_json::to_value(anthropic_body).unwrap_or_else(|_| json!({}));
        if let Some(model) = model_override {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("model".to_string(), json!(model));
            }
        }
        (body, session_id)
    }
}

pub struct AnthropicUpstreamRequestBuilder;

impl AnthropicUpstreamRequestBuilder {
    pub fn build_request(
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        _session_id: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        client
            .post(target_url)
            .header("Content-Type", "application/json")
            .header("x-api-key", api_key)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("x-anthropic-version", anthropic_version)
            .header("User-Agent", "Anthropic-Node/0.3.4")
            .header("Accept", "text/event-stream")
            .body(body.to_string())
    }
}

#[derive(Default)]
pub struct AnthropicPassthroughResponseTransformer {
    pending_event: Option<String>,
}

impl ResponseTransformer for AnthropicPassthroughResponseTransformer {
    fn transform_line(&mut self, line: &str) -> Vec<String> {
        let normalized = line.trim_end_matches('\r');

        if normalized.starts_with(':') {
            return vec![format!("{}\n\n", normalized)];
        }

        if let Some(event_name) = normalized.strip_prefix("event: ") {
            self.pending_event = Some(event_name.to_string());
            return Vec::new();
        }

        if normalized.starts_with("data: ") {
            let mut chunk = String::new();
            if let Some(event_name) = self.pending_event.take() {
                chunk.push_str("event: ");
                chunk.push_str(&event_name);
                chunk.push('\n');
            }
            chunk.push_str(normalized);
            chunk.push_str("\n\n");
            return vec![chunk];
        }

        Vec::new()
    }

    fn transform_event(&mut self, event: &str) -> Vec<String> {
        let normalized = event.trim_end_matches(|ch| ch == '\n' || ch == '\r');
        if normalized.trim().is_empty() {
            return Vec::new();
        }

        let mut chunk = String::new();
        for line in normalized.lines() {
            let line = line.trim_end_matches('\r');
            if line.starts_with(':') {
                return vec![format!("{}\n\n", line)];
            }
            if line.starts_with("event: ") || line.starts_with("data: ") || line.starts_with("id: ")
            {
                chunk.push_str(line);
                chunk.push('\n');
            }
        }

        if chunk.is_empty() {
            Vec::new()
        } else {
            chunk.push('\n');
            vec![chunk]
        }
    }
}
