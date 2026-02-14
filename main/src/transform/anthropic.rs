use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::{ResponseTransformer, TransformBackend, TransformContext};
use crate::models::AnthropicRequest;

pub struct AnthropicBackend;

#[derive(Default)]
pub struct AnthropicPassthroughResponseTransformer {
    pending_event: Option<String>,
}

impl ResponseTransformer for AnthropicPassthroughResponseTransformer {
    fn transform_line(&mut self, line: &str) -> Vec<String> {
        let normalized = line.trim_end_matches('\r');

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

        if normalized.starts_with(':') {
            return vec![format!("{}\n\n", normalized)];
        }

        Vec::new()
    }
}

impl TransformBackend for AnthropicBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        _log_tx: Option<&broadcast::Sender<String>>,
        _ctx: &TransformContext,
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

    fn build_upstream_request(
        &self,
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

    fn create_response_transformer(&self, _model: &str) -> Box<dyn ResponseTransformer> {
        Box::new(AnthropicPassthroughResponseTransformer::default())
    }
}

#[cfg(test)]
mod tests {
    use super::AnthropicPassthroughResponseTransformer;
    use crate::transform::ResponseTransformer;

    #[test]
    fn test_passthrough_event_data_pair() {
        let mut transformer = AnthropicPassthroughResponseTransformer::default();
        assert!(transformer.transform_line("event: message_start").is_empty());
        let chunks = transformer.transform_line("data: {\"type\":\"message_start\"}");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("event: message_start"));
        assert!(chunks[0].contains("data: {\"type\":\"message_start\"}"));
    }

    #[test]
    fn test_passthrough_data_only() {
        let mut transformer = AnthropicPassthroughResponseTransformer::default();
        let chunks = transformer.transform_line("data: {\"ok\":true}");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "data: {\"ok\":true}\n\n");
    }
}
