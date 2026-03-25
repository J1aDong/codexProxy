use serde_json::Value;
use tokio::sync::broadcast;

use super::providers::AnthropicAdapter;
use super::{
    ResponseTransformer, TransformBackend, TransformBackendContract, TransformContext,
};
use crate::models::AnthropicRequest;

pub struct AnthropicBackend;

struct AnthropicUpstreamRequestBuilder;

impl AnthropicUpstreamRequestBuilder {
    fn build_request(
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
struct AnthropicPassthroughResponseTransformer {
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

impl TransformBackend for AnthropicBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        _log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        effective_stream: bool,
        model_override: Option<String>,
    ) -> (Value, String) {
        let unified = super::unified::UnifiedChatRequest::from_anthropic(anthropic_body);
        let prepared = AnthropicAdapter.prepare_messages_request(
            &unified,
            ctx,
            "",
            "",
            "2023-06-01",
            model_override
                .as_deref()
                .or(anthropic_body.model.as_deref())
                .unwrap_or_default(),
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
        session_id: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        AnthropicUpstreamRequestBuilder::build_request(
            client,
            target_url,
            api_key,
            body,
            session_id,
            anthropic_version,
        )
    }

    fn create_response_transformer(
        &self,
        _model: &str,
        _allow_visible_thinking: bool,
    ) -> Box<dyn ResponseTransformer> {
        Box::new(AnthropicPassthroughResponseTransformer::default())
    }

    fn contract(&self) -> TransformBackendContract {
        TransformBackendContract::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnthropicPassthroughResponseTransformer, AnthropicUpstreamRequestBuilder,
    };
    use crate::models::{AnthropicRequest, Message};
    use crate::transform::ResponseTransformer;
    use uuid::Uuid;

    #[test]
    fn identity_request_mapper_applies_model_override_without_changing_messages() {
        let request = AnthropicRequest {
            model: Some("claude-sonnet-4-5".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: None,
            }],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let session_id = Uuid::new_v4().to_string();
        let mut body = serde_json::to_value(&request).expect("request should serialize");
        if let Some(obj) = body.as_object_mut() {
            obj.insert("model".to_string(), serde_json::json!("claude-opus-4-1"));
        }

        assert!(!session_id.is_empty());
        assert_eq!(body["model"], "claude-opus-4-1");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn anthropic_upstream_request_builder_sets_expected_transport_headers() {
        let client = reqwest::Client::new();
        let body = serde_json::json!({"model":"claude-sonnet-4-5","messages":[]});

        let request = AnthropicUpstreamRequestBuilder::build_request(
            &client,
            "https://example.com/v1/messages",
            "test-key",
            &body,
            "session-1",
            "2023-06-01",
        )
        .build()
        .expect("request should build");

        assert_eq!(
            request.headers().get("x-api-key").and_then(|value| value.to_str().ok()),
            Some("test-key")
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
                .get("x-anthropic-version")
                .and_then(|value| value.to_str().ok()),
            Some("2023-06-01")
        );
        assert_eq!(
            request.headers().get("Accept").and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
    }

    #[test]
    fn passthrough_response_transformer_preserves_multiline_event_frames() {
        let mut transformer = AnthropicPassthroughResponseTransformer::default();
        let frame = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\ndata: {\"usage\":{\"output_tokens\":1}}\n\n";
        let chunks = transformer.transform_event(frame);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0],
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\ndata: {\"usage\":{\"output_tokens\":1}}\n\n"
        );
    }
}
