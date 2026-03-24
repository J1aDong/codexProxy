use serde_json::Value;
use tokio::sync::broadcast;

use super::baseline::{
    AnthropicPassthroughResponseTransformer, AnthropicUpstreamRequestBuilder,
    IdentityAnthropicRequestMapper,
};
use super::{
    ResponseTransformer, TransformBackend, TransformBackendContract, TransformContext,
};
use crate::models::AnthropicRequest;

pub struct AnthropicBackend;

impl TransformBackend for AnthropicBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        _log_tx: Option<&broadcast::Sender<String>>,
        _ctx: &TransformContext,
        _effective_stream: bool,
        model_override: Option<String>,
    ) -> (Value, String) {
        IdentityAnthropicRequestMapper::map_request(anthropic_body, model_override)
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
    use super::super::baseline::{
        AnthropicPassthroughResponseTransformer, AnthropicUpstreamRequestBuilder,
        IdentityAnthropicRequestMapper,
    };
    use crate::models::{AnthropicRequest, Message};
    use crate::transform::ResponseTransformer;

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

        let (body, session_id) = IdentityAnthropicRequestMapper::map_request(
            &request,
            Some("claude-opus-4-1".to_string()),
        );

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
