pub mod anthropic;
pub mod codex;
pub mod gemini;
pub mod openai;
pub mod providers;
pub mod unified;
#[cfg(test)]
mod processor;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    pub session_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountTokensMode {
    Native,
    Estimate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCountTokensRequest {
    pub mode: CountTokensMode,
    pub request: Option<PreparedRequest>,
}

impl PreparedCountTokensRequest {
    pub fn native(request: PreparedRequest) -> Self {
        Self {
            mode: CountTokensMode::Native,
            request: Some(request),
        }
    }

    pub fn estimate() -> Self {
        Self {
            mode: CountTokensMode::Estimate,
            request: None,
        }
    }
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
pub use providers::{AnthropicAdapter, CodexAdapter, GeminiAdapter, OpenAIChatAdapter};
pub use unified::UnifiedChatRequest;

#[cfg(test)]
mod tests {
    use super::{
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
    fn unified_chat_request_converts_core_anthropic_blocks() {
        use crate::models::{
            ContentBlock, ImageUrlValue, Message, MessageContent, RequestThinkingConfig,
        };

        let request = AnthropicRequest {
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: Some(MessageContent::Blocks(vec![
                        ContentBlock::Text {
                            text: "hello".to_string(),
                        },
                        ContentBlock::InputImage {
                            image_url: Some(ImageUrlValue::Str(
                                "data:image/png;base64,abc".to_string(),
                            )),
                            url: None,
                            detail: Some("auto".to_string()),
                        },
                    ])),
                },
                Message {
                    role: "assistant".to_string(),
                    content: Some(MessageContent::Blocks(vec![
                        ContentBlock::Thinking {
                            thinking: "reason step".to_string(),
                            signature: Some("sig-1".to_string()),
                        },
                        ContentBlock::Text {
                            text: "working".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: Some("toolu_1".to_string()),
                            name: "search".to_string(),
                            input: json!({"q":"rust"}),
                            signature: None,
                        },
                    ])),
                },
                Message {
                    role: "user".to_string(),
                    content: Some(MessageContent::Blocks(vec![
                        ContentBlock::ToolResult {
                            tool_use_id: Some("toolu_1".to_string()),
                            id: None,
                            content: Some(json!({"text":"done"})),
                        },
                    ])),
                },
            ],
            system: Some(SystemContent::Blocks(vec![
                SystemBlock::Text {
                    text: "sys-a".to_string(),
                },
                SystemBlock::PlainString("sys-b".to_string()),
            ])),
            tools: Some(vec![json!({
                "name": "search",
                "description": "Search docs",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "q": { "type": "string" }
                    },
                    "required": ["q"]
                }
            })]),
            metadata: None,
            tool_choice: Some(json!({"type":"tool","name":"search"})),
            thinking: Some(RequestThinkingConfig {
                kind: Some("enabled".to_string()),
                extra: std::collections::HashMap::from([(
                    "budget_tokens".to_string(),
                    json!(2048),
                )]),
            }),
            stream: true,
            max_tokens: Some(4096),
            temperature: Some(0.2),
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let unified = crate::transform::unified::UnifiedChatRequest::from_anthropic(&request);

        assert_eq!(unified.model, "claude-3-5-sonnet-20241022");
        assert_eq!(unified.messages.len(), 4);
        assert_eq!(unified.messages[0].role.as_str(), "system");
        assert_eq!(
            unified.messages[0].content_text().as_deref(),
            Some("sys-a\nsys-b")
        );
        assert_eq!(unified.messages[1].role.as_str(), "user");
        assert_eq!(unified.messages[1].content_items().len(), 2);
        assert_eq!(unified.messages[2].role.as_str(), "assistant");
        assert_eq!(unified.messages[2].tool_calls.len(), 1);
        assert_eq!(unified.messages[2].tool_calls[0].function.name, "search");
        assert_eq!(
            unified.messages[2]
                .thinking
                .as_ref()
                .and_then(|thinking| thinking.signature.as_deref()),
            Some("sig-1")
        );
        assert_eq!(unified.messages[3].role.as_str(), "tool");
        assert_eq!(unified.messages[3].tool_call_id.as_deref(), Some("toolu_1"));
        assert_eq!(unified.tools.as_ref().map(|tools| tools.len()), Some(1));
        assert_eq!(
            unified.tool_choice.as_ref().map(|choice| choice.kind()),
            Some("function")
        );
        assert_eq!(
            unified.reasoning.as_ref().map(|reasoning| reasoning.enabled),
            Some(true)
        );
    }

    #[test]
    fn provider_adapters_prepare_requests_from_unified_chat_request() {
        use crate::models::{Message, MessageContent};

        let request = AnthropicRequest {
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("hello".to_string())),
            }],
            system: Some(SystemContent::Text("system prompt".to_string())),
            tools: Some(vec![json!({
                "name": "lookup",
                "description": "Lookup",
                "input_schema": {"type":"object","properties":{"id":{"type":"string"}}}
            })]),
            metadata: None,
            tool_choice: Some(json!({"type":"auto"})),
            thinking: None,
            stream: true,
            max_tokens: Some(1024),
            temperature: Some(0.3),
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let unified = crate::transform::unified::UnifiedChatRequest::from_anthropic(&request);
        let ctx = crate::transform::TransformContext {
            reasoning_mapping: crate::models::ReasoningEffortMapping::default(),
            codex_model_mapping: crate::models::CodexModelMapping::default(),
            anthropic_model_mapping: crate::models::AnthropicModelMapping::default(),
            openai_model_mapping: crate::models::OpenAIModelMapping::default(),
            openai_max_tokens_mapping: crate::models::OpenAIMaxTokensMapping::default(),
            custom_injection_prompt: "unused".to_string(),
            converter: "codex".to_string(),
            codex_model: "gpt-5.3-codex".to_string(),
            gemini_reasoning_effort: crate::models::GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let codex = crate::transform::providers::CodexAdapter;
        let codex_req = codex.prepare_messages_request(
            &unified,
            &ctx,
            "https://example.com/v1/responses",
            "test-key",
            "2023-06-01",
            "gpt-5.3-codex",
            true,
        );
        assert_eq!(codex_req.url, "https://example.com/v1/responses");
        assert_eq!(codex_req.body["model"], "gpt-5.3-codex");
        assert!(codex_req.body["input"].is_array());

        let openai = crate::transform::providers::OpenAIChatAdapter;
        let openai_req = openai.prepare_messages_request(
            &unified,
            &ctx,
            "https://api.openai.com/v1/chat/completions",
            "test-key",
            "2023-06-01",
            "gpt-4o-mini",
            true,
        );
        assert_eq!(openai_req.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(openai_req.body["model"], "gpt-4o-mini");
        assert!(openai_req.body["messages"].is_array());

        let gemini = crate::transform::providers::GeminiAdapter;
        let gemini_req = gemini.prepare_messages_request(
            &unified,
            &ctx,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse",
            "test-key",
            "2023-06-01",
            "gemini-2.0-flash",
            true,
        );
        assert!(gemini_req.body["contents"].is_array());
        assert_eq!(
            gemini_req.url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );

        let anthropic = crate::transform::providers::AnthropicAdapter;
        let anthropic_req = anthropic.prepare_messages_request(
            &unified,
            &ctx,
            "https://api.anthropic.com/v1/messages",
            "test-key",
            "2023-06-01",
            "claude-3-7-sonnet-latest",
            true,
        );
        assert_eq!(anthropic_req.body["model"], "claude-3-7-sonnet-latest");
        assert!(anthropic_req.body["messages"].is_array());
    }

    #[test]
    fn openai_count_tokens_preparation_uses_estimate_mode() {
        use crate::models::{Message, MessageContent};

        let request = AnthropicRequest {
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text("hello world".to_string())),
            }],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: false,
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let unified = crate::transform::unified::UnifiedChatRequest::from_anthropic(&request);
        let ctx = crate::transform::TransformContext {
            reasoning_mapping: crate::models::ReasoningEffortMapping::default(),
            codex_model_mapping: crate::models::CodexModelMapping::default(),
            anthropic_model_mapping: crate::models::AnthropicModelMapping::default(),
            openai_model_mapping: crate::models::OpenAIModelMapping::default(),
            openai_max_tokens_mapping: crate::models::OpenAIMaxTokensMapping::default(),
            custom_injection_prompt: String::new(),
            converter: "openai".to_string(),
            codex_model: "gpt-5.3-codex".to_string(),
            gemini_reasoning_effort: crate::models::GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let mode = crate::transform::providers::OpenAIChatAdapter.prepare_count_tokens_request(
            &unified,
            &ctx,
            "https://api.openai.com/v1/chat/completions",
            "test-key",
            "2023-06-01",
            "gpt-4o-mini",
        );

        assert!(matches!(
            mode.mode,
            crate::transform::CountTokensMode::Estimate
        ));
    }

    #[test]
    fn unified_request_preserves_local_image_paths_for_codex() {
        use crate::models::{ContentBlock, ImageSource, Message, MessageContent};

        let request = AnthropicRequest {
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Blocks(vec![ContentBlock::Image {
                    source: Some(ImageSource {
                        source_type: Some("file".to_string()),
                        media_type: Some("image/png".to_string()),
                        mime_type: None,
                        data: None,
                        url: None,
                        uri: None,
                        path: Some("/tmp/screenshot.png".to_string()),
                    }),
                    source_raw: None,
                    image_url: None,
                }])),
            }],
            system: None,
            tools: None,
            metadata: None,
            tool_choice: None,
            thinking: None,
            stream: true,
            max_tokens: Some(256),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let unified = crate::transform::unified::UnifiedChatRequest::from_anthropic(&request);
        let user = unified
            .messages
            .iter()
            .find(|message| message.role.as_str() == "user")
            .expect("user message");
        let image = user
            .content_items()
            .iter()
            .find_map(|item| match item {
                crate::transform::unified::UnifiedContent::ImageUrl { url, .. } => Some(url),
                _ => None,
            })
            .expect("image item");

        assert_eq!(image, "file:///tmp/screenshot.png");
    }

    #[test]
    fn codex_adapter_restores_prompt_cache_key_and_default_store_flag() {
        use crate::models::{Message, MessageContent};

        let request = AnthropicRequest {
            model: Some("claude-sonnet-4-5".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text(
                    "<environment_context><cwd>/Users/mr.j/project</cwd></environment_context>\nhello".to_string(),
                )),
            }],
            system: Some(SystemContent::Text("You are Claude Code.".to_string())),
            tools: Some(vec![json!({
                "name": "Read",
                "description": "Read files",
                "input_schema": {"type":"object","properties":{"file_path":{"type":"string"}}}
            })]),
            metadata: Some(json!({"user_id": "acct__session_abc-123"})),
            tool_choice: Some(json!({"type":"auto"})),
            thinking: None,
            stream: true,
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            top_k: None,
            stop_sequences: None,
        };

        let unified_a = crate::transform::unified::UnifiedChatRequest::from_anthropic(&request);
        let unified_b = crate::transform::unified::UnifiedChatRequest::from_anthropic(&request);
        let hints = crate::transform::providers::codex_request_hints_from_anthropic(&request);
        let ctx = crate::transform::TransformContext {
            reasoning_mapping: crate::models::ReasoningEffortMapping::default(),
            codex_model_mapping: crate::models::CodexModelMapping::default(),
            anthropic_model_mapping: crate::models::AnthropicModelMapping::default(),
            openai_model_mapping: crate::models::OpenAIModelMapping::default(),
            openai_max_tokens_mapping: crate::models::OpenAIMaxTokensMapping::default(),
            custom_injection_prompt: String::new(),
            converter: "codex".to_string(),
            codex_model: "gpt-5.3-codex".to_string(),
            gemini_reasoning_effort: crate::models::GeminiReasoningEffortMapping::default(),
            enable_codex_tool_schema_compaction: false,
            enable_codex_fast_mode: false,
            enable_skill_routing_hint: false,
        };

        let adapter = crate::transform::providers::CodexAdapter;
        let prepared_a = adapter.prepare_messages_request_with_hints(
            &unified_a,
            &ctx,
            "https://api.openai.com/v1/responses",
            "test-key",
            "2023-06-01",
            "gpt-5.3-codex",
            true,
            &hints,
        );
        let prepared_b = adapter.prepare_messages_request_with_hints(
            &unified_b,
            &ctx,
            "https://api.openai.com/v1/responses",
            "test-key",
            "2023-06-01",
            "gpt-5.3-codex",
            true,
            &hints,
        );

        let key_a = prepared_a
            .body
            .get("prompt_cache_key")
            .and_then(|value| value.as_str())
            .expect("prompt_cache_key");
        let key_b = prepared_b
            .body
            .get("prompt_cache_key")
            .and_then(|value| value.as_str())
            .expect("prompt_cache_key");

        assert_eq!(prepared_a.body.get("store"), Some(&json!(false)));
        assert_eq!(key_a, key_b);
        assert!(
            key_a.contains("session"),
            "session-aware codex requests should restore stable cache keys"
        );
    }
}
