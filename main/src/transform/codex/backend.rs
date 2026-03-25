use serde_json::Value;
use tokio::sync::broadcast;

use super::response::TransformResponse;
use crate::models::AnthropicRequest;
use crate::transform::{
    providers::CodexAdapter, request_envelope_hints_from_anthropic, ResponseTransformer,
    TransformBackend, TransformContext,
};

/// Codex 后端 —— 将 Anthropic 请求转为 Codex Responses API 格式
pub struct CodexBackend;

struct CodexUpstreamRequestBuilder;

impl CodexUpstreamRequestBuilder {
    fn apply_standard_headers(
        builder: reqwest::RequestBuilder,
        api_key: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        builder
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("x-api-key", api_key)
            .header("User-Agent", "Anthropic-Node/0.3.4")
            .header("x-anthropic-version", anthropic_version)
            .header("originator", "codex_cli_rs")
            .header("Accept", "text/event-stream")
    }

    fn apply_session_headers(
        builder: reqwest::RequestBuilder,
        session_id: &str,
    ) -> reqwest::RequestBuilder {
        builder
            .header("conversation_id", session_id)
            .header("session_id", session_id)
    }

    fn build_request(
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        session_id: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        let builder = client.post(target_url);
        let builder = Self::apply_standard_headers(builder, api_key, anthropic_version);
        let builder = Self::apply_session_headers(builder, session_id);
        builder.body(body.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::CodexUpstreamRequestBuilder;
    use serde_json::json;

    #[test]
    fn codex_upstream_request_builder_sets_transport_headers_and_session_ids() {
        let client = reqwest::Client::new();
        let body = json!({"model": "gpt-5.3-codex", "input": [], "stream": true});

        let request = CodexUpstreamRequestBuilder::build_request(
            &client,
            "https://example.com/v1/responses",
            "test-key",
            &body,
            "session-123",
            "2023-06-01",
        )
        .build()
        .expect("request should build");

        assert_eq!(request.url().as_str(), "https://example.com/v1/responses");
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
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("test-key")
        );
        assert_eq!(
            request
                .headers()
                .get("Accept")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        assert_eq!(
            request
                .headers()
                .get("conversation_id")
                .and_then(|value| value.to_str().ok()),
            Some("session-123")
        );
        assert_eq!(
            request
                .headers()
                .get("session_id")
                .and_then(|value| value.to_str().ok()),
            Some("session-123")
        );
    }
}

impl TransformBackend for CodexBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        _log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        effective_stream: bool,
        model_override: Option<String>,
    ) -> (Value, String) {
        let unified = crate::transform::unified::UnifiedChatRequest::from_anthropic(anthropic_body);
        let hints = request_envelope_hints_from_anthropic(anthropic_body);
        let prepared = CodexAdapter.prepare_messages_request_with_hints(
            &unified,
            ctx,
            "",
            "",
            "2023-06-01",
            model_override.as_deref().unwrap_or(&ctx.codex_model),
            effective_stream,
            &hints,
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
        CodexUpstreamRequestBuilder::build_request(
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
        model: &str,
        allow_visible_thinking: bool,
    ) -> Box<dyn ResponseTransformer> {
        Box::new(TransformResponse::new_with_visible_thinking(
            model,
            allow_visible_thinking,
        ))
    }
}
