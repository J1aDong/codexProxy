use serde_json::Value;
use tokio::sync::broadcast;

use super::request::TransformRequest;
use super::response::TransformResponse;
use crate::models::AnthropicRequest;
use crate::transform::{ResponseTransformer, TransformBackend, TransformContext};

/// Codex 后端 —— 将 Anthropic 请求转为 Codex Responses API 格式
pub struct CodexBackend;

impl TransformBackend for CodexBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        model_override: Option<String>,
    ) -> (Value, String) {
        TransformRequest::transform_with_options(
            anthropic_body,
            log_tx,
            &ctx.reasoning_mapping,
            &ctx.custom_injection_prompt,
            model_override.as_deref().unwrap_or(&ctx.codex_model),
            ctx.enable_codex_tool_schema_compaction,
            ctx.enable_skill_routing_hint,
        )
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
        client
            .post(target_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("x-api-key", api_key)
            .header("User-Agent", "Anthropic-Node/0.3.4")
            .header("x-anthropic-version", anthropic_version)
            .header("originator", "codex_cli_rs")
            .header("Accept", "text/event-stream")
            .header("conversation_id", session_id)
            .header("session_id", session_id)
            .body(body.to_string())
    }

    fn create_response_transformer(&self, model: &str) -> Box<dyn ResponseTransformer> {
        Box::new(TransformResponse::new(model))
    }
}
