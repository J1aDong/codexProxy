pub mod anthropic;
pub mod codex;
pub mod gemini;
pub mod processor;

use serde_json::Value;
use tokio::sync::broadcast;

use crate::models::{AnthropicModelMapping, AnthropicRequest, ReasoningEffortMapping, GeminiReasoningEffortMapping, CodexModelMapping};

/// 转换上下文 —— 从 ProxyServer 配置派生，传入 transform 方法
#[derive(Clone)]
pub struct TransformContext {
    pub reasoning_mapping: ReasoningEffortMapping,
    pub codex_model_mapping: CodexModelMapping,
    pub anthropic_model_mapping: AnthropicModelMapping,
    pub skill_injection_prompt: String,
    pub converter: String,
    pub codex_model: String,
    pub gemini_reasoning_effort: GeminiReasoningEffortMapping,
}

/// 协议转换后端 —— 每种上游 API 实现一份
///
/// 职责：
/// 1. 将 Anthropic 请求转为上游格式（请求体）
/// 2. 构建发送给上游的 HTTP 请求
/// 3. 创建响应转换器将上游 SSE 转回 Anthropic SSE
pub trait TransformBackend: Send + Sync {
    /// 将 Anthropic 请求体转换为上游请求的 JSON body + session_id
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        model_override: Option<String>,
    ) -> (Value, String);

    /// 构建发送给上游的 reqwest::RequestBuilder
    fn build_upstream_request(
        &self,
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        session_id: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder;

    /// 创建响应转换器（有状态，每个请求一个实例）
    fn create_response_transformer(&self, model: &str) -> Box<dyn ResponseTransformer>;
}

/// 响应转换器 trait —— 有状态，逐行处理 SSE
pub trait ResponseTransformer: Send {
    /// 将上游的一行 SSE 转换为 Anthropic 格式的多行输出
    fn transform_line(&mut self, line: &str) -> Vec<String>;
}

// Re-export backends
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use anthropic::AnthropicBackend;
pub use processor::MessageProcessor;
