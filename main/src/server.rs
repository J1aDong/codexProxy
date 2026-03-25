use crate::load_balancer::{
    EndpointPermit, LoadBalancerRuntime, ModelSlot, ResolvedEndpoint, UpstreamOutcomeAction,
};
use crate::logger::AppLogger;
use crate::models::{
    AnthropicModelMapping, AnthropicRequest, CodexModelMapping, ContentBlock,
    GeminiReasoningEffortMapping, Message, MessageContent, OpenAIMaxTokensMapping,
    OpenAIModelMapping, ReasoningEffort, ReasoningEffortMapping,
};
use crate::transform::{
    AnthropicAdapter, AnthropicBackend, CodexAdapter, CodexBackend, CountTokensMode, GeminiAdapter,
    GeminiBackend, OpenAIChatAdapter, OpenAIChatBackend, PreparedRequest,
    ResponseTransformRequestContext, TransformBackend, TransformContext, UnifiedChatRequest,
};
use crate::transform::providers::codex_request_hints_from_anthropic;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

mod stream_decision;
use stream_decision::{OutputDisposition, StreamDecisionState};

pub struct ProxyServer {
    port: u16,
    allow_external_access: bool,
    target_url: String,
    api_key: Option<String>,
    reasoning_mapping: ReasoningEffortMapping,
    custom_injection_prompt: String,
    converter: String,
    codex_model: String,
    codex_model_mapping: CodexModelMapping,
    anthropic_model_mapping: AnthropicModelMapping,
    openai_model_mapping: OpenAIModelMapping,
    openai_max_tokens_mapping: OpenAIMaxTokensMapping,
    gemini_reasoning_effort: GeminiReasoningEffortMapping,
    max_concurrency: u32,
    ignore_probe_requests: bool,
    allow_count_tokens_fallback_estimate: bool,
    enable_codex_fast_mode: bool,
    force_stream_for_codex: bool,
    enable_sse_frame_parser: bool,
    enable_stream_heartbeat: bool,
    stream_heartbeat_interval_ms: u64,
    enable_stream_log_sampling: bool,
    stream_log_sample_every_n: u32,
    stream_log_max_chars: usize,
    enable_stream_metrics: bool,
    enable_stream_event_metrics: bool,
    stream_silence_warn_ms: u64,
    stream_silence_error_ms: u64,
    enable_stall_retry: bool,
    stall_timeout_ms: u64,
    stall_retry_max_attempts: u32,
    stall_retry_only_heartbeat_phase: bool,
    enable_empty_completion_retry: bool,
    empty_completion_retry_max_attempts: u32,
    enable_incomplete_stream_retry: bool,
    incomplete_stream_retry_max_attempts: u32,
    enable_sibling_tool_error_retry: bool,
    prefer_codex_v1_path: bool,
    enable_codex_tool_schema_compaction: bool,
    enable_skill_routing_hint: bool,
    enable_stateful_responses_chain: bool,
    load_balancer_runtime: Option<LoadBalancerRuntime>,
}

#[derive(Clone)]
pub struct RuntimeConfigUpdate {
    pub target_url: String,
    pub api_key: Option<String>,
    pub ctx: TransformContext,
    pub ignore_probe_requests: bool,
    pub allow_count_tokens_fallback_estimate: bool,
    pub enable_codex_fast_mode: bool,
    pub force_stream_for_codex: bool,
    pub enable_sse_frame_parser: bool,
    pub enable_stream_heartbeat: bool,
    pub stream_heartbeat_interval_ms: u64,
    pub enable_stream_log_sampling: bool,
    pub stream_log_sample_every_n: u32,
    pub stream_log_max_chars: usize,
    pub enable_stream_metrics: bool,
    pub enable_stream_event_metrics: bool,
    pub stream_silence_warn_ms: u64,
    pub stream_silence_error_ms: u64,
    pub enable_stall_retry: bool,
    pub stall_timeout_ms: u64,
    pub stall_retry_max_attempts: u32,
    pub stall_retry_only_heartbeat_phase: bool,
    pub enable_empty_completion_retry: bool,
    pub empty_completion_retry_max_attempts: u32,
    pub enable_incomplete_stream_retry: bool,
    pub incomplete_stream_retry_max_attempts: u32,
    pub enable_sibling_tool_error_retry: bool,
    pub prefer_codex_v1_path: bool,
    pub enable_codex_tool_schema_compaction: bool,
    pub enable_skill_routing_hint: bool,
    pub enable_stateful_responses_chain: bool,
    pub load_balancer_runtime: Option<LoadBalancerRuntime>,
}

#[derive(Clone)]
struct RuntimeConfigState {
    target_url: String,
    api_key: Option<String>,
    ctx: TransformContext,
    ignore_probe_requests: bool,
    allow_count_tokens_fallback_estimate: bool,
    enable_codex_fast_mode: bool,
    force_stream_for_codex: bool,
    enable_sse_frame_parser: bool,
    enable_stream_heartbeat: bool,
    stream_heartbeat_interval_ms: u64,
    enable_stream_log_sampling: bool,
    stream_log_sample_every_n: u32,
    stream_log_max_chars: usize,
    enable_stream_metrics: bool,
    enable_stream_event_metrics: bool,
    stream_silence_warn_ms: u64,
    stream_silence_error_ms: u64,
    stall_timeout_ms: u64,
    enable_incomplete_stream_retry: bool,
    incomplete_stream_retry_max_attempts: u32,
    enable_sibling_tool_error_retry: bool,
    prefer_codex_v1_path: bool,
    enable_codex_tool_schema_compaction: bool,
    enable_skill_routing_hint: bool,
    enable_stateful_responses_chain: bool,
    load_balancer_runtime: Option<LoadBalancerRuntime>,
}

impl From<RuntimeConfigUpdate> for RuntimeConfigState {
    fn from(value: RuntimeConfigUpdate) -> Self {
        let mut ctx = value.ctx;
        ctx.enable_codex_tool_schema_compaction = value.enable_codex_tool_schema_compaction;
        ctx.enable_codex_fast_mode = value.enable_codex_fast_mode;
        ctx.enable_skill_routing_hint = value.enable_skill_routing_hint;
        Self {
            target_url: value.target_url,
            api_key: value.api_key,
            ctx,
            ignore_probe_requests: value.ignore_probe_requests,
            allow_count_tokens_fallback_estimate: value.allow_count_tokens_fallback_estimate,
            enable_codex_fast_mode: value.enable_codex_fast_mode,
            force_stream_for_codex: value.force_stream_for_codex,
            enable_sse_frame_parser: value.enable_sse_frame_parser,
            enable_stream_heartbeat: value.enable_stream_heartbeat,
            stream_heartbeat_interval_ms: value.stream_heartbeat_interval_ms,
            enable_stream_log_sampling: value.enable_stream_log_sampling,
            stream_log_sample_every_n: value.stream_log_sample_every_n,
            stream_log_max_chars: value.stream_log_max_chars,
            enable_stream_metrics: value.enable_stream_metrics,
            enable_stream_event_metrics: value.enable_stream_event_metrics,
            stream_silence_warn_ms: value.stream_silence_warn_ms,
            stream_silence_error_ms: value.stream_silence_error_ms,
            stall_timeout_ms: value.stall_timeout_ms,
            enable_incomplete_stream_retry: value.enable_incomplete_stream_retry,
            incomplete_stream_retry_max_attempts: value.incomplete_stream_retry_max_attempts,
            enable_sibling_tool_error_retry: value.enable_sibling_tool_error_retry,
            prefer_codex_v1_path: value.prefer_codex_v1_path,
            enable_codex_tool_schema_compaction: value.enable_codex_tool_schema_compaction,
            enable_skill_routing_hint: value.enable_skill_routing_hint,
            enable_stateful_responses_chain: value.enable_stateful_responses_chain,
            load_balancer_runtime: value.load_balancer_runtime,
        }
    }
}

#[derive(Clone)]
pub struct ProxyRuntimeHandle {
    state: Arc<RwLock<RuntimeConfigState>>,
}

impl ProxyRuntimeHandle {
    pub fn apply_update(&self, update: RuntimeConfigUpdate) {
        let next = RuntimeConfigState::from(update);
        match self.state.write() {
            Ok(mut guard) => {
                *guard = next;
            }
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = next;
            }
        }
    }

    fn snapshot(&self) -> RuntimeConfigState {
        match self.state.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

fn detect_model_family(model: &str) -> Option<&'static str> {
    let lower = model.to_ascii_lowercase();
    if lower.contains("opus") {
        Some("opus")
    } else if lower.contains("sonnet") {
        Some("sonnet")
    } else if lower.contains("haiku") {
        Some("haiku")
    } else {
        None
    }
}

fn build_backend_by_converter(converter: &str) -> Arc<dyn TransformBackend> {
    if converter.eq_ignore_ascii_case("gemini") {
        Arc::new(GeminiBackend)
    } else if converter.eq_ignore_ascii_case("anthropic") {
        Arc::new(AnthropicBackend)
    } else if converter.eq_ignore_ascii_case("openai") {
        Arc::new(OpenAIChatBackend)
    } else {
        Arc::new(CodexBackend)
    }
}

fn backend_label_by_converter(converter: &str) -> &'static str {
    if converter.eq_ignore_ascii_case("gemini") {
        "Gemini API"
    } else if converter.eq_ignore_ascii_case("anthropic") {
        "Anthropic API"
    } else if converter.eq_ignore_ascii_case("openai") {
        "OpenAI API"
    } else {
        "Codex API"
    }
}

fn resolve_model_for_converter(
    converter: &str,
    input_model: &str,
    reasoning_mapping: &ReasoningEffortMapping,
    codex_model_mapping: &CodexModelMapping,
    anthropic_model_mapping: &AnthropicModelMapping,
    openai_model_mapping: &OpenAIModelMapping,
    gemini_reasoning_effort: &GeminiReasoningEffortMapping,
) -> String {
    if converter.eq_ignore_ascii_case("anthropic") {
        if let Some(family) = detect_model_family(input_model) {
            let model = match family {
                "opus" => anthropic_model_mapping.opus.trim(),
                "sonnet" => anthropic_model_mapping.sonnet.trim(),
                "haiku" => anthropic_model_mapping.haiku.trim(),
                _ => "",
            };
            if !model.is_empty() {
                return model.to_string();
            }
        } else {
            let effort = crate::models::get_reasoning_effort(input_model, reasoning_mapping);
            let model = match effort {
                ReasoningEffort::Xhigh => anthropic_model_mapping.opus.trim(),
                ReasoningEffort::High | ReasoningEffort::Medium => {
                    anthropic_model_mapping.sonnet.trim()
                }
                ReasoningEffort::Low => anthropic_model_mapping.haiku.trim(),
            };
            if !model.is_empty() {
                return model.to_string();
            }
        }
        return input_model.to_string();
    }

    if converter.eq_ignore_ascii_case("openai") {
        if let Some(family) = detect_model_family(input_model) {
            let model = match family {
                "opus" => openai_model_mapping.opus.trim(),
                "sonnet" => openai_model_mapping.sonnet.trim(),
                "haiku" => openai_model_mapping.haiku.trim(),
                _ => "",
            };
            if !model.is_empty() {
                return model.to_string();
            }
        }
        return input_model.to_string();
    }

    let effort = crate::models::get_reasoning_effort(input_model, reasoning_mapping);
    if converter.eq_ignore_ascii_case("gemini") {
        return match effort {
            ReasoningEffort::Xhigh => gemini_reasoning_effort.opus.clone(),
            ReasoningEffort::High | ReasoningEffort::Medium => {
                gemini_reasoning_effort.sonnet.clone()
            }
            ReasoningEffort::Low => gemini_reasoning_effort.haiku.clone(),
        };
    }

    match effort {
        ReasoningEffort::Xhigh => codex_model_mapping.opus.clone(),
        ReasoningEffort::High | ReasoningEffort::Medium => codex_model_mapping.sonnet.clone(),
        ReasoningEffort::Low => codex_model_mapping.haiku.clone(),
    }
}

fn transform_request_with_optional_codex_effort_override(
    converter: &str,
    request_backend: &Arc<dyn TransformBackend>,
    anthropic_body: &AnthropicRequest,
    log_tx: &broadcast::Sender<String>,
    ctx: &TransformContext,
    model_name: &str,
    reasoning_effort_override: Option<ReasoningEffort>,
    effective_stream: bool,
) -> (Value, String) {
    if converter.eq_ignore_ascii_case("codex") {
        if let Some(override_effort) = reasoning_effort_override {
            let override_mapping = ReasoningEffortMapping::new()
                .with_opus(override_effort)
                .with_sonnet(override_effort)
                .with_haiku(override_effort);
            let mut override_ctx = ctx.clone();
            override_ctx.reasoning_mapping = override_mapping;
            return request_backend.transform_request(
                anthropic_body,
                Some(log_tx),
                &override_ctx,
                effective_stream,
                Some(model_name.to_string()),
            );
        }
    }

    request_backend.transform_request(
        anthropic_body,
        Some(log_tx),
        ctx,
        effective_stream,
        Some(model_name.to_string()),
    )
}

async fn send_prepared_json_request(
    client: &reqwest::Client,
    prepared: &PreparedRequest,
    anthropic_beta: Option<&String>,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut builder = client.post(&prepared.url);
    for (name, value) in &prepared.headers {
        builder = builder.header(name, value);
    }
    builder = builder.body(prepared.body.to_string());
    if let Some(beta) = anthropic_beta {
        builder = builder.header("anthropic-beta", beta);
    }
    builder.send().await
}

fn normalize_log_text(text: &str) -> String {
    text.replace('\n', " ").replace('\r', " ")
}

fn head_chars(text: &str, max_chars: usize) -> String {
    normalize_log_text(text).chars().take(max_chars).collect()
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let normalized = normalize_log_text(text);
    let total = normalized.chars().count();
    if total <= max_chars {
        return normalized;
    }
    let tail: String = normalized.chars().skip(total - max_chars).collect();
    format!("...{}", tail)
}

fn summarize_request_messages(messages: &[Message]) -> String {
    let msg_summaries: Vec<String> = messages
        .iter()
        .map(|message| {
            let content_summary = match &message.content {
                Some(MessageContent::Text(text)) => head_chars(text, 18),
                Some(MessageContent::Blocks(blocks)) => {
                    let mut block_summary = String::new();
                    for block in blocks.iter().take(4) {
                        let token = match block {
                            ContentBlock::Text { text } => head_chars(text, 12),
                            ContentBlock::Thinking { thinking, .. } => {
                                format!("[T:{}]", head_chars(thinking, 10))
                            }
                            ContentBlock::ToolUse { name, .. } => format!("[U:{}]", name),
                            ContentBlock::ToolResult { .. } => "[R]".to_string(),
                            ContentBlock::Image { .. }
                            | ContentBlock::ImageUrl { .. }
                            | ContentBlock::InputImage { .. } => "[I]".to_string(),
                            ContentBlock::Document { .. } => "[D]".to_string(),
                            ContentBlock::OtherValue(_) => "[O]".to_string(),
                        };
                        block_summary.push_str(&token);
                        if block_summary.chars().count() >= 24 {
                            break;
                        }
                    }
                    tail_chars(&block_summary, 24)
                }
                None => "empty".to_string(),
            };

            let role_prefix = message.role.chars().next().unwrap_or('?');
            format!("{}:{}", role_prefix, content_summary)
        })
        .collect();

    tail_chars(&msg_summaries.join(" > "), 80)
}

#[derive(Clone, Copy)]
struct StreamRuntimeOptions {
    #[allow(dead_code)]
    force_stream_for_codex: bool,
    enable_sse_frame_parser: bool,
    enable_stream_heartbeat: bool,
    stream_heartbeat_interval_ms: u64,
    enable_stream_log_sampling: bool,
    stream_log_sample_every_n: u32,
    stream_log_max_chars: usize,
    enable_stream_metrics: bool,
    enable_stream_event_metrics: bool,
    stream_silence_warn_ms: u64,
    stream_silence_error_ms: u64,
    stall_timeout_ms: u64,
    enable_incomplete_stream_retry: bool,
    incomplete_stream_retry_max_attempts: u32,
    enable_sibling_tool_error_retry: bool,
}

impl StreamRuntimeOptions {
    fn from_state(state: &RuntimeConfigState) -> Self {
        Self {
            force_stream_for_codex: state.force_stream_for_codex,
            enable_sse_frame_parser: state.enable_sse_frame_parser,
            enable_stream_heartbeat: state.enable_stream_heartbeat,
            stream_heartbeat_interval_ms: state.stream_heartbeat_interval_ms,
            enable_stream_log_sampling: state.enable_stream_log_sampling,
            stream_log_sample_every_n: state.stream_log_sample_every_n,
            stream_log_max_chars: state.stream_log_max_chars,
            enable_stream_metrics: state.enable_stream_metrics,
            enable_stream_event_metrics: state.enable_stream_event_metrics,
            stream_silence_warn_ms: state.stream_silence_warn_ms,
            stream_silence_error_ms: state.stream_silence_error_ms,
            stall_timeout_ms: state.stall_timeout_ms,
            enable_incomplete_stream_retry: state.enable_incomplete_stream_retry,
            incomplete_stream_retry_max_attempts: state.incomplete_stream_retry_max_attempts,
            enable_sibling_tool_error_retry: state.enable_sibling_tool_error_retry,
        }
    }
}

const STATEFUL_CHAIN_MAX_ENTRIES: usize = 128;
const STATEFUL_CHAIN_HINT_HEADERS: [&str; 6] = [
    "x-codex-proxy-session",
    "x-claude-session-id",
    "x-session-id",
    "session-id",
    "x-conversation-id",
    "conversation-id",
];

#[derive(Clone)]
struct StatefulChainRequestMeta {
    chain_key: String,
    endpoint_key: String,
    full_input: Vec<Value>,
    static_prefix_summary: Option<String>,
    static_prefix_same_as_prior: Option<bool>,
    non_input_fingerprint: Option<String>,
}

#[derive(Clone)]
struct StatefulChainEntry {
    response_id: String,
    endpoint_key: String,
    full_input: Vec<Value>,
    output_items: Vec<Value>,
    static_prefix_summary: Option<String>,
    non_input_fingerprint: Option<String>,
    turn_state: Option<String>,
    updated_at: Instant,
}

type StatefulChainStore = Arc<Mutex<HashMap<String, StatefulChainEntry>>>;
type StatefulChainUnsupportedEndpointStore = Arc<Mutex<HashSet<String>>>;
type CodexV1UnsupportedEndpointStore = Arc<Mutex<HashSet<String>>>;
type CodexFastUnsupportedEndpointStore = Arc<Mutex<HashSet<String>>>;

fn is_stateful_endpoint_previous_response_id_unsupported(
    unsupported_store: &StatefulChainUnsupportedEndpointStore,
    endpoint_key: &str,
) -> bool {
    match unsupported_store.lock() {
        Ok(guard) => guard.contains(endpoint_key),
        Err(poisoned) => poisoned.into_inner().contains(endpoint_key),
    }
}

fn mark_stateful_endpoint_previous_response_id_unsupported(
    unsupported_store: &StatefulChainUnsupportedEndpointStore,
    endpoint_key: &str,
) {
    match unsupported_store.lock() {
        Ok(mut guard) => {
            guard.insert(endpoint_key.to_string());
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.insert(endpoint_key.to_string());
        }
    }
}

fn is_previous_response_id_unsupported_error(status: u16, error_text: &str) -> bool {
    if status != StatusCode::BAD_REQUEST.as_u16() {
        return false;
    }
    let normalized = error_text.to_ascii_lowercase();
    normalized.contains("previous_response_id")
        && (normalized.contains("unsupported parameter")
            || normalized.contains("unknown parameter")
            || normalized.contains("additional properties are not allowed"))
}

fn hash_to_u64(parts: &[&str]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    hasher.finish()
}

fn strip_stateful_output_metadata(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut cleaned = Map::new();
            for (key, val) in map {
                if matches!(
                    key.as_str(),
                    "id" | "status" | "created_at" | "completed_at"
                ) {
                    continue;
                }
                cleaned.insert(key.clone(), strip_stateful_output_metadata(val));
            }
            Value::Object(cleaned)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(strip_stateful_output_metadata).collect())
        }
        _ => value.clone(),
    }
}

fn extract_stateful_chain_output_items(snapshot: &Value) -> Vec<Value> {
    snapshot
        .get("output")
        .and_then(|value| value.as_array())
        .map(|items| items.iter().map(strip_stateful_output_metadata).collect())
        .unwrap_or_default()
}

fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<_> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            let pairs: Vec<String> = sorted
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", k, canonical_json_string(v)))
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json_string).collect();
            format!("[{}]", items.join(","))
        }
        Value::String(s) => format!("\"{}\"", s),
        _ => value.to_string(),
    }
}

fn compute_non_input_fingerprint(body: &Value) -> Option<String> {
    let obj = body.as_object()?;
    let keys_to_check = [
        "model",
        "instructions",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "reasoning",
        "store",
        "stream",
        "include",
        "prompt_cache_key",
    ];

    let mut fingerprint_parts: Vec<(&str, String)> = Vec::new();
    for key in keys_to_check {
        if let Some(value) = obj.get(key) {
            fingerprint_parts.push((key, canonical_json_string(value)));
        }
    }

    if fingerprint_parts.is_empty() {
        return None;
    }

    let combined = fingerprint_parts
        .iter()
        .map(|(k, v)| format!("{}:{}", k, v))
        .collect::<Vec<_>>()
        .join("|");

    Some(format!("{:016x}", hash_to_u64(&[&combined])))
}

fn build_stateful_endpoint_key(
    converter: &str,
    target_url: &str,
    model: &str,
    api_key: &str,
) -> String {
    let fingerprint = hash_to_u64(&[converter, target_url, model, api_key]);
    format!("{}:{:016x}", converter.to_ascii_lowercase(), fingerprint)
}

fn build_codex_v1_endpoint_key(converter: &str, target_url: &str, api_key: &str) -> String {
    let fingerprint = hash_to_u64(&[converter, target_url, api_key]);
    format!("{}:{:016x}", converter.to_ascii_lowercase(), fingerprint)
}

fn is_codex_v1_endpoint_unsupported(
    unsupported_store: &CodexV1UnsupportedEndpointStore,
    endpoint_key: &str,
) -> bool {
    match unsupported_store.lock() {
        Ok(guard) => guard.contains(endpoint_key),
        Err(poisoned) => poisoned.into_inner().contains(endpoint_key),
    }
}

fn mark_codex_v1_endpoint_unsupported(
    unsupported_store: &CodexV1UnsupportedEndpointStore,
    endpoint_key: &str,
) {
    match unsupported_store.lock() {
        Ok(mut guard) => {
            guard.insert(endpoint_key.to_string());
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.insert(endpoint_key.to_string());
        }
    }
}

fn is_codex_fast_endpoint_unsupported(
    unsupported_store: &CodexFastUnsupportedEndpointStore,
    endpoint_key: &str,
) -> bool {
    match unsupported_store.lock() {
        Ok(guard) => guard.contains(endpoint_key),
        Err(poisoned) => poisoned.into_inner().contains(endpoint_key),
    }
}

fn mark_codex_fast_endpoint_unsupported(
    unsupported_store: &CodexFastUnsupportedEndpointStore,
    endpoint_key: &str,
) {
    match unsupported_store.lock() {
        Ok(mut guard) => {
            guard.insert(endpoint_key.to_string());
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.insert(endpoint_key.to_string());
        }
    }
}

fn extract_stateful_chain_hint(req: &Request<hyper::body::Incoming>) -> Option<String> {
    for name in STATEFUL_CHAIN_HINT_HEADERS {
        if let Some(value) = req.headers().get(name).and_then(|v| v.to_str().ok()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}
fn extract_session_hint_from_user_id(user_id: &str) -> Option<String> {
    let lower = user_id.to_ascii_lowercase();
    if let Some(idx) = lower.find("session_") {
        let tail = &user_id[idx + "session_".len()..];
        let token = tail
            .split(|ch: char| ch.is_whitespace() || ch == ';' || ch == ',' || ch == '"')
            .next()
            .unwrap_or("");
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

fn extract_stateful_chain_hint_from_request(request: &AnthropicRequest) -> Option<String> {
    let metadata = request.metadata.as_ref()?;
    let read_str = |key: &str| {
        metadata
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    };
    if let Some(value) = read_str("session_id") {
        return Some(value);
    }
    if let Some(value) = read_str("conversation_id") {
        return Some(value);
    }
    if let Some(user_id) = metadata.get("user_id").and_then(|value| value.as_str()) {
        return extract_session_hint_from_user_id(user_id);
    }
    None
}
struct StatefulChainHintInfo {
    value: Option<String>,
    source: &'static str,
}

fn resolve_stateful_chain_hint_info(
    header_hint: Option<String>,
    request: &AnthropicRequest,
) -> StatefulChainHintInfo {
    let header_hint = header_hint.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    if let Some(value) = header_hint {
        return StatefulChainHintInfo {
            value: Some(value),
            source: "header",
        };
    }
    if let Some(value) = extract_stateful_chain_hint_from_request(request) {
        return StatefulChainHintInfo {
            value: Some(value),
            source: "metadata",
        };
    }
    StatefulChainHintInfo {
        value: None,
        source: "none",
    }
}

fn build_message_prefix_signature(request: &AnthropicRequest) -> String {
    request
        .messages
        .iter()
        .take(4)
        .map(|message| {
            let preview = extract_message_text(message).unwrap_or_default();
            let normalized_preview = normalize_stateful_prefix_preview(&preview);
            let compact_preview: String = normalized_preview.chars().take(80).collect();
            format!(
                "{}:{}",
                message.role.to_ascii_lowercase(),
                compact_preview.replace('\n', " ").replace('\r', " ")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn normalize_stateful_prefix_preview(preview: &str) -> String {
    let normalized = preview.replace('\r', "").replace('\n', " ");
    let lower = normalized.to_ascii_lowercase();

    if lower.contains("<system-reminder>") && lower.contains("sessionstart") {
        return "<system-reminder>:sessionstart".to_string();
    }
    if lower.contains("<system-reminder>") && lower.contains("plan mode is active") {
        return "<system-reminder>:plan-mode-active".to_string();
    }

    normalized
}

fn derive_stateful_chain_key(
    hint: Option<&str>,
    converter: &str,
    model: &str,
    request: &AnthropicRequest,
) -> String {
    if let Some(hint_value) = hint {
        let normalized = hint_value.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            return format!("hint:{}:{}", converter.to_ascii_lowercase(), normalized);
        }
    }

    let system_text = request
        .system
        .as_ref()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let prefix_signature = build_message_prefix_signature(request);
    let fingerprint = hash_to_u64(&[converter, model, &system_text, &prefix_signature]);
    format!(
        "fallback:{}:{:016x}",
        converter.to_ascii_lowercase(),
        fingerprint
    )
}

fn common_prefix_len(a: &[Value], b: &[Value]) -> usize {
    let mut len = 0usize;
    for (lhs, rhs) in a.iter().zip(b.iter()) {
        if lhs == rhs {
            len += 1;
        } else {
            break;
        }
    }
    len
}

fn derive_static_prefix_summary_from_upstream_body(body: &Value) -> Option<String> {
    body.get("prompt_cache_key")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn format_optional_bool_for_log(value: Option<bool>) -> String {
    value
        .map(|flag| flag.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn prepare_stateful_chain_request(
    body: &mut Value,
    chain_store: &StatefulChainStore,
    unsupported_endpoints: &StatefulChainUnsupportedEndpointStore,
    chain_key: &str,
    endpoint_key: &str,
    request_id: &str,
    log_tx: &broadcast::Sender<String>,
    logger: &Option<Arc<AppLogger>>,
) -> Option<(StatefulChainRequestMeta, Option<String>)> {
    let static_prefix_summary = derive_static_prefix_summary_from_upstream_body(body);
    let current_non_input_fingerprint = compute_non_input_fingerprint(body);
    let obj = body.as_object_mut()?;
    let full_input = obj.get("input").and_then(|v| v.as_array()).cloned()?;
    obj.insert("store".to_string(), json!(true));

    if is_stateful_endpoint_previous_response_id_unsupported(unsupported_endpoints, endpoint_key) {
        emit_stream_diag(
            log_tx,
            logger,
            format!(
                "[StatefulChain] #{} previous_response_id_skipped reason=endpoint_unsupported static_prefix_same_as_prior=unknown",
                request_id
            ),
        );
        return Some((
            StatefulChainRequestMeta {
                chain_key: chain_key.to_string(),
                endpoint_key: endpoint_key.to_string(),
                full_input,
                static_prefix_summary,
                static_prefix_same_as_prior: None,
                non_input_fingerprint: current_non_input_fingerprint,
            },
            None,
        ));
    }

    let existing_entry = match chain_store.lock() {
        Ok(guard) => guard.get(chain_key).cloned(),
        Err(poisoned) => poisoned.into_inner().get(chain_key).cloned(),
    };

    let static_prefix_same_as_prior = existing_entry
        .as_ref()
        .and_then(|entry| entry.static_prefix_summary.as_ref())
        .zip(static_prefix_summary.as_ref())
        .map(|(previous, current)| previous == current);

    emit_stream_diag(
        log_tx,
        logger,
        format!(
            "[StatefulChain] #{} static_prefix_same_as_prior={}",
            request_id,
            format_optional_bool_for_log(static_prefix_same_as_prior)
        ),
    );

    let mut turn_state_to_inject: Option<String> = None;

    if let Some(entry) = existing_entry {
        if entry.endpoint_key != endpoint_key {
            emit_stream_diag(
                log_tx,
                logger,
                format!(
                    "[StatefulChain] #{} previous_response_id_skipped reason=endpoint_changed",
                    request_id
                ),
            );
        } else {
            // 非 input 字段一致性校验
            let non_input_matches = entry
                .non_input_fingerprint
                .as_ref()
                .zip(current_non_input_fingerprint.as_ref())
                .map(|(prev, curr)| prev == curr)
                .unwrap_or(true); // 兼容旧数据，无指纹时允许通过

            if !non_input_matches {
                emit_stream_diag(
                    log_tx,
                    logger,
                    format!(
                        "[StatefulChain] #{} previous_response_id_skipped reason=non_input_fields_changed",
                        request_id
                    ),
                );
            } else {
                let mut baseline = entry.full_input.clone();
                if !entry.output_items.is_empty() {
                    baseline.extend(entry.output_items.clone());
                }
                let baseline_len = baseline.len();
                let prefix_len = common_prefix_len(&baseline, &full_input);
                if prefix_len == baseline_len && full_input.len() > prefix_len {
                    let incremental_input = full_input[prefix_len..].to_vec();
                    obj.insert("input".to_string(), Value::Array(incremental_input));
                    obj.insert(
                        "previous_response_id".to_string(),
                        json!(entry.response_id.clone()),
                    );
                    // 保存 turn-state 用于注入到请求头
                    turn_state_to_inject = entry.turn_state.clone();
                    emit_stream_diag(
                        log_tx,
                        logger,
                        format!(
                            "[StatefulChain] #{} enabled=true previous_response_id_attached=true trimmed_prefix_items={} original_input_items={} incremental_input_items={}",
                            request_id,
                            prefix_len,
                            full_input.len(),
                            full_input.len() - prefix_len
                        ),
                    );
                } else {
                    emit_stream_diag(
                        log_tx,
                        logger,
                        format!(
                            "[StatefulChain] #{} previous_response_id_skipped reason=prefix_mismatch_or_no_delta matched_prefix_items={} stored_items={} stored_output_items={} current_items={}",
                            request_id,
                            prefix_len,
                            entry.full_input.len(),
                            entry.output_items.len(),
                            full_input.len()
                        ),
                    );
                }
            }
        }
    } else {
        emit_stream_diag(
            log_tx,
            logger,
            format!(
                "[StatefulChain] #{} previous_response_id_skipped reason=no_prior_entry",
                request_id
            ),
        );
    }

    Some((
        StatefulChainRequestMeta {
            chain_key: chain_key.to_string(),
            endpoint_key: endpoint_key.to_string(),
            full_input,
            static_prefix_summary,
            static_prefix_same_as_prior,
            non_input_fingerprint: current_non_input_fingerprint,
        },
        turn_state_to_inject,
    ))
}

fn record_stateful_chain_entry(
    chain_store: &StatefulChainStore,
    meta: &StatefulChainRequestMeta,
    response_id: &str,
    output_items: Vec<Value>,
    turn_state: Option<String>,
) {
    let mut guard = match chain_store.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    if guard.len() >= STATEFUL_CHAIN_MAX_ENTRIES && !guard.contains_key(&meta.chain_key) {
        if let Some((oldest_key, _)) = guard
            .iter()
            .min_by_key(|(_, entry)| entry.updated_at)
            .map(|(key, entry)| (key.clone(), entry.updated_at))
        {
            guard.remove(&oldest_key);
        }
    }

    guard.insert(
        meta.chain_key.clone(),
        StatefulChainEntry {
            response_id: response_id.to_string(),
            endpoint_key: meta.endpoint_key.clone(),
            full_input: meta.full_input.clone(),
            output_items,
            static_prefix_summary: meta.static_prefix_summary.clone(),
            non_input_fingerprint: meta.non_input_fingerprint.clone(),
            turn_state,
            updated_at: Instant::now(),
        },
    );
}

#[derive(Default)]
struct SseFrameParser {
    buffer: String,
}

impl SseFrameParser {
    fn push_chunk(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        let mut frames = Vec::new();

        while let Some((pos, delim_len)) = Self::find_delimiter(&self.buffer) {
            let frame = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + delim_len..].to_string();
            if !frame.trim().is_empty() {
                frames.push(frame);
            }
        }

        frames
    }

    fn take_remaining(&mut self) -> Option<String> {
        let remaining = self.buffer.trim().to_string();
        self.buffer.clear();
        if remaining.is_empty() {
            None
        } else {
            Some(remaining)
        }
    }

    fn find_delimiter(buffer: &str) -> Option<(usize, usize)> {
        let lf = buffer.find("\n\n").map(|idx| (idx, 2));
        let crlf = buffer.find("\r\n\r\n").map(|idx| (idx, 4));
        match (lf, crlf) {
            (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}

struct StreamMetrics {
    started_at: Instant,
    first_upstream_byte_at: Option<Instant>,
    first_delta_at: Option<Instant>,
    last_emit_at: Option<Instant>,
    max_silent_gap_ms: u128,
}

impl StreamMetrics {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            first_upstream_byte_at: None,
            first_delta_at: None,
            last_emit_at: None,
            max_silent_gap_ms: 0,
        }
    }

    fn mark_upstream_chunk(&mut self) {
        if self.first_upstream_byte_at.is_none() {
            self.first_upstream_byte_at = Some(Instant::now());
        }
    }

    fn mark_downstream_output(&mut self, output: &str) {
        let now = Instant::now();
        if let Some(last) = self.last_emit_at {
            let gap = now.duration_since(last).as_millis();
            if gap > self.max_silent_gap_ms {
                self.max_silent_gap_ms = gap;
            }
        }
        if self.first_delta_at.is_none() && output.contains("event: content_block_delta") {
            self.first_delta_at = Some(now);
        }
        self.last_emit_at = Some(now);
    }

    fn emit(&self, log_tx: &broadcast::Sender<String>, request_id: &str, enabled: bool) {
        if !enabled {
            return;
        }

        let total_ms = Instant::now().duration_since(self.started_at).as_millis();
        let ttfb_ms = self
            .first_upstream_byte_at
            .map(|ts| ts.duration_since(self.started_at).as_millis().to_string())
            .unwrap_or_else(|| "-".to_string());
        let first_delta_ms = self
            .first_delta_at
            .map(|ts| ts.duration_since(self.started_at).as_millis().to_string())
            .unwrap_or_else(|| "-".to_string());
        let _ = log_tx.send(format!(
            "[Metrics] #{} ttfb_ms={} first_delta_ms={} max_silent_gap_ms={} stream_total_ms={}",
            request_id, ttfb_ms, first_delta_ms, self.max_silent_gap_ms, total_ms
        ));
    }
}

#[derive(Default)]
struct StreamEventCounters {
    upstream_chunks: u64,
    upstream_frames: u64,
    upstream_response_completed: u64,
    upstream_response_incomplete: u64,
    upstream_response_failed: u64,
    downstream_message_start: u64,
    downstream_message_delta: u64,
    downstream_message_stop: u64,
    downstream_content_block_start_text: u64,
    downstream_content_block_start_tool_use: u64,
    downstream_content_block_start_thinking: u64,
    downstream_content_block_delta_text: u64,
    downstream_content_block_delta_input_json: u64,
    downstream_content_block_delta_thinking: u64,
    downstream_error: u64,
    downstream_keepalive: u64,
}

impl StreamEventCounters {
    fn mark_upstream_chunk(&mut self) {
        self.upstream_chunks += 1;
    }

    fn mark_upstream_frame(&mut self) {
        self.upstream_frames += 1;
    }

    fn mark_response_completed(&mut self) {
        self.upstream_response_completed += 1;
    }

    fn mark_response_incomplete(&mut self) {
        self.upstream_response_incomplete += 1;
    }

    fn mark_response_failed(&mut self) {
        self.upstream_response_failed += 1;
    }

    fn mark_keepalive(&mut self) {
        self.downstream_keepalive += 1;
    }

    fn mark_downstream_chunk(&mut self, chunk: &str) {
        if chunk.starts_with(": keep-alive") {
            self.downstream_keepalive += 1;
            return;
        }

        let Some((event, payload)) = parse_sse_chunk(chunk) else {
            return;
        };

        match event.as_str() {
            "ping" => self.downstream_keepalive += 1,
            "message_start" => self.downstream_message_start += 1,
            "message_delta" => self.downstream_message_delta += 1,
            "message_stop" => self.downstream_message_stop += 1,
            "error" => self.downstream_error += 1,
            "content_block_start" => {
                let block_type = payload
                    .get("content_block")
                    .and_then(|v| v.get("type"))
                    .and_then(|v| v.as_str());
                match block_type {
                    Some("text") => self.downstream_content_block_start_text += 1,
                    Some("tool_use") => self.downstream_content_block_start_tool_use += 1,
                    Some("thinking") => self.downstream_content_block_start_thinking += 1,
                    _ => {}
                }
            }
            "content_block_delta" => {
                let delta_type = payload
                    .get("delta")
                    .and_then(|v| v.get("type"))
                    .and_then(|v| v.as_str());
                match delta_type {
                    Some("text_delta") => self.downstream_content_block_delta_text += 1,
                    Some("input_json_delta") => self.downstream_content_block_delta_input_json += 1,
                    Some("thinking_delta") => self.downstream_content_block_delta_thinking += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn observe_upstream_chunk_events(
    chunk: &str,
    saw_response_completed: &mut bool,
    saw_response_incomplete: &mut bool,
    saw_response_failed: &mut bool,
    saw_sibling_tool_call_error: &mut bool,
    upstream_error_event_type: &mut Option<String>,
    upstream_error_message: &mut Option<String>,
    upstream_error_code: &mut Option<String>,
    counters: &mut StreamEventCounters,
) {
    counters.mark_upstream_frame();
    if upstream_chunk_contains_event(chunk, "response.completed")
        || upstream_chunk_indicates_done(chunk)
    {
        *saw_response_completed = true;
        counters.mark_response_completed();
    }
    if upstream_chunk_contains_event(chunk, "response.incomplete") {
        *saw_response_incomplete = true;
        *saw_response_completed = true;
        counters.mark_response_incomplete();
    }

    let saw_failed_event = upstream_chunk_contains_event(chunk, "response.failed");
    let saw_error_event = upstream_chunk_contains_event(chunk, "error");
    if saw_failed_event || saw_error_event {
        *saw_response_failed = true;
        counters.mark_response_failed();
        if saw_failed_event {
            *upstream_error_event_type = Some("response.failed".to_string());
        } else if saw_error_event {
            *upstream_error_event_type = Some("error".to_string());
        }
    }

    if let Some((event_type, message, code)) = extract_upstream_error_details(chunk) {
        *upstream_error_event_type = Some(event_type.clone());
        if let Some(message) = message {
            if event_type == "response.failed" && is_sibling_tool_call_error_message(&message) {
                *saw_sibling_tool_call_error = true;
            }
            *upstream_error_message = Some(message);
        } else if event_type == "response.failed" && chunk_contains_sibling_tool_call_error(chunk) {
            *saw_sibling_tool_call_error = true;
        }
        if let Some(code) = code {
            *upstream_error_code = Some(code);
        }
    } else if saw_failed_event && chunk_contains_sibling_tool_call_error(chunk) {
        *saw_sibling_tool_call_error = true;
    }
}

fn derive_stream_close_cause(
    explicit_cause: Option<&str>,
    saw_response_completed: bool,
    saw_response_incomplete: bool,
    saw_response_failed: bool,
    saw_message_stop: bool,
) -> String {
    if let Some(cause) = explicit_cause {
        return cause.to_string();
    }
    if saw_response_failed {
        return "response_failed".to_string();
    }
    if saw_response_incomplete && saw_message_stop {
        return "response_incomplete".to_string();
    }
    if saw_response_incomplete {
        return "response_incomplete_without_message_stop".to_string();
    }
    if saw_response_completed && saw_message_stop {
        return "completed".to_string();
    }
    if saw_response_completed {
        return "completed_without_message_stop".to_string();
    }
    "ended_before_response_completed".to_string()
}

fn emit_stream_terminal_summary(
    log_tx: &broadcast::Sender<String>,
    logger: &Option<Arc<AppLogger>>,
    request_id: &str,
    close_cause: &str,
    counters: &StreamEventCounters,
    metrics: &StreamMetrics,
    saw_response_completed: bool,
    saw_response_incomplete: bool,
    saw_response_failed: bool,
    saw_message_stop: bool,
    emitted_business_event: bool,
    fallback_completion_injected: bool,
    empty_completion_retry_attempts: u32,
    empty_completion_retry_succeeded: bool,
    incomplete_stream_retry_attempts: u32,
    incomplete_stream_retry_succeeded: bool,
) {
    let summary = json!({
        "close_cause": close_cause,
        "flags": {
            "saw_response_completed": saw_response_completed,
            "saw_response_incomplete": saw_response_incomplete,
            "saw_response_failed": saw_response_failed,
            "saw_message_stop": saw_message_stop,
            "emitted_business_event": emitted_business_event,
            "fallback_completion_injected": fallback_completion_injected,
        },
        "event_counts": {
            "upstream_chunks": counters.upstream_chunks,
            "upstream_frames": counters.upstream_frames,
            "upstream_response_completed": counters.upstream_response_completed,
            "upstream_response_incomplete": counters.upstream_response_incomplete,
            "upstream_response_failed": counters.upstream_response_failed,
            "downstream_message_start": counters.downstream_message_start,
            "downstream_message_delta": counters.downstream_message_delta,
            "downstream_message_stop": counters.downstream_message_stop,
            "downstream_content_block_start_text": counters.downstream_content_block_start_text,
            "downstream_content_block_start_tool_use": counters.downstream_content_block_start_tool_use,
            "downstream_content_block_start_thinking": counters.downstream_content_block_start_thinking,
            "downstream_content_block_delta_text": counters.downstream_content_block_delta_text,
            "downstream_content_block_delta_input_json": counters.downstream_content_block_delta_input_json,
            "downstream_content_block_delta_thinking": counters.downstream_content_block_delta_thinking,
            "downstream_error": counters.downstream_error,
            "downstream_keepalive": counters.downstream_keepalive,
        },
        "max_silent_gap_ms": metrics.max_silent_gap_ms,
        "retry_summary": {
            "empty_completion_retry_attempts": empty_completion_retry_attempts,
            "empty_completion_retry_succeeded": empty_completion_retry_succeeded,
            "incomplete_stream_retry_attempts": incomplete_stream_retry_attempts,
            "incomplete_stream_retry_succeeded": incomplete_stream_retry_succeeded,
        }
    });
    emit_stream_diag(
        log_tx,
        logger,
        format!("[StreamSummary] #{} {}", request_id, summary),
    );
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    let clipped: String = text.chars().take(max_chars).collect();
    format!("{}... (truncated, len={})", clipped, char_count)
}

fn maybe_log_stream_upstream(
    logger: &Option<Arc<AppLogger>>,
    status: u16,
    text: &str,
    opts: StreamRuntimeOptions,
    counter: &mut u64,
) {
    let Some(l) = logger else {
        return;
    };

    *counter += 1;
    if opts.enable_stream_log_sampling {
        let sample_n = opts.stream_log_sample_every_n.max(1) as u64;
        if *counter != 1 && *counter % sample_n != 0 {
            return;
        }
    }

    let msg = truncate_chars(text, opts.stream_log_max_chars);
    l.log_upstream_response(status, &msg);
}

fn maybe_log_stream_downstream(
    logger: &Option<Arc<AppLogger>>,
    text: &str,
    opts: StreamRuntimeOptions,
    counter: &mut u64,
) {
    let Some(l) = logger else {
        return;
    };

    *counter += 1;
    if opts.enable_stream_log_sampling {
        let sample_n = opts.stream_log_sample_every_n.max(1) as u64;
        if *counter != 1 && *counter % sample_n != 0 {
            return;
        }
    }

    let msg = truncate_chars(text, opts.stream_log_max_chars);
    l.log_anthropic_response(&msg);
}

fn emit_stream_diag(
    log_tx: &broadcast::Sender<String>,
    logger: &Option<Arc<AppLogger>>,
    msg: String,
) {
    let _ = log_tx.send(msg.clone());
    if let Some(l) = logger {
        l.log(&msg);
    }
}

fn emit_transform_diag(
    log_tx: &broadcast::Sender<String>,
    logger: &Option<Arc<AppLogger>>,
    request_id: &str,
    session_id: &str,
    summary: &Value,
) {
    emit_stream_diag(
        log_tx,
        logger,
        format!(
            "[TransformDiag] #{} session_id={} {}",
            request_id, session_id, summary
        ),
    );
}

#[derive(Clone, Copy)]
enum ConnErrorClass {
    ClientDisconnect,
    ProtocolOrNetwork,
}

impl ConnErrorClass {
    fn as_str(self) -> &'static str {
        match self {
            ConnErrorClass::ClientDisconnect => "client_disconnect",
            ConnErrorClass::ProtocolOrNetwork => "protocol_or_network",
        }
    }

    fn log_prefix(self) -> &'static str {
        match self {
            ConnErrorClass::ClientDisconnect => "[ConnWarn]",
            ConnErrorClass::ProtocolOrNetwork => "[ConnError]",
        }
    }
}

fn collect_error_source_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut current = err.source();
    while let Some(source) = current {
        parts.push(source.to_string());
        current = source.source();
    }
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(" <- ")
    }
}

fn classify_connection_error(display: &str, debug: &str, source_chain: &str) -> ConnErrorClass {
    let haystack = format!(
        "{}\n{}\n{}",
        display.to_ascii_lowercase(),
        debug.to_ascii_lowercase(),
        source_chain.to_ascii_lowercase()
    );
    let client_disconnect_markers = [
        "connection closed before message completed",
        "broken pipe",
        "connection reset",
        "connection aborted",
        "unexpected eof",
        "reset by peer",
    ];
    if client_disconnect_markers
        .iter()
        .any(|marker| haystack.contains(marker))
    {
        ConnErrorClass::ClientDisconnect
    } else {
        ConnErrorClass::ProtocolOrNetwork
    }
}

fn emit_connection_error_diag(
    log_tx: &broadcast::Sender<String>,
    logger: &Arc<AppLogger>,
    conn_id: &str,
    peer_addr: &SocketAddr,
    local_port: u16,
    err: &(dyn std::error::Error + 'static),
) {
    let display = err.to_string();
    let debug = format!("{err:?}");
    let source_chain = collect_error_source_chain(err);
    let class = classify_connection_error(&display, &debug, &source_chain);
    let payload = json!({
        "conn_id": conn_id,
        "peer_addr": peer_addr.to_string(),
        "local_port": local_port,
        "class": class.as_str(),
        "display": truncate_chars(&display, 1024),
        "debug": truncate_chars(&debug, 1024),
        "source_chain": truncate_chars(&source_chain, 2048),
    });
    let msg = format!("{} {}", class.log_prefix(), payload);
    let _ = log_tx.send(msg.clone());
    logger.log(&msg);
}

async fn try_send_keep_alive(
    tx: &tokio::sync::mpsc::Sender<Result<Frame<Bytes>, Infallible>>,
    log_tx: &broadcast::Sender<String>,
    request_id: &str,
    metrics: &mut StreamMetrics,
    enable_stream_metrics: bool,
    disconnect_context: &str,
) -> bool {
    let keep_alive = "event: ping\ndata: {\"type\": \"ping\"}\n\n";
    if tx
        .send(Ok(Frame::data(Bytes::from(keep_alive))))
        .await
        .is_err()
    {
        let _ = log_tx.send(format!(
            "[Warning] #{} Client disconnected while {}",
            request_id, disconnect_context
        ));
        metrics.emit(log_tx, request_id, enable_stream_metrics);
        return false;
    }
    metrics.mark_downstream_output(keep_alive);
    true
}

fn drain_complete_lines(buffer: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pos) = buffer.find('\n') {
        let mut line = buffer[..pos].to_string();
        if line.ends_with('\r') {
            line.pop();
        }
        *buffer = buffer[pos + 1..].to_string();
        if line.trim().is_empty() {
            continue;
        }
        lines.push(line);
    }
    lines
}

fn resolve_effective_stream(
    requested_stream: bool,
    _converter: &str,
    _accept_header: Option<&str>,
    _opts: StreamRuntimeOptions,
) -> bool {
    requested_stream
}

fn parse_sse_chunk(chunk: &str) -> Option<(String, Value)> {
    let mut event_name: Option<String> = None;
    let mut data: Option<Value> = None;

    for line in chunk.lines() {
        if let Some(v) = line.strip_prefix("event: ") {
            event_name = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("data: ") {
            if let Ok(parsed) = serde_json::from_str::<Value>(v) {
                data = Some(parsed);
            }
        }
    }

    match (event_name, data) {
        (Some(event), Some(payload)) => Some((event, payload)),
        _ => None,
    }
}

fn extract_upstream_response_id(chunk: &str) -> Option<String> {
    let (event, payload) = parse_sse_chunk(chunk)?;
    let normalized_event = event.trim();
    if normalized_event != "response.completed"
        && normalized_event != "response.done"
        && normalized_event != "response.incomplete"
    {
        return None;
    }

    payload
        .pointer("/response/id")
        .or_else(|| payload.pointer("/response_id"))
        .or_else(|| payload.get("id"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn extract_codex_terminal_response_snapshot(chunk: &str) -> Option<Value> {
    let (event, payload) = parse_sse_chunk(chunk)?;
    let normalized_event = event.trim();
    if normalized_event != "response.completed"
        && normalized_event != "response.done"
        && normalized_event != "response.incomplete"
    {
        return None;
    }

    payload
        .get("response")
        .cloned()
        .or_else(|| payload.get("output").map(|_| payload.clone()))
}

fn upstream_chunk_indicates_done(chunk: &str) -> bool {
    for line in chunk.lines() {
        if let Some(value) = line.strip_prefix("event: ") {
            if value.trim() == "done" {
                return true;
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("data: ") {
            if value.trim() == "[DONE]" {
                return true;
            }
        }
    }
    false
}

fn upstream_chunk_contains_event(chunk: &str, target_event_type: &str) -> bool {
    let target = target_event_type.trim();
    if target.is_empty() {
        return false;
    }

    for line in chunk.lines() {
        if let Some(value) = line.strip_prefix("event: ") {
            if value.trim() == target {
                return true;
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("data: ") {
            if let Ok(parsed) = serde_json::from_str::<Value>(value) {
                if parsed.get("type").and_then(|v| v.as_str()) == Some(target) {
                    return true;
                }
            }
        }
    }

    false
}

fn chunk_is_message_stop(output: &str) -> bool {
    parse_sse_chunk(output)
        .map(|(event, _)| event == "message_stop")
        .unwrap_or_else(|| output.contains("event: message_stop"))
}

fn should_drop_post_message_stop_output(output: &str) -> bool {
    if output.trim_start().starts_with(':') {
        return false;
    }
    parse_sse_chunk(output)
        .map(|(event, _)| event != "ping")
        .unwrap_or(false)
}

fn should_suppress_premature_message_stop(
    output: &str,
    is_codex_stream: bool,
    saw_response_completed: bool,
    saw_response_incomplete: bool,
    saw_response_failed: bool,
) -> bool {
    let saw_terminal = saw_response_completed || saw_response_incomplete || saw_response_failed;
    is_codex_stream && chunk_is_message_stop(output) && !saw_terminal
}

fn should_drop_duplicate_message_start(
    output: &str,
    sent_message_start_to_client: &mut bool,
) -> bool {
    if !output.contains("event: message_start") {
        return false;
    }

    if *sent_message_start_to_client {
        return true;
    }

    *sent_message_start_to_client = true;
    false
}

fn is_business_stream_output(chunk: &str) -> bool {
    let Some((event, payload)) = parse_sse_chunk(chunk) else {
        return false;
    };

    match event.as_str() {
        "content_block_start" => payload
            .get("content_block")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str())
            .map(|kind| matches!(kind, "text" | "tool_use" | "thinking"))
            .unwrap_or(false),
        "content_block_delta" => {
            let delta = payload.get("delta");
            let kind = delta
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match kind {
                "text_delta" | "input_json_delta" | "thinking_delta" => true,
                _ => false,
            }
        }
        _ => false,
    }
}

fn is_tool_stream_output(chunk: &str) -> bool {
    let Some((event, payload)) = parse_sse_chunk(chunk) else {
        return false;
    };

    match event.as_str() {
        "content_block_start" => payload
            .get("content_block")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str())
            .map(|kind| kind == "tool_use")
            .unwrap_or(false),
        "content_block_delta" => payload
            .get("delta")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str())
            .map(|kind| kind == "input_json_delta")
            .unwrap_or(false),
        _ => false,
    }
}

fn should_skip_transformed_output(
    decision: &mut StreamDecisionState,
    output: &str,
    is_codex_stream: bool,
    log_tx: &broadcast::Sender<String>,
    logger: &Option<Arc<AppLogger>>,
    request_id: &str,
) -> bool {
    match decision.classify_output(output, is_codex_stream) {
        OutputDisposition::SkipDuplicateMessageStart => true,
        OutputDisposition::SkipPrematureMessageStop => {
            if !decision.logged_premature_stop_suppression {
                emit_stream_diag(
                    log_tx,
                    logger,
                    format!(
                        "[Stream] #{} suppress_premature_message_stop=true",
                        request_id
                    ),
                );
                decision.logged_premature_stop_suppression = true;
            }
            true
        }
        OutputDisposition::SkipPostMessageStopOutput => {
            if !decision.logged_post_message_stop_drop {
                emit_stream_diag(
                    log_tx,
                    logger,
                    format!(
                        "[Stream] #{} drop_post_message_stop_output=true",
                        request_id
                    ),
                );
                decision.logged_post_message_stop_drop = true;
            }
            true
        }
        OutputDisposition::Accepted { .. } => false,
    }
}

fn append_block_text(block: &mut Value, field: &str, delta: &str) {
    if let Some(obj) = block.as_object_mut() {
        let current = obj
            .get(field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        obj.insert(field.to_string(), json!(format!("{}{}", current, delta)));
    }
}

fn finalize_tool_input_block(
    index: usize,
    blocks: &mut BTreeMap<usize, Value>,
    tool_input_buffers: &mut HashMap<usize, String>,
) {
    let Some(partial_json) = tool_input_buffers.remove(&index) else {
        return;
    };

    let parsed_input = serde_json::from_str::<Value>(&partial_json).unwrap_or_else(|_| {
        json!({
            "_parse_error": "invalid_json",
            "_raw_input": partial_json
        })
    });
    if let Some(block) = blocks.get_mut(&index) {
        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
            if let Some(obj) = block.as_object_mut() {
                obj.insert("input".to_string(), parsed_input);
            }
        }
    }
}

fn apply_sse_chunk_to_non_stream_message(
    chunk: &str,
    message_state: &mut Option<Value>,
    blocks: &mut BTreeMap<usize, Value>,
    tool_input_buffers: &mut HashMap<usize, String>,
    stop_reason_state: &mut Option<String>,
    usage_input_tokens: &mut u64,
    usage_output_tokens: &mut u64,
) {
    let Some((event, payload)) = parse_sse_chunk(chunk) else {
        return;
    };

    match event.as_str() {
        "message_start" => {
            *message_state = payload.get("message").cloned();
        }
        "content_block_start" => {
            if let Some(index) = payload.get("index").and_then(|v| v.as_u64()) {
                if let Some(block) = payload.get("content_block") {
                    blocks.insert(index as usize, block.clone());
                }
            }
        }
        "content_block_delta" => {
            let Some(index) = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
            else {
                return;
            };

            let Some(delta_obj) = payload.get("delta") else {
                return;
            };

            let delta_type = delta_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match delta_type {
                "text_delta" => {
                    if let Some(text_delta) = delta_obj.get("text").and_then(|v| v.as_str()) {
                        if let Some(block) = blocks.get_mut(&index) {
                            append_block_text(block, "text", text_delta);
                        }
                    }
                }
                "thinking_delta" => {
                    if let Some(thinking_delta) = delta_obj.get("thinking").and_then(|v| v.as_str())
                    {
                        if let Some(block) = blocks.get_mut(&index) {
                            append_block_text(block, "thinking", thinking_delta);
                        }
                    }
                }
                "input_json_delta" => {
                    let partial = delta_obj
                        .get("partial_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !partial.is_empty() {
                        let entry = tool_input_buffers.entry(index).or_default();
                        entry.push_str(partial);
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            if let Some(index) = payload.get("index").and_then(|v| v.as_u64()) {
                finalize_tool_input_block(index as usize, blocks, tool_input_buffers);
            }
        }
        "message_delta" => {
            if let Some(reason) = payload
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
            {
                *stop_reason_state = Some(reason.to_string());
            }

            if let Some(usage) = payload.get("usage") {
                *usage_input_tokens = usage
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(*usage_input_tokens);
                *usage_output_tokens = usage
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(*usage_output_tokens);
            }
        }
        "message_stop" => {
            if let Some(reason) = payload.get("stop_reason").and_then(|v| v.as_str()) {
                *stop_reason_state = Some(reason.to_string());
            }
        }
        _ => {}
    }
}

fn sorted_object_keys(value: &Value) -> Vec<String> {
    let mut keys = value
        .as_object()
        .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    keys
}

fn summarize_message_content_block(block: &Value) -> String {
    let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("?");
    let keys = sorted_object_keys(block).join(",");
    format!("{}<{}>", block_type, keys)
}

fn summarize_codex_input_item(index: usize, item: &Value) -> String {
    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("?");
    let keys = sorted_object_keys(item).join(",");

    let mut summary = format!("{}:{}<{}>", index, item_type, keys);

    if item_type == "message" {
        if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
            let block_summaries = content
                .iter()
                .take(3)
                .map(summarize_message_content_block)
                .collect::<Vec<_>>();

            if !block_summaries.is_empty() {
                summary.push_str(&format!(" content=[{}]", block_summaries.join(";")));
            }

            if content.len() > 3 {
                summary.push_str(&format!(" ...(+{})", content.len() - 3));
            }
        }
    }

    summary
}

fn summarize_codex_payload(payload: &Value) -> Option<String> {
    let input = payload.get("input")?.as_array()?;
    let mut items = input
        .iter()
        .take(8)
        .enumerate()
        .map(|(index, item)| summarize_codex_input_item(index, item))
        .collect::<Vec<_>>();

    if input.len() > 8 {
        items.push(format!("...(+{} items)", input.len() - 8));
    }

    Some(items.join(" | "))
}

fn contains_tool_call_text_leak(content: &Value) -> bool {
    let Some(blocks) = content.as_array() else {
        return false;
    };

    blocks.iter().any(|block| {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if block_type != "text" {
            return false;
        }

        let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
        text.contains("assistant to=multi_tool_use.parallel")
            || text.contains("assistant to=functions.")
            || text.contains("to=multi_tool_use.parallel")
            || text.contains("to=functions.")
    })
}

fn extract_assistant_preview_from_content(content: &Value) -> Option<String> {
    let blocks = content.as_array()?;
    let preview = blocks
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match block_type {
                "text" => block
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|text| text.to_string()),
                "tool_use" => block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|name| format!("[tool_use:{}]", name)),
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = preview.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn log_non_stream_assistant_preview(
    log_tx: &broadcast::Sender<String>,
    request_id: &str,
    payload: &Value,
) {
    let preview = payload
        .get("content")
        .and_then(extract_assistant_preview_from_content)
        .unwrap_or_else(|| "<empty>".to_string());
    let _ = log_tx.send(format!(
        "[NonStream] #{} assistant_preview={}",
        request_id,
        head_chars(&preview, 120),
    ));
}

fn build_anthropic_message_from_codex_json_response(
    response: &Value,
    fallback_model: &str,
) -> Value {
    let mut content = Vec::new();
    let mut first_message_id: Option<String> = None;

    if let Some(output) = response.get("output").and_then(|v| v.as_array()) {
        for item in output {
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                "message" => {
                    if item.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                        continue;
                    }
                    if first_message_id.is_none() {
                        first_message_id = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(|value| value.to_string());
                    }
                    if let Some(parts) = item.get("content").and_then(|v| v.as_array()) {
                        for part in parts {
                            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if matches!(part_type, "output_text" | "text" | "refusal") {
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    if !text.is_empty() {
                                        content.push(json!({ "type": "text", "text": text }));
                                    }
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let tool_use_id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(|v| v.as_str())
                        .filter(|value| !value.is_empty())
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4().simple()));
                    let input = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .and_then(|value| serde_json::from_str::<Value>(value).ok())
                        .unwrap_or_else(|| json!({}));
                    content.push(json!({
                        "type": "tool_use",
                        "id": tool_use_id,
                        "name": name,
                        "input": input,
                    }));
                }
                _ => {}
            }
        }
    }

    let stop_reason = if content
        .iter()
        .any(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_use"))
    {
        "tool_use"
    } else {
        "end_turn"
    };

    json!({
        "id": first_message_id.unwrap_or_else(|| format!("msg_{}", chrono::Utc::now().timestamp_millis())),
        "type": "message",
        "role": "assistant",
        "model": response
            .get("model")
            .cloned()
            .unwrap_or_else(|| json!(fallback_model)),
        "content": content,
        "stop_reason": stop_reason,
        "usage": response.get("usage").cloned().unwrap_or_else(|| json!({"input_tokens":0,"output_tokens":0})),
    })
}

fn backfill_non_stream_payload_from_codex_snapshot(
    payload: Value,
    snapshot: Option<&Value>,
    fallback_model: &str,
) -> Value {
    let has_content = payload
        .get("content")
        .and_then(|value| value.as_array())
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    if has_content {
        return payload;
    }

    let Some(snapshot) = snapshot else {
        return payload;
    };

    build_anthropic_message_from_codex_json_response(snapshot, fallback_model)
}

fn extract_cached_input_tokens_from_response_usage(usage: &Value) -> Option<u64> {
    usage
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(|value| value.as_u64())
}

fn log_usage_tokens(
    log_tx: &broadcast::Sender<String>,
    request_id: &str,
    usage: Option<&Value>,
    source: &str,
) {
    let Some(usage) = usage else {
        return;
    };
    let input_tokens = usage.get("input_tokens").and_then(|value| value.as_u64());
    let output_tokens = usage.get("output_tokens").and_then(|value| value.as_u64());
    if input_tokens.is_none() && output_tokens.is_none() {
        return;
    }
    let cached_input_tokens = extract_cached_input_tokens_from_response_usage(usage);
    let _ = log_tx.send(format!(
        "[Tokens] #{} input_tokens={} output_tokens={} cached_input_tokens={} source={}",
        request_id,
        input_tokens.unwrap_or(0),
        output_tokens.unwrap_or(0),
        cached_input_tokens.unwrap_or(0),
        source
    ));
}

fn log_prompt_cache_observation(
    log_tx: &broadcast::Sender<String>,
    request_id: &str,
    cached_input_tokens: Option<u64>,
    static_prefix_same_as_prior: Option<bool>,
) {
    let _ = log_tx.send(format!(
        "[PromptCache] #{} prompt_cache_observed={} cached_input_tokens={} static_prefix_same_as_prior={}",
        request_id,
        cached_input_tokens.is_some(),
        cached_input_tokens.unwrap_or(0),
        format_optional_bool_for_log(static_prefix_same_as_prior)
    ));
}

fn disable_parallel_tool_calls_in_upstream_body(body: &Value) -> Option<Value> {
    let mut next = body.clone();
    let obj = next.as_object_mut()?;
    let current = obj.get("parallel_tool_calls").and_then(|v| v.as_bool())?;
    if !current {
        return None;
    }
    obj.insert("parallel_tool_calls".to_string(), json!(false));
    Some(next)
}

fn body_uses_priority_service_tier(body: &Value) -> bool {
    body.get("service_tier")
        .and_then(|v| v.as_str())
        .map(|value| value.eq_ignore_ascii_case("priority"))
        .unwrap_or(false)
}

fn remove_priority_service_tier_from_upstream_body(body: &Value) -> Option<Value> {
    if !body_uses_priority_service_tier(body) {
        return None;
    }

    let mut next = body.clone();
    let obj = next.as_object_mut()?;
    obj.remove("service_tier");
    Some(next)
}

const RETRY_NO_TOOL_TEXT_MIX_GUARDRAIL_TAG: &str = "RETRY_GUARDRAIL_NO_TOOL_TEXT_MIX";
const RETRY_NO_TOOL_TEXT_MIX_GUARDRAIL_INSTRUCTION: &str = r#"On retry, enforce strict tool/text channel separation:
- Never emit tool protocol text in assistant natural-language output.
- Never print raw tool JSON arguments in text.
- Do not output markers like `assistant to=...`, `to=functions...`, or `{"tool_uses":...}`.
- Emit tool calls only via structured tool/function call events.
- Keep plain text and tool payloads physically separated in the stream."#;

fn inject_retry_no_tool_text_mix_guardrail(body: &Value) -> Option<Value> {
    let mut next = body.clone();
    let obj = next.as_object_mut()?;

    let existing = obj
        .get("instructions")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if existing.contains(RETRY_NO_TOOL_TEXT_MIX_GUARDRAIL_TAG) {
        return Some(next);
    }

    let injected = format!(
        "\n\n[{}]\n{}\n[/{}]",
        RETRY_NO_TOOL_TEXT_MIX_GUARDRAIL_TAG,
        RETRY_NO_TOOL_TEXT_MIX_GUARDRAIL_INSTRUCTION,
        RETRY_NO_TOOL_TEXT_MIX_GUARDRAIL_TAG
    );

    let merged = if existing.is_empty() {
        injected
    } else {
        format!("{}{}", existing, injected)
    };

    obj.insert("instructions".to_string(), json!(merged));
    Some(next)
}

struct StreamRetrySuccess {
    response: reqwest::Response,
    status: u16,
    session_id: String,
}

async fn execute_stream_retry_request(
    request_backend: &Arc<dyn TransformBackend>,
    http_client: &reqwest::Client,
    upstream_url: &str,
    upstream_api_key: &str,
    upstream_body: &Value,
    anthropic_version: &str,
    anthropic_beta: Option<&str>,
    request_id: &str,
    retry_label: &str,
    log_tx: &broadcast::Sender<String>,
    logger: &Option<Arc<AppLogger>>,
) -> Option<StreamRetrySuccess> {
    let retry_session_id = Uuid::new_v4().to_string();
    let retry_req = request_backend.build_upstream_request(
        http_client,
        upstream_url,
        upstream_api_key,
        upstream_body,
        &retry_session_id,
        anthropic_version,
    );
    let retry_req = if let Some(beta) = anthropic_beta {
        retry_req.header("anthropic-beta", beta)
    } else {
        retry_req
    };

    match retry_req.send().await {
        Ok(retry_response) => {
            let retry_status = retry_response.status().as_u16();
            if retry_response.status().is_success() {
                emit_stream_diag(
                    log_tx,
                    logger,
                    format!(
                        "[Stream] #{} {}_succeeded status={}",
                        request_id, retry_label, retry_status
                    ),
                );
                Some(StreamRetrySuccess {
                    response: retry_response,
                    status: retry_status,
                    session_id: retry_session_id,
                })
            } else {
                let retry_error = retry_response.text().await.unwrap_or_default();
                emit_stream_diag(
                    log_tx,
                    logger,
                    format!(
                        "[Stream] #{} {}_failed status={} body={}",
                        request_id, retry_label, retry_status, retry_error
                    ),
                );
                None
            }
        }
        Err(err) => {
            emit_stream_diag(
                log_tx,
                logger,
                format!(
                    "[Stream] #{} {}_failed network_error={}",
                    request_id, retry_label, err
                ),
            );
            None
        }
    }
}

fn extract_upstream_error_details(chunk: &str) -> Option<(String, Option<String>, Option<String>)> {
    let mut event_name: Option<String> = None;

    for line in chunk.lines() {
        if let Some(event) = line.strip_prefix("event: ") {
            event_name = Some(event.trim().to_string());
            continue;
        }

        let Some(data_line) = line.strip_prefix("data: ") else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(data_line) else {
            continue;
        };

        let payload_type = parsed
            .get("type")
            .and_then(|value| value.as_str())
            .map(|value| value.trim());
        let event_type = match (payload_type, event_name.as_deref()) {
            (Some("response.failed"), _) => "response.failed",
            (Some("error"), _) => "error",
            (_, Some("response.failed")) => "response.failed",
            (_, Some("error")) => "error",
            _ => continue,
        };

        let message = parsed
            .pointer("/response/error/message")
            .or_else(|| parsed.pointer("/error/message"))
            .or_else(|| parsed.pointer("/message"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let code = parsed
            .pointer("/response/error/code")
            .or_else(|| parsed.pointer("/error/code"))
            .or_else(|| parsed.pointer("/response/error/type"))
            .or_else(|| parsed.pointer("/error/type"))
            .or_else(|| parsed.pointer("/code"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        return Some((event_type.to_string(), message, code));
    }

    None
}

fn extract_response_failed_error_message(chunk: &str) -> Option<String> {
    let mut response_failed_event_seen = false;

    for line in chunk.lines() {
        if let Some(event_name) = line.strip_prefix("event: ") {
            response_failed_event_seen = event_name.trim() == "response.failed";
            continue;
        }

        let Some(data_line) = line.strip_prefix("data: ") else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(data_line) else {
            continue;
        };

        let type_is_failed = parsed.get("type").and_then(|v| v.as_str()) == Some("response.failed");
        if !response_failed_event_seen && !type_is_failed {
            continue;
        }

        if let Some(message) = parsed
            .pointer("/response/error/message")
            .or_else(|| parsed.pointer("/error/message"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            return Some(message.to_string());
        }
    }

    None
}

fn is_sibling_tool_call_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("sibling tool call errored")
}

fn chunk_contains_sibling_tool_call_error(chunk: &str) -> bool {
    if let Some(message) = extract_response_failed_error_message(chunk) {
        return is_sibling_tool_call_error_message(&message);
    }
    chunk
        .to_ascii_lowercase()
        .contains("sibling tool call errored")
}

fn allow_sibling_tool_error_retry(
    decision: &StreamDecisionState,
    opts: StreamRuntimeOptions,
    has_serial_fallback: bool,
) -> bool {
    opts.enable_sibling_tool_error_retry && decision.allow_sibling_tool_retry(has_serial_fallback)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ToolLeakRetrySignal {
    dropped_leaked_marker_fragments: u64,
    dropped_raw_tool_json_fragments: u64,
    dropped_high_risk_raw_tool_json_fragments: u64,
    dropped_incomplete_tool_json_fragments: u64,
}

impl ToolLeakRetrySignal {
    fn total(self) -> u64 {
        self.dropped_leaked_marker_fragments
            + self.dropped_raw_tool_json_fragments
            + self.dropped_high_risk_raw_tool_json_fragments
            + self.dropped_incomplete_tool_json_fragments
    }
}

fn extract_tool_leak_retry_signal(summary: &Value) -> Option<ToolLeakRetrySignal> {
    if summary.get("type").and_then(|v| v.as_str()) != Some("codex_response_transform_summary") {
        return None;
    }

    let counters = summary.get("counters")?.as_object()?;
    let signal = ToolLeakRetrySignal {
        dropped_leaked_marker_fragments: counters
            .get("dropped_leaked_marker_fragments")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        dropped_raw_tool_json_fragments: counters
            .get("dropped_raw_tool_json_fragments")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        dropped_high_risk_raw_tool_json_fragments: counters
            .get("dropped_high_risk_raw_tool_json_fragments")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        dropped_incomplete_tool_json_fragments: counters
            .get("dropped_incomplete_tool_json_fragments")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    };

    if signal.total() == 0 {
        None
    } else {
        Some(signal)
    }
}

fn allow_leaked_tool_text_retry(
    decision: &StreamDecisionState,
    is_codex_stream: bool,
    has_serial_fallback: bool,
    signal: Option<ToolLeakRetrySignal>,
) -> bool {
    if signal
        .map(|s| s.dropped_high_risk_raw_tool_json_fragments > 0)
        .unwrap_or(false)
    {
        return false;
    }
    decision.allow_leaked_tool_retry(is_codex_stream, signal.is_some(), has_serial_fallback)
}

fn leaked_tool_text_retry_skip_reason(
    decision: &StreamDecisionState,
    is_codex_stream: bool,
    has_serial_fallback: bool,
    signal: Option<ToolLeakRetrySignal>,
) -> Option<&'static str> {
    if signal.is_none() {
        return None;
    }
    if allow_leaked_tool_text_retry(decision, is_codex_stream, has_serial_fallback, signal) {
        return None;
    }
    if !is_codex_stream {
        return Some("non_codex_stream");
    }
    if signal
        .map(|s| s.dropped_high_risk_raw_tool_json_fragments > 0)
        .unwrap_or(false)
    {
        return Some("high_risk_leak_requires_confirmation");
    }
    if decision.saw_response_failed {
        return Some("response_failed_seen");
    }
    if decision.saw_message_stop {
        return Some("message_stop_seen");
    }
    if decision.emitted_tool_event {
        return Some("tool_event_emitted");
    }
    if decision.leaked_tool_text_retry_attempted {
        return Some("already_attempted");
    }
    if !has_serial_fallback {
        return Some("serial_fallback_unavailable");
    }
    Some("guard_blocked")
}

fn sibling_tool_error_retry_skip_reason(
    decision: &StreamDecisionState,
    opts: StreamRuntimeOptions,
    has_serial_fallback: bool,
) -> Option<&'static str> {
    if !decision.saw_response_failed || !decision.saw_sibling_tool_call_error {
        return None;
    }
    if allow_sibling_tool_error_retry(decision, opts, has_serial_fallback) {
        return None;
    }

    if !opts.enable_sibling_tool_error_retry {
        return Some("feature_disabled");
    }
    if decision.saw_message_stop {
        return Some("message_stop_seen");
    }
    if decision.emitted_business_event {
        return Some("business_event_emitted");
    }
    if decision.sibling_tool_error_retry_attempted {
        return Some("already_attempted");
    }
    if !has_serial_fallback {
        return Some("serial_fallback_unavailable");
    }

    Some("guard_blocked")
}

fn extract_message_text(message: &Message) -> Option<String> {
    match &message.content {
        Some(MessageContent::Text(text)) => Some(text.clone()),
        Some(MessageContent::Blocks(blocks)) => {
            let text = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");

            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None => None,
    }
}

fn count_background_agent_completion_payloads_in_value(value: &Value) -> usize {
    match value {
        Value::Object(map) => map
            .get("kind")
            .and_then(Value::as_str)
            .map(|kind| usize::from(kind == "background_agent_completion"))
            .unwrap_or(0),
        Value::Array(items) => items
            .iter()
            .map(count_background_agent_completion_payloads_in_value)
            .sum(),
        _ => 0,
    }
}

fn count_background_agent_completion_markers_in_text(text: &str) -> usize {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }

    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
        let structured_count = count_background_agent_completion_payloads_in_value(&parsed);
        if structured_count > 0 {
            return structured_count;
        }
    }

    let rewritten_count = trimmed
        .matches("\"kind\":\"background_agent_completion\"")
        .count();
    if rewritten_count > 0 {
        return rewritten_count;
    }

    trimmed
        .matches("<task-notification>")
        .filter(|_| trimmed.contains("<status>completed</status>"))
        .count()
        .min(trimmed.matches("<summary>Agent \"").count())
}

fn count_background_agent_completion_markers_in_message(message: &Message) -> usize {
    match &message.content {
        Some(MessageContent::Text(text)) => count_background_agent_completion_markers_in_text(text),
        Some(MessageContent::Blocks(blocks)) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .map(count_background_agent_completion_markers_in_text)
            .sum(),
        None => 0,
    }
}

fn request_contains_background_agent_completion(request: &AnthropicRequest) -> bool {
    request
        .messages
        .iter()
        .any(|message| count_background_agent_completion_markers_in_message(message) > 0)
}

fn count_historical_background_agent_launches(request: &AnthropicRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role.eq_ignore_ascii_case("assistant"))
        .filter_map(|message| match &message.content {
            Some(MessageContent::Blocks(blocks)) => Some(blocks),
            _ => None,
        })
        .flat_map(|blocks| blocks.iter())
        .filter(|block| match block {
            ContentBlock::ToolUse { name, input, .. } => {
                name.eq_ignore_ascii_case("Agent")
                    && input
                        .get("run_in_background")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
            }
            _ => false,
        })
        .count()
}

fn count_terminal_background_agent_completions(request: &AnthropicRequest) -> usize {
    request
        .messages
        .iter()
        .map(count_background_agent_completion_markers_in_message)
        .sum()
}

fn extract_plan_file_path_from_text(text: &str) -> Option<String> {
    const PLAN_PREFIX: &str = "You should create your plan at ";
    const PLAN_SUFFIX: &str = " using the Write tool";

    for line in text.lines() {
        let trimmed = line.trim();
        let Some(start_idx) = trimmed.find(PLAN_PREFIX) else {
            continue;
        };
        let after_prefix = &trimmed[start_idx + PLAN_PREFIX.len()..];
        let candidate = after_prefix
            .split(PLAN_SUFFIX)
            .next()
            .map(str::trim)
            .filter(|value| value.starts_with('/') && value.ends_with(".md"))?;
        return Some(candidate.to_string());
    }

    None
}

fn extract_codex_plan_file_path_from_request(request: &AnthropicRequest) -> Option<String> {
    if let Some(system_text) = request.system.as_ref().map(|system| system.to_string()) {
        if let Some(path) = extract_plan_file_path_from_text(&system_text) {
            return Some(path);
        }
    }

    for message in &request.messages {
        if let Some(text) = extract_message_text(message) {
            if let Some(path) = extract_plan_file_path_from_text(&text) {
                return Some(path);
            }
        }
    }

    None
}

fn detect_probe_request(request: &AnthropicRequest) -> Option<&'static str> {
    if request.messages.len() != 1 {
        return None;
    }

    let message = request.messages.first()?;
    if !message.role.eq_ignore_ascii_case("user") {
        return None;
    }

    let text = extract_message_text(message)?.trim().to_ascii_lowercase();
    match text.as_str() {
        "foo" => Some("foo"),
        "count" => Some("count"),
        _ => None,
    }
}

fn build_probe_stream_payload(model: &str) -> String {
    let message_id = format!("msg_{}", Uuid::new_v4().simple());
    let mut payload = String::new();

    payload.push_str(&format!(
        "event: message_start\ndata: {}\n\n",
        json!({
            "type": "message_start",
            "message": {
                "id": message_id,
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": model,
                "stop_reason": Value::Null,
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }
        })
    ));

    payload.push_str(&format!(
        "event: content_block_start\ndata: {}\n\n",
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": { "type": "text", "text": "" }
        })
    ));

    payload.push_str(&format!(
        "event: content_block_delta\ndata: {}\n\n",
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "ok" }
        })
    ));

    payload.push_str(&format!(
        "event: content_block_stop\ndata: {}\n\n",
        json!({ "type": "content_block_stop", "index": 0 })
    ));

    payload.push_str(&format!(
        "event: message_delta\ndata: {}\n\n",
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn" },
            "usage": { "input_tokens": 0, "output_tokens": 1 }
        })
    ));

    payload.push_str(&format!(
        "event: message_stop\ndata: {}\n\n",
        json!({ "type": "message_stop", "stop_reason": "end_turn" })
    ));

    payload
}

fn build_probe_json_payload(model: &str) -> Value {
    json!({
        "id": format!("msg_{}", Uuid::new_v4().simple()),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{ "type": "text", "text": "ok" }],
        "stop_reason": "end_turn",
        "stop_sequence": Value::Null,
        "usage": { "input_tokens": 0, "output_tokens": 1 }
    })
}

fn parse_seconds_str(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    trimmed
        .trim_end_matches(['s', 'S'])
        .parse::<u64>()
        .ok()
        .filter(|secs| *secs > 0)
}

fn parse_json_seconds(v: &Value) -> Option<u64> {
    if let Some(u) = v.as_u64() {
        return Some(u).filter(|secs| *secs > 0);
    }
    if let Some(i) = v.as_i64() {
        return (i > 0).then_some(i as u64);
    }
    if let Some(s) = v.as_str() {
        return parse_seconds_str(s);
    }
    if let Some(f) = v.as_f64() {
        return (f.is_finite() && f > 0.0).then_some(f.ceil() as u64);
    }
    None
}

fn extract_cooldown_info(
    status: u16,
    error_text: &str,
    retry_after_header: &str,
    default_model: &str,
) -> Option<(String, u64, String)> {
    if status != StatusCode::TOO_MANY_REQUESTS.as_u16() {
        return None;
    }

    let retry_after_secs = parse_seconds_str(retry_after_header);
    let lower = error_text.to_ascii_lowercase();
    let parsed = serde_json::from_str::<Value>(error_text).ok();
    let error_obj = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .or(parsed.as_ref());

    let code = error_obj
        .and_then(|value| value.get("code"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let normalized_code = code
        .as_ref()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    let quota_signal = normalized_code.contains("quota")
        || normalized_code.contains("insufficient")
        || lower.contains("insufficient_quota")
        || lower.contains("quota exceeded")
        || lower.contains("out of credits")
        || lower.contains("insufficient balance")
        || lower.contains("billing")
        || lower.contains("额度")
        || lower.contains("余额")
        || lower.contains("欠费");

    if !quota_signal {
        return None;
    }

    let model = error_obj
        .and_then(|value| value.get("model"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| default_model.to_string());

    let seconds = error_obj
        .and_then(|value| value.get("reset_seconds"))
        .and_then(parse_json_seconds)
        .or_else(|| {
            error_obj
                .and_then(|value| value.get("reset_time"))
                .and_then(|value| value.as_str())
                .and_then(parse_seconds_str)
        })
        .or(retry_after_secs)?;

    let reason = code.unwrap_or_else(|| "retry_after".to_string());
    Some((model, seconds, reason))
}

fn get_active_cooldown_seconds(
    cooldowns: &Arc<Mutex<HashMap<String, Instant>>>,
    model: &str,
) -> Option<u64> {
    let mut map = cooldowns.lock().ok()?;
    let until = *map.get(model)?;
    let now = Instant::now();

    if until <= now {
        map.remove(model);
        return None;
    }

    let remaining = until.saturating_duration_since(now);
    Some(remaining.as_secs().max(1))
}

fn set_model_cooldown(cooldowns: &Arc<Mutex<HashMap<String, Instant>>>, model: &str, seconds: u64) {
    if seconds == 0 {
        return;
    }
    if let Ok(mut map) = cooldowns.lock() {
        map.insert(
            model.to_string(),
            Instant::now() + Duration::from_secs(seconds),
        );
    }
}

const ENDPOINT_PARALLEL_TOOL_DEGRADE_SECONDS: u64 = 300;

fn build_parallel_tool_degrade_key(
    route: Option<&ResolvedEndpoint>,
    target_url: &str,
    converter: &str,
    model: &str,
) -> String {
    if let Some(route) = route {
        return format!("lb:{}", route.route_key);
    }
    format!(
        "single:{}:{}:{}",
        converter.to_ascii_lowercase(),
        model.to_ascii_lowercase(),
        strip_query(target_url.to_string())
    )
}

fn get_parallel_tool_degrade_remaining_seconds(
    degrade_map: &Arc<Mutex<HashMap<String, Instant>>>,
    key: &str,
) -> Option<u64> {
    let mut map = degrade_map.lock().ok()?;
    let until = *map.get(key)?;
    let now = Instant::now();
    if until <= now {
        map.remove(key);
        return None;
    }
    Some(until.saturating_duration_since(now).as_secs().max(1))
}

fn mark_parallel_tool_degrade(
    degrade_map: &Arc<Mutex<HashMap<String, Instant>>>,
    key: &str,
    seconds: u64,
) {
    if seconds == 0 {
        return;
    }
    if let Ok(mut map) = degrade_map.lock() {
        map.insert(
            key.to_string(),
            Instant::now() + Duration::from_secs(seconds),
        );
    }
}

fn mark_parallel_tool_degrade_and_log(
    log_tx: &broadcast::Sender<String>,
    logger: &Option<Arc<AppLogger>>,
    request_id: &str,
    degrade_map: &Arc<Mutex<HashMap<String, Instant>>>,
    key: &str,
    reason: &str,
) {
    mark_parallel_tool_degrade(degrade_map, key, ENDPOINT_PARALLEL_TOOL_DEGRADE_SECONDS);
    emit_stream_diag(
        log_tx,
        logger,
        format!(
            "[Stream] #{} endpoint_parallel_tool_degrade_marked key={} reason={} ttl={}s",
            request_id, key, reason, ENDPOINT_PARALLEL_TOOL_DEGRADE_SECONDS
        ),
    );
}

fn strip_query(url: String) -> String {
    if let Some((head, _)) = url.split_once('?') {
        head.to_string()
    } else {
        url
    }
}

fn extract_url_path(url: &str) -> String {
    let clean = strip_query(url.to_string());
    if let Some((_, rest)) = clean.split_once("://") {
        if let Some((_, path)) = rest.split_once('/') {
            return format!("/{}", path);
        }
    }
    "/".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamOperation {
    Messages,
    CountTokens,
}

struct RouteSelection {
    target_url: String,
    api_key: String,
    converter: String,
    model_name: String,
    route: Option<ResolvedEndpoint>,
    route_permit: Option<EndpointPermit>,
    reasoning_effort_override: Option<ReasoningEffort>,
}

fn build_gemini_count_tokens_endpoint(target_url: &str, model: &str) -> String {
    if target_url.contains(":streamGenerateContent") || target_url.contains(":generateContent") {
        let endpoint = target_url
            .replace(":streamGenerateContent", ":countTokens")
            .replace(":generateContent", ":countTokens");
        return strip_query(endpoint);
    }

    if target_url.contains("{model}") {
        let endpoint = target_url.replace("{model}", model);
        if endpoint.contains(":countTokens") {
            return strip_query(endpoint);
        }
        if endpoint.contains(":streamGenerateContent") || endpoint.contains(":generateContent") {
            return strip_query(
                endpoint
                    .replace(":streamGenerateContent", ":countTokens")
                    .replace(":generateContent", ":countTokens"),
            );
        }
    }

    let base = target_url.trim_end_matches('/');
    format!("{}/v1beta/models/{}:countTokens", base, model)
}

fn build_gemini_messages_endpoint(target_url: &str, model: &str) -> String {
    if target_url.contains(":streamGenerateContent") {
        return target_url.to_string();
    }

    if target_url.contains("{model}") {
        let endpoint = target_url.replace("{model}", model);
        if endpoint.contains(":streamGenerateContent") {
            return endpoint;
        }
        if endpoint.contains(":generateContent") {
            return endpoint.replace(":generateContent", ":streamGenerateContent");
        }
    }

    if target_url.contains(":generateContent") {
        return target_url.replace(":generateContent", ":streamGenerateContent");
    }

    let base = target_url.trim_end_matches('/');
    format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
        base, model
    )
}

fn build_codex_input_tokens_endpoint(target_url: &str) -> String {
    let clean = strip_query(target_url.to_string());
    if let Some(idx) = clean.rfind("/responses") {
        let mut endpoint = clean;
        endpoint.replace_range(idx..idx + "/responses".len(), "/responses/input_tokens");
        return endpoint;
    }

    let base = clean.trim_end_matches('/');
    format!("{}/responses/input_tokens", base)
}

fn build_codex_messages_endpoint(target_url: &str) -> String {
    let endpoint = build_codex_input_tokens_endpoint(target_url);
    if let Some(idx) = endpoint.rfind("/responses/input_tokens") {
        let mut normalized = endpoint;
        normalized.replace_range(idx..idx + "/responses/input_tokens".len(), "/responses");
        return normalized;
    }
    endpoint
}

fn build_codex_endpoint_with_path_preference(
    target_url: &str,
    operation: UpstreamOperation,
    prefer_v1: bool,
) -> String {
    let legacy_endpoint = match operation {
        UpstreamOperation::Messages => build_codex_messages_endpoint(target_url),
        UpstreamOperation::CountTokens => build_codex_input_tokens_endpoint(target_url),
    };
    let desired_suffix = match operation {
        UpstreamOperation::Messages => {
            if prefer_v1 {
                "/v1/responses"
            } else {
                "/responses"
            }
        }
        UpstreamOperation::CountTokens => {
            if prefer_v1 {
                "/v1/responses/input_tokens"
            } else {
                "/responses/input_tokens"
            }
        }
    };
    let known_suffixes = [
        "/v1/responses/input_tokens",
        "/responses/input_tokens",
        "/v1/responses",
        "/responses",
    ];

    for suffix in known_suffixes {
        if let Some(idx) = legacy_endpoint.rfind(suffix) {
            let mut endpoint = legacy_endpoint.clone();
            endpoint.replace_range(idx..idx + suffix.len(), desired_suffix);
            return endpoint;
        }
    }

    let base = legacy_endpoint.trim_end_matches('/');
    if prefer_v1 {
        if base.ends_with("/v1") {
            format!("{}/{}", base, desired_suffix.trim_start_matches("/v1/"))
        } else {
            format!("{}/{}", base, desired_suffix.trim_start_matches('/'))
        }
    } else {
        format!("{}/{}", base, desired_suffix.trim_start_matches('/'))
    }
}

fn is_codex_v1_responses_path(url: &str) -> bool {
    let clean = strip_query(url.to_string());
    clean.contains("/v1/responses")
}

fn should_retry_codex_v1_path_with_legacy(status: u16, error_text: &str) -> bool {
    if status == StatusCode::NOT_FOUND.as_u16() {
        let lower = error_text.to_ascii_lowercase();
        return lower.contains("route ") && lower.contains(" not found")
            || lower.contains("route_not_found")
            || lower.contains("no route")
            || lower.contains("unknown route");
    }

    if status == StatusCode::BAD_REQUEST.as_u16()
        || status == StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    {
        let lower = error_text.to_ascii_lowercase();
        return lower.contains("unsupported")
            && (lower.contains("path")
                || lower.contains("endpoint")
                || lower.contains("url")
                || lower.contains("/v1/responses"));
    }

    false
}

fn should_retry_codex_fast_without_service_tier(status: u16, error_text: &str) -> bool {
    if status != StatusCode::BAD_REQUEST.as_u16()
        && status != StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    {
        return false;
    }

    let lower = error_text.to_ascii_lowercase();
    let mentions_service_tier = lower.contains("service_tier")
        || lower.contains("service tier")
        || (lower.contains("priority") && lower.contains("service"));
    if !mentions_service_tier {
        return false;
    }

    lower.contains("unsupported")
        || lower.contains("unknown parameter")
        || lower.contains("unknown field")
        || lower.contains("invalid field")
        || lower.contains("invalid value")
        || lower.contains("unrecognized")
        || lower.contains("additional properties are not allowed")
        || lower.contains("extra inputs are not permitted")
}

fn resolve_upstream_url_with_codex_path_preference(
    converter: &str,
    target_url: &str,
    operation: UpstreamOperation,
    model: &str,
    prefer_codex_v1_path: bool,
) -> String {
    if converter.eq_ignore_ascii_case("codex") {
        return build_codex_endpoint_with_path_preference(
            target_url,
            operation,
            prefer_codex_v1_path,
        );
    }

    resolve_upstream_url(converter, target_url, operation, model)
}

fn build_anthropic_messages_endpoint(target_url: &str) -> String {
    let clean = strip_query(target_url.to_string());

    if clean.contains("/messages/count_tokens") {
        if let Some(idx) = clean.rfind("/messages/count_tokens") {
            let mut endpoint = clean;
            endpoint.replace_range(idx..idx + "/messages/count_tokens".len(), "/messages");
            return endpoint;
        }
    }

    if clean.contains("/messages") {
        return clean;
    }

    if let Some(idx) = clean.rfind("/responses/input_tokens") {
        let mut endpoint = clean;
        endpoint.replace_range(idx..idx + "/responses/input_tokens".len(), "/messages");
        return endpoint;
    }

    if let Some(idx) = clean.rfind("/responses") {
        let mut endpoint = clean;
        endpoint.replace_range(idx..idx + "/responses".len(), "/messages");
        return endpoint;
    }

    let base = clean.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{}/messages", base)
    } else {
        format!("{}/v1/messages", base)
    }
}

fn build_anthropic_count_tokens_endpoint(target_url: &str) -> String {
    let messages_endpoint = build_anthropic_messages_endpoint(target_url);
    if let Some(idx) = messages_endpoint.rfind("/messages") {
        let mut endpoint = messages_endpoint;
        endpoint.replace_range(idx..idx + "/messages".len(), "/messages/count_tokens");
        return endpoint;
    }

    let base = messages_endpoint.trim_end_matches('/');
    format!("{}/messages/count_tokens", base)
}

fn build_openai_messages_endpoint(target_url: &str) -> String {
    if target_url.contains("/chat/completions") || target_url.contains("openai.azure.com") {
        return target_url.to_string();
    }

    let clean = strip_query(target_url.to_string());
    let base = clean.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else {
        format!("{}/v1/chat/completions", base)
    }
}

fn resolve_upstream_url(
    converter: &str,
    target_url: &str,
    operation: UpstreamOperation,
    model: &str,
) -> String {
    if converter.eq_ignore_ascii_case("anthropic") {
        return match operation {
            UpstreamOperation::Messages => build_anthropic_messages_endpoint(target_url),
            UpstreamOperation::CountTokens => build_anthropic_count_tokens_endpoint(target_url),
        };
    }

    if converter.eq_ignore_ascii_case("gemini") {
        return match operation {
            UpstreamOperation::Messages => build_gemini_messages_endpoint(target_url, model),
            UpstreamOperation::CountTokens => build_gemini_count_tokens_endpoint(target_url, model),
        };
    }

    if converter.eq_ignore_ascii_case("openai") {
        return match operation {
            UpstreamOperation::Messages => build_openai_messages_endpoint(target_url),
            UpstreamOperation::CountTokens => build_openai_messages_endpoint(target_url),
        };
    }

    match operation {
        UpstreamOperation::Messages => build_codex_messages_endpoint(target_url),
        UpstreamOperation::CountTokens => build_codex_input_tokens_endpoint(target_url),
    }
}

fn resolve_route_selection(
    request_id: &str,
    input_model: &str,
    input_slot: ModelSlot,
    target_url: &str,
    final_api_key: &str,
    ctx: &TransformContext,
    load_balancer_runtime: Option<&LoadBalancerRuntime>,
    log_tx: &broadcast::Sender<String>,
) -> Result<RouteSelection, Response<BoxBody<Bytes, Infallible>>> {
    let mut resolved_target_url = target_url.to_string();
    let mut resolved_api_key = final_api_key.to_string();
    let mut request_converter = ctx.converter.clone();
    let mut selected_lb_route: Option<ResolvedEndpoint> = None;
    let mut lb_permit: Option<EndpointPermit> = None;
    let mut request_reasoning_effort_override: Option<ReasoningEffort> = None;

    let mut model_name = resolve_model_for_converter(
        &request_converter,
        input_model,
        &ctx.reasoning_mapping,
        &ctx.codex_model_mapping,
        &ctx.anthropic_model_mapping,
        &ctx.openai_model_mapping,
        &ctx.gemini_reasoning_effort,
    );

    if let Some(runtime) = load_balancer_runtime {
        if let Some((resolved, permit)) = runtime.resolve_and_acquire(input_model) {
            resolved_target_url = resolved.target_url.clone();
            if let Some(key) = resolved.api_key.clone() {
                resolved_api_key = key;
            }

            request_converter = resolved.converter.clone();
            if let Some(overridden_model) = resolved.model.clone() {
                model_name = overridden_model;
            } else {
                model_name = resolve_model_for_converter(
                    &request_converter,
                    input_model,
                    &ctx.reasoning_mapping,
                    &ctx.codex_model_mapping,
                    &ctx.anthropic_model_mapping,
                    &ctx.openai_model_mapping,
                    &ctx.gemini_reasoning_effort,
                );
            }

            if let Some(custom_effort) = resolved.reasoning_effort.clone() {
                request_reasoning_effort_override = Some(ReasoningEffort::from_str(&custom_effort));
            }

            selected_lb_route = Some(resolved);
            lb_permit = Some(permit);
        } else {
            let _ = log_tx.send(format!(
                "[Warn] #{} lb_unavailable slot={} model={} reason=no_available_candidate",
                request_id,
                input_slot.as_str(),
                input_model,
            ));

            return Err(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header("Content-Type", "application/json")
                .body(full_body(
                    json!({
                        "error": {
                            "type": "service_unavailable",
                            "message": format!(
                                "No available upstream route in slot '{}' for model '{}'",
                                input_slot.as_str(),
                                input_model
                            )
                        }
                    })
                    .to_string(),
                ))
                .unwrap());
        }
    }

    Ok(RouteSelection {
        target_url: resolved_target_url,
        api_key: resolved_api_key,
        converter: request_converter,
        model_name,
        route: selected_lb_route,
        route_permit: lb_permit,
        reasoning_effort_override: request_reasoning_effort_override,
    })
}

fn parse_input_tokens(value: &Value) -> Option<u64> {
    value
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| value.get("inputTokens").and_then(|v| v.as_u64()))
        .or_else(|| value.get("totalTokens").and_then(|v| v.as_u64()))
        .or_else(|| value.get("total_tokens").and_then(|v| v.as_u64()))
        .or_else(|| {
            value
                .get("usage")
                .and_then(|usage| usage.get("input_tokens"))
                .and_then(|v| v.as_u64())
        })
}

fn estimate_input_tokens(request: &AnthropicRequest) -> u64 {
    let mut chars = 0usize;

    if let Some(system) = &request.system {
        chars += system.to_string().chars().count();
    }

    for message in &request.messages {
        chars += message.role.chars().count();
        match &message.content {
            Some(MessageContent::Text(text)) => {
                chars += text.chars().count();
            }
            Some(MessageContent::Blocks(blocks)) => {
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => chars += text.chars().count(),
                        ContentBlock::Thinking { thinking, .. } => {
                            chars += thinking.chars().count()
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            chars += name.chars().count();
                            chars += serde_json::to_string(input)
                                .unwrap_or_default()
                                .chars()
                                .count();
                        }
                        ContentBlock::ToolResult { content, .. } => {
                            chars += content
                                .as_ref()
                                .map(|v| {
                                    serde_json::to_string(v).unwrap_or_default().chars().count()
                                })
                                .unwrap_or(0);
                        }
                        ContentBlock::Image { .. }
                        | ContentBlock::ImageUrl { .. }
                        | ContentBlock::InputImage { .. }
                        | ContentBlock::Document { .. }
                        | ContentBlock::OtherValue(_) => {
                            chars += 64;
                        }
                    }
                }
            }
            None => {}
        }
    }

    if let Some(tools) = &request.tools {
        chars += serde_json::to_string(tools)
            .unwrap_or_default()
            .chars()
            .count();
    }

    ((chars as f64) / 4.0).ceil() as u64
}

impl ProxyServer {
    pub fn new(port: u16, target_url: String, api_key: Option<String>) -> Self {
        Self {
            port,
            allow_external_access: false,
            target_url,
            api_key,
            reasoning_mapping: ReasoningEffortMapping::default(),
            custom_injection_prompt: String::new(),
            converter: "codex".to_string(),
            codex_model: "gpt-5.3-codex".to_string(),
            codex_model_mapping: CodexModelMapping::default(),
            anthropic_model_mapping: AnthropicModelMapping::default(),
            openai_model_mapping: OpenAIModelMapping::default(),
            openai_max_tokens_mapping: OpenAIMaxTokensMapping::default(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            max_concurrency: 0,
            ignore_probe_requests: false,
            allow_count_tokens_fallback_estimate: true,
            enable_codex_fast_mode: true,
            force_stream_for_codex: true,
            enable_sse_frame_parser: true,
            enable_stream_heartbeat: true,
            stream_heartbeat_interval_ms: 3_000,
            enable_stream_log_sampling: true,
            stream_log_sample_every_n: 20,
            stream_log_max_chars: 512,
            enable_stream_metrics: true,
            enable_stream_event_metrics: true,
            stream_silence_warn_ms: 20_000,
            stream_silence_error_ms: 90_000,
            enable_stall_retry: false,
            stall_timeout_ms: 300_000,
            stall_retry_max_attempts: 0,
            stall_retry_only_heartbeat_phase: false,
            enable_empty_completion_retry: false,
            empty_completion_retry_max_attempts: 0,
            enable_incomplete_stream_retry: true,
            incomplete_stream_retry_max_attempts: 2,
            enable_sibling_tool_error_retry: true,
            prefer_codex_v1_path: true,
            enable_codex_tool_schema_compaction: true,
            enable_skill_routing_hint: false,
            enable_stateful_responses_chain: true,
            load_balancer_runtime: None,
        }
    }

    pub fn with_reasoning_mapping(mut self, mapping: ReasoningEffortMapping) -> Self {
        self.reasoning_mapping = mapping;
        self
    }

    pub fn with_custom_injection_prompt(mut self, prompt: String) -> Self {
        self.custom_injection_prompt = prompt;
        self
    }

    pub fn with_converter(mut self, converter: String) -> Self {
        self.converter = converter;
        self
    }

    pub fn with_codex_model(mut self, model: String) -> Self {
        self.codex_model = model;
        self
    }

    pub fn with_codex_model_mapping(mut self, mapping: CodexModelMapping) -> Self {
        self.codex_model_mapping = mapping;
        self
    }

    pub fn with_anthropic_model_mapping(mut self, mapping: AnthropicModelMapping) -> Self {
        self.anthropic_model_mapping = mapping;
        self
    }

    pub fn with_openai_model_mapping(mut self, mapping: OpenAIModelMapping) -> Self {
        self.openai_model_mapping = mapping;
        self
    }

    pub fn with_openai_max_tokens_mapping(mut self, mapping: OpenAIMaxTokensMapping) -> Self {
        self.openai_max_tokens_mapping = mapping;
        self
    }

    pub fn with_gemini_reasoning_effort(mut self, effort: GeminiReasoningEffortMapping) -> Self {
        self.gemini_reasoning_effort = effort;
        self
    }

    pub fn with_max_concurrency(mut self, max: u32) -> Self {
        self.max_concurrency = max;
        self
    }

    pub fn with_allow_external_access(mut self, allow: bool) -> Self {
        self.allow_external_access = allow;
        self
    }

    pub fn with_ignore_probe_requests(mut self, ignore: bool) -> Self {
        self.ignore_probe_requests = ignore;
        self
    }

    pub fn with_allow_count_tokens_fallback_estimate(mut self, allow: bool) -> Self {
        self.allow_count_tokens_fallback_estimate = allow;
        self
    }

    pub fn with_enable_codex_fast_mode(mut self, enable: bool) -> Self {
        self.enable_codex_fast_mode = enable;
        self
    }

    pub fn with_force_stream_for_codex(mut self, enable: bool) -> Self {
        self.force_stream_for_codex = enable;
        self
    }

    pub fn with_enable_sse_frame_parser(mut self, enable: bool) -> Self {
        self.enable_sse_frame_parser = enable;
        self
    }

    pub fn with_enable_stream_heartbeat(mut self, enable: bool) -> Self {
        self.enable_stream_heartbeat = enable;
        self
    }

    pub fn with_stream_heartbeat_interval_ms(mut self, interval_ms: u64) -> Self {
        self.stream_heartbeat_interval_ms = interval_ms;
        self
    }

    pub fn with_enable_stream_log_sampling(mut self, enable: bool) -> Self {
        self.enable_stream_log_sampling = enable;
        self
    }

    pub fn with_stream_log_sample_every_n(mut self, every_n: u32) -> Self {
        self.stream_log_sample_every_n = every_n;
        self
    }

    pub fn with_stream_log_max_chars(mut self, max_chars: usize) -> Self {
        self.stream_log_max_chars = max_chars;
        self
    }

    pub fn with_enable_stream_metrics(mut self, enable: bool) -> Self {
        self.enable_stream_metrics = enable;
        self
    }

    pub fn with_enable_stream_event_metrics(mut self, enable: bool) -> Self {
        self.enable_stream_event_metrics = enable;
        self
    }

    pub fn with_stream_silence_warn_ms(mut self, warn_ms: u64) -> Self {
        self.stream_silence_warn_ms = warn_ms;
        self
    }

    pub fn with_stream_silence_error_ms(mut self, error_ms: u64) -> Self {
        self.stream_silence_error_ms = error_ms;
        self
    }

    pub fn with_enable_stall_retry(mut self, enable: bool) -> Self {
        self.enable_stall_retry = enable;
        self
    }

    pub fn with_stall_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.stall_timeout_ms = timeout_ms;
        self
    }

    pub fn with_stall_retry_max_attempts(mut self, attempts: u32) -> Self {
        self.stall_retry_max_attempts = attempts;
        self
    }

    pub fn with_stall_retry_only_heartbeat_phase(mut self, enable: bool) -> Self {
        self.stall_retry_only_heartbeat_phase = enable;
        self
    }

    pub fn with_enable_empty_completion_retry(mut self, enable: bool) -> Self {
        self.enable_empty_completion_retry = enable;
        self
    }

    pub fn with_empty_completion_retry_max_attempts(mut self, attempts: u32) -> Self {
        self.empty_completion_retry_max_attempts = attempts;
        self
    }

    pub fn with_enable_incomplete_stream_retry(mut self, enable: bool) -> Self {
        self.enable_incomplete_stream_retry = enable;
        self
    }

    pub fn with_incomplete_stream_retry_max_attempts(mut self, attempts: u32) -> Self {
        self.incomplete_stream_retry_max_attempts = attempts;
        self
    }

    pub fn with_enable_sibling_tool_error_retry(mut self, enable: bool) -> Self {
        self.enable_sibling_tool_error_retry = enable;
        self
    }

    pub fn with_prefer_codex_v1_path(mut self, enable: bool) -> Self {
        self.prefer_codex_v1_path = enable;
        self
    }

    pub fn with_enable_codex_tool_schema_compaction(mut self, enable: bool) -> Self {
        self.enable_codex_tool_schema_compaction = enable;
        self
    }

    pub fn with_enable_skill_routing_hint(mut self, enable: bool) -> Self {
        self.enable_skill_routing_hint = enable;
        self
    }

    pub fn with_enable_stateful_responses_chain(mut self, enable: bool) -> Self {
        self.enable_stateful_responses_chain = enable;
        self
    }

    pub fn with_load_balancer_runtime(mut self, runtime: LoadBalancerRuntime) -> Self {
        self.load_balancer_runtime = Some(runtime);
        self
    }

    fn runtime_update(&self) -> RuntimeConfigUpdate {
        RuntimeConfigUpdate {
            target_url: self.target_url.clone(),
            api_key: self.api_key.clone(),
            ctx: TransformContext {
                reasoning_mapping: self.reasoning_mapping.clone(),
                codex_model_mapping: self.codex_model_mapping.clone(),
                anthropic_model_mapping: self.anthropic_model_mapping.clone(),
                openai_model_mapping: self.openai_model_mapping.clone(),
                openai_max_tokens_mapping: self.openai_max_tokens_mapping.clone(),
                custom_injection_prompt: self.custom_injection_prompt.clone(),
                converter: self.converter.clone(),
                codex_model: self.codex_model.clone(),
                gemini_reasoning_effort: self.gemini_reasoning_effort.clone(),
                enable_codex_tool_schema_compaction: self.enable_codex_tool_schema_compaction,
                enable_codex_fast_mode: self.enable_codex_fast_mode,
                enable_skill_routing_hint: self.enable_skill_routing_hint,
            },
            ignore_probe_requests: self.ignore_probe_requests,
            allow_count_tokens_fallback_estimate: self.allow_count_tokens_fallback_estimate,
            enable_codex_fast_mode: self.enable_codex_fast_mode,
            force_stream_for_codex: self.force_stream_for_codex,
            enable_sse_frame_parser: self.enable_sse_frame_parser,
            enable_stream_heartbeat: self.enable_stream_heartbeat,
            stream_heartbeat_interval_ms: self.stream_heartbeat_interval_ms,
            enable_stream_log_sampling: self.enable_stream_log_sampling,
            stream_log_sample_every_n: self.stream_log_sample_every_n,
            stream_log_max_chars: self.stream_log_max_chars,
            enable_stream_metrics: self.enable_stream_metrics,
            enable_stream_event_metrics: self.enable_stream_event_metrics,
            stream_silence_warn_ms: self.stream_silence_warn_ms,
            stream_silence_error_ms: self.stream_silence_error_ms,
            enable_stall_retry: self.enable_stall_retry,
            stall_timeout_ms: self.stall_timeout_ms,
            stall_retry_max_attempts: self.stall_retry_max_attempts,
            stall_retry_only_heartbeat_phase: self.stall_retry_only_heartbeat_phase,
            enable_empty_completion_retry: self.enable_empty_completion_retry,
            empty_completion_retry_max_attempts: self.empty_completion_retry_max_attempts,
            enable_incomplete_stream_retry: self.enable_incomplete_stream_retry,
            incomplete_stream_retry_max_attempts: self.incomplete_stream_retry_max_attempts,
            enable_sibling_tool_error_retry: self.enable_sibling_tool_error_retry,
            prefer_codex_v1_path: self.prefer_codex_v1_path,
            enable_codex_tool_schema_compaction: self.enable_codex_tool_schema_compaction,
            enable_skill_routing_hint: self.enable_skill_routing_hint,
            enable_stateful_responses_chain: self.enable_stateful_responses_chain,
            load_balancer_runtime: self.load_balancer_runtime.clone(),
        }
    }

    /// Start the proxy server and return a shutdown sender + JoinHandle
    /// Send () to the returned sender to stop the server
    pub async fn start(
        &self,
        log_tx: broadcast::Sender<String>,
    ) -> Result<
        (
            broadcast::Sender<()>,
            tokio::task::JoinHandle<()>,
            ProxyRuntimeHandle,
        ),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // 初始化全局日志记录器
        let logger = AppLogger::init(None);
        logger.log("=== Codex Proxy Started ===");

        let addr = if self.allow_external_access {
            SocketAddr::from(([0, 0, 0, 0], self.port))
        } else {
            SocketAddr::from(([127, 0, 0, 1], self.port))
        };
        let listener = TcpListener::bind(addr).await?;

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_tx_clone = shutdown_tx.clone();

        let model_cooldowns: Arc<Mutex<HashMap<String, Instant>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let parallel_tool_degrade_until: Arc<Mutex<HashMap<String, Instant>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let stateful_chain_store: StatefulChainStore = Arc::new(Mutex::new(HashMap::new()));
        let stateful_chain_unsupported_endpoints: StatefulChainUnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        let codex_v1_unsupported_endpoints: CodexV1UnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        let codex_fast_unsupported_endpoints: CodexFastUnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        let runtime_handle = ProxyRuntimeHandle {
            state: Arc::new(RwLock::new(RuntimeConfigState::from(self.runtime_update()))),
        };
        let runtime_handle_for_server = runtime_handle.clone();

        // 并发控制：0 = 不限制
        let semaphore: Option<Arc<Semaphore>> = if self.max_concurrency > 0 {
            let _ = log_tx.send(format!(
                "[System] Max concurrency: {}",
                self.max_concurrency
            ));
            Some(Arc::new(Semaphore::new(self.max_concurrency as usize)))
        } else {
            None
        };

        let http_client = Arc::new(
            reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .build()
                .unwrap(),
        );

        let listen_host = if self.allow_external_access {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        let _ = log_tx.send(format!(
            "[System] Init success: Codex Proxy (Rust) listening on http://{}:{}",
            listen_host, self.port
        ));
        let _ = log_tx.send(format!("[System] Target: {}", self.target_url));
        logger.log(&format!(
            "Listening on http://{}:{}",
            listen_host, self.port
        ));
        logger.log(&format!("Target: {}", self.target_url));
        let listen_port = self.port;

        // Spawn the server loop in a separate task
        let handle = tokio::spawn(async move {
            let mut conn_tasks = tokio::task::JoinSet::new();

            loop {
                let mut shutdown_rx = shutdown_tx_clone.subscribe();

                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, peer_addr)) => {
                                let io = TokioIo::new(stream);
                                let http_client = Arc::clone(&http_client);
                                let semaphore = semaphore.clone();
                                let model_cooldowns = Arc::clone(&model_cooldowns);
                                let parallel_tool_degrade_until =
                                    Arc::clone(&parallel_tool_degrade_until);
                                let stateful_chain_store = Arc::clone(&stateful_chain_store);
                                let stateful_chain_unsupported_endpoints =
                                    Arc::clone(&stateful_chain_unsupported_endpoints);
                                let codex_v1_unsupported_endpoints =
                                    Arc::clone(&codex_v1_unsupported_endpoints);
                                let codex_fast_unsupported_endpoints =
                                    Arc::clone(&codex_fast_unsupported_endpoints);
                                let runtime_handle = runtime_handle_for_server.clone();
                                let log_tx = log_tx.clone();
                                let log_tx_for_request = log_tx.clone();
                                let logger_for_conn = logger.clone();
                                let conn_id: String = Uuid::new_v4()
                                    .simple()
                                    .to_string()
                                    .chars()
                                    .take(8)
                                    .collect();

                                conn_tasks.spawn(async move {
                                    let service = service_fn(move |req| {
                                        handle_request(
                                            req,
                                            runtime_handle.clone(),
                                            Arc::clone(&http_client),
                                            semaphore.clone(),
                                            Arc::clone(&model_cooldowns),
                                            Arc::clone(&parallel_tool_degrade_until),
                                            Arc::clone(&stateful_chain_store),
                                            Arc::clone(&stateful_chain_unsupported_endpoints),
                                            Arc::clone(&codex_v1_unsupported_endpoints),
                                            Arc::clone(&codex_fast_unsupported_endpoints),
                                            log_tx_for_request.clone(),
                                        )
                                    });

                                    if let Err(e) = http1::Builder::new()
                                        .serve_connection(io, service)
                                        .await
                                    {
                                        emit_connection_error_diag(
                                            &log_tx,
                                            &logger_for_conn,
                                            &conn_id,
                                            &peer_addr,
                                            listen_port,
                                            &e,
                                        );
                                        let class = classify_connection_error(
                                            &e.to_string(),
                                            &format!("{e:?}"),
                                            &collect_error_source_chain(&e),
                                        );
                                        if matches!(class, ConnErrorClass::ProtocolOrNetwork) {
                                            eprintln!(
                                                "Connection error [{} {}] peer={} local_port={}: {}",
                                                conn_id,
                                                class.as_str(),
                                                peer_addr,
                                                listen_port,
                                                e
                                            );
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                let _ = log_tx.send(format!("[Error] Accept failed: {}", e));
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        let _ = log_tx.send("[System] Proxy server shutting down, aborting all connections...".to_string());
                        conn_tasks.abort_all();
                        break;
                    }
                }
            }
        });

        Ok((shutdown_tx, handle, runtime_handle))
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    runtime_handle: ProxyRuntimeHandle,
    http_client: Arc<reqwest::Client>,
    semaphore: Option<Arc<Semaphore>>,
    model_cooldowns: Arc<Mutex<HashMap<String, Instant>>>,
    parallel_tool_degrade_until: Arc<Mutex<HashMap<String, Instant>>>,
    stateful_chain_store: StatefulChainStore,
    stateful_chain_unsupported_endpoints: StatefulChainUnsupportedEndpointStore,
    codex_v1_unsupported_endpoints: CodexV1UnsupportedEndpointStore,
    codex_fast_unsupported_endpoints: CodexFastUnsupportedEndpointStore,
    log_tx: broadcast::Sender<String>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let path = req.uri().path().to_string();
    let normalized_path = path.trim_end_matches('/');
    let method = req.method();
    let request_id: String = Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect();
    let request_started_at = Instant::now();

    // 处理新增的 GET 路由
    if method == Method::GET {
        match normalized_path {
            "/v1/models" => {
                let _ = log_tx.send(format!(
                    "[System] Processing #{} GET /v1/models",
                    request_id
                ));
                return handle_models_list();
            }
            path if path.starts_with("/v1/models/") => {
                let model_id = &path[11..]; // 去掉 "/v1/models/" 前缀
                let _ = log_tx.send(format!(
                    "[System] Processing #{} GET /v1/models/{}",
                    request_id, model_id
                ));
                return handle_model_detail(model_id);
            }
            "/health" | "/" => {
                let _ = log_tx.send(format!(
                    "[System] Processing #{} GET {}",
                    request_id, normalized_path
                ));
                return handle_health_check();
            }
            _ => {
                // 继续到 404 处理
            }
        }
    }

    // 处理 OPTIONS 请求（CORS 预检）
    if method == Method::OPTIONS {
        let _ = log_tx.send(format!(
            "[System] Processing #{} OPTIONS {}",
            request_id, normalized_path
        ));
        return handle_cors_preflight();
    }

    let is_messages = normalized_path == "/messages" || normalized_path == "/v1/messages";
    let is_count_tokens = normalized_path == "/messages/count_tokens"
        || normalized_path == "/v1/messages/count_tokens";

    let runtime_state = runtime_handle.snapshot();
    let stream_opts = StreamRuntimeOptions::from_state(&runtime_state);
    let target_url = runtime_state.target_url;
    let api_key = runtime_state.api_key;
    let mut ctx = runtime_state.ctx;
    // Keep transformer compaction behavior aligned with runtime switches even if context was stale.
    ctx.enable_codex_tool_schema_compaction = runtime_state.enable_codex_tool_schema_compaction;
    ctx.enable_codex_fast_mode = runtime_state.enable_codex_fast_mode;
    ctx.enable_skill_routing_hint = runtime_state.enable_skill_routing_hint;
    let ignore_probe_requests = runtime_state.ignore_probe_requests;
    let allow_count_tokens_fallback_estimate = runtime_state.allow_count_tokens_fallback_estimate;
    let prefer_codex_v1_path = runtime_state.prefer_codex_v1_path;
    let enable_stateful_responses_chain = runtime_state.enable_stateful_responses_chain;
    let load_balancer_runtime = runtime_state.load_balancer_runtime;

    // 只处理 POST /messages、/v1/messages、/messages/count_tokens、/v1/messages/count_tokens
    if method != Method::POST || (!is_messages && !is_count_tokens) {
        let _ = log_tx.send(format!("[Debug] Ignored {} request to {}", method, path));
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(full_body(
                json!({"error": {"type": "not_found", "message": "Not found"}}).to_string(),
            ))
            .unwrap());
    }

    let _ = log_tx.send(format!(
        "[System] Processing #{} {} {}",
        request_id,
        req.method(),
        path
    ));

    // 并发控制：获取许可证，FIFO 排队
    let permit: Option<OwnedSemaphorePermit> = if let Some(ref sem) = semaphore {
        let _ = log_tx.send(format!(
            "[System] #{} waiting for concurrency permit (available: {})",
            request_id,
            sem.available_permits(),
        ));
        Some(
            Arc::clone(sem)
                .acquire_owned()
                .await
                .expect("semaphore closed"),
        )
    } else {
        None
    };

    // 获取认证信息
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let x_api_key = req
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let anthropic_version = req
        .headers()
        .get("x-anthropic-version")
        .or_else(|| req.headers().get("anthropic-version"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("2023-06-01")
        .to_string();

    let anthropic_beta = req
        .headers()
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .map(|value| value.to_string());

    let accept_header = req
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let stateful_chain_hint_header = extract_stateful_chain_hint(&req);

    // 确定最终使用的 API key
    let final_api_key = if let Some(key) = api_key.clone() {
        // 环境变量配置的 key 优先
        Some(key)
    } else {
        x_api_key.clone().or_else(|| {
            auth_header.as_ref().and_then(|h| {
                h.strip_prefix("Bearer ")
                    .or_else(|| h.strip_prefix("bearer "))
                    .map(|s| s.to_string())
            })
        })
    };

    let Some(final_api_key) = final_api_key else {
        return Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("Content-Type", "application/json")
            .body(full_body(
                json!({"error": {"type": "unauthorized", "message": "Missing API key"}})
                    .to_string(),
            ))
            .unwrap());
    };

    // 读取请求体
    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(full_body(
                    json!({"error": {"message": format!("Failed to read body: {}", e)}})
                        .to_string(),
                ))
                .unwrap());
        }
    };

    // 先保留原始 JSON（anthropic 透传时直接转发，避免结构体二次序列化改变字段形态）
    let raw_request_body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(body) => body,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(full_body(
                    json!({"error": {"message": format!("Invalid JSON: {}", e)}}).to_string(),
                ))
                .unwrap());
        }
    };

    // 再解析为结构体用于日志统计、模型路由等逻辑
    let anthropic_body: AnthropicRequest =
        match serde_json::from_value(raw_request_body.clone()) {
            Ok(body) => body,
            Err(e) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("Content-Type", "application/json")
                    .body(full_body(
                        json!({"error": {"message": format!("Invalid JSON: {}", e)}}).to_string(),
                    ))
                    .unwrap());
            }
        };

    let stateful_chain_hint_info =
        resolve_stateful_chain_hint_info(stateful_chain_hint_header, &anthropic_body);
    let stateful_chain_hint = stateful_chain_hint_info.value.clone();

    if is_messages && ignore_probe_requests {
        if let Some(probe_kind) = detect_probe_request(&anthropic_body) {
            let probe_model = anthropic_body
                .model
                .as_deref()
                .unwrap_or("claude-3-5-sonnet-20240620");
            let _ = log_tx.send(format!(
                "[Probe] #{} locally_ignored kind={} stream={} tools={}",
                request_id,
                probe_kind,
                anthropic_body.stream,
                anthropic_body
                    .tools
                    .as_ref()
                    .map(|tools| tools.len())
                    .unwrap_or(0),
            ));

            if anthropic_body.stream {
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/event-stream")
                    .header("Cache-Control", "no-cache")
                    .body(full_body(build_probe_stream_payload(probe_model)))
                    .unwrap());
            }

            let payload = build_probe_json_payload(probe_model);
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(full_body(payload.to_string()))
                .unwrap());
        }
    }

    let input_model_owned = anthropic_body
        .model
        .as_deref()
        .unwrap_or("claude-3-5-sonnet-20240620")
        .to_string();
    let input_model = input_model_owned.as_str();
    let input_slot = ModelSlot::from_model_name(&input_model);

    // count_tokens 请求不计入统计
    if !is_count_tokens {
        if let Some(family) = detect_model_family(input_model) {
            let _ = log_tx.send(format!("[Stat] model_request:{}", family));
        }
    }

    let display_summary = summarize_request_messages(&anthropic_body.messages);
    let tool_count = anthropic_body
        .tools
        .as_ref()
        .map(|tools| tools.len())
        .unwrap_or(0);
    let system_chars = anthropic_body
        .system
        .as_ref()
        .map(|system| system.to_string().chars().count())
        .unwrap_or(0);
    let response_transform_request_ctx = ResponseTransformRequestContext {
        codex_plan_file_path: extract_codex_plan_file_path_from_request(&anthropic_body),
        contains_background_agent_completion: request_contains_background_agent_completion(
            &anthropic_body,
        ),
        historical_background_agent_launch_count: count_historical_background_agent_launches(
            &anthropic_body,
        ),
        terminal_background_agent_completion_count: count_terminal_background_agent_completions(
            &anthropic_body,
        ),
    };
    let logger = AppLogger::get();

    if is_count_tokens {
        let route_selection = match resolve_route_selection(
            &request_id,
            input_model,
            input_slot,
            &target_url,
            &final_api_key,
            &ctx,
            load_balancer_runtime.as_ref(),
            &log_tx,
        ) {
            Ok(selection) => selection,
            Err(response) => return Ok(response),
        };

        let route_mode = if route_selection.route.is_some() {
            "lb"
        } else {
            "single"
        };
        let route_slot = route_selection
            .route
            .as_ref()
            .map(|route| route.slot.as_str())
            .unwrap_or(input_slot.as_str());
        let route_endpoint = route_selection
            .route
            .as_ref()
            .map(|route| route.endpoint_id.as_str())
            .unwrap_or("single");
        let route_key = route_selection
            .route
            .as_ref()
            .map(|route| route.route_key.as_str())
            .unwrap_or("-");

        let _ = log_tx.send(format!(
            "[Req] #{} mode=count_tokens converter={} in={} out={}",
            request_id, route_selection.converter, input_model, route_selection.model_name,
        ));

        let unified_count_request = UnifiedChatRequest::from_anthropic(&anthropic_body);
        let mut token_count: Option<u64> = None;
        let mut upstream_status: Option<u16> = None;
        let mut source = "estimate".to_string();
        let mut count_tokens_ctx = ctx.clone();
        if route_selection.converter.eq_ignore_ascii_case("codex") {
            if let Some(override_effort) = route_selection.reasoning_effort_override {
                count_tokens_ctx.reasoning_mapping = ReasoningEffortMapping::new()
                    .with_opus(override_effort)
                    .with_sonnet(override_effort)
                    .with_haiku(override_effort);
            }
        }
        let codex_v1_endpoint_key = if route_selection.converter.eq_ignore_ascii_case("codex") {
            Some(build_codex_v1_endpoint_key(
                &route_selection.converter,
                &route_selection.target_url,
                &route_selection.api_key,
            ))
        } else {
            None
        };
        let prefer_codex_v1_path_for_route =
            route_selection.converter.eq_ignore_ascii_case("codex")
                && prefer_codex_v1_path
                && codex_v1_endpoint_key.as_ref().map_or(true, |key| {
                    !is_codex_v1_endpoint_unsupported(&codex_v1_unsupported_endpoints, key)
                });
        if route_selection.converter.eq_ignore_ascii_case("codex")
            && prefer_codex_v1_path
            && !prefer_codex_v1_path_for_route
        {
            let _ = log_tx.send(format!(
                "[Route] #{} codex_v1_preferred=false reason=endpoint_cached_unsupported",
                request_id
            ));
        }
        let count_tokens_endpoint = resolve_upstream_url_with_codex_path_preference(
            &route_selection.converter,
            &route_selection.target_url,
            UpstreamOperation::CountTokens,
            &route_selection.model_name,
            prefer_codex_v1_path_for_route,
        );
        let parse_count_tokens_value = |converter: &str, value: &Value| -> Option<u64> {
            if converter.eq_ignore_ascii_case("gemini") {
                value
                    .get("totalTokens")
                    .and_then(|v| v.as_u64())
                    .or_else(|| value.get("total_tokens").and_then(|v| v.as_u64()))
            } else {
                parse_input_tokens(value)
            }
        };

        let codex_hints = route_selection
            .converter
            .eq_ignore_ascii_case("codex")
            .then(|| codex_request_hints_from_anthropic(&anthropic_body));

        let mut prepared_count_tokens = if route_selection.converter.eq_ignore_ascii_case("openai") {
            OpenAIChatAdapter.prepare_count_tokens_request(
                &unified_count_request,
                &count_tokens_ctx,
                &count_tokens_endpoint,
                &route_selection.api_key,
                &anthropic_version,
                &route_selection.model_name,
            )
        } else if route_selection.converter.eq_ignore_ascii_case("gemini") {
            GeminiAdapter.prepare_count_tokens_request(
                &unified_count_request,
                &count_tokens_ctx,
                &count_tokens_endpoint,
                &route_selection.api_key,
                &anthropic_version,
                &route_selection.model_name,
            )
        } else if route_selection.converter.eq_ignore_ascii_case("anthropic") {
            AnthropicAdapter.prepare_count_tokens_request(
                &unified_count_request,
                &count_tokens_ctx,
                &count_tokens_endpoint,
                &route_selection.api_key,
                &anthropic_version,
                &route_selection.model_name,
            )
        } else {
            CodexAdapter.prepare_count_tokens_request_with_hints(
                &unified_count_request,
                &count_tokens_ctx,
                &count_tokens_endpoint,
                &route_selection.api_key,
                &anthropic_version,
                &route_selection.model_name,
                codex_hints.as_ref().expect("codex hints"),
            )
        };

        match prepared_count_tokens.mode {
            CountTokensMode::Estimate => {
                if route_selection.converter.eq_ignore_ascii_case("openai") {
                    source = "estimate_openai".to_string();
                }
            }
            CountTokensMode::Native => {
                if let Some(prepared_request) = prepared_count_tokens.request.as_ref() {
                    let response = send_prepared_json_request(
                        &http_client,
                        prepared_request,
                        anthropic_beta.as_ref(),
                    )
                    .await;

                    if let Ok(resp) = response {
                        upstream_status = Some(resp.status().as_u16());
                        if resp.status().is_success() {
                            if let Ok(text) = resp.text().await {
                                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                                    token_count =
                                        parse_count_tokens_value(&route_selection.converter, &value);
                                    if token_count.is_some() {
                                        source = if route_selection.converter.eq_ignore_ascii_case("gemini") {
                                            "gemini_countTokens".to_string()
                                        } else if route_selection.converter.eq_ignore_ascii_case("anthropic") {
                                            "anthropic_count_tokens".to_string()
                                        } else {
                                            "codex_input_tokens".to_string()
                                        };
                                    }
                                }
                            }
                        } else if route_selection.converter.eq_ignore_ascii_case("codex") {
                            let status = resp.status().as_u16();
                            let error_text = resp.text().await.unwrap_or_default();
                            if is_codex_v1_responses_path(&count_tokens_endpoint)
                                && should_retry_codex_v1_path_with_legacy(status, &error_text)
                            {
                                let fallback_endpoint = resolve_upstream_url_with_codex_path_preference(
                                    &route_selection.converter,
                                    &route_selection.target_url,
                                    UpstreamOperation::CountTokens,
                                    &route_selection.model_name,
                                    false,
                                );
                                let _ = log_tx.send(format!(
                                    "[Route] #{} codex_v1_count_tokens_fallback=true from={} to={}",
                                    request_id, count_tokens_endpoint, fallback_endpoint
                                ));
                                prepared_count_tokens = CodexAdapter.prepare_count_tokens_request_with_hints(
                                    &unified_count_request,
                                    &count_tokens_ctx,
                                    &fallback_endpoint,
                                    &route_selection.api_key,
                                    &anthropic_version,
                                    &route_selection.model_name,
                                    codex_hints.as_ref().expect("codex hints"),
                                );

                                if let Some(retry_request) = prepared_count_tokens.request.as_ref() {
                                    match send_prepared_json_request(
                                        &http_client,
                                        retry_request,
                                        anthropic_beta.as_ref(),
                                    )
                                    .await
                                    {
                                        Ok(retry_resp) => {
                                            upstream_status = Some(retry_resp.status().as_u16());
                                            if retry_resp.status().is_success() {
                                                if let Some(endpoint_key) = codex_v1_endpoint_key.as_ref() {
                                                    mark_codex_v1_endpoint_unsupported(
                                                        &codex_v1_unsupported_endpoints,
                                                        endpoint_key,
                                                    );
                                                }
                                                if let Ok(text) = retry_resp.text().await {
                                                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                                                        token_count = parse_input_tokens(&value);
                                                        if token_count.is_some() {
                                                            source = "codex_input_tokens".to_string();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(_) => {
                                            upstream_status = None;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(route) = route_selection.route.as_ref() {
            if let Some(runtime) = load_balancer_runtime.as_ref() {
                runtime.handle_upstream_outcome(
                    route,
                    upstream_status,
                    upstream_status.is_none(),
                    None,
                );
            }
        }

        let input_tokens = if let Some(tokens) = token_count {
            tokens
        } else if allow_count_tokens_fallback_estimate {
            source = "estimate".to_string();
            estimate_input_tokens(&anthropic_body)
        } else {
            let _ = log_tx.send(format!(
                "[Tokens] #{} failed mode={} slot={} endpoint={} route_key={} upstream_status={} fallback=disabled",
                request_id,
                route_mode,
                route_slot,
                route_endpoint,
                route_key,
                upstream_status
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ));

            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(full_body(
                    json!({
                        "error": {
                            "type": "count_tokens_failed",
                            "message": "Failed to query upstream count_tokens and fallback estimate is disabled",
                            "upstream_status": upstream_status,
                        }
                    })
                    .to_string(),
                ))
                .unwrap());
        };

        let _ = log_tx.send(format!(
            "[Tokens] #{} input_tokens={} source={} mode={} slot={} endpoint={} route_key={} upstream_status={}",
            request_id,
            input_tokens,
            source,
            route_mode,
            route_slot,
            route_endpoint,
            route_key,
            upstream_status
                .map(|status| status.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));

        let payload = json!({ "input_tokens": input_tokens });
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(full_body(payload.to_string()))
            .unwrap());
    }

    let max_lb_attempts = load_balancer_runtime
        .as_ref()
        .map(|runtime| runtime.candidate_count_for_model(input_model).max(1))
        .unwrap_or(1);

    if let Some(ref l) = logger {
        l.log_anthropic_request(&body_bytes);
    }

    let mut attempt_index = 0usize;
    let mut successful_response: Option<reqwest::Response> = None;
    let mut successful_backend: Option<Arc<dyn TransformBackend>> = None;
    let mut successful_model = String::new();
    let mut successful_converter = String::new();
    let mut successful_upstream_status: Option<u16> = None;
    let mut successful_lb_permit: Option<EndpointPermit> = None;
    let mut successful_resolved_target_url = String::new();
    let mut successful_api_key = String::new();
    let mut successful_upstream_body: Option<Value> = None;
    let mut successful_parallel_tool_degrade_key: Option<String> = None;
    let mut successful_session_id = String::new();
    let mut successful_stateful_chain_meta: Option<StatefulChainRequestMeta> = None;
    let mut successful_effective_stream = anthropic_body.stream;

    while attempt_index < max_lb_attempts {
        attempt_index += 1;
        let _ = log_tx.send(format!(
            "[LB] #{} request_attempt={}/{} slot={}",
            request_id,
            attempt_index,
            max_lb_attempts,
            input_slot.as_str(),
        ));

        let mut route_selection = match resolve_route_selection(
            &request_id,
            input_model,
            input_slot,
            &target_url,
            &final_api_key,
            &ctx,
            load_balancer_runtime.as_ref(),
            &log_tx,
        ) {
            Ok(selection) => selection,
            Err(response) => return Ok(response),
        };

        let route_mode = if route_selection.route.is_some() {
            "lb"
        } else {
            "single"
        };
        let route_slot = route_selection
            .route
            .as_ref()
            .map(|route| route.slot.as_str())
            .unwrap_or(input_slot.as_str());
        let route_endpoint = route_selection
            .route
            .as_ref()
            .map(|route| route.endpoint_id.as_str())
            .unwrap_or("single");
        let route_key = route_selection
            .route
            .as_ref()
            .map(|route| route.route_key.as_str())
            .unwrap_or("-");
        let route_effort = if route_selection.converter.eq_ignore_ascii_case("codex") {
            route_selection
                .reasoning_effort_override
                .map(|effort| effort.as_str().to_string())
                .unwrap_or_else(|| {
                    crate::models::get_reasoning_effort(input_model, &ctx.reasoning_mapping)
                        .as_str()
                        .to_string()
                })
        } else {
            "-".to_string()
        };

        let codex_v1_endpoint_key = if route_selection.converter.eq_ignore_ascii_case("codex") {
            Some(build_codex_v1_endpoint_key(
                &route_selection.converter,
                &route_selection.target_url,
                &route_selection.api_key,
            ))
        } else {
            None
        };
        let prefer_codex_v1_path_for_route =
            route_selection.converter.eq_ignore_ascii_case("codex")
                && prefer_codex_v1_path
                && codex_v1_endpoint_key.as_ref().map_or(true, |key| {
                    !is_codex_v1_endpoint_unsupported(&codex_v1_unsupported_endpoints, key)
                });
        if route_selection.converter.eq_ignore_ascii_case("codex")
            && prefer_codex_v1_path
            && !prefer_codex_v1_path_for_route
        {
            let _ = log_tx.send(format!(
                "[Route] #{} codex_v1_preferred=false reason=endpoint_cached_unsupported",
                request_id
            ));
        }
        let codex_fast_endpoint_key = codex_v1_endpoint_key.clone();
        let mut resolved_target_url = resolve_upstream_url_with_codex_path_preference(
            &route_selection.converter,
            &route_selection.target_url,
            UpstreamOperation::Messages,
            &route_selection.model_name,
            prefer_codex_v1_path_for_route,
        );
        let mut parallel_tool_degrade_key = build_parallel_tool_degrade_key(
            route_selection.route.as_ref(),
            &resolved_target_url,
            &route_selection.converter,
            &route_selection.model_name,
        );

        let effective_stream_for_attempt = resolve_effective_stream(
            anthropic_body.stream,
            &route_selection.converter,
            accept_header.as_deref(),
            stream_opts,
        );

        let _ = log_tx.send(format!(
            "[Req] #{} in={} out={} msgs={} requested_stream={} effective_stream={} tools={} system_chars={} summary={}",
            request_id,
            input_model,
            route_selection.model_name,
            anthropic_body.messages.len(),
            anthropic_body.stream,
            effective_stream_for_attempt,
            tool_count,
            system_chars,
            display_summary,
        ));

        let _ = log_tx.send(format!(
            "[Route] #{} mode={} slot={} endpoint={} base={} converter={} model={} effort={}",
            request_id,
            route_mode,
            route_slot,
            route_endpoint,
            resolved_target_url,
            route_selection.converter,
            route_selection.model_name,
            route_effort,
        ));

        if let Some(remaining_secs) =
            get_active_cooldown_seconds(&model_cooldowns, &route_selection.model_name)
        {
            let _ = log_tx.send(format!(
                "[RateLimit] #{} local_cooldown model={} retry_after={}s in={} out={} msgs={} summary={}",
                request_id,
                route_selection.model_name,
                remaining_secs,
                input_model,
                route_selection.model_name,
                anthropic_body.messages.len(),
                display_summary,
            ));

            if let (Some(runtime), Some(route)) = (
                load_balancer_runtime.as_ref(),
                route_selection.route.as_ref(),
            ) {
                runtime.mark_unavailable(route, "local_cooldown");
            }

            if load_balancer_runtime.is_some() && attempt_index < max_lb_attempts {
                let _ = log_tx.send(format!(
                    "[LB] #{} failover continue reason=local_cooldown from_route={}",
                    request_id, route_key
                ));
                continue;
            }

            let payload = json!({
                "error": {
                    "type": "rate_limit_error",
                    "source": "local_cooldown",
                    "model": route_selection.model_name,
                    "retry_after": remaining_secs,
                    "message": format!("Model is cooling down, retry after {}s", remaining_secs)
                }
            });

            return Ok(Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("Content-Type", "application/json")
                .header("Retry-After", remaining_secs.to_string())
                .body(full_body(payload.to_string()))
                .unwrap());
        }

        let request_backend = build_backend_by_converter(&route_selection.converter);
        let (mut upstream_body, session_id) = transform_request_with_optional_codex_effort_override(
            &route_selection.converter,
            &request_backend,
            &anthropic_body,
            &log_tx,
            &ctx,
            &route_selection.model_name,
            route_selection.reasoning_effort_override,
            effective_stream_for_attempt,
        );

        let mut stateful_chain_meta_for_attempt = if enable_stateful_responses_chain
            && route_selection.converter.eq_ignore_ascii_case("codex")
        {
            let hint_tail = stateful_chain_hint
                .as_deref()
                .map(|value| tail_chars(value, 18))
                .unwrap_or_else(|| "-".to_string());
            emit_stream_diag(
                &log_tx,
                &logger,
                format!(
                    "[StatefulChain] #{} hint_source={} hint={}",
                    request_id, stateful_chain_hint_info.source, hint_tail
                ),
            );
            let endpoint_key = build_stateful_endpoint_key(
                &route_selection.converter,
                &resolved_target_url,
                &route_selection.model_name,
                &route_selection.api_key,
            );
            let chain_key = derive_stateful_chain_key(
                stateful_chain_hint.as_deref(),
                &route_selection.converter,
                &route_selection.model_name,
                &anthropic_body,
            );
            prepare_stateful_chain_request(
                &mut upstream_body,
                &stateful_chain_store,
                &stateful_chain_unsupported_endpoints,
                &chain_key,
                &endpoint_key,
                &request_id,
                &log_tx,
                &logger,
            )
            .map(|(meta, _turn_state)| meta) // 解构元组，只保留 meta
        } else {
            None
        };

        let stateful_chain_mode = if !enable_stateful_responses_chain {
            "disabled".to_string()
        } else if !route_selection.converter.eq_ignore_ascii_case("codex") {
            "disabled_non_codex".to_string()
        } else if upstream_body
            .get("previous_response_id")
            .and_then(|v| v.as_str())
            .is_some()
        {
            "attached".to_string()
        } else if let Some(meta) = stateful_chain_meta_for_attempt.as_ref() {
            if is_stateful_endpoint_previous_response_id_unsupported(
                &stateful_chain_unsupported_endpoints,
                &meta.endpoint_key,
            ) {
                "skipped_endpoint_unsupported".to_string()
            } else {
                "skipped_no_previous_response_id".to_string()
            }
        } else {
            "skipped_not_applicable".to_string()
        };

        let mut codex_fast_mode = if !route_selection.converter.eq_ignore_ascii_case("codex") {
            "not_applicable".to_string()
        } else if !ctx.enable_codex_fast_mode {
            "disabled_by_config".to_string()
        } else {
            "enabled".to_string()
        };

        if route_selection.converter.eq_ignore_ascii_case("codex") && ctx.enable_codex_fast_mode {
            let fast_cached_unsupported = codex_fast_endpoint_key.as_ref().map_or(false, |key| {
                is_codex_fast_endpoint_unsupported(&codex_fast_unsupported_endpoints, key)
            });
            if fast_cached_unsupported {
                if let Some(standard_body) =
                    remove_priority_service_tier_from_upstream_body(&upstream_body)
                {
                    upstream_body = standard_body;
                }
                codex_fast_mode = "skipped_endpoint_unsupported".to_string();
                let _ = log_tx.send(format!(
                    "[Route] #{} codex_fast_mode=skipped_endpoint_unsupported",
                    request_id
                ));
            } else if !body_uses_priority_service_tier(&upstream_body) {
                codex_fast_mode = "disabled_not_in_body".to_string();
            }
        }

        if route_selection.converter.eq_ignore_ascii_case("codex") {
            if let Some(remaining_secs) = get_parallel_tool_degrade_remaining_seconds(
                &parallel_tool_degrade_until,
                &parallel_tool_degrade_key,
            ) {
                if let Some(serial_body) =
                    disable_parallel_tool_calls_in_upstream_body(&upstream_body)
                {
                    upstream_body = serial_body;
                    let _ = log_tx.send(format!(
                        "[Stream] #{} endpoint_parallel_tool_auto_degraded=true key={} retry_after={}s",
                        request_id, parallel_tool_degrade_key, remaining_secs
                    ));
                }
            }
        }

        if let Some(input_summary) = summarize_codex_payload(&upstream_body) {
            let top_keys = sorted_object_keys(&upstream_body).join(",");
            let input_items = upstream_body
                .get("input")
                .and_then(|v| v.as_array())
                .map(|arr| arr.len())
                .unwrap_or(0);
            let request_body_bytes = serde_json::to_vec(&upstream_body)
                .map(|buf| buf.len())
                .unwrap_or(0);
            let tools_bytes_before = anthropic_body
                .tools
                .as_ref()
                .and_then(|tools| serde_json::to_vec(tools).ok())
                .map(|buf| buf.len())
                .unwrap_or(0);
            let tools_bytes_after = upstream_body
                .get("tools")
                .and_then(|tools| serde_json::to_vec(tools).ok())
                .map(|buf| buf.len())
                .unwrap_or(0);
            let resolved_url_path = extract_url_path(&resolved_target_url);
            let upstream_body_stream = upstream_body
                .get("stream")
                .and_then(|v| v.as_bool())
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let _ = log_tx.send(format!(
                "[ReqPayload] #{} keys={} input_items={} resolved_url_path={} requested_stream={} effective_stream={} upstream_body_stream={} request_body_bytes={} tools_bytes_before={} tools_bytes_after={} stateful_chain_mode={} codex_fast_mode={} summary={}",
                request_id,
                top_keys,
                input_items,
                resolved_url_path,
                anthropic_body.stream,
                effective_stream_for_attempt,
                upstream_body_stream,
                request_body_bytes,
                tools_bytes_before,
                tools_bytes_after,
                stateful_chain_mode,
                codex_fast_mode,
                tail_chars(&input_summary, 320),
            ));
        }

        if let Some(ref l) = logger {
            let headers = vec![
                ("Content-Type", "application/json"),
                ("Authorization", "Bearer <API_KEY>"),
                ("User-Agent", "Anthropic-Node/0.3.4"),
                ("x-anthropic-version", &anthropic_version),
                ("Accept", "text/event-stream"),
                ("session_id", &session_id),
            ];
            l.log_curl_request(
                "POST",
                &resolved_target_url,
                &headers,
                &upstream_body,
                backend_label_by_converter(&route_selection.converter),
            );
        }

        let upstream_req = request_backend.build_upstream_request(
            &http_client,
            &resolved_target_url,
            &route_selection.api_key,
            &upstream_body,
            &session_id,
            &anthropic_version,
        );

        let upstream_req = if let Some(beta) = &anthropic_beta {
            upstream_req.header("anthropic-beta", beta)
        } else {
            upstream_req
        };

        let mut response = match upstream_req.send().await {
            Ok(resp) => resp,
            Err(e) => {
                let action = if let (Some(runtime), Some(route)) = (
                    load_balancer_runtime.as_ref(),
                    route_selection.route.as_ref(),
                ) {
                    runtime.handle_upstream_outcome(route, None, true, None)
                } else {
                    UpstreamOutcomeAction::ReturnToClient
                };

                let _ = log_tx.send(format!(
                    "[Error] #{} ctx incoming_api={} configured_api={} upstream_api={} mode={} slot={} endpoint={} route_key={} converter={} in_model={} out_model={} effort={}",
                    request_id,
                    path,
                    target_url,
                    resolved_target_url,
                    route_mode,
                    route_slot,
                    route_endpoint,
                    route_key,
                    route_selection.converter,
                    input_model,
                    route_selection.model_name,
                    route_effort,
                ));
                let _ = log_tx.send(format!("[Error] Request failed: {}", e));

                if action == UpstreamOutcomeAction::RetryNextCandidate
                    && load_balancer_runtime.is_some()
                    && attempt_index < max_lb_attempts
                {
                    let _ = log_tx.send(format!(
                        "[LB] #{} failover continue reason=network_error from_route={}",
                        request_id, route_key
                    ));
                    continue;
                }

                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .header("Content-Type", "application/json")
                    .body(full_body(
                        json!({"error": {"message": format!("Upstream error: {}", e)}}).to_string(),
                    ))
                    .unwrap());
            }
        };

        if !response.status().is_success() {
            let mut retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-")
                .to_string();
            let mut status = response.status().as_u16();
            let mut error_text = response.text().await.unwrap_or_default();
            let mut recovered_response: Option<reqwest::Response> = None;
            let mut used_legacy_codex_route = false;
            let mut used_fast_codex_fallback = false;

            if route_selection.converter.eq_ignore_ascii_case("codex")
                && is_codex_v1_responses_path(&resolved_target_url)
                && should_retry_codex_v1_path_with_legacy(status, &error_text)
            {
                let fallback_target_url = resolve_upstream_url_with_codex_path_preference(
                    &route_selection.converter,
                    &route_selection.target_url,
                    UpstreamOperation::Messages,
                    &route_selection.model_name,
                    false,
                );

                if fallback_target_url != resolved_target_url {
                    emit_stream_diag(
                        &log_tx,
                        &logger,
                        format!(
                            "[Route] #{} codex_v1_fallback=true from={} to={}",
                            request_id, resolved_target_url, fallback_target_url
                        ),
                    );
                    resolved_target_url = fallback_target_url.clone();
                    parallel_tool_degrade_key = build_parallel_tool_degrade_key(
                        route_selection.route.as_ref(),
                        &resolved_target_url,
                        &route_selection.converter,
                        &route_selection.model_name,
                    );

                    if let Some(ref l) = logger {
                        let headers = vec![
                            ("Content-Type", "application/json"),
                            ("Authorization", "Bearer <API_KEY>"),
                            ("User-Agent", "Anthropic-Node/0.3.4"),
                            ("x-anthropic-version", &anthropic_version),
                            ("Accept", "text/event-stream"),
                            ("session_id", &session_id),
                        ];
                        l.log_curl_request(
                            "POST",
                            &resolved_target_url,
                            &headers,
                            &upstream_body,
                            backend_label_by_converter(&route_selection.converter),
                        );
                    }

                    let fallback_req = request_backend.build_upstream_request(
                        &http_client,
                        &resolved_target_url,
                        &route_selection.api_key,
                        &upstream_body,
                        &session_id,
                        &anthropic_version,
                    );

                    let fallback_req = if let Some(beta) = &anthropic_beta {
                        fallback_req.header("anthropic-beta", beta)
                    } else {
                        fallback_req
                    };

                    match fallback_req.send().await {
                        Ok(fallback_resp) if fallback_resp.status().is_success() => {
                            used_legacy_codex_route = true;
                            if let Some(endpoint_key) = codex_v1_endpoint_key.as_ref() {
                                mark_codex_v1_endpoint_unsupported(
                                    &codex_v1_unsupported_endpoints,
                                    endpoint_key,
                                );
                            }
                            if let Some(meta) = stateful_chain_meta_for_attempt.as_mut() {
                                meta.endpoint_key = build_stateful_endpoint_key(
                                    &route_selection.converter,
                                    &resolved_target_url,
                                    &route_selection.model_name,
                                    &route_selection.api_key,
                                );
                            }
                            recovered_response = Some(fallback_resp);
                        }
                        Ok(fallback_resp) => {
                            retry_after = fallback_resp
                                .headers()
                                .get("retry-after")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("-")
                                .to_string();
                            status = fallback_resp.status().as_u16();
                            error_text = fallback_resp.text().await.unwrap_or_default();
                        }
                        Err(fallback_err) => {
                            retry_after = "-".to_string();
                            status = StatusCode::BAD_GATEWAY.as_u16();
                            error_text = json!({
                                "error": {
                                    "message": format!("Upstream error after codex_v1_fallback: {}", fallback_err)
                                }
                            })
                            .to_string();
                        }
                    }
                }
            }

            if route_selection.converter.eq_ignore_ascii_case("codex")
                && recovered_response.is_none()
                && body_uses_priority_service_tier(&upstream_body)
                && should_retry_codex_fast_without_service_tier(status, &error_text)
            {
                if let Some(fallback_body) =
                    remove_priority_service_tier_from_upstream_body(&upstream_body)
                {
                    emit_stream_diag(
                        &log_tx,
                        &logger,
                        format!(
                            "[Route] #{} codex_fast_fallback=true resolved_url_path={}",
                            request_id,
                            extract_url_path(&resolved_target_url),
                        ),
                    );
                    upstream_body = fallback_body;

                    if let Some(ref l) = logger {
                        let headers = vec![
                            ("Content-Type", "application/json"),
                            ("Authorization", "Bearer <API_KEY>"),
                            ("User-Agent", "Anthropic-Node/0.3.4"),
                            ("x-anthropic-version", &anthropic_version),
                            ("Accept", "text/event-stream"),
                            ("session_id", &session_id),
                        ];
                        l.log_curl_request(
                            "POST",
                            &resolved_target_url,
                            &headers,
                            &upstream_body,
                            backend_label_by_converter(&route_selection.converter),
                        );
                    }

                    let fallback_req = request_backend.build_upstream_request(
                        &http_client,
                        &resolved_target_url,
                        &route_selection.api_key,
                        &upstream_body,
                        &session_id,
                        &anthropic_version,
                    );

                    let fallback_req = if let Some(beta) = &anthropic_beta {
                        fallback_req.header("anthropic-beta", beta)
                    } else {
                        fallback_req
                    };

                    match fallback_req.send().await {
                        Ok(fallback_resp) if fallback_resp.status().is_success() => {
                            used_fast_codex_fallback = true;
                            codex_fast_mode = "fallback_succeeded".to_string();
                            if let Some(endpoint_key) = codex_fast_endpoint_key.as_ref() {
                                mark_codex_fast_endpoint_unsupported(
                                    &codex_fast_unsupported_endpoints,
                                    endpoint_key,
                                );
                            }
                            recovered_response = Some(fallback_resp);
                        }
                        Ok(fallback_resp) => {
                            retry_after = fallback_resp
                                .headers()
                                .get("retry-after")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("-")
                                .to_string();
                            status = fallback_resp.status().as_u16();
                            error_text = fallback_resp.text().await.unwrap_or_default();
                            codex_fast_mode = "fallback_failed".to_string();
                        }
                        Err(fallback_err) => {
                            retry_after = "-".to_string();
                            status = StatusCode::BAD_GATEWAY.as_u16();
                            error_text = json!({
                                "error": {
                                    "message": format!("Upstream error after codex_fast_fallback: {}", fallback_err)
                                }
                            })
                            .to_string();
                            codex_fast_mode = "fallback_failed".to_string();
                        }
                    }
                }
            }

            let can_retry_without_previous_response_id =
                route_selection.converter.eq_ignore_ascii_case("codex")
                    && recovered_response.is_none()
                    && upstream_body
                        .get("previous_response_id")
                        .and_then(|v| v.as_str())
                        .is_some()
                    && stateful_chain_meta_for_attempt.is_some()
                    && is_previous_response_id_unsupported_error(status, &error_text);

            if can_retry_without_previous_response_id {
                let mut retry_body = upstream_body.clone();
                if let Some(obj) = retry_body.as_object_mut() {
                    obj.remove("previous_response_id");
                    if let Some(meta) = stateful_chain_meta_for_attempt.as_ref() {
                        obj.insert("input".to_string(), Value::Array(meta.full_input.clone()));
                        mark_stateful_endpoint_previous_response_id_unsupported(
                            &stateful_chain_unsupported_endpoints,
                            &meta.endpoint_key,
                        );
                    }
                }

                let _ = log_tx.send(format!(
                    "[StatefulChain] #{} previous_response_id_unsupported -> retry_without_previous_response_id=true",
                    request_id
                ));

                if let Some(ref l) = logger {
                    let headers = vec![
                        ("Content-Type", "application/json"),
                        ("Authorization", "Bearer <API_KEY>"),
                        ("User-Agent", "Anthropic-Node/0.3.4"),
                        ("x-anthropic-version", &anthropic_version),
                        ("Accept", "text/event-stream"),
                        ("session_id", &session_id),
                    ];
                    l.log_curl_request(
                        "POST",
                        &resolved_target_url,
                        &headers,
                        &retry_body,
                        backend_label_by_converter(&route_selection.converter),
                    );
                }

                let retry_req = request_backend.build_upstream_request(
                    &http_client,
                    &resolved_target_url,
                    &route_selection.api_key,
                    &retry_body,
                    &session_id,
                    &anthropic_version,
                );

                let retry_req = if let Some(beta) = &anthropic_beta {
                    retry_req.header("anthropic-beta", beta)
                } else {
                    retry_req
                };

                match retry_req.send().await {
                    Ok(retry_resp) if retry_resp.status().is_success() => {
                        let _ = log_tx.send(format!(
                            "[StatefulChain] #{} retry_without_previous_response_id=succeeded",
                            request_id
                        ));
                        upstream_body = retry_body;
                        recovered_response = Some(retry_resp);
                    }
                    Ok(retry_resp) => {
                        retry_after = retry_resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("-")
                            .to_string();
                        status = retry_resp.status().as_u16();
                        error_text = retry_resp.text().await.unwrap_or_default();
                        let _ = log_tx.send(format!(
                            "[StatefulChain] #{} retry_without_previous_response_id=failed status={}",
                            request_id, status
                        ));
                    }
                    Err(retry_err) => {
                        retry_after = "-".to_string();
                        status = StatusCode::BAD_GATEWAY.as_u16();
                        error_text = json!({
                            "error": {
                                "message": format!(
                                    "Upstream error after retry_without_previous_response_id: {}",
                                    retry_err
                                )
                            }
                        })
                        .to_string();
                        let _ = log_tx.send(format!(
                            "[StatefulChain] #{} retry_without_previous_response_id=failed network_error={}",
                            request_id, retry_err
                        ));
                    }
                }
            }

            if let Some(recovered) = recovered_response {
                response = recovered;
                if used_legacy_codex_route {
                    let _ = log_tx.send(format!(
                        "[Route] #{} codex_v1_fallback_result=succeeded resolved_url_path={}",
                        request_id,
                        extract_url_path(&resolved_target_url),
                    ));
                }
                if used_fast_codex_fallback {
                    let _ = log_tx.send(format!(
                        "[Route] #{} codex_fast_fallback_result=succeeded resolved_url_path={}",
                        request_id,
                        extract_url_path(&resolved_target_url),
                    ));
                }
            } else {
                let action = if let (Some(runtime), Some(route)) = (
                    load_balancer_runtime.as_ref(),
                    route_selection.route.as_ref(),
                ) {
                    runtime.handle_upstream_outcome(route, Some(status), false, Some(&error_text))
                } else {
                    UpstreamOutcomeAction::ReturnToClient
                };

                let _ = log_tx.send(format!(
                    "[Error] #{} ctx incoming_api={} configured_api={} upstream_api={} mode={} slot={} endpoint={} route_key={} converter={} in_model={} out_model={} effort={} status={} retry_after={}",
                    request_id,
                    path,
                    target_url,
                    resolved_target_url,
                    route_mode,
                    route_slot,
                    route_endpoint,
                    route_key,
                    route_selection.converter,
                    input_model,
                    route_selection.model_name,
                    route_effort,
                    status,
                    retry_after,
                ));

                if let Some((cooldown_model, cooldown_secs, reason)) = extract_cooldown_info(
                    status,
                    &error_text,
                    &retry_after,
                    &route_selection.model_name,
                ) {
                    set_model_cooldown(&model_cooldowns, &cooldown_model, cooldown_secs);
                    let _ = log_tx.send(format!(
                        "[RateLimit] #{} upstream=429 reason={} model={} retry_after={}s in={} out={} msgs={} summary={}",
                        request_id,
                        reason,
                        cooldown_model,
                        cooldown_secs,
                        input_model,
                        route_selection.model_name,
                        anthropic_body.messages.len(),
                        display_summary,
                    ));
                }

                let _ = log_tx.send(format!(
                    "[Error] #{} Upstream returned {}: {}",
                    request_id, status, error_text
                ));

                if let Some(ref l) = logger {
                    l.log_upstream_response(status, &error_text);
                }

                if action == UpstreamOutcomeAction::RetryNextCandidate
                    && load_balancer_runtime.is_some()
                    && attempt_index < max_lb_attempts
                {
                    let _ = log_tx.send(format!(
                        "[LB] #{} failover continue reason=upstream_status_{} from_route={}",
                        request_id, status, route_key
                    ));
                    continue;
                }

                return Ok(Response::builder()
                    .status(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY))
                    .header("Content-Type", "application/json")
                    .body(full_body(error_text))
                    .unwrap());
            }
        }

        let upstream_status = response.status().as_u16();
        if let (Some(runtime), Some(route)) = (
            load_balancer_runtime.as_ref(),
            route_selection.route.as_ref(),
        ) {
            runtime.handle_upstream_outcome(route, Some(upstream_status), false, None);
        }

        successful_response = Some(response);
        successful_backend = Some(request_backend);
        successful_model = route_selection.model_name.clone();
        successful_converter = route_selection.converter.clone();
        successful_upstream_status = Some(upstream_status);
        successful_lb_permit = route_selection.route_permit.take();
        successful_resolved_target_url = resolved_target_url.clone();
        successful_api_key = route_selection.api_key.clone();
        successful_upstream_body = Some(upstream_body.clone());
        successful_parallel_tool_degrade_key = Some(parallel_tool_degrade_key);
        successful_session_id = session_id.clone();
        successful_stateful_chain_meta = stateful_chain_meta_for_attempt;
        successful_effective_stream = effective_stream_for_attempt;
        break;
    }

    let response = successful_response.expect("upstream response must exist after successful loop");
    let request_backend = successful_backend.expect("backend must exist after successful loop");
    let model = successful_model;
    let request_converter = successful_converter;
    let upstream_status =
        successful_upstream_status.expect("upstream status must exist after successful loop");
    let resolved_target_url_for_stream = successful_resolved_target_url;
    let api_key_for_stream = successful_api_key;
    let upstream_body_for_stream =
        successful_upstream_body.expect("upstream body must exist after successful loop");
    let parallel_tool_degrade_key_for_stream = successful_parallel_tool_degrade_key;
    let session_id_for_request = successful_session_id;
    let stateful_chain_meta_for_request = successful_stateful_chain_meta;
    let _lb_permit = successful_lb_permit;
    let allow_visible_thinking_for_request = !anthropic_body.is_thinking_disabled();
    let effective_stream = successful_effective_stream;

    let _ = log_tx.send(format!(
        "[System] #{} Request transformed and forwarding to upstream API",
        request_id,
    ));

    if !anthropic_body.stream && effective_stream {
        let _ = log_tx.send(format!(
            "[Stream] #{} stream=false overridden to SSE (converter={}, force_stream_for_codex=true)",
            request_id, request_converter
        ));
    }

    if request_converter.eq_ignore_ascii_case("anthropic") && !effective_stream {
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_string();
        let body_text = response.text().await.unwrap_or_default();

        if let Some(ref l) = AppLogger::get() {
            l.log_upstream_response(upstream_status, &body_text);
            l.log("════════════════════════════════════════════════════════════════");
            l.log("✅ Request completed");
            l.log("════════════════════════════════════════════════════════════════");
        }

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", content_type)
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(body_text))
            .unwrap());
    }

    // 非流式：把上游 SSE 聚合成 Anthropic JSON
    if !effective_stream {
        let response_content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        if request_converter.eq_ignore_ascii_case("codex")
            && response_content_type
                .to_ascii_lowercase()
                .contains("application/json")
        {
            let body_text = response.text().await.unwrap_or_default();
            if let Some(ref l) = logger {
                l.log_upstream_response(upstream_status, &body_text);
            }
            let parsed = match serde_json::from_str::<Value>(&body_text) {
                Ok(value) => value,
                Err(error) => {
                    let _ = log_tx.send(format!(
                        "[Error] #{} Failed to parse non-stream Codex JSON response: {}",
                        request_id, error
                    ));
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .header("Content-Type", "application/json")
                        .body(full_body(
                            json!({"error": {"message": format!("Failed to parse non-stream Codex response: {}", error)}}).to_string(),
                        ))
                        .unwrap());
                }
            };

            let payload = build_anthropic_message_from_codex_json_response(&parsed, &model);
            log_usage_tokens(
                &log_tx,
                &request_id,
                parsed.get("usage"),
                "codex_non_stream",
            );
            log_prompt_cache_observation(
                &log_tx,
                &request_id,
                parsed
                    .get("usage")
                    .and_then(extract_cached_input_tokens_from_response_usage),
                stateful_chain_meta_for_request
                    .as_ref()
                    .and_then(|meta| meta.static_prefix_same_as_prior),
            );
            log_non_stream_assistant_preview(&log_tx, &request_id, &payload);

            if request_converter.eq_ignore_ascii_case("codex") {
                if let (Some(meta), Some(response_id)) = (
                    stateful_chain_meta_for_request.as_ref(),
                    parsed.get("id").and_then(|v| v.as_str()),
                ) {
                    record_stateful_chain_entry(
                        &stateful_chain_store,
                        meta,
                        response_id,
                        extract_stateful_chain_output_items(&parsed),
                        None, // turn_state will be extracted from response headers in streaming mode
                    );
                    emit_stream_diag(
                        &log_tx,
                        &logger,
                        format!(
                            "[StatefulChain] #{} stored response_id={} mode=non_stream",
                            request_id, response_id
                        ),
                    );
                }
            }

            if let Some(ref l) = AppLogger::get() {
                l.log("════════════════════════════════════════════════════════════════");
                l.log("✅ Request completed");
                l.log("════════════════════════════════════════════════════════════════");
            }
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .header("Access-Control-Allow-Origin", "*")
                .body(full_body(payload.to_string()))
                .unwrap());
        }

        let mut stream = response.bytes_stream();
        let mut line_buffer = String::new();
        let mut frame_parser = SseFrameParser::default();
        let mut transformer =
            request_backend.create_response_transformer(&model, allow_visible_thinking_for_request);
        transformer.configure_request_context(&response_transform_request_ctx);
        let mut metrics = StreamMetrics::new(request_started_at);

        let mut message_state: Option<Value> = None;
        let mut blocks: BTreeMap<usize, Value> = BTreeMap::new();
        let mut tool_input_buffers: HashMap<usize, String> = HashMap::new();
        let mut stop_reason_state: Option<String> = None;
        let mut usage_input_tokens: u64 = 0;
        let mut usage_output_tokens: u64 = 0;
        let mut latest_upstream_response_id: Option<String> = None;
        let mut latest_codex_terminal_snapshot: Option<Value> = None;

        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(300), stream.next()).await {
                Ok(Some(chunk_result)) => match chunk_result {
                    Ok(chunk) => {
                        metrics.mark_upstream_chunk();
                        let chunk_text = String::from_utf8_lossy(&chunk).to_string();

                        if stream_opts.enable_sse_frame_parser {
                            for frame in frame_parser.push_chunk(&chunk_text) {
                                if let Some(response_id) = extract_upstream_response_id(&frame) {
                                    latest_upstream_response_id = Some(response_id);
                                }
                                if request_converter.eq_ignore_ascii_case("codex") {
                                    if let Some(snapshot) =
                                        extract_codex_terminal_response_snapshot(&frame)
                                    {
                                        latest_codex_terminal_snapshot = Some(snapshot);
                                    }
                                }
                                if let Some(ref l) = logger {
                                    l.log_upstream_response(upstream_status, &frame);
                                }
                                for event_chunk in transformer.transform_event(&frame) {
                                    metrics.mark_downstream_output(&event_chunk);
                                    if let Some(ref l) = logger {
                                        l.log_anthropic_response(&event_chunk);
                                    }
                                    apply_sse_chunk_to_non_stream_message(
                                        &event_chunk,
                                        &mut message_state,
                                        &mut blocks,
                                        &mut tool_input_buffers,
                                        &mut stop_reason_state,
                                        &mut usage_input_tokens,
                                        &mut usage_output_tokens,
                                    );
                                }
                            }
                        } else {
                            line_buffer.push_str(&chunk_text);
                            for line in drain_complete_lines(&mut line_buffer) {
                                if let Some(response_id) = extract_upstream_response_id(&line) {
                                    latest_upstream_response_id = Some(response_id);
                                }
                                if let Some(ref l) = logger {
                                    l.log_upstream_response(upstream_status, &line);
                                }

                                for event_chunk in transformer.transform_line(&line) {
                                    metrics.mark_downstream_output(&event_chunk);
                                    if let Some(ref l) = logger {
                                        l.log_anthropic_response(&event_chunk);
                                    }
                                    apply_sse_chunk_to_non_stream_message(
                                        &event_chunk,
                                        &mut message_state,
                                        &mut blocks,
                                        &mut tool_input_buffers,
                                        &mut stop_reason_state,
                                        &mut usage_input_tokens,
                                        &mut usage_output_tokens,
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        metrics.emit(&log_tx, &request_id, stream_opts.enable_stream_metrics);
                        let _ = log_tx.send(format!("[Error] #{} Stream error: {}", request_id, e));
                        return Ok(Response::builder()
                                .status(StatusCode::BAD_GATEWAY)
                                .header("Content-Type", "application/json")
                                .body(full_body(
                                    json!({"error": {"message": format!("Upstream stream error: {}", e)}}).to_string(),
                                ))
                                .unwrap());
                    }
                },
                Ok(None) => break,
                Err(_) => {
                    metrics.emit(&log_tx, &request_id, stream_opts.enable_stream_metrics);
                    let _ = log_tx.send(format!(
                        "[Error] #{} Upstream read timeout (300s)",
                        request_id
                    ));
                    return Ok(Response::builder()
                        .status(StatusCode::GATEWAY_TIMEOUT)
                        .header("Content-Type", "application/json")
                        .body(full_body(
                            json!({"error": {"message": "Upstream read timeout (300s)"}})
                                .to_string(),
                        ))
                        .unwrap());
                }
            }
        }

        if stream_opts.enable_sse_frame_parser {
            if let Some(remaining) = frame_parser.take_remaining() {
                if let Some(response_id) = extract_upstream_response_id(&remaining) {
                    latest_upstream_response_id = Some(response_id);
                }
                if request_converter.eq_ignore_ascii_case("codex") {
                    if let Some(snapshot) = extract_codex_terminal_response_snapshot(&remaining) {
                        latest_codex_terminal_snapshot = Some(snapshot);
                    }
                }
                if let Some(ref l) = logger {
                    l.log_upstream_response(upstream_status, &remaining);
                }
                for event_chunk in transformer.transform_event(&remaining) {
                    metrics.mark_downstream_output(&event_chunk);
                    if let Some(ref l) = logger {
                        l.log_anthropic_response(&event_chunk);
                    }
                    apply_sse_chunk_to_non_stream_message(
                        &event_chunk,
                        &mut message_state,
                        &mut blocks,
                        &mut tool_input_buffers,
                        &mut stop_reason_state,
                        &mut usage_input_tokens,
                        &mut usage_output_tokens,
                    );
                }
            }
        } else if !line_buffer.trim().is_empty() {
            if let Some(response_id) = extract_upstream_response_id(&line_buffer) {
                latest_upstream_response_id = Some(response_id);
            }
            if let Some(ref l) = logger {
                l.log_upstream_response(upstream_status, &line_buffer);
            }

            for event_chunk in transformer.transform_line(&line_buffer) {
                metrics.mark_downstream_output(&event_chunk);
                if let Some(ref l) = logger {
                    l.log_anthropic_response(&event_chunk);
                }
                apply_sse_chunk_to_non_stream_message(
                    &event_chunk,
                    &mut message_state,
                    &mut blocks,
                    &mut tool_input_buffers,
                    &mut stop_reason_state,
                    &mut usage_input_tokens,
                    &mut usage_output_tokens,
                );
            }
        }

        let pending_tool_indices: Vec<usize> = tool_input_buffers.keys().copied().collect();
        for idx in pending_tool_indices {
            finalize_tool_input_block(idx, &mut blocks, &mut tool_input_buffers);
        }

        let mut message = message_state.unwrap_or_else(|| {
            json!({
                "id": format!("msg_{}", chrono::Utc::now().timestamp_millis()),
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": model,
                "stop_reason": null,
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            })
        });

        let content: Vec<Value> = blocks.into_values().collect();
        let stop_reason = stop_reason_state.unwrap_or_else(|| "end_turn".to_string());
        if let Some(message_obj) = message.as_object_mut() {
            message_obj.insert("content".to_string(), Value::Array(content));
            message_obj.insert("stop_reason".to_string(), json!(stop_reason));

            let usage = json!({
                "input_tokens": usage_input_tokens,
                "output_tokens": usage_output_tokens,
            });
            message_obj.insert("usage".to_string(), usage);
        }

        let payload = json!({
            "id": message.get("id").cloned().unwrap_or_else(|| json!(format!("msg_{}", chrono::Utc::now().timestamp_millis()))),
            "type": "message",
            "role": "assistant",
            "model": message.get("model").cloned().unwrap_or_else(|| json!(model)),
            "content": message.get("content").cloned().unwrap_or_else(|| json!([])),
            "stop_reason": message.get("stop_reason").cloned().unwrap_or_else(|| json!("end_turn")),
            "usage": message.get("usage").cloned().unwrap_or_else(|| json!({"input_tokens":0,"output_tokens":0}))
        });

        let payload = backfill_non_stream_payload_from_codex_snapshot(
            payload,
            latest_codex_terminal_snapshot.as_ref(),
            &model,
        );
        log_prompt_cache_observation(
            &log_tx,
            &request_id,
            latest_codex_terminal_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.get("usage"))
                .and_then(extract_cached_input_tokens_from_response_usage),
            stateful_chain_meta_for_request
                .as_ref()
                .and_then(|meta| meta.static_prefix_same_as_prior),
        );
        if payload
            .get("content")
            .and_then(|value| value.as_array())
            .map(|items| !items.is_empty())
            .unwrap_or(false)
            && latest_codex_terminal_snapshot.is_some()
        {
            let _ = log_tx.send(format!(
                "[NonStream] #{} terminal_snapshot_backfill_applied=true",
                request_id
            ));
        }
        log_non_stream_assistant_preview(&log_tx, &request_id, &payload);

        if contains_tool_call_text_leak(payload.get("content").unwrap_or(&json!([]))) {
            let _ = log_tx.send(format!(
                "[Error] #{} Detected tool-call text leak in non-stream response (likely upstream tool-call formatting mismatch)",
                request_id
            ));
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(full_body(
                    json!({"error": {"message": "Detected malformed tool call text output from upstream. Please retry the request; if it persists, disable parallel tool calls and verify tool-call protocol mapping."}})
                        .to_string(),
                ))
                .unwrap());
        }

        if let Some(summary) = transformer.take_diagnostics_summary() {
            emit_transform_diag(
                &log_tx,
                &logger,
                &request_id,
                &session_id_for_request,
                &summary,
            );
        }

        if request_converter.eq_ignore_ascii_case("codex") {
            if let (Some(meta), Some(response_id)) = (
                stateful_chain_meta_for_request.as_ref(),
                latest_upstream_response_id.as_deref(),
            ) {
                record_stateful_chain_entry(
                    &stateful_chain_store,
                    meta,
                    response_id,
                    latest_codex_terminal_snapshot
                        .as_ref()
                        .map(extract_stateful_chain_output_items)
                        .unwrap_or_default(),
                    None, // turn_state will be extracted from response headers in streaming mode
                );
                emit_stream_diag(
                    &log_tx,
                    &logger,
                    format!(
                        "[StatefulChain] #{} stored response_id={} mode=non_stream",
                        request_id, response_id
                    ),
                );
            }
        }

        if let Some(ref l) = AppLogger::get() {
            l.log("════════════════════════════════════════════════════════════════");
            l.log("✅ Request completed");
            l.log("════════════════════════════════════════════════════════════════");
        }
        metrics.emit(&log_tx, &request_id, stream_opts.enable_stream_metrics);

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(payload.to_string()))
            .unwrap());
    }

    // 流式：使用 channel 进行 SSE 转发
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, Infallible>>(256);

    let log_tx_clone = log_tx.clone();
    let request_id_for_stream = request_id.clone();
    let logger_for_stream = logger.clone();
    let stream_opts_for_task = stream_opts;
    let request_started_at_for_stream = request_started_at;
    let permit_for_stream = permit;
    let request_backend_for_stream = request_backend.clone();
    let model_for_stream = model.clone();
    let upstream_url_for_stream = resolved_target_url_for_stream.clone();
    let upstream_api_key_for_stream = api_key_for_stream.clone();
    let upstream_body_for_stream = upstream_body_for_stream.clone();
    let session_id_for_stream = session_id_for_request.clone();
    let serial_fallback_upstream_body_for_stream =
        disable_parallel_tool_calls_in_upstream_body(&upstream_body_for_stream);
    let http_client_for_stream = http_client.clone();
    let anthropic_version_for_stream = anthropic_version.clone();
    let anthropic_beta_for_stream = anthropic_beta.clone();
    let is_codex_stream_for_task = request_converter.eq_ignore_ascii_case("codex");
    let stateful_chain_enabled_for_stream =
        enable_stateful_responses_chain && request_converter.eq_ignore_ascii_case("codex");
    let stateful_chain_meta_for_stream = stateful_chain_meta_for_request.clone();
    let stateful_chain_store_for_stream = stateful_chain_store.clone();
    let parallel_tool_degrade_until_for_stream = parallel_tool_degrade_until.clone();
    let parallel_tool_degrade_key_for_stream = parallel_tool_degrade_key_for_stream.clone();
    let response_transform_request_ctx_for_stream = response_transform_request_ctx.clone();
    tokio::spawn(async move {
        let _permit_guard = permit_for_stream;
        let mut stream = response.bytes_stream();
        let mut transformer = request_backend_for_stream
            .create_response_transformer(&model_for_stream, allow_visible_thinking_for_request);
        transformer.configure_request_context(&response_transform_request_ctx_for_stream);
        let mut active_upstream_body_for_stream = upstream_body_for_stream.clone();
        let mut active_session_id_for_stream = session_id_for_stream;
        let mut line_buffer = String::new();
        let mut frame_parser = SseFrameParser::default();
        let mut upstream_log_counter = 0u64;
        let mut downstream_log_counter = 0u64;
        let mut metrics = StreamMetrics::new(request_started_at_for_stream);
        let mut event_counters = StreamEventCounters::default();
        let hard_timeout = Duration::from_secs(600);
        let stream_idle_timeout =
            Duration::from_millis(stream_opts_for_task.stall_timeout_ms.max(1_000));
        let heartbeat_interval =
            Duration::from_millis(stream_opts_for_task.stream_heartbeat_interval_ms.max(500));
        let silence_warn_threshold =
            Duration::from_millis(stream_opts_for_task.stream_silence_warn_ms.max(1_000));
        let silence_error_threshold = Duration::from_millis(
            stream_opts_for_task
                .stream_silence_error_ms
                .max(stream_opts_for_task.stream_silence_warn_ms.max(1_000) + 1_000),
        );
        let mut last_upstream_activity = Instant::now();
        let mut last_downstream_activity = Instant::now();
        let mut current_upstream_status = upstream_status;
        let mut decision = StreamDecisionState::default();
        let mut latest_upstream_response_id: Option<String> = None;
        let mut latest_codex_terminal_snapshot: Option<Value> = None;
        let mut silence_warn_logged = false;
        let mut silence_error_logged = false;
        let mut endpoint_parallel_degrade_marked = false;

        'stream_attempt: loop {
            loop {
                let wait_duration = if stream_opts_for_task.enable_stream_heartbeat {
                    heartbeat_interval
                } else {
                    hard_timeout
                };

                match tokio::time::timeout(wait_duration, stream.next()).await {
                    Ok(Some(chunk_result)) => match chunk_result {
                        Ok(chunk) => {
                            metrics.mark_upstream_chunk();
                            event_counters.mark_upstream_chunk();
                            last_upstream_activity = Instant::now();
                            silence_warn_logged = false;
                            silence_error_logged = false;
                            let chunk_text = String::from_utf8_lossy(&chunk).to_string();

                            if stream_opts_for_task.enable_sse_frame_parser {
                                for frame in frame_parser.push_chunk(&chunk_text) {
                                    if let Some(response_id) = extract_upstream_response_id(&frame)
                                    {
                                        latest_upstream_response_id = Some(response_id);
                                    }
                                    if is_codex_stream_for_task {
                                        if let Some(snapshot) =
                                            extract_codex_terminal_response_snapshot(&frame)
                                        {
                                            latest_codex_terminal_snapshot = Some(snapshot);
                                        }
                                    }
                                    let mut emitted_output_for_frame = false;
                                    maybe_log_stream_upstream(
                                        &logger_for_stream,
                                        current_upstream_status,
                                        &frame,
                                        stream_opts_for_task,
                                        &mut upstream_log_counter,
                                    );
                                    observe_upstream_chunk_events(
                                        &frame,
                                        &mut decision.saw_response_completed,
                                        &mut decision.saw_response_incomplete,
                                        &mut decision.saw_response_failed,
                                        &mut decision.saw_sibling_tool_call_error,
                                        &mut decision.upstream_error_event_type,
                                        &mut decision.upstream_error_message,
                                        &mut decision.upstream_error_code,
                                        &mut event_counters,
                                    );

                                    for output in transformer.transform_event(&frame) {
                                        if should_skip_transformed_output(
                                            &mut decision,
                                            &output,
                                            is_codex_stream_for_task,
                                            &log_tx_clone,
                                            &logger_for_stream,
                                            &request_id_for_stream,
                                        ) {
                                            continue;
                                        }
                                        metrics.mark_downstream_output(&output);
                                        event_counters.mark_downstream_chunk(&output);
                                        maybe_log_stream_downstream(
                                            &logger_for_stream,
                                            &output,
                                            stream_opts_for_task,
                                            &mut downstream_log_counter,
                                        );
                                        if tx
                                            .send(Ok(Frame::data(Bytes::from(output))))
                                            .await
                                            .is_err()
                                        {
                                            let _ = log_tx_clone.send(format!(
                                                "[Warning] #{} Client disconnected, stopping stream",
                                                request_id_for_stream
                                            ));
                                            metrics.emit(
                                                &log_tx_clone,
                                                &request_id_for_stream,
                                                stream_opts_for_task.enable_stream_metrics,
                                            );
                                            return;
                                        }
                                        last_downstream_activity = Instant::now();
                                        emitted_output_for_frame = true;
                                    }

                                    if stream_opts_for_task.enable_stream_heartbeat
                                        && !emitted_output_for_frame
                                        && last_downstream_activity.elapsed() >= heartbeat_interval
                                    {
                                        if !try_send_keep_alive(
                                            &tx,
                                            &log_tx_clone,
                                            &request_id_for_stream,
                                            &mut metrics,
                                            stream_opts_for_task.enable_stream_metrics,
                                            "sending keepalive during upstream-only events",
                                        )
                                        .await
                                        {
                                            return;
                                        }
                                        event_counters.mark_keepalive();
                                        last_downstream_activity = Instant::now();
                                    }
                                }
                            } else {
                                line_buffer.push_str(&chunk_text);
                                for line in drain_complete_lines(&mut line_buffer) {
                                    if let Some(response_id) = extract_upstream_response_id(&line) {
                                        latest_upstream_response_id = Some(response_id);
                                    }
                                    let mut emitted_output_for_line = false;
                                    maybe_log_stream_upstream(
                                        &logger_for_stream,
                                        current_upstream_status,
                                        &line,
                                        stream_opts_for_task,
                                        &mut upstream_log_counter,
                                    );
                                    observe_upstream_chunk_events(
                                        &line,
                                        &mut decision.saw_response_completed,
                                        &mut decision.saw_response_incomplete,
                                        &mut decision.saw_response_failed,
                                        &mut decision.saw_sibling_tool_call_error,
                                        &mut decision.upstream_error_event_type,
                                        &mut decision.upstream_error_message,
                                        &mut decision.upstream_error_code,
                                        &mut event_counters,
                                    );

                                    for output in transformer.transform_line(&line) {
                                        if should_skip_transformed_output(
                                            &mut decision,
                                            &output,
                                            is_codex_stream_for_task,
                                            &log_tx_clone,
                                            &logger_for_stream,
                                            &request_id_for_stream,
                                        ) {
                                            continue;
                                        }
                                        metrics.mark_downstream_output(&output);
                                        event_counters.mark_downstream_chunk(&output);
                                        maybe_log_stream_downstream(
                                            &logger_for_stream,
                                            &output,
                                            stream_opts_for_task,
                                            &mut downstream_log_counter,
                                        );
                                        if tx
                                            .send(Ok(Frame::data(Bytes::from(output))))
                                            .await
                                            .is_err()
                                        {
                                            let _ = log_tx_clone.send(format!(
                                                "[Warning] #{} Client disconnected, stopping stream",
                                                request_id_for_stream
                                            ));
                                            metrics.emit(
                                                &log_tx_clone,
                                                &request_id_for_stream,
                                                stream_opts_for_task.enable_stream_metrics,
                                            );
                                            return;
                                        }
                                        last_downstream_activity = Instant::now();
                                        emitted_output_for_line = true;
                                    }

                                    if stream_opts_for_task.enable_stream_heartbeat
                                        && !emitted_output_for_line
                                        && last_downstream_activity.elapsed() >= heartbeat_interval
                                    {
                                        if !try_send_keep_alive(
                                            &tx,
                                            &log_tx_clone,
                                            &request_id_for_stream,
                                            &mut metrics,
                                            stream_opts_for_task.enable_stream_metrics,
                                            "sending keepalive during upstream-only events",
                                        )
                                        .await
                                        {
                                            return;
                                        }
                                        event_counters.mark_keepalive();
                                        last_downstream_activity = Instant::now();
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = log_tx_clone.send(format!(
                                "[Error] #{} Stream error: {}",
                                request_id_for_stream, e
                            ));
                            decision.stream_close_cause = Some("upstream_stream_error");
                            break;
                        }
                    },
                    Ok(None) => {
                        decision.stream_close_cause = Some("upstream_eof");
                        break;
                    }
                    Err(_) => {
                        if stream_opts_for_task.enable_stream_heartbeat {
                            let silent_elapsed = last_upstream_activity.elapsed();
                            if stream_opts_for_task.enable_stream_event_metrics {
                                if !silence_warn_logged && silent_elapsed >= silence_warn_threshold
                                {
                                    emit_stream_diag(
                                        &log_tx_clone,
                                        &logger_for_stream,
                                        format!(
                                            "[Stream] #{} silence_warn silent_ms={} warn_threshold_ms={}",
                                            request_id_for_stream,
                                            silent_elapsed.as_millis(),
                                            silence_warn_threshold.as_millis()
                                        ),
                                    );
                                    silence_warn_logged = true;
                                }
                                if !silence_error_logged
                                    && silent_elapsed >= silence_error_threshold
                                {
                                    emit_stream_diag(
                                        &log_tx_clone,
                                        &logger_for_stream,
                                        format!(
                                            "[Stream] #{} silence_error silent_ms={} error_threshold_ms={}",
                                            request_id_for_stream,
                                            silent_elapsed.as_millis(),
                                            silence_error_threshold.as_millis()
                                        ),
                                    );
                                    silence_error_logged = true;
                                }
                            }
                            if silent_elapsed >= hard_timeout {
                                let _ = log_tx_clone.send(format!(
                                    "[Error] #{} Upstream read timeout ({}s)",
                                    request_id_for_stream,
                                    hard_timeout.as_secs()
                                ));
                                decision.stream_close_cause = Some("upstream_read_timeout");
                                break;
                            }

                            if silent_elapsed >= stream_idle_timeout {
                                emit_stream_diag(
                                    &log_tx_clone,
                                    &logger_for_stream,
                                    format!(
                                        "[Stream] #{} stream_idle_timeout_reached silent_ms={} threshold_ms={}",
                                        request_id_for_stream,
                                        silent_elapsed.as_millis(),
                                        stream_idle_timeout.as_millis()
                                    ),
                                );
                                decision.stream_close_cause = Some("stream_idle_timeout");
                                break;
                            }

                            if !try_send_keep_alive(
                                &tx,
                                &log_tx_clone,
                                &request_id_for_stream,
                                &mut metrics,
                                stream_opts_for_task.enable_stream_metrics,
                                "sending heartbeat",
                            )
                            .await
                            {
                                return;
                            }
                            event_counters.mark_keepalive();
                            last_downstream_activity = Instant::now();
                            continue;
                        }

                        let _ = log_tx_clone.send(format!(
                            "[Error] #{} Upstream read timeout ({}s)",
                            request_id_for_stream,
                            hard_timeout.as_secs()
                        ));
                        decision.stream_close_cause = Some("upstream_read_timeout");
                        break;
                    }
                }
            }

            if stream_opts_for_task.enable_sse_frame_parser {
                if let Some(remaining) = frame_parser.take_remaining() {
                    if let Some(response_id) = extract_upstream_response_id(&remaining) {
                        latest_upstream_response_id = Some(response_id);
                    }
                    if is_codex_stream_for_task {
                        if let Some(snapshot) = extract_codex_terminal_response_snapshot(&remaining)
                        {
                            latest_codex_terminal_snapshot = Some(snapshot);
                        }
                    }
                    maybe_log_stream_upstream(
                        &logger_for_stream,
                        current_upstream_status,
                        &remaining,
                        stream_opts_for_task,
                        &mut upstream_log_counter,
                    );
                    observe_upstream_chunk_events(
                        &remaining,
                        &mut decision.saw_response_completed,
                        &mut decision.saw_response_incomplete,
                        &mut decision.saw_response_failed,
                        &mut decision.saw_sibling_tool_call_error,
                        &mut decision.upstream_error_event_type,
                        &mut decision.upstream_error_message,
                        &mut decision.upstream_error_code,
                        &mut event_counters,
                    );

                    for output in transformer.transform_event(&remaining) {
                        if should_skip_transformed_output(
                            &mut decision,
                            &output,
                            is_codex_stream_for_task,
                            &log_tx_clone,
                            &logger_for_stream,
                            &request_id_for_stream,
                        ) {
                            continue;
                        }
                        metrics.mark_downstream_output(&output);
                        event_counters.mark_downstream_chunk(&output);
                        maybe_log_stream_downstream(
                            &logger_for_stream,
                            &output,
                            stream_opts_for_task,
                            &mut downstream_log_counter,
                        );
                        if tx.send(Ok(Frame::data(Bytes::from(output)))).await.is_err() {
                            let _ = log_tx_clone.send(format!(
                                "[Warning] #{} Client disconnected during flush",
                                request_id_for_stream
                            ));
                            metrics.emit(
                                &log_tx_clone,
                                &request_id_for_stream,
                                stream_opts_for_task.enable_stream_metrics,
                            );
                            return;
                        }
                    }
                }
            } else if !line_buffer.trim().is_empty() {
                if let Some(response_id) = extract_upstream_response_id(&line_buffer) {
                    latest_upstream_response_id = Some(response_id);
                }
                if is_codex_stream_for_task {
                    if let Some(snapshot) = extract_codex_terminal_response_snapshot(&line_buffer) {
                        latest_codex_terminal_snapshot = Some(snapshot);
                    }
                }
                maybe_log_stream_upstream(
                    &logger_for_stream,
                    current_upstream_status,
                    &line_buffer,
                    stream_opts_for_task,
                    &mut upstream_log_counter,
                );
                observe_upstream_chunk_events(
                    &line_buffer,
                    &mut decision.saw_response_completed,
                    &mut decision.saw_response_incomplete,
                    &mut decision.saw_response_failed,
                    &mut decision.saw_sibling_tool_call_error,
                    &mut decision.upstream_error_event_type,
                    &mut decision.upstream_error_message,
                    &mut decision.upstream_error_code,
                    &mut event_counters,
                );

                for output in transformer.transform_line(&line_buffer) {
                    if should_skip_transformed_output(
                        &mut decision,
                        &output,
                        is_codex_stream_for_task,
                        &log_tx_clone,
                        &logger_for_stream,
                        &request_id_for_stream,
                    ) {
                        continue;
                    }
                    metrics.mark_downstream_output(&output);
                    event_counters.mark_downstream_chunk(&output);
                    maybe_log_stream_downstream(
                        &logger_for_stream,
                        &output,
                        stream_opts_for_task,
                        &mut downstream_log_counter,
                    );
                    if tx.send(Ok(Frame::data(Bytes::from(output)))).await.is_err() {
                        let _ = log_tx_clone.send(format!(
                            "[Warning] #{} Client disconnected during flush",
                            request_id_for_stream
                        ));
                        metrics.emit(
                            &log_tx_clone,
                            &request_id_for_stream,
                            stream_opts_for_task.enable_stream_metrics,
                        );
                        return;
                    }
                }
            }

            let has_serial_fallback = serial_fallback_upstream_body_for_stream.is_some();
            let sibling_tool_error_retry_allowed = allow_sibling_tool_error_retry(
                &decision,
                stream_opts_for_task,
                has_serial_fallback,
            );

            if let Some(skip_reason) = sibling_tool_error_retry_skip_reason(
                &decision,
                stream_opts_for_task,
                has_serial_fallback,
            ) {
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} sibling_tool_error_retry_skipped reason={}",
                        request_id_for_stream, skip_reason
                    ),
                );
            }

            if !endpoint_parallel_degrade_marked && decision.saw_sibling_tool_call_error {
                if let Some(key) = parallel_tool_degrade_key_for_stream.as_deref() {
                    mark_parallel_tool_degrade_and_log(
                        &log_tx_clone,
                        &logger_for_stream,
                        &request_id_for_stream,
                        &parallel_tool_degrade_until_for_stream,
                        key,
                        "sibling_tool_call_error",
                    );
                    endpoint_parallel_degrade_marked = true;
                }
            }

            if sibling_tool_error_retry_allowed {
                decision.sibling_tool_error_retry_attempted = true;
                active_upstream_body_for_stream = serial_fallback_upstream_body_for_stream
                    .as_ref()
                    .cloned()
                    .expect("serial fallback body should exist");
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} sibling_tool_error_retry_started mode=serial_parallel_tool_calls_false",
                        request_id_for_stream
                    ),
                );

                if let Some(retry) = execute_stream_retry_request(
                    &request_backend_for_stream,
                    &http_client_for_stream,
                    &upstream_url_for_stream,
                    &upstream_api_key_for_stream,
                    &active_upstream_body_for_stream,
                    &anthropic_version_for_stream,
                    anthropic_beta_for_stream.as_deref(),
                    &request_id_for_stream,
                    "sibling_tool_error_retry",
                    &log_tx_clone,
                    &logger_for_stream,
                )
                .await
                {
                    active_session_id_for_stream = retry.session_id;
                    current_upstream_status = retry.status;
                    stream = retry.response.bytes_stream();
                    transformer = request_backend_for_stream.create_response_transformer(
                        &model_for_stream,
                        allow_visible_thinking_for_request,
                    );
                    transformer
                        .configure_request_context(&response_transform_request_ctx_for_stream);
                    line_buffer.clear();
                    frame_parser = SseFrameParser::default();
                    decision.on_retry_success_reset();
                    decision.emitted_non_heartbeat_event = false;
                    decision.emitted_business_event = false;
                    decision.emitted_tool_event = false;
                    last_upstream_activity = Instant::now();
                    continue 'stream_attempt;
                }
            }

            let tool_leak_signal = transformer
                .take_diagnostics_summary()
                .as_ref()
                .and_then(extract_tool_leak_retry_signal);
            if !endpoint_parallel_degrade_marked && tool_leak_signal.is_some() {
                if let Some(key) = parallel_tool_degrade_key_for_stream.as_deref() {
                    mark_parallel_tool_degrade_and_log(
                        &log_tx_clone,
                        &logger_for_stream,
                        &request_id_for_stream,
                        &parallel_tool_degrade_until_for_stream,
                        key,
                        "tool_text_leak",
                    );
                    endpoint_parallel_degrade_marked = true;
                }
            }
            let leaked_tool_text_retry_allowed = allow_leaked_tool_text_retry(
                &decision,
                is_codex_stream_for_task,
                has_serial_fallback,
                tool_leak_signal,
            );

            if let Some(skip_reason) = leaked_tool_text_retry_skip_reason(
                &decision,
                is_codex_stream_for_task,
                has_serial_fallback,
                tool_leak_signal,
            ) {
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} leaked_tool_text_retry_skipped reason={}",
                        request_id_for_stream, skip_reason
                    ),
                );
            }

            if leaked_tool_text_retry_allowed {
                decision.leaked_tool_text_retry_attempted = true;
                let mut retry_upstream_body = serial_fallback_upstream_body_for_stream
                    .as_ref()
                    .cloned()
                    .expect("serial fallback body should exist");
                let guardrail_injected = if let Some(injected) =
                    inject_retry_no_tool_text_mix_guardrail(&retry_upstream_body)
                {
                    retry_upstream_body = injected;
                    true
                } else {
                    false
                };
                active_upstream_body_for_stream = retry_upstream_body;
                let signal = tool_leak_signal.expect("signal should exist when retry allowed");
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} leaked_tool_text_retry_started mode=serial_parallel_tool_calls_false guardrail_injected={} marker_drops={} raw_json_drops={} incomplete_json_drops={}",
                        request_id_for_stream,
                        guardrail_injected,
                        signal.dropped_leaked_marker_fragments,
                        signal.dropped_raw_tool_json_fragments,
                        signal.dropped_incomplete_tool_json_fragments
                    ),
                );

                if let Some(retry) = execute_stream_retry_request(
                    &request_backend_for_stream,
                    &http_client_for_stream,
                    &upstream_url_for_stream,
                    &upstream_api_key_for_stream,
                    &active_upstream_body_for_stream,
                    &anthropic_version_for_stream,
                    anthropic_beta_for_stream.as_deref(),
                    &request_id_for_stream,
                    "leaked_tool_text_retry",
                    &log_tx_clone,
                    &logger_for_stream,
                )
                .await
                {
                    active_session_id_for_stream = retry.session_id;
                    current_upstream_status = retry.status;
                    stream = retry.response.bytes_stream();
                    transformer = request_backend_for_stream.create_response_transformer(
                        &model_for_stream,
                        allow_visible_thinking_for_request,
                    );
                    transformer
                        .configure_request_context(&response_transform_request_ctx_for_stream);
                    line_buffer.clear();
                    frame_parser = SseFrameParser::default();
                    decision.on_retry_success_reset();
                    decision.emitted_non_heartbeat_event = false;
                    decision.emitted_business_event = false;
                    decision.emitted_tool_event = false;
                    last_upstream_activity = Instant::now();
                    continue 'stream_attempt;
                }
            }

            let stream_incomplete = !decision.saw_response_completed;
            let incomplete_retry_allowed = decision.allow_incomplete_retry(stream_opts_for_task);

            if incomplete_retry_allowed {
                decision.incomplete_stream_retry_attempts += 1;
                if decision.emitted_business_event {
                    emit_stream_diag(
                        &log_tx_clone,
                        &logger_for_stream,
                        format!(
                            "[Stream] #{} stream_retry_with_partial_output=true",
                            request_id_for_stream
                        ),
                    );
                }
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} stream_retry_started attempt={}/{}",
                        request_id_for_stream,
                        decision.incomplete_stream_retry_attempts,
                        stream_opts_for_task.incomplete_stream_retry_max_attempts
                    ),
                );

                if let Some(retry) = execute_stream_retry_request(
                    &request_backend_for_stream,
                    &http_client_for_stream,
                    &upstream_url_for_stream,
                    &upstream_api_key_for_stream,
                    &active_upstream_body_for_stream,
                    &anthropic_version_for_stream,
                    anthropic_beta_for_stream.as_deref(),
                    &request_id_for_stream,
                    "stream_retry",
                    &log_tx_clone,
                    &logger_for_stream,
                )
                .await
                {
                    active_session_id_for_stream = retry.session_id;
                    decision.incomplete_stream_retry_succeeded = true;
                    current_upstream_status = retry.status;
                    stream = retry.response.bytes_stream();
                    line_buffer.clear();
                    frame_parser = SseFrameParser::default();
                    decision.on_retry_success_reset();
                    last_upstream_activity = Instant::now();
                    continue 'stream_attempt;
                }
            } else if stream_incomplete && !decision.saw_message_stop {
                let reason = decision.incomplete_retry_skip_reason(stream_opts_for_task);
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} stream_retry_skipped reason={}",
                        request_id_for_stream, reason
                    ),
                );
            }

            break 'stream_attempt;
        }

        if decision.incomplete_stream_retry_attempts > 0 {
            emit_stream_diag(
                &log_tx_clone,
                &logger_for_stream,
                format!(
                    "[Stream] #{} incomplete_stream_retry_hits={} success={}",
                    request_id_for_stream,
                    decision.incomplete_stream_retry_attempts,
                    decision.incomplete_stream_retry_succeeded
                ),
            );
        }

        if decision.saw_response_failed && !decision.saw_message_stop {
            decision.fallback_completion_injected = true;
            emit_stream_diag(
                &log_tx_clone,
                &logger_for_stream,
                format!(
                    "[Stream] #{} response_failed_fallback_error_emitted event_type={} code={} message={}",
                    request_id_for_stream,
                    decision
                        .upstream_error_event_type
                        .as_deref()
                        .unwrap_or("unknown"),
                    decision.upstream_error_code.as_deref().unwrap_or("-"),
                    decision.upstream_error_message.as_deref().unwrap_or("-")
                ),
            );

            let fallback_error_message = if decision.saw_sibling_tool_call_error {
                if decision.sibling_tool_error_retry_attempted {
                    "上游返回 response.failed（Sibling tool call errored），已自动降级串行重试 1 次仍失败，请直接重试。".to_string()
                } else {
                    "上游返回 response.failed（Sibling tool call errored），建议直接重试；若持续失败，可暂时降低并行工具调用。".to_string()
                }
            } else if let Some(message) = decision.upstream_error_message.as_deref() {
                let code_suffix = decision
                    .upstream_error_code
                    .as_deref()
                    .map(|code| format!("（{}）", code))
                    .unwrap_or_default();
                format!("上游返回错误{}：{}。请直接重试。", code_suffix, message)
            } else if let Some(event_type) = decision.upstream_error_event_type.as_deref() {
                format!("上游返回 {}，已终止本次流式输出，请直接重试。", event_type)
            } else {
                "上游返回 response.failed，已终止本次流式输出，请直接重试。".to_string()
            };

            let error_event = format!(
                "event: error\ndata: {}\n\n",
                json!({
                    "type": "error",
                    "error": {
                        "type": "api_error",
                        "message": fallback_error_message
                    }
                })
            );
            metrics.mark_downstream_output(&error_event);
            event_counters.mark_downstream_chunk(&error_event);

            if tx
                .send(Ok(Frame::data(Bytes::from(error_event))))
                .await
                .is_err()
            {
                let _ = log_tx_clone.send(format!(
                    "[Warning] #{} Client disconnected while sending response.failed fallback error",
                    request_id_for_stream
                ));
                metrics.emit(
                    &log_tx_clone,
                    &request_id_for_stream,
                    stream_opts_for_task.enable_stream_metrics,
                );
                return;
            }
            let stop_event = format!(
                "event: message_stop\ndata: {}\n\n",
                json!({ "type": "message_stop" })
            );
            metrics.mark_downstream_output(&stop_event);
            event_counters.mark_downstream_chunk(&stop_event);
            if tx
                .send(Ok(Frame::data(Bytes::from(stop_event))))
                .await
                .is_err()
            {
                let _ = log_tx_clone.send(format!(
                    "[Warning] #{} Client disconnected while sending response.failed fallback stop",
                    request_id_for_stream
                ));
                metrics.emit(
                    &log_tx_clone,
                    &request_id_for_stream,
                    stream_opts_for_task.enable_stream_metrics,
                );
                return;
            }
            decision.saw_message_stop = true;
        }

        if !decision.saw_message_stop {
            decision.fallback_completion_injected = true;

            if decision.saw_response_completed {
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Warning] #{} completed_without_message_stop; emitting interruption error",
                        request_id_for_stream
                    ),
                );
            } else {
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Warning] #{} Upstream stream ended before response.completed; emitting interruption error instead of synthetic completion",
                        request_id_for_stream
                    ),
                );
            }

            let interrupted_error = format!(
                "event: error\ndata: {}\n\n",
                json!({
                    "type": "error",
                    "error": {
                        "type": "api_error",
                        "message": "流式响应未正常收口（缺少 message_stop 或 response.completed 前中断）。系统已自动重试但未恢复，请直接重试。"
                    }
                })
            );
            metrics.mark_downstream_output(&interrupted_error);
            event_counters.mark_downstream_chunk(&interrupted_error);
            if tx
                .send(Ok(Frame::data(Bytes::from(interrupted_error))))
                .await
                .is_err()
            {
                let _ = log_tx_clone.send(format!(
                    "[Warning] #{} Client disconnected while sending interruption error",
                    request_id_for_stream
                ));
                metrics.emit(
                    &log_tx_clone,
                    &request_id_for_stream,
                    stream_opts_for_task.enable_stream_metrics,
                );
                return;
            }
            let stop_event = format!(
                "event: message_stop\ndata: {}\n\n",
                json!({ "type": "message_stop" })
            );
            metrics.mark_downstream_output(&stop_event);
            event_counters.mark_downstream_chunk(&stop_event);
            if tx
                .send(Ok(Frame::data(Bytes::from(stop_event))))
                .await
                .is_err()
            {
                let _ = log_tx_clone.send(format!(
                    "[Warning] #{} Client disconnected while sending interruption stop",
                    request_id_for_stream
                ));
                metrics.emit(
                    &log_tx_clone,
                    &request_id_for_stream,
                    stream_opts_for_task.enable_stream_metrics,
                );
                return;
            }
            decision.saw_message_stop = true;
            decision.stream_close_cause = Some("upstream_incomplete_before_completed");
        }

        let close_cause = derive_stream_close_cause(
            decision.stream_close_cause,
            decision.saw_response_completed,
            decision.saw_response_incomplete,
            decision.saw_response_failed,
            decision.saw_message_stop,
        );
        let stream_outcome = decision.stream_outcome();
        emit_stream_diag(
            &log_tx_clone,
            &logger_for_stream,
            format!(
                "[Stream] #{} stream_outcome={} saw_response_completed={} saw_response_incomplete={} saw_response_failed={} saw_message_stop={} emitted_business_event={} final_fallback_emitted={}",
                request_id_for_stream,
                stream_outcome,
                decision.saw_response_completed,
                decision.saw_response_incomplete,
                decision.saw_response_failed,
                decision.saw_message_stop,
                decision.emitted_business_event,
                decision.emitted_empty_completion_fallback_notice || decision.fallback_completion_injected
            ),
        );
        if stream_opts_for_task.enable_stream_event_metrics {
            emit_stream_terminal_summary(
                &log_tx_clone,
                &logger_for_stream,
                &request_id_for_stream,
                &close_cause,
                &event_counters,
                &metrics,
                decision.saw_response_completed,
                decision.saw_response_incomplete,
                decision.saw_response_failed,
                decision.saw_message_stop,
                decision.emitted_business_event,
                decision.emitted_empty_completion_fallback_notice
                    || decision.fallback_completion_injected,
                decision.empty_completion_retry_attempts,
                decision.empty_completion_retry_succeeded,
                decision.incomplete_stream_retry_attempts,
                decision.incomplete_stream_retry_succeeded,
            );
        }

        log_prompt_cache_observation(
            &log_tx_clone,
            &request_id_for_stream,
            latest_codex_terminal_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.get("usage"))
                .and_then(extract_cached_input_tokens_from_response_usage),
            stateful_chain_meta_for_stream
                .as_ref()
                .and_then(|meta| meta.static_prefix_same_as_prior),
        );

        log_usage_tokens(
            &log_tx_clone,
            &request_id_for_stream,
            latest_codex_terminal_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.get("usage")),
            "codex_stream",
        );

        if stateful_chain_enabled_for_stream
            && decision.saw_response_completed
            && !decision.saw_response_failed
        {
            if let (Some(meta), Some(response_id)) = (
                stateful_chain_meta_for_stream.as_ref(),
                latest_upstream_response_id.as_deref(),
            ) {
                record_stateful_chain_entry(
                    &stateful_chain_store_for_stream,
                    meta,
                    response_id,
                    latest_codex_terminal_snapshot
                        .as_ref()
                        .map(extract_stateful_chain_output_items)
                        .unwrap_or_default(),
                    None, // turn_state extracted from response headers
                );
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[StatefulChain] #{} stored response_id={} mode=stream",
                        request_id_for_stream, response_id
                    ),
                );
            }
        }

        if let Some(summary) = transformer.take_diagnostics_summary() {
            emit_transform_diag(
                &log_tx_clone,
                &logger_for_stream,
                &request_id_for_stream,
                &active_session_id_for_stream,
                &summary,
            );
        }

        metrics.emit(
            &log_tx_clone,
            &request_id_for_stream,
            stream_opts_for_task.enable_stream_metrics,
        );

        if let Some(ref l) = logger_for_stream {
            l.log("════════════════════════════════════════════════════════════════");
            l.log("✅ Request completed");
            l.log("════════════════════════════════════════════════════════════════");
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let body = StreamBody::new(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("Access-Control-Allow-Origin", "*")
        .body(BoxBody::new(body.map_err(|_: Infallible| unreachable!())))
        .unwrap())
}

/// 处理 GET /v1/models 请求
fn handle_models_list() -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let models = generate_model_list();
    let response_body = json!({
        "object": "list",
        "data": models
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(full_body(response_body.to_string()))
        .unwrap())
}

/// 处理 GET /v1/models/{model_id} 请求
fn handle_model_detail(model_id: &str) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let supported_models = ["opus", "sonnet", "haiku"];

    if !supported_models.contains(&model_id) {
        let error_response = json!({
            "error": {
                "type": "not_found",
                "message": format!("Model '{}' not found", model_id)
            }
        });

        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(error_response.to_string()))
            .unwrap());
    }

    let model_info = json!({
        "id": model_id,
        "object": "model",
        "created": 1677610602,
        "owned_by": "codex-proxy",
        "permission": [],
        "root": model_id,
        "parent": null
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(full_body(model_info.to_string()))
        .unwrap())
}

/// 处理健康检查请求 GET /health 和 GET /
fn handle_health_check() -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let health_response = json!({
        "status": "ok",
        "version": "0.1.3"
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(full_body(health_response.to_string()))
        .unwrap())
}

/// 处理 OPTIONS 请求（CORS 预检）
fn handle_cors_preflight() -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header(
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization, x-api-key",
        )
        .header("Access-Control-Max-Age", "86400")
        .body(full_body("".to_string()))
        .unwrap())
}

/// 生成模型列表
fn generate_model_list() -> Vec<Value> {
    let models = ["opus", "sonnet", "haiku"];

    models
        .iter()
        .map(|&model_id| {
            json!({
                "id": model_id,
                "object": "model",
                "created": 1677610602,
                "owned_by": "codex-proxy",
                "permission": [],
                "root": model_id,
                "parent": null
            })
        })
        .collect()
}

fn full_body(s: String) -> BoxBody<Bytes, Infallible> {
    BoxBody::new(Full::new(Bytes::from(s)).map_err(|_: Infallible| unreachable!()))
}

#[cfg(test)]
mod tests {
    use super::{
        allow_leaked_tool_text_retry, allow_sibling_tool_error_retry,
        backfill_non_stream_payload_from_codex_snapshot, body_uses_priority_service_tier,
        build_anthropic_message_from_codex_json_response, build_parallel_tool_degrade_key,
        chunk_contains_sibling_tool_call_error, classify_connection_error,
        derive_stateful_chain_key, derive_stream_close_cause,
        disable_parallel_tool_calls_in_upstream_body,
        extract_cached_input_tokens_from_response_usage, extract_codex_plan_file_path_from_request,
        extract_codex_terminal_response_snapshot, extract_stateful_chain_hint_from_request,
        extract_stateful_chain_output_items, extract_tool_leak_retry_signal,
        extract_upstream_response_id,
        get_parallel_tool_degrade_remaining_seconds, inject_retry_no_tool_text_mix_guardrail,
        is_business_stream_output, is_codex_fast_endpoint_unsupported, is_codex_v1_responses_path,
        is_previous_response_id_unsupported_error, leaked_tool_text_retry_skip_reason,
        mark_codex_fast_endpoint_unsupported, mark_parallel_tool_degrade,
        observe_upstream_chunk_events, prepare_stateful_chain_request, record_stateful_chain_entry,
        remove_priority_service_tier_from_upstream_body, resolve_effective_stream,
        resolve_stateful_chain_hint_info, resolve_upstream_url,
        resolve_upstream_url_with_codex_path_preference, should_drop_post_message_stop_output,
        should_retry_codex_fast_without_service_tier, should_retry_codex_v1_path_with_legacy,
        should_suppress_premature_message_stop, sibling_tool_error_retry_skip_reason,
        CodexFastUnsupportedEndpointStore, ConnErrorClass, SseFrameParser, StatefulChainEntry, StatefulChainRequestMeta,
        StatefulChainStore, StatefulChainUnsupportedEndpointStore, StreamEventCounters,
        StreamRuntimeOptions, UpstreamOperation,
    };
    use crate::models::AnthropicRequest;
    use serde_json::{json, Value};
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::sync::broadcast;

    fn stream_opts() -> StreamRuntimeOptions {
        StreamRuntimeOptions {
            force_stream_for_codex: true,
            enable_sse_frame_parser: true,
            enable_stream_heartbeat: true,
            stream_heartbeat_interval_ms: 3_000,
            enable_stream_log_sampling: true,
            stream_log_sample_every_n: 20,
            stream_log_max_chars: 512,
            enable_stream_metrics: true,
            enable_stream_event_metrics: true,
            stream_silence_warn_ms: 20_000,
            stream_silence_error_ms: 90_000,
            stall_timeout_ms: 60_000,
            enable_incomplete_stream_retry: true,
            incomplete_stream_retry_max_attempts: 1,
            enable_sibling_tool_error_retry: true,
        }
    }

    #[test]
    fn test_route_level_custom_injection_prompt_is_not_injected_into_unified_request_paths() {
        let prompt = "Auto-install dependencies please.";
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role": "user", "content": "你好"}],
            "system": "You are Claude Code.",
            "tools": [{
                "name": "Read",
                "description": "Read files",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "file_path": {"type": "string"}
                    }
                }
            }],
            "stream": false
        }))
        .expect("valid request");

        let server = super::ProxyServer::new(8889, "http://localhost:3000".to_string(), None)
            .with_custom_injection_prompt(prompt.to_string());
        let runtime = server.runtime_update();
        let log_tx = broadcast::channel(8).0;

        let openai_model = super::resolve_model_for_converter(
            "openai",
            request.model.as_deref().unwrap_or("claude-sonnet-4-6"),
            &runtime.ctx.reasoning_mapping,
            &runtime.ctx.codex_model_mapping,
            &runtime.ctx.anthropic_model_mapping,
            &runtime.ctx.openai_model_mapping,
            &runtime.ctx.gemini_reasoning_effort,
        );
        let openai_backend = super::build_backend_by_converter("openai");
        let (openai_body, _) = super::transform_request_with_optional_codex_effort_override(
            "openai",
            &openai_backend,
            &request,
            &log_tx,
            &runtime.ctx,
            &openai_model,
            None,
            false,
        );
        let openai_serialized = openai_body.to_string();
        assert!(
            !openai_serialized.contains(prompt),
            "openai route should not include codex-only custom injection prompt"
        );

        let codex_model = super::resolve_model_for_converter(
            "codex",
            request.model.as_deref().unwrap_or("claude-sonnet-4-6"),
            &runtime.ctx.reasoning_mapping,
            &runtime.ctx.codex_model_mapping,
            &runtime.ctx.anthropic_model_mapping,
            &runtime.ctx.openai_model_mapping,
            &runtime.ctx.gemini_reasoning_effort,
        );
        let codex_backend = super::build_backend_by_converter("codex");
        let (codex_body, _) = super::transform_request_with_optional_codex_effort_override(
            "codex",
            &codex_backend,
            &request,
            &log_tx,
            &runtime.ctx,
            &codex_model,
            None,
            false,
        );
        let codex_instructions = codex_body
            .get("instructions")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        assert!(
            !codex_instructions.contains(prompt),
            "codex route should no longer inject custom prompts after unifying request conversion"
        );
    }

    #[test]
    fn test_extract_codex_plan_file_path_from_request() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "messages": [{
                "role": "user",
                "content": "<system-reminder>\nPlan mode is active.\n\n## Plan File Info:\nNo plan file exists yet. You should create your plan at /Users/mr.j/.claude/plans/encapsulated-tickling-nebula.md using the Write tool.\n</system-reminder>\n\n请给我方案"
            }],
            "stream": true
        }))
        .expect("valid request");

        let path = extract_codex_plan_file_path_from_request(&request);
        assert_eq!(
            path.as_deref(),
            Some("/Users/mr.j/.claude/plans/encapsulated-tickling-nebula.md"),
            "server should recover Claude plan file path from the original plan-mode reminder"
        );
    }

    #[test]
    fn test_count_terminal_background_agent_completions_recognizes_raw_task_notifications() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "<task-notification>\n<task-id>a1</task-id>\n<status>completed</status>\n<summary>Agent \"Check Beijing weather\" completed</summary>\n<result>北京天气结果</result>\n</task-notification>"
                    },
                    {
                        "type": "text",
                        "text": "<task-notification>\n<task-id>a2</task-id>\n<status>completed</status>\n<summary>Agent \"Check Guangdong weather\" completed</summary>\n<result>广东天气结果</result>\n</task-notification>"
                    },
                    {
                        "type": "text",
                        "text": "<task-notification>\n<task-id>a3</task-id>\n<status>completed</status>\n<summary>Background command \"Run tests\" completed (exit code 0)</summary>\n</task-notification>"
                    }
                ]
            }],
            "stream": true
        }))
        .expect("valid request");

        assert_eq!(
            super::count_terminal_background_agent_completions(&request),
            2
        );
    }

    #[test]
    fn test_background_agent_completion_detection_recognizes_structured_json_payloads() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "{\n  \"kind\": \"background_agent_completion\",\n  \"source\": \"idle_notification\",\n  \"status\": \"completed\",\n  \"summary\": \"Check Beijing weather completed\"\n}"
                }]
            }],
            "stream": true
        }))
        .expect("valid request");

        assert!(super::request_contains_background_agent_completion(
            &request
        ));
        assert_eq!(
            super::count_terminal_background_agent_completions(&request),
            1
        );
    }

    #[test]
    fn test_resolve_upstream_url_codex_messages_appends_responses() {
        let url = resolve_upstream_url(
            "codex",
            "https://codex.funai.vip/openai",
            UpstreamOperation::Messages,
            "gpt-5.3-codex",
        );
        assert_eq!(url, "https://codex.funai.vip/openai/responses");
    }

    #[test]
    fn test_resolve_upstream_url_codex_count_tokens() {
        let url = resolve_upstream_url(
            "codex",
            "https://codex.funai.vip/openai",
            UpstreamOperation::CountTokens,
            "gpt-5.3-codex",
        );
        assert_eq!(url, "https://codex.funai.vip/openai/responses/input_tokens");
    }

    #[test]
    fn test_resolve_upstream_url_with_codex_preference_uses_v1_path() {
        let url = resolve_upstream_url_with_codex_path_preference(
            "codex",
            "https://codex.funai.vip/openai",
            UpstreamOperation::Messages,
            "gpt-5.3-codex",
            true,
        );
        assert_eq!(url, "https://codex.funai.vip/openai/v1/responses");

        let count_tokens_url = resolve_upstream_url_with_codex_path_preference(
            "codex",
            "https://codex.funai.vip/openai",
            UpstreamOperation::CountTokens,
            "gpt-5.3-codex",
            true,
        );
        assert_eq!(
            count_tokens_url,
            "https://codex.funai.vip/openai/v1/responses/input_tokens"
        );
    }

    #[test]
    fn test_resolve_upstream_url_with_codex_preference_can_fallback_legacy() {
        let url = resolve_upstream_url_with_codex_path_preference(
            "codex",
            "https://codex.funai.vip/openai/v1/responses",
            UpstreamOperation::Messages,
            "gpt-5.3-codex",
            false,
        );
        assert_eq!(url, "https://codex.funai.vip/openai/responses");
    }

    #[test]
    fn test_should_retry_codex_v1_path_with_legacy_detects_route_not_found() {
        assert!(should_retry_codex_v1_path_with_legacy(
            404,
            r#"{"error":{"message":"Route not found"}}"#
        ));
        assert!(should_retry_codex_v1_path_with_legacy(
            400,
            r#"{"error":{"message":"Unsupported endpoint /v1/responses"}}"#
        ));
        assert!(!should_retry_codex_v1_path_with_legacy(
            429,
            r#"{"error":{"message":"rate limit exceeded"}}"#
        ));
    }

    #[test]
    fn test_is_codex_v1_responses_path_matcher() {
        assert!(is_codex_v1_responses_path(
            "https://example.com/openai/v1/responses"
        ));
        assert!(is_codex_v1_responses_path(
            "https://example.com/openai/v1/responses/input_tokens"
        ));
        assert!(!is_codex_v1_responses_path(
            "https://example.com/openai/responses"
        ));
    }

    #[test]
    fn test_should_retry_codex_fast_without_service_tier_matcher() {
        assert!(should_retry_codex_fast_without_service_tier(
            400,
            r#"{"error":{"message":"Unknown field service_tier for priority processing"}}"#
        ));
        assert!(should_retry_codex_fast_without_service_tier(
            422,
            r#"{"error":{"message":"additional properties are not allowed: service_tier"}}"#
        ));
        assert!(!should_retry_codex_fast_without_service_tier(
            429,
            r#"{"error":{"message":"rate limit exceeded"}}"#
        ));
    }

    #[test]
    fn test_remove_priority_service_tier_from_upstream_body() {
        let original = json!({
            "model": "gpt-5.3-codex",
            "service_tier": "priority",
            "stream": true
        });

        assert!(body_uses_priority_service_tier(&original));
        let rewritten = remove_priority_service_tier_from_upstream_body(&original)
            .expect("should remove priority service tier");
        assert!(rewritten.get("service_tier").is_none());
        assert_eq!(rewritten.get("model"), original.get("model"));
    }

    #[test]
    fn test_codex_fast_unsupported_endpoint_store_helpers() {
        let store: CodexFastUnsupportedEndpointStore = Arc::new(Mutex::new(HashSet::new()));
        let endpoint_key = "codex:deadbeef";

        assert!(!is_codex_fast_endpoint_unsupported(&store, endpoint_key));
        mark_codex_fast_endpoint_unsupported(&store, endpoint_key);
        assert!(is_codex_fast_endpoint_unsupported(&store, endpoint_key));
    }

    #[test]
    fn test_resolve_upstream_url_anthropic_messages_from_responses_base() {
        let url = resolve_upstream_url(
            "anthropic",
            "https://codex.funai.vip/openai/responses",
            UpstreamOperation::Messages,
            "claude-opus-4-6",
        );
        assert_eq!(url, "https://codex.funai.vip/openai/messages");
    }

    #[test]
    fn test_resolve_upstream_url_gemini_messages_from_base() {
        let url = resolve_upstream_url(
            "gemini",
            "http://localhost:8317",
            UpstreamOperation::Messages,
            "gemini-3-flash-preview",
        );
        assert_eq!(
            url,
            "http://localhost:8317/v1beta/models/gemini-3-flash-preview:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn test_resolve_upstream_url_openai_messages_from_v1_base() {
        let url = resolve_upstream_url(
            "openai",
            "https://api.openai.com/v1",
            UpstreamOperation::Messages,
            "gpt-4o",
        );
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_resolve_effective_stream_respects_non_stream_requests_for_codex() {
        let opts = stream_opts();
        assert!(!resolve_effective_stream(
            false,
            "codex",
            Some("text/event-stream"),
            opts
        ));
    }

    #[test]
    fn test_resolve_effective_stream_keeps_stream_true_requests() {
        let opts = stream_opts();
        assert!(resolve_effective_stream(
            true,
            "codex",
            Some("application/json"),
            opts
        ));
    }

    #[test]
    fn test_extract_codex_terminal_response_snapshot_prefers_nested_response() {
        let frame = concat!(
            "event: response.completed
",
            r#"data: {"type":"response.completed","response":{"id":"resp_1","model":"gpt-5.4","output":[{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"问候交流"}]}]}}"#,
            "

"
        );

        let snapshot = extract_codex_terminal_response_snapshot(frame).expect("snapshot");
        assert_eq!(snapshot.get("id").and_then(Value::as_str), Some("resp_1"));
        assert_eq!(
            snapshot
                .pointer("/output/0/content/0/text")
                .and_then(Value::as_str),
            Some("问候交流")
        );
    }

    #[test]
    fn test_backfill_non_stream_payload_from_codex_snapshot_uses_terminal_snapshot_when_empty() {
        let payload = json!({
            "id": "msg_empty",
            "type": "message",
            "role": "assistant",
            "model": "gpt-5.4",
            "content": [],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let snapshot = json!({
            "id": "resp_title_2",
            "model": "gpt-5.4",
            "usage": {"input_tokens": 10, "output_tokens": 2},
            "output": [{
                "id": "msg_title_2",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "问候交流"}]
            }]
        });

        let filled =
            backfill_non_stream_payload_from_codex_snapshot(payload, Some(&snapshot), "gpt-5.4");
        assert_eq!(
            filled.pointer("/content/0/text").and_then(Value::as_str),
            Some("问候交流")
        );
    }

    #[test]
    fn test_build_anthropic_message_from_codex_json_response_text() {
        let response = json!({
            "id": "resp_title_1",
            "model": "gpt-5.4",
            "usage": {"input_tokens": 12, "output_tokens": 3},
            "output": [{
                "id": "msg_title_1",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "问候交流"
                }]
            }]
        });

        let payload = build_anthropic_message_from_codex_json_response(&response, "gpt-5.4");
        assert_eq!(
            payload.pointer("/content/0/type").and_then(|v| v.as_str()),
            Some("text")
        );
        assert_eq!(
            payload.pointer("/content/0/text").and_then(|v| v.as_str()),
            Some("问候交流")
        );
        assert_eq!(
            payload.get("stop_reason").and_then(|v| v.as_str()),
            Some("end_turn")
        );
    }

    #[test]
    fn test_sse_frame_parser_handles_partial_frames() {
        let mut parser = SseFrameParser::default();
        let frames_1 = parser.push_chunk("event: x\ndata: 1\n\n");
        assert_eq!(frames_1.len(), 1);
        assert!(frames_1[0].contains("event: x"));

        let frames_2 = parser.push_chunk("event: y\ndata");
        assert!(frames_2.is_empty());
        let frames_3 = parser.push_chunk(": 2\n\n");
        assert_eq!(frames_3.len(), 1);
        assert!(frames_3[0].contains("event: y"));
    }

    #[test]
    fn test_is_business_stream_output_text_delta_true() {
        let chunk = r#"event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}

"#;
        assert!(is_business_stream_output(chunk));
    }

    #[test]
    fn test_is_business_stream_output_message_start_false() {
        let chunk = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[]}}

"#;
        assert!(!is_business_stream_output(chunk));
    }

    #[test]
    fn test_is_business_stream_output_thinking_delta_true() {
        let chunk = r#"event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"分析调用参数"}}

"#;
        assert!(is_business_stream_output(chunk));
    }

    #[test]
    fn test_disable_parallel_tool_calls_in_upstream_body() {
        let original = json!({
            "model": "gpt-5.3-codex",
            "parallel_tool_calls": true,
            "input": []
        });
        let rewritten =
            disable_parallel_tool_calls_in_upstream_body(&original).expect("should rewrite");
        assert_eq!(
            rewritten
                .get("parallel_tool_calls")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(rewritten.get("model"), original.get("model"));
    }

    #[test]
    fn test_inject_retry_no_tool_text_mix_guardrail_appends_once() {
        let original = json!({
            "instructions": "base instructions",
            "parallel_tool_calls": false
        });

        let first = inject_retry_no_tool_text_mix_guardrail(&original)
            .expect("guardrail should be injectable");
        let first_instructions = first
            .get("instructions")
            .and_then(|v| v.as_str())
            .expect("instructions should remain string");
        assert!(
            first_instructions.contains("RETRY_GUARDRAIL_NO_TOOL_TEXT_MIX"),
            "retry guardrail tag should be injected"
        );
        assert!(
            first_instructions.contains("Never print raw tool JSON arguments in text."),
            "retry guardrail body should be appended"
        );

        let second = inject_retry_no_tool_text_mix_guardrail(&first)
            .expect("guardrail reinjection should still return body");
        let second_instructions = second
            .get("instructions")
            .and_then(|v| v.as_str())
            .expect("instructions should remain string");
        assert_eq!(
            second_instructions
                .matches("RETRY_GUARDRAIL_NO_TOOL_TEXT_MIX")
                .count(),
            2,
            "guardrail wrapper tag pair should not be duplicated on reinjection"
        );
    }

    #[test]
    fn test_chunk_contains_sibling_tool_call_error_from_response_failed() {
        let chunk = r#"event: response.failed
data: {"type":"response.failed","response":{"error":{"message":"Sibling tool call errored: Invalid tool parameters"}}}

"#;
        assert!(chunk_contains_sibling_tool_call_error(chunk));
    }

    #[test]
    fn test_chunk_contains_sibling_tool_call_error_false_for_other_errors() {
        let chunk = r#"event: response.failed
data: {"type":"response.failed","response":{"error":{"message":"rate_limit_exceeded"}}}

"#;
        assert!(!chunk_contains_sibling_tool_call_error(chunk));
    }

    #[test]
    fn test_allow_sibling_tool_error_retry_honors_runtime_switch() {
        let state = super::StreamDecisionState {
            saw_response_failed: true,
            saw_sibling_tool_call_error: true,
            ..Default::default()
        };

        let opts_enabled = stream_opts();
        assert!(allow_sibling_tool_error_retry(&state, opts_enabled, true));

        let mut opts_disabled = stream_opts();
        opts_disabled.enable_sibling_tool_error_retry = false;
        assert!(!allow_sibling_tool_error_retry(&state, opts_disabled, true));
    }

    #[test]
    fn test_sibling_tool_error_retry_skip_reason_reports_feature_disabled() {
        let state = super::StreamDecisionState {
            saw_response_failed: true,
            saw_sibling_tool_call_error: true,
            ..Default::default()
        };
        let mut opts = stream_opts();
        opts.enable_sibling_tool_error_retry = false;

        assert_eq!(
            sibling_tool_error_retry_skip_reason(&state, opts, true),
            Some("feature_disabled")
        );
    }

    #[test]
    fn test_extract_tool_leak_retry_signal_from_transform_diag() {
        let summary = json!({
            "type": "codex_response_transform_summary",
            "counters": {
                "dropped_leaked_marker_fragments": 2,
                "dropped_raw_tool_json_fragments": 1,
                "dropped_high_risk_raw_tool_json_fragments": 0,
                "dropped_incomplete_tool_json_fragments": 0
            }
        });

        let signal = extract_tool_leak_retry_signal(&summary).expect("should parse signal");
        assert_eq!(signal.dropped_leaked_marker_fragments, 2);
        assert_eq!(signal.dropped_raw_tool_json_fragments, 1);
        assert_eq!(signal.dropped_high_risk_raw_tool_json_fragments, 0);
        assert_eq!(signal.dropped_incomplete_tool_json_fragments, 0);
    }

    #[test]
    fn test_allow_leaked_tool_text_retry_requires_signal_and_clean_state() {
        let signal = Some(super::ToolLeakRetrySignal {
            dropped_leaked_marker_fragments: 1,
            dropped_raw_tool_json_fragments: 0,
            dropped_high_risk_raw_tool_json_fragments: 0,
            dropped_incomplete_tool_json_fragments: 0,
        });
        let mut state = super::StreamDecisionState::default();
        assert!(allow_leaked_tool_text_retry(&state, true, true, signal));

        state.emitted_business_event = true;
        assert!(allow_leaked_tool_text_retry(&state, true, true, signal));

        state.emitted_tool_event = true;
        assert!(!allow_leaked_tool_text_retry(&state, true, true, signal));

        state.emitted_tool_event = false;
        state.leaked_tool_text_retry_attempted = true;
        assert!(!allow_leaked_tool_text_retry(&state, true, true, signal));

        state.leaked_tool_text_retry_attempted = false;
        assert!(!allow_leaked_tool_text_retry(&state, true, true, None));
    }

    #[test]
    fn test_leaked_tool_text_retry_skip_reason_reports_tool_event_block() {
        let state = super::StreamDecisionState {
            emitted_tool_event: true,
            ..Default::default()
        };
        let signal = Some(super::ToolLeakRetrySignal {
            dropped_leaked_marker_fragments: 1,
            dropped_raw_tool_json_fragments: 1,
            dropped_high_risk_raw_tool_json_fragments: 0,
            dropped_incomplete_tool_json_fragments: 0,
        });

        assert_eq!(
            leaked_tool_text_retry_skip_reason(&state, true, true, signal),
            Some("tool_event_emitted")
        );
    }

    #[test]
    fn test_leaked_tool_text_retry_skip_reason_reports_high_risk_confirmation_required() {
        let state = super::StreamDecisionState::default();
        let signal = Some(super::ToolLeakRetrySignal {
            dropped_leaked_marker_fragments: 0,
            dropped_raw_tool_json_fragments: 1,
            dropped_high_risk_raw_tool_json_fragments: 1,
            dropped_incomplete_tool_json_fragments: 0,
        });

        assert!(!allow_leaked_tool_text_retry(&state, true, true, signal));
        assert_eq!(
            leaked_tool_text_retry_skip_reason(&state, true, true, signal),
            Some("high_risk_leak_requires_confirmation")
        );
    }

    #[test]
    fn test_finalize_tool_input_block_marks_invalid_json_instead_of_empty_object() {
        let mut blocks = std::collections::BTreeMap::new();
        blocks.insert(
            0,
            json!({
                "type": "tool_use",
                "id": "call_1",
                "name": "get_weather",
                "input": {}
            }),
        );
        let mut tool_input_buffers = std::collections::HashMap::new();
        tool_input_buffers.insert(0, "{".to_string());

        super::finalize_tool_input_block(0, &mut blocks, &mut tool_input_buffers);

        let input = blocks
            .get(&0)
            .and_then(|block| block.get("input"))
            .cloned()
            .expect("tool_use block should retain input field");

        assert_eq!(
            input.get("_parse_error").and_then(|value| value.as_str()),
            Some("invalid_json"),
            "invalid tool input should be explicitly marked instead of silently becoming an empty object"
        );
        assert_eq!(
            input.get("_raw_input").and_then(|value| value.as_str()),
            Some("{"),
            "invalid tool input should preserve the raw partial payload for debugging"
        );
    }

    #[test]
    fn test_non_stream_aggregation_preserves_multi_tool_order_and_usage() {
        let mut message_state = Some(json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": "gpt-4o",
            "stop_reason": null,
            "usage": {"input_tokens": 0, "output_tokens": 0}
        }));
        let mut blocks = std::collections::BTreeMap::new();
        let mut tool_input_buffers = std::collections::HashMap::new();
        let mut stop_reason_state = None;
        let mut usage_input_tokens = 0_u64;
        let mut usage_output_tokens = 0_u64;

        super::apply_sse_chunk_to_non_stream_message(
            r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_1","name":"get_weather","input":{}}}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );
        super::apply_sse_chunk_to_non_stream_message(
            r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );
        super::apply_sse_chunk_to_non_stream_message(
            r#"event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_2","name":"get_time","input":{}}}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );
        super::apply_sse_chunk_to_non_stream_message(
            r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );
        super::apply_sse_chunk_to_non_stream_message(
            r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"Beijing\"}"}}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );
        super::apply_sse_chunk_to_non_stream_message(
            r#"event: content_block_stop
data: {"type":"content_block_stop","index":1}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );
        super::apply_sse_chunk_to_non_stream_message(
            r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );
        super::apply_sse_chunk_to_non_stream_message(
            r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"input_tokens":11,"output_tokens":7}}

"#,
            &mut message_state,
            &mut blocks,
            &mut tool_input_buffers,
            &mut stop_reason_state,
            &mut usage_input_tokens,
            &mut usage_output_tokens,
        );

        let mut message = message_state.expect("message state should exist");
        let content: Vec<Value> = blocks.into_values().collect();
        let stop_reason = stop_reason_state.unwrap_or_else(|| "end_turn".to_string());
        if let Some(message_obj) = message.as_object_mut() {
            message_obj.insert("content".to_string(), Value::Array(content));
            message_obj.insert("stop_reason".to_string(), json!(stop_reason));
            message_obj.insert(
                "usage".to_string(),
                json!({
                    "input_tokens": usage_input_tokens,
                    "output_tokens": usage_output_tokens,
                }),
            );
        }

        let content = message
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .expect("content should exist");
        assert_eq!(content.len(), 2);
        assert_eq!(
            content[0].get("name").and_then(Value::as_str),
            Some("get_weather")
        );
        assert_eq!(
            content[1].get("name").and_then(Value::as_str),
            Some("get_time")
        );
        assert_eq!(content[0].get("input"), Some(&json!({"city": "Beijing"})));
        assert_eq!(content[1].get("input"), Some(&json!({})));
        assert_eq!(
            message.get("usage"),
            Some(&json!({"input_tokens": 11, "output_tokens": 7}))
        );
        assert_eq!(
            message.get("stop_reason").and_then(Value::as_str),
            Some("tool_use")
        );
    }

    #[test]
    fn test_stream_event_counters_classify_downstream_events() {
        let mut counters = StreamEventCounters::default();
        counters.mark_downstream_chunk(
            r#"event: content_block_start
data: {"type":"content_block_start","content_block":{"type":"tool_use"}}

"#,
        );
        counters.mark_downstream_chunk(
            r#"event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}

"#,
        );
        counters.mark_downstream_chunk(": keep-alive\n\n");
        counters.mark_downstream_chunk(
            r#"event: message_stop
data: {"type":"message_stop"}

"#,
        );

        assert_eq!(counters.downstream_content_block_start_tool_use, 1);
        assert_eq!(counters.downstream_content_block_delta_input_json, 1);
        assert_eq!(counters.downstream_keepalive, 1);
        assert_eq!(counters.downstream_message_stop, 1);
    }

    #[test]
    fn test_stream_event_counters_recognize_ping_keepalive() {
        let mut counters = StreamEventCounters::default();
        counters.mark_downstream_chunk(
            r#"event: ping
data: {"type": "ping"}

"#,
        );
        assert_eq!(counters.downstream_keepalive, 1);
    }

    #[test]
    fn test_extract_upstream_response_id_from_completed_event() {
        let frame = r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_123","status":"completed"}}

"#;
        assert_eq!(
            extract_upstream_response_id(frame).as_deref(),
            Some("resp_123")
        );
    }

    #[test]
    fn test_prepare_stateful_chain_request_attaches_previous_response_id_and_trims_input() {
        let chain_store: StatefulChainStore = Arc::new(Mutex::new(HashMap::new()));
        let unsupported_store: StatefulChainUnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        {
            let mut guard = chain_store.lock().expect("lock");
            guard.insert(
                "test-chain".to_string(),
                StatefulChainEntry {
                    response_id: "resp_prev".to_string(),
                    endpoint_key: "ep_1".to_string(),
                    full_input: vec![
                        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}),
                        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}),
                    ],
                    output_items: Vec::new(),
                    static_prefix_summary: Some("pk_same".to_string()),
                    non_input_fingerprint: None,
                    turn_state: None,
                    updated_at: Instant::now(),
                },
            );
        }

        let mut body = json!({
            "model": "gpt-5.3-codex",
            "input": [
                {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"next"}]}
            ],
            "store": false
        });

        let (log_tx, _log_rx) = tokio::sync::broadcast::channel::<String>(8);
        let logger = None;
        let (meta, _turn_state) = prepare_stateful_chain_request(
            &mut body,
            &chain_store,
            &unsupported_store,
            "test-chain",
            "ep_1",
            "req_1",
            &log_tx,
            &logger,
        )
        .expect("stateful meta");

        assert_eq!(
            body.get("previous_response_id").and_then(|v| v.as_str()),
            Some("resp_prev")
        );
        assert_eq!(body.get("store").and_then(|v| v.as_bool()), Some(true));
        let input = body
            .get("input")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(input.len(), 1, "only incremental suffix should remain");
        assert_eq!(
            input[0].pointer("/content/0/text").and_then(|v| v.as_str()),
            Some("next")
        );
        assert_eq!(meta.full_input.len(), 3);
        assert_eq!(meta.static_prefix_same_as_prior, None);
    }

    #[test]
    fn test_prepare_stateful_chain_request_uses_output_items_in_prefix() {
        let chain_store: StatefulChainStore = Arc::new(Mutex::new(HashMap::new()));
        let unsupported_store: StatefulChainUnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        {
            let mut guard = chain_store.lock().expect("lock");
            guard.insert(
                "test-chain".to_string(),
                StatefulChainEntry {
                    response_id: "resp_prev".to_string(),
                    endpoint_key: "ep_1".to_string(),
                    full_input: vec![
                        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}),
                    ],
                    output_items: vec![
                        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}),
                    ],
                    static_prefix_summary: Some("pk_same".to_string()),
                    non_input_fingerprint: None,
                    turn_state: None,
                    updated_at: Instant::now(),
                },
            );
        }

        let mut body = json!({
            "model": "gpt-5.3-codex",
            "input": [
                {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"next"}]}
            ],
            "store": false
        });

        let (log_tx, _log_rx) = tokio::sync::broadcast::channel::<String>(8);
        let logger = None;
        let (_meta, _turn_state) = prepare_stateful_chain_request(
            &mut body,
            &chain_store,
            &unsupported_store,
            "test-chain",
            "ep_1",
            "req_1",
            &log_tx,
            &logger,
        )
        .expect("stateful meta");

        let input = body
            .get("input")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(input.len(), 1, "only new user turn should remain");
        assert_eq!(
            input[0].pointer("/content/0/text").and_then(|v| v.as_str()),
            Some("next")
        );
    }

    #[test]
    fn test_extract_stateful_chain_output_items_strips_metadata() {
        let snapshot = json!({
            "output": [{
                "id": "msg_1",
                "status": "completed",
                "created_at": 123,
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "hi"}]
            }]
        });

        let items = extract_stateful_chain_output_items(&snapshot);
        assert_eq!(items.len(), 1);
        assert!(items[0].get("id").is_none());
        assert!(items[0].get("status").is_none());
        assert!(items[0].get("created_at").is_none());
        assert_eq!(
            items[0].pointer("/content/0/text").and_then(|v| v.as_str()),
            Some("hi")
        );
    }
    #[test]
    fn test_prepare_stateful_chain_request_skips_previous_response_id_for_unsupported_endpoint() {
        let chain_store: StatefulChainStore = Arc::new(Mutex::new(HashMap::new()));
        let unsupported_store: StatefulChainUnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        {
            let mut guard = unsupported_store.lock().expect("lock");
            guard.insert("ep_unsupported".to_string());
        }
        {
            let mut guard = chain_store.lock().expect("lock");
            guard.insert(
                "test-chain".to_string(),
                StatefulChainEntry {
                    response_id: "resp_prev".to_string(),
                    endpoint_key: "ep_unsupported".to_string(),
                    full_input: vec![
                        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}),
                    ],
                    output_items: Vec::new(),
                    static_prefix_summary: Some("pk_unsupported".to_string()),
                    non_input_fingerprint: None,
                    turn_state: None,
                    updated_at: Instant::now(),
                },
            );
        }

        let mut body = json!({
            "model": "gpt-5.3-codex",
            "input": [
                {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"next"}]}
            ],
            "store": false
        });

        let (log_tx, _log_rx) = tokio::sync::broadcast::channel::<String>(8);
        let logger = None;
        let (_meta, _turn_state) = prepare_stateful_chain_request(
            &mut body,
            &chain_store,
            &unsupported_store,
            "test-chain",
            "ep_unsupported",
            "req_1",
            &log_tx,
            &logger,
        )
        .expect("stateful meta");

        assert_eq!(body.get("store").and_then(|v| v.as_bool()), Some(true));
        assert!(
            body.get("previous_response_id").is_none(),
            "unsupported endpoint should skip previous_response_id"
        );
        let input = body
            .get("input")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(input.len(), 2, "input should remain full when skipped");
        assert_eq!(_meta.static_prefix_same_as_prior, None);
    }

    #[test]
    fn test_prepare_stateful_chain_request_marks_static_prefix_changed_when_prompt_cache_key_differs(
    ) {
        let chain_store: StatefulChainStore = Arc::new(Mutex::new(HashMap::new()));
        let unsupported_store: StatefulChainUnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        {
            let mut guard = chain_store.lock().expect("lock");
            guard.insert(
                "test-chain".to_string(),
                StatefulChainEntry {
                    response_id: "resp_prev".to_string(),
                    endpoint_key: "ep_1".to_string(),
                    full_input: vec![json!("x")],
                    output_items: Vec::new(),
                    static_prefix_summary: Some("pk_old".to_string()),
                    non_input_fingerprint: None,
                    turn_state: None,
                    updated_at: Instant::now(),
                },
            );
        }

        let mut body = json!({
            "model": "gpt-5.3-codex",
            "input": [json!("x"), json!("y")],
            "store": false,
            "prompt_cache_key": "pk_new"
        });

        let (log_tx, _log_rx) = tokio::sync::broadcast::channel::<String>(8);
        let logger = None;
        let (meta, _turn_state) = prepare_stateful_chain_request(
            &mut body,
            &chain_store,
            &unsupported_store,
            "test-chain",
            "ep_1",
            "req_changed",
            &log_tx,
            &logger,
        )
        .expect("stateful meta");

        assert_eq!(meta.static_prefix_same_as_prior, Some(false));
    }

    #[test]
    fn test_prepare_stateful_chain_request_marks_static_prefix_same_when_prompt_cache_key_matches()
    {
        let chain_store: StatefulChainStore = Arc::new(Mutex::new(HashMap::new()));
        let unsupported_store: StatefulChainUnsupportedEndpointStore =
            Arc::new(Mutex::new(HashSet::new()));
        {
            let mut guard = chain_store.lock().expect("lock");
            guard.insert(
                "test-chain".to_string(),
                StatefulChainEntry {
                    response_id: "resp_prev".to_string(),
                    endpoint_key: "ep_1".to_string(),
                    full_input: vec![json!("x")],
                    output_items: Vec::new(),
                    static_prefix_summary: Some("pk_same".to_string()),
                    non_input_fingerprint: None,
                    turn_state: None,
                    updated_at: Instant::now(),
                },
            );
        }

        let mut body = json!({
            "model": "gpt-5.3-codex",
            "input": [json!("x"), json!("y")],
            "store": false,
            "prompt_cache_key": "pk_same"
        });

        let (log_tx, _log_rx) = tokio::sync::broadcast::channel::<String>(8);
        let logger = None;
        let (meta, _turn_state) = prepare_stateful_chain_request(
            &mut body,
            &chain_store,
            &unsupported_store,
            "test-chain",
            "ep_1",
            "req_same",
            &log_tx,
            &logger,
        )
        .expect("stateful meta");

        assert_eq!(meta.static_prefix_same_as_prior, Some(true));
    }

    #[test]
    fn test_extract_cached_input_tokens_from_response_usage_reads_nested_details() {
        let usage = json!({
            "input_tokens": 100,
            "input_tokens_details": {"cached_tokens": 88},
            "output_tokens": 5
        });

        assert_eq!(
            extract_cached_input_tokens_from_response_usage(&usage),
            Some(88)
        );
    }

    #[test]
    fn test_record_stateful_chain_entry_stores_latest_response() {
        let chain_store: StatefulChainStore = Arc::new(Mutex::new(HashMap::new()));
        let meta = StatefulChainRequestMeta {
            chain_key: "chain-a".to_string(),
            endpoint_key: "ep-a".to_string(),
            full_input: vec![Value::String("x".to_string())],
            static_prefix_summary: Some("pk_chain_a".to_string()),
            static_prefix_same_as_prior: None,
            non_input_fingerprint: None,
        };

        record_stateful_chain_entry(&chain_store, &meta, "resp_001", Vec::new(), None);

        let guard = chain_store.lock().expect("lock");
        let entry = guard.get("chain-a").expect("entry");
        assert_eq!(entry.response_id, "resp_001");
        assert_eq!(entry.endpoint_key, "ep-a");
        assert_eq!(entry.full_input.len(), 1);
        assert_eq!(entry.output_items.len(), 0);
        assert_eq!(entry.static_prefix_summary.as_deref(), Some("pk_chain_a"));
    }

    #[test]
    fn test_extract_stateful_chain_hint_from_metadata_user_id() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "messages": [],
            "metadata": {"user_id": "user_abc_account__session_123e4567-e89b-12d3-a456-426614174000"},
            "stream": true
        }))
        .expect("valid request");

        let hint = extract_stateful_chain_hint_from_request(&request);
        assert_eq!(
            hint.as_deref(),
            Some("123e4567-e89b-12d3-a456-426614174000"),
            "metadata user_id should supply stable session hint"
        );
    }

    #[test]
    fn test_resolve_stateful_chain_hint_from_metadata() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "messages": [],
            "metadata": {"user_id": "user_abc_account__session_123e4567-e89b-12d3-a456-426614174000"},
            "stream": true
        }))
        .expect("valid request");

        let info = resolve_stateful_chain_hint_info(None, &request);
        assert_eq!(
            info.value.as_deref(),
            Some("123e4567-e89b-12d3-a456-426614174000")
        );
        assert_eq!(info.source, "metadata");
    }

    #[test]
    fn test_derive_stateful_chain_key_prefers_hint_header() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role":"user","content":"hello"}],
            "stream": true
        }))
        .expect("request");
        let key =
            derive_stateful_chain_key(Some("session-123"), "codex", "gpt-5.3-codex", &request);
        assert_eq!(key, "hint:codex:session-123");
    }

    #[test]
    fn test_derive_stateful_chain_key_ignores_sessionstart_noise() {
        let request_a: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":[{"type":"text","text":"<system-reminder>\nSessionStart:startup hook success: Success\n</system-reminder>"}]
            }],
            "stream": true
        }))
        .expect("request_a");

        let request_b: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-5",
            "messages": [{
                "role":"user",
                "content":[{"type":"text","text":"<system-reminder>\nSessionStart hook additional context: another variant\n</system-reminder>"}]
            }],
            "stream": true
        }))
        .expect("request_b");

        let key_a = derive_stateful_chain_key(None, "codex", "gpt-5.3-codex", &request_a);
        let key_b = derive_stateful_chain_key(None, "codex", "gpt-5.3-codex", &request_b);
        assert_eq!(key_a, key_b);
    }

    #[test]
    fn test_is_previous_response_id_unsupported_error_matcher() {
        assert!(is_previous_response_id_unsupported_error(
            400,
            r#"{"detail":"Unsupported parameter: previous_response_id"}"#
        ));
        assert!(!is_previous_response_id_unsupported_error(
            429,
            r#"{"detail":"Unsupported parameter: previous_response_id"}"#
        ));
        assert!(!is_previous_response_id_unsupported_error(
            400,
            r#"{"detail":"Unsupported parameter: parallel_tool_calls"}"#
        ));
    }

    #[test]
    fn test_observe_upstream_chunk_events_marks_flags_and_counts() {
        let mut saw_completed = false;
        let mut saw_incomplete = false;
        let mut saw_failed = false;
        let mut saw_sibling = false;
        let mut upstream_error_event_type = None;
        let mut upstream_error_message = None;
        let mut upstream_error_code = None;
        let mut counters = StreamEventCounters::default();
        let chunk = r#"event: response.failed
data: {"type":"response.failed","response":{"error":{"message":"Sibling tool call errored: Invalid tool parameters"}}}

"#;

        observe_upstream_chunk_events(
            chunk,
            &mut saw_completed,
            &mut saw_incomplete,
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(!saw_completed);
        assert!(!saw_incomplete);
        assert!(saw_failed);
        assert!(saw_sibling);
        assert_eq!(
            upstream_error_event_type.as_deref(),
            Some("response.failed")
        );
        assert_eq!(
            upstream_error_message.as_deref(),
            Some("Sibling tool call errored: Invalid tool parameters")
        );
        assert_eq!(upstream_error_code, None);
        assert_eq!(counters.upstream_frames, 1);
        assert_eq!(counters.upstream_response_failed, 1);
    }

    #[test]
    fn test_observe_upstream_chunk_events_marks_done_as_completed() {
        let mut saw_completed = false;
        let mut saw_incomplete = false;
        let mut saw_failed = false;
        let mut saw_sibling = false;
        let mut upstream_error_event_type = None;
        let mut upstream_error_message = None;
        let mut upstream_error_code = None;
        let mut counters = StreamEventCounters::default();
        let chunk = r#"event: done
data: [DONE]

"#;

        observe_upstream_chunk_events(
            chunk,
            &mut saw_completed,
            &mut saw_incomplete,
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(saw_completed);
        assert!(!saw_incomplete);
        assert!(!saw_failed);
        assert!(!saw_sibling);
        assert_eq!(upstream_error_event_type, None);
        assert_eq!(upstream_error_message, None);
        assert_eq!(upstream_error_code, None);
        assert_eq!(counters.upstream_response_completed, 1);
    }

    #[test]
    fn test_observe_upstream_chunk_events_marks_incomplete_as_completed() {
        let mut saw_completed = false;
        let mut saw_incomplete = false;
        let mut saw_failed = false;
        let mut saw_sibling = false;
        let mut upstream_error_event_type = None;
        let mut upstream_error_message = None;
        let mut upstream_error_code = None;
        let mut counters = StreamEventCounters::default();
        let chunk = r#"event: response.incomplete
data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}

"#;

        observe_upstream_chunk_events(
            chunk,
            &mut saw_completed,
            &mut saw_incomplete,
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(saw_completed);
        assert!(saw_incomplete);
        assert!(!saw_failed);
        assert!(!saw_sibling);
        assert_eq!(upstream_error_event_type, None);
        assert_eq!(upstream_error_message, None);
        assert_eq!(upstream_error_code, None);
        assert_eq!(counters.upstream_response_incomplete, 1);
        assert_eq!(counters.upstream_response_completed, 0);
    }

    #[test]
    fn test_observe_upstream_chunk_events_captures_error_event_message_and_code() {
        let mut saw_completed = false;
        let mut saw_incomplete = false;
        let mut saw_failed = false;
        let mut saw_sibling = false;
        let mut upstream_error_event_type = None;
        let mut upstream_error_message = None;
        let mut upstream_error_code = None;
        let mut counters = StreamEventCounters::default();
        let chunk = r#"event: error
data: {"type":"error","error":{"message":"Rate limit exceeded","code":"rate_limit_exceeded"}}

"#;

        observe_upstream_chunk_events(
            chunk,
            &mut saw_completed,
            &mut saw_incomplete,
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(!saw_completed);
        assert!(!saw_incomplete);
        assert!(saw_failed);
        assert!(!saw_sibling);
        assert_eq!(upstream_error_event_type.as_deref(), Some("error"));
        assert_eq!(
            upstream_error_message.as_deref(),
            Some("Rate limit exceeded")
        );
        assert_eq!(upstream_error_code.as_deref(), Some("rate_limit_exceeded"));
        assert_eq!(counters.upstream_response_failed, 1);
    }

    #[test]
    fn test_derive_stream_close_cause_prefers_explicit_value() {
        let cause =
            derive_stream_close_cause(Some("client_disconnected"), false, false, false, false);
        assert_eq!(cause, "client_disconnected");
    }

    #[test]
    fn test_derive_stream_close_cause_completed_without_stop() {
        let cause = derive_stream_close_cause(None, true, false, false, false);
        assert_eq!(cause, "completed_without_message_stop");
    }

    #[test]
    fn test_derive_stream_close_cause_incomplete_with_stop() {
        let cause = derive_stream_close_cause(None, true, true, false, true);
        assert_eq!(cause, "response_incomplete");
    }

    #[test]
    fn test_classify_connection_error_client_disconnect() {
        let class = classify_connection_error(
            "connection closed before message completed",
            "",
            "io error: broken pipe",
        );
        assert!(matches!(class, ConnErrorClass::ClientDisconnect));
    }

    #[test]
    fn test_classify_connection_error_protocol_or_network() {
        let class = classify_connection_error(
            "http parse error: invalid header value",
            "hyper::Error(User(InvalidHeader))",
            "-",
        );
        assert!(matches!(class, ConnErrorClass::ProtocolOrNetwork));
    }

    #[test]
    fn test_should_suppress_premature_message_stop_for_codex() {
        let output = r#"event: message_stop
data: {"type":"message_stop"}

"#;

        assert!(should_suppress_premature_message_stop(
            output, true, false, false, false
        ));
        assert!(!should_suppress_premature_message_stop(
            output, false, false, false, false
        ));
        assert!(!should_suppress_premature_message_stop(
            output, true, true, false, false
        ));
        assert!(!should_suppress_premature_message_stop(
            output, true, false, true, false
        ));
    }

    #[test]
    fn test_should_drop_post_message_stop_output() {
        let content_delta = r#"event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"late"}}

"#;
        let ping = r#"event: ping
data: {"type":"ping"}

"#;
        let keepalive = ": keep-alive\n\n";

        assert!(should_drop_post_message_stop_output(content_delta));
        assert!(!should_drop_post_message_stop_output(ping));
        assert!(!should_drop_post_message_stop_output(keepalive));
    }

    #[test]
    fn test_parallel_tool_degrade_helpers_work() {
        let degrade_map: Arc<Mutex<HashMap<String, Instant>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let key = build_parallel_tool_degrade_key(
            None,
            "https://example.com/openai/responses",
            "codex",
            "gpt-5.3-codex",
        );

        assert!(key.starts_with("single:codex:gpt-5.3-codex:"));
        assert!(get_parallel_tool_degrade_remaining_seconds(&degrade_map, &key).is_none());

        mark_parallel_tool_degrade(&degrade_map, &key, 2);
        let remaining = get_parallel_tool_degrade_remaining_seconds(&degrade_map, &key)
            .expect("degrade ttl should be active");
        assert!((1..=2).contains(&remaining));

        if let Ok(mut guard) = degrade_map.lock() {
            guard.insert(key.clone(), Instant::now() - Duration::from_secs(1));
        }
        assert!(get_parallel_tool_degrade_remaining_seconds(&degrade_map, &key).is_none());
    }
}
