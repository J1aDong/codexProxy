pub mod anthropic;
pub mod baseline;
pub mod codex;
pub mod gemini;
pub mod openai;
pub mod processor;
pub mod shared;
pub mod tool_router;

use serde_json::Value;
use tokio::sync::broadcast;

use crate::models::{
    AnthropicModelMapping, AnthropicRequest, CodexModelMapping, GeminiReasoningEffortMapping,
    OpenAIMaxTokensMapping, OpenAIModelMapping, ReasoningEffortMapping,
};

#[derive(Clone, Debug, Default)]
pub struct ResponseTransformRequestContext {
    pub codex_plan_file_path: Option<String>,
    pub contains_background_agent_completion: bool,
    pub historical_background_agent_launch_count: usize,
    pub terminal_background_agent_completion_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalTransformModel {
    AnthropicMessages,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendBehavior {
    Identity,
    Override,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransformBackendContract {
    pub canonical_model: CanonicalTransformModel,
    pub backend_behavior: BackendBehavior,
    pub preserves_canonical_sse: bool,
}

impl TransformBackendContract {
    pub const fn identity() -> Self {
        Self {
            canonical_model: CanonicalTransformModel::AnthropicMessages,
            backend_behavior: BackendBehavior::Identity,
            preserves_canonical_sse: true,
        }
    }

    pub const fn provider_override() -> Self {
        Self {
            canonical_model: CanonicalTransformModel::AnthropicMessages,
            backend_behavior: BackendBehavior::Override,
            preserves_canonical_sse: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedToolInvocation {
    pub tool_name: String,
    pub call_id: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalToolResult {
    pub tool_use_id: String,
    pub content: Value,
    pub is_error: bool,
}

/// 转换上下文 —— 从 ProxyServer 配置派生，传入 transform 方法
#[derive(Clone)]
pub struct TransformContext {
    pub reasoning_mapping: ReasoningEffortMapping,
    pub codex_model_mapping: CodexModelMapping,
    pub anthropic_model_mapping: AnthropicModelMapping,
    pub openai_model_mapping: OpenAIModelMapping,
    pub openai_max_tokens_mapping: OpenAIMaxTokensMapping,
    pub custom_injection_prompt: String,
    pub converter: String,
    pub codex_model: String,
    pub gemini_reasoning_effort: GeminiReasoningEffortMapping,
    pub enable_codex_tool_schema_compaction: bool,
    pub enable_codex_fast_mode: bool,
    pub enable_skill_routing_hint: bool,
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
        effective_stream: bool,
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
    fn create_response_transformer(
        &self,
        model: &str,
        allow_visible_thinking: bool,
    ) -> Box<dyn ResponseTransformer>;

    /// 描述当前 backend 在 canonical transformer 架构中的角色。
    fn contract(&self) -> TransformBackendContract {
        TransformBackendContract::provider_override()
    }
}

/// 响应转换器 trait —— 有状态，逐行处理 SSE
pub trait ResponseTransformer: Send {
    /// 将上游的一行 SSE 转换为 Anthropic 格式的多行输出
    fn transform_line(&mut self, line: &str) -> Vec<String>;

    /// 注入当前请求相关的附加上下文（默认忽略）
    fn configure_request_context(&mut self, _ctx: &ResponseTransformRequestContext) {}

    /// 导出转换器诊断摘要（可选）
    fn take_diagnostics_summary(&mut self) -> Option<Value> {
        None
    }

    /// 导出本轮标准化 tool invocation（默认无拦截导出）。
    fn take_normalized_tool_invocations(&mut self) -> Vec<NormalizedToolInvocation> {
        Vec::new()
    }

    /// 导出本轮 canonical tool result（默认无拦截导出）。
    fn take_canonical_tool_results(&mut self) -> Vec<CanonicalToolResult> {
        Vec::new()
    }

    /// 将上游一个完整 SSE 事件帧转换为 Anthropic 格式输出
    /// 默认实现兼容旧逻辑：按行回退到 transform_line。
    fn transform_event(&mut self, event: &str) -> Vec<String> {
        let mut output = Vec::new();
        for line in event.lines() {
            let normalized = line.trim_end_matches('\r');
            if normalized.trim().is_empty() {
                continue;
            }
            output.extend(self.transform_line(normalized));
        }
        output
    }
}

// Re-export backends
pub use anthropic::AnthropicBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use openai::OpenAIChatBackend;
pub use processor::MessageProcessor;

#[cfg(test)]
mod tests {
    use super::{
        tool_router::{RequestScopedToolRouter, ToolInterceptionAction},
        AnthropicBackend, BackendBehavior, CanonicalTransformModel, CodexBackend, GeminiBackend,
        OpenAIChatBackend, ResponseTransformer, TransformBackend,
    };
    use crate::models::{AnthropicRequest, SystemBlock, SystemContent};
    use serde_json::json;

    struct DummyTransformer;

    impl ResponseTransformer for DummyTransformer {
        fn transform_line(&mut self, _line: &str) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn backend_contracts_describe_identity_vs_override_roles() {
        let anthropic_contract = AnthropicBackend.contract();
        assert_eq!(
            anthropic_contract.canonical_model,
            CanonicalTransformModel::AnthropicMessages
        );
        assert_eq!(anthropic_contract.backend_behavior, BackendBehavior::Identity);
        assert!(anthropic_contract.preserves_canonical_sse);

        for contract in [
            CodexBackend.contract(),
            OpenAIChatBackend.contract(),
            GeminiBackend.contract(),
        ] {
            assert_eq!(
                contract.canonical_model,
                CanonicalTransformModel::AnthropicMessages
            );
            assert_eq!(contract.backend_behavior, BackendBehavior::Override);
            assert!(!contract.preserves_canonical_sse);
        }
    }

    #[test]
    fn response_transformer_default_interception_exports_are_empty() {
        let mut transformer = DummyTransformer;
        assert!(transformer.take_normalized_tool_invocations().is_empty());
        assert!(transformer.take_canonical_tool_results().is_empty());
    }

    #[test]
    fn shared_helpers_flatten_system_text_and_resolve_parallel_tool_calls() {
        let system = SystemContent::Blocks(vec![
            SystemBlock::Text {
                text: "alpha".to_string(),
            },
            SystemBlock::PlainString("beta".to_string()),
        ]);
        assert_eq!(
            super::shared::flatten_system_text(Some(&system)).as_deref(),
            Some("alpha\nbeta")
        );

        let request = AnthropicRequest {
            model: Some("claude-sonnet-4-5".to_string()),
            messages: vec![],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: Some(json!({
                "type": "auto",
                "disable_parallel_tool_use": true
            })),
            thinking: None,
            stream: true,
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };
        assert!(!super::shared::resolve_parallel_tool_calls(&request));
    }

    #[test]
    fn request_scoped_tool_router_intercepts_web_search_variants_and_passthroughs_unknown_tools() {
        let router = RequestScopedToolRouter::new(["web_search"]);

        for tool_name in ["WebSearch", "websearch", "web_search"] {
            let intercepted = router.decide_by_name(tool_name);
            assert_eq!(intercepted.action, ToolInterceptionAction::Intercept);
            assert_eq!(intercepted.normalized_tool_name.as_deref(), Some("web_search"));
        }

        let passthrough = router.decide_by_name("Read");
        assert_eq!(passthrough.action, ToolInterceptionAction::PassThrough);
        assert!(passthrough.normalized_tool_name.is_none());
    }
}
