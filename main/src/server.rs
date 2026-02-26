use crate::load_balancer::{
    EndpointPermit, LoadBalancerRuntime, ModelSlot, ResolvedEndpoint, UpstreamOutcomeAction,
};
use crate::logger::AppLogger;
use crate::models::{
    AnthropicModelMapping, AnthropicRequest, CodexModelMapping, ContentBlock,
    GeminiReasoningEffortMapping, Message, MessageContent, ReasoningEffort, ReasoningEffortMapping,
};
use crate::transform::{
    AnthropicBackend, CodexBackend, GeminiBackend, TransformBackend, TransformContext,
};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
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
    gemini_reasoning_effort: GeminiReasoningEffortMapping,
    max_concurrency: u32,
    ignore_probe_requests: bool,
    allow_count_tokens_fallback_estimate: bool,
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
    load_balancer_runtime: Option<LoadBalancerRuntime>,
}

#[derive(Clone)]
pub struct RuntimeConfigUpdate {
    pub target_url: String,
    pub api_key: Option<String>,
    pub ctx: TransformContext,
    pub ignore_probe_requests: bool,
    pub allow_count_tokens_fallback_estimate: bool,
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
    pub load_balancer_runtime: Option<LoadBalancerRuntime>,
}

#[derive(Clone)]
struct RuntimeConfigState {
    target_url: String,
    api_key: Option<String>,
    ctx: TransformContext,
    ignore_probe_requests: bool,
    allow_count_tokens_fallback_estimate: bool,
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
    enable_empty_completion_retry: bool,
    empty_completion_retry_max_attempts: u32,
    enable_incomplete_stream_retry: bool,
    incomplete_stream_retry_max_attempts: u32,
    enable_sibling_tool_error_retry: bool,
    load_balancer_runtime: Option<LoadBalancerRuntime>,
}

impl From<RuntimeConfigUpdate> for RuntimeConfigState {
    fn from(value: RuntimeConfigUpdate) -> Self {
        Self {
            target_url: value.target_url,
            api_key: value.api_key,
            ctx: value.ctx,
            ignore_probe_requests: value.ignore_probe_requests,
            allow_count_tokens_fallback_estimate: value.allow_count_tokens_fallback_estimate,
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
            enable_empty_completion_retry: value.enable_empty_completion_retry,
            empty_completion_retry_max_attempts: value.empty_completion_retry_max_attempts,
            enable_incomplete_stream_retry: value.enable_incomplete_stream_retry,
            incomplete_stream_retry_max_attempts: value.incomplete_stream_retry_max_attempts,
            enable_sibling_tool_error_retry: value.enable_sibling_tool_error_retry,
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
    } else {
        Arc::new(CodexBackend)
    }
}

fn backend_label_by_converter(converter: &str) -> &'static str {
    if converter.eq_ignore_ascii_case("gemini") {
        "Gemini API"
    } else if converter.eq_ignore_ascii_case("anthropic") {
        "Anthropic API"
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
) -> (Value, String) {
    if converter.eq_ignore_ascii_case("codex") {
        if let Some(override_effort) = reasoning_effort_override {
            let override_mapping = ReasoningEffortMapping::new()
                .with_opus(override_effort)
                .with_sonnet(override_effort)
                .with_haiku(override_effort);
            return crate::transform::codex::TransformRequest::transform(
                anthropic_body,
                Some(log_tx),
                &override_mapping,
                &ctx.custom_injection_prompt,
                model_name,
            );
        }
    }

    request_backend.transform_request(
        anthropic_body,
        Some(log_tx),
        ctx,
        Some(model_name.to_string()),
    )
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
    enable_empty_completion_retry: bool,
    empty_completion_retry_max_attempts: u32,
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
            enable_empty_completion_retry: state.enable_empty_completion_retry,
            empty_completion_retry_max_attempts: state.empty_completion_retry_max_attempts,
            enable_incomplete_stream_retry: state.enable_incomplete_stream_retry,
            incomplete_stream_retry_max_attempts: state.incomplete_stream_retry_max_attempts,
            enable_sibling_tool_error_retry: state.enable_sibling_tool_error_retry,
        }
    }
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
    saw_response_failed: bool,
    saw_message_stop: bool,
) -> String {
    if let Some(cause) = explicit_cause {
        return cause.to_string();
    }
    if saw_response_failed {
        return "response_failed".to_string();
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

fn accept_header_allows_sse(accept_header: Option<&str>) -> bool {
    let Some(value) = accept_header else {
        return false;
    };
    let normalized = value.to_ascii_lowercase();
    normalized.contains("text/event-stream") || normalized.contains("*/*")
}

fn accept_header_explicit_json_only(accept_header: Option<&str>) -> bool {
    let Some(value) = accept_header else {
        return false;
    };
    let normalized = value.to_ascii_lowercase();
    normalized.contains("application/json") && !normalized.contains("text/event-stream")
}

fn resolve_effective_stream(
    requested_stream: bool,
    converter: &str,
    accept_header: Option<&str>,
    opts: StreamRuntimeOptions,
) -> bool {
    if requested_stream {
        return true;
    }
    if !opts.force_stream_for_codex || !converter.eq_ignore_ascii_case("codex") {
        return false;
    }
    if accept_header_explicit_json_only(accept_header) {
        return false;
    }
    accept_header_allows_sse(accept_header)
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

fn should_suppress_premature_message_stop(
    output: &str,
    is_codex_stream: bool,
    saw_response_completed: bool,
    saw_response_failed: bool,
) -> bool {
    is_codex_stream
        && chunk_is_message_stop(output)
        && !saw_response_completed
        && !saw_response_failed
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
        "content_block_delta" => payload
            .get("delta")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str())
            .map(|kind| matches!(kind, "text_delta" | "input_json_delta" | "thinking_delta"))
            .unwrap_or(false),
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

    let parsed_input = serde_json::from_str::<Value>(&partial_json).unwrap_or_else(|_| json!({}));
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
    dropped_incomplete_tool_json_fragments: u64,
}

impl ToolLeakRetrySignal {
    fn total(self) -> u64 {
        self.dropped_leaked_marker_fragments
            + self.dropped_raw_tool_json_fragments
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

fn strip_query(url: String) -> String {
    if let Some((head, _)) = url.split_once('?') {
        head.to_string()
    } else {
        url
    }
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

fn build_codex_messages_endpoint(target_url: &str) -> String {
    let clean = strip_query(target_url.to_string());
    if let Some(idx) = clean.rfind("/responses/input_tokens") {
        let mut endpoint = clean;
        endpoint.replace_range(idx..idx + "/responses/input_tokens".len(), "/responses");
        return endpoint;
    }

    if clean.contains("/responses") {
        return clean;
    }

    let base = clean.trim_end_matches('/');
    format!("{}/responses", base)
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
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            max_concurrency: 0,
            ignore_probe_requests: false,
            allow_count_tokens_fallback_estimate: true,
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
            incomplete_stream_retry_max_attempts: 5,
            enable_sibling_tool_error_retry: true,
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
                custom_injection_prompt: self.custom_injection_prompt.clone(),
                converter: self.converter.clone(),
                codex_model: self.codex_model.clone(),
                gemini_reasoning_effort: self.gemini_reasoning_effort.clone(),
            },
            ignore_probe_requests: self.ignore_probe_requests,
            allow_count_tokens_fallback_estimate: self.allow_count_tokens_fallback_estimate,
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
    let ctx = runtime_state.ctx;
    let ignore_probe_requests = runtime_state.ignore_probe_requests;
    let allow_count_tokens_fallback_estimate = runtime_state.allow_count_tokens_fallback_estimate;
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
    let anthropic_body: AnthropicRequest = match serde_json::from_value(raw_request_body.clone()) {
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

    let input_model = anthropic_body
        .model
        .as_deref()
        .unwrap_or("claude-3-5-sonnet-20240620");
    let input_slot = ModelSlot::from_model_name(input_model);

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

        let request_backend = build_backend_by_converter(&route_selection.converter);
        let mut token_count: Option<u64> = None;
        let mut upstream_status: Option<u16> = None;
        let mut source = "estimate".to_string();
        let count_tokens_endpoint = resolve_upstream_url(
            &route_selection.converter,
            &route_selection.target_url,
            UpstreamOperation::CountTokens,
            &route_selection.model_name,
        );

        if route_selection.converter.eq_ignore_ascii_case("gemini") {
            let (messages, _) = crate::transform::MessageProcessor::transform_messages(
                &anthropic_body.messages,
                Some(&log_tx),
            );
            let contents = GeminiBackend::build_contents_for_count(&messages);
            let body = json!({ "contents": contents });

            let response = http_client
                .post(&count_tokens_endpoint)
                .header("Content-Type", "application/json")
                .header("x-goog-api-key", &route_selection.api_key)
                .header(
                    "Authorization",
                    format!("Bearer {}", &route_selection.api_key),
                )
                .body(body.to_string())
                .send()
                .await;

            if let Ok(resp) = response {
                upstream_status = Some(resp.status().as_u16());
                if resp.status().is_success() {
                    if let Ok(text) = resp.text().await {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            token_count = value
                                .get("totalTokens")
                                .and_then(|v| v.as_u64())
                                .or_else(|| value.get("total_tokens").and_then(|v| v.as_u64()));
                            if token_count.is_some() {
                                source = "gemini_countTokens".to_string();
                            }
                        }
                    }
                }
            }
        } else if route_selection.converter.eq_ignore_ascii_case("anthropic") {
            let mut request_body = raw_request_body.clone();
            if let Some(obj) = request_body.as_object_mut() {
                obj.remove("stream");
                obj.insert(
                    "model".to_string(),
                    json!(route_selection.model_name.clone()),
                );
            }

            let response = http_client
                .post(&count_tokens_endpoint)
                .header("Content-Type", "application/json")
                .header("x-api-key", &route_selection.api_key)
                .header(
                    "Authorization",
                    format!("Bearer {}", &route_selection.api_key),
                )
                .header("x-anthropic-version", &anthropic_version)
                .body(request_body.to_string());

            let response = if let Some(beta) = &anthropic_beta {
                response.header("anthropic-beta", beta).send().await
            } else {
                response.send().await
            };

            if let Ok(resp) = response {
                upstream_status = Some(resp.status().as_u16());
                if resp.status().is_success() {
                    if let Ok(text) = resp.text().await {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            token_count = parse_input_tokens(&value);
                            if token_count.is_some() {
                                source = "anthropic_count_tokens".to_string();
                            }
                        }
                    }
                }
            }
        } else {
            let (codex_body, _) = transform_request_with_optional_codex_effort_override(
                &route_selection.converter,
                &request_backend,
                &anthropic_body,
                &log_tx,
                &ctx,
                &route_selection.model_name,
                route_selection.reasoning_effort_override,
            );

            let response = http_client
                .post(&count_tokens_endpoint)
                .header("Content-Type", "application/json")
                .header(
                    "Authorization",
                    format!("Bearer {}", &route_selection.api_key),
                )
                .header("x-api-key", &route_selection.api_key)
                .header("x-anthropic-version", &anthropic_version)
                .header("originator", "codex_cli_rs")
                .body(codex_body.to_string());

            let response = if let Some(beta) = &anthropic_beta {
                response.header("anthropic-beta", beta).send().await
            } else {
                response.send().await
            };

            if let Ok(resp) = response {
                upstream_status = Some(resp.status().as_u16());
                if resp.status().is_success() {
                    if let Ok(text) = resp.text().await {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            token_count = parse_input_tokens(&value);
                            if token_count.is_some() {
                                source = "codex_input_tokens".to_string();
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

    let logger = AppLogger::get();
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
    let mut successful_session_id = String::new();

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

        let resolved_target_url = resolve_upstream_url(
            &route_selection.converter,
            &route_selection.target_url,
            UpstreamOperation::Messages,
            &route_selection.model_name,
        );

        let _ = log_tx.send(format!(
            "[Req] #{} in={} out={} msgs={} stream={} tools={} system_chars={} summary={}",
            request_id,
            input_model,
            route_selection.model_name,
            anthropic_body.messages.len(),
            resolve_effective_stream(
                anthropic_body.stream,
                &route_selection.converter,
                accept_header.as_deref(),
                stream_opts,
            ),
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
        let (upstream_body, session_id) =
            if route_selection.converter.eq_ignore_ascii_case("anthropic") {
                let mut request_body = raw_request_body.clone();
                if let Some(obj) = request_body.as_object_mut() {
                    obj.insert(
                        "model".to_string(),
                        json!(route_selection.model_name.clone()),
                    );
                }
                (request_body, Uuid::new_v4().to_string())
            } else {
                transform_request_with_optional_codex_effort_override(
                    &route_selection.converter,
                    &request_backend,
                    &anthropic_body,
                    &log_tx,
                    &ctx,
                    &route_selection.model_name,
                    route_selection.reasoning_effort_override,
                )
            };

        if let Some(input_summary) = summarize_codex_payload(&upstream_body) {
            let top_keys = sorted_object_keys(&upstream_body).join(",");
            let input_items = upstream_body
                .get("input")
                .and_then(|v| v.as_array())
                .map(|arr| arr.len())
                .unwrap_or(0);
            let _ = log_tx.send(format!(
                "[ReqPayload] #{} keys={} input_items={} summary={}",
                request_id,
                top_keys,
                input_items,
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

        let response = match upstream_req.send().await {
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
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-")
                .to_string();
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();

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
        successful_session_id = session_id.clone();
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
    let session_id_for_request = successful_session_id;
    let _lb_permit = successful_lb_permit;
    let effective_stream = resolve_effective_stream(
        anthropic_body.stream,
        &request_converter,
        accept_header.as_deref(),
        stream_opts,
    );

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
        let mut stream = response.bytes_stream();
        let mut line_buffer = String::new();
        let mut frame_parser = SseFrameParser::default();
        let mut transformer = request_backend.create_response_transformer(&model);
        let mut metrics = StreamMetrics::new(request_started_at);

        let mut message_state: Option<Value> = None;
        let mut blocks: BTreeMap<usize, Value> = BTreeMap::new();
        let mut tool_input_buffers: HashMap<usize, String> = HashMap::new();
        let mut stop_reason_state: Option<String> = None;
        let mut usage_input_tokens: u64 = 0;
        let mut usage_output_tokens: u64 = 0;

        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(300), stream.next()).await {
                Ok(Some(chunk_result)) => match chunk_result {
                    Ok(chunk) => {
                        metrics.mark_upstream_chunk();
                        let chunk_text = String::from_utf8_lossy(&chunk).to_string();

                        if stream_opts.enable_sse_frame_parser {
                            for frame in frame_parser.push_chunk(&chunk_text) {
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
    tokio::spawn(async move {
        let _permit_guard = permit_for_stream;
        let mut stream = response.bytes_stream();
        let mut transformer =
            request_backend_for_stream.create_response_transformer(&model_for_stream);
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
        let mut silence_warn_logged = false;
        let mut silence_error_logged = false;

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

                let retry_session_id = Uuid::new_v4().to_string();
                active_session_id_for_stream = retry_session_id.clone();
                let retry_req = request_backend_for_stream.build_upstream_request(
                    &http_client_for_stream,
                    &upstream_url_for_stream,
                    &upstream_api_key_for_stream,
                    &active_upstream_body_for_stream,
                    &retry_session_id,
                    &anthropic_version_for_stream,
                );
                let retry_req = if let Some(beta) = anthropic_beta_for_stream.as_ref() {
                    retry_req.header("anthropic-beta", beta)
                } else {
                    retry_req
                };

                match retry_req.send().await {
                    Ok(retry_response) => {
                        let retry_status = retry_response.status().as_u16();
                        if retry_response.status().is_success() {
                            emit_stream_diag(
                                &log_tx_clone,
                                &logger_for_stream,
                                format!(
                                    "[Stream] #{} sibling_tool_error_retry_succeeded status={}",
                                    request_id_for_stream, retry_status
                                ),
                            );
                            current_upstream_status = retry_status;
                            stream = retry_response.bytes_stream();
                            transformer = request_backend_for_stream
                                .create_response_transformer(&model_for_stream);
                            line_buffer.clear();
                            frame_parser = SseFrameParser::default();
                            decision.on_retry_success_reset();
                            decision.emitted_non_heartbeat_event = false;
                            decision.emitted_business_event = false;
                            decision.emitted_tool_event = false;
                            last_upstream_activity = Instant::now();
                            continue 'stream_attempt;
                        }

                        let retry_error = retry_response.text().await.unwrap_or_default();
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} sibling_tool_error_retry_failed status={} body={}",
                                request_id_for_stream, retry_status, retry_error
                            ),
                        );
                    }
                    Err(err) => {
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} sibling_tool_error_retry_failed network_error={}",
                                request_id_for_stream, err
                            ),
                        );
                    }
                }
            }

            let tool_leak_signal = transformer
                .take_diagnostics_summary()
                .as_ref()
                .and_then(extract_tool_leak_retry_signal);
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
                active_upstream_body_for_stream = serial_fallback_upstream_body_for_stream
                    .as_ref()
                    .cloned()
                    .expect("serial fallback body should exist");
                let signal = tool_leak_signal.expect("signal should exist when retry allowed");
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} leaked_tool_text_retry_started mode=serial_parallel_tool_calls_false marker_drops={} raw_json_drops={} incomplete_json_drops={}",
                        request_id_for_stream,
                        signal.dropped_leaked_marker_fragments,
                        signal.dropped_raw_tool_json_fragments,
                        signal.dropped_incomplete_tool_json_fragments
                    ),
                );

                let retry_session_id = Uuid::new_v4().to_string();
                active_session_id_for_stream = retry_session_id.clone();
                let retry_req = request_backend_for_stream.build_upstream_request(
                    &http_client_for_stream,
                    &upstream_url_for_stream,
                    &upstream_api_key_for_stream,
                    &active_upstream_body_for_stream,
                    &retry_session_id,
                    &anthropic_version_for_stream,
                );
                let retry_req = if let Some(beta) = anthropic_beta_for_stream.as_ref() {
                    retry_req.header("anthropic-beta", beta)
                } else {
                    retry_req
                };

                match retry_req.send().await {
                    Ok(retry_response) => {
                        let retry_status = retry_response.status().as_u16();
                        if retry_response.status().is_success() {
                            emit_stream_diag(
                                &log_tx_clone,
                                &logger_for_stream,
                                format!(
                                    "[Stream] #{} leaked_tool_text_retry_succeeded status={}",
                                    request_id_for_stream, retry_status
                                ),
                            );
                            current_upstream_status = retry_status;
                            stream = retry_response.bytes_stream();
                            transformer = request_backend_for_stream
                                .create_response_transformer(&model_for_stream);
                            line_buffer.clear();
                            frame_parser = SseFrameParser::default();
                            decision.on_retry_success_reset();
                            decision.emitted_non_heartbeat_event = false;
                            decision.emitted_business_event = false;
                            decision.emitted_tool_event = false;
                            last_upstream_activity = Instant::now();
                            continue 'stream_attempt;
                        }

                        let retry_error = retry_response.text().await.unwrap_or_default();
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} leaked_tool_text_retry_failed status={} body={}",
                                request_id_for_stream, retry_status, retry_error
                            ),
                        );
                    }
                    Err(err) => {
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} leaked_tool_text_retry_failed network_error={}",
                                request_id_for_stream, err
                            ),
                        );
                    }
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

                let retry_session_id = Uuid::new_v4().to_string();
                active_session_id_for_stream = retry_session_id.clone();
                let retry_req = request_backend_for_stream.build_upstream_request(
                    &http_client_for_stream,
                    &upstream_url_for_stream,
                    &upstream_api_key_for_stream,
                    &active_upstream_body_for_stream,
                    &retry_session_id,
                    &anthropic_version_for_stream,
                );
                let retry_req = if let Some(beta) = anthropic_beta_for_stream.as_ref() {
                    retry_req.header("anthropic-beta", beta)
                } else {
                    retry_req
                };

                match retry_req.send().await {
                    Ok(retry_response) => {
                        let retry_status = retry_response.status().as_u16();
                        if retry_response.status().is_success() {
                            emit_stream_diag(
                                &log_tx_clone,
                                &logger_for_stream,
                                format!(
                                    "[Stream] #{} stream_retry_succeeded status={}",
                                    request_id_for_stream, retry_status
                                ),
                            );
                            decision.incomplete_stream_retry_succeeded = true;
                            current_upstream_status = retry_status;
                            stream = retry_response.bytes_stream();
                            line_buffer.clear();
                            frame_parser = SseFrameParser::default();
                            decision.on_retry_success_reset();
                            last_upstream_activity = Instant::now();
                            continue 'stream_attempt;
                        }

                        let retry_error = retry_response.text().await.unwrap_or_default();
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} stream_retry_failed status={} body={}",
                                request_id_for_stream, retry_status, retry_error
                            ),
                        );
                    }
                    Err(err) => {
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} stream_retry_failed network_error={}",
                                request_id_for_stream, err
                            ),
                        );
                    }
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

            // V2 alignment: remove empty-completion dedicated retry branch.
            let empty_retry_allowed =
                false && decision.allow_empty_completion_retry(stream_opts_for_task);

            if empty_retry_allowed {
                decision.empty_completion_retry_attempts += 1;
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} empty_completion_retry_started attempt={}/{}",
                        request_id_for_stream,
                        decision.empty_completion_retry_attempts,
                        stream_opts_for_task.empty_completion_retry_max_attempts
                    ),
                );

                let retry_session_id = Uuid::new_v4().to_string();
                active_session_id_for_stream = retry_session_id.clone();
                let retry_req = request_backend_for_stream.build_upstream_request(
                    &http_client_for_stream,
                    &upstream_url_for_stream,
                    &upstream_api_key_for_stream,
                    &active_upstream_body_for_stream,
                    &retry_session_id,
                    &anthropic_version_for_stream,
                );
                let retry_req = if let Some(beta) = anthropic_beta_for_stream.as_ref() {
                    retry_req.header("anthropic-beta", beta)
                } else {
                    retry_req
                };

                match retry_req.send().await {
                    Ok(retry_response) => {
                        let retry_status = retry_response.status().as_u16();
                        if retry_response.status().is_success() {
                            emit_stream_diag(
                                &log_tx_clone,
                                &logger_for_stream,
                                format!(
                                    "[Stream] #{} empty_completion_retry_succeeded status={}",
                                    request_id_for_stream, retry_status
                                ),
                            );
                            decision.empty_completion_retry_succeeded = true;
                            current_upstream_status = retry_status;
                            stream = retry_response.bytes_stream();
                            line_buffer.clear();
                            frame_parser = SseFrameParser::default();
                            decision.on_retry_success_reset();
                            last_upstream_activity = Instant::now();
                            continue 'stream_attempt;
                        }

                        let retry_error = retry_response.text().await.unwrap_or_default();
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} empty_completion_retry_failed status={} body={}",
                                request_id_for_stream, retry_status, retry_error
                            ),
                        );
                    }
                    Err(err) => {
                        emit_stream_diag(
                            &log_tx_clone,
                            &logger_for_stream,
                            format!(
                                "[Stream] #{} empty_completion_retry_failed network_error={}",
                                request_id_for_stream, err
                            ),
                        );
                    }
                }
            } else if decision.saw_response_completed
                && !decision.emitted_business_event
                && !decision.saw_message_stop
            {
                let reason = decision.empty_completion_retry_skip_reason(stream_opts_for_task);
                emit_stream_diag(
                    &log_tx_clone,
                    &logger_for_stream,
                    format!(
                        "[Stream] #{} empty_completion_retry_skipped reason={}",
                        request_id_for_stream, reason
                    ),
                );
            }

            break 'stream_attempt;
        }

        if decision.empty_completion_retry_attempts > 0 {
            emit_stream_diag(
                &log_tx_clone,
                &logger_for_stream,
                format!(
                    "[Stream] #{} empty_completion_retry_hits={} success={}",
                    request_id_for_stream,
                    decision.empty_completion_retry_attempts,
                    decision.empty_completion_retry_succeeded
                ),
            );
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

        let should_emit_empty_notice = false && decision.should_emit_empty_notice();

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

        if should_emit_empty_notice {
            decision.emitted_empty_completion_fallback_notice = true;
            emit_stream_diag(
                &log_tx_clone,
                &logger_for_stream,
                format!(
                    "[Stream] #{} empty_completion_fallback_notice_emitted",
                    request_id_for_stream
                ),
            );
            let synthetic_notice_delta = format!(
                "event: response.output_text.delta\ndata: {}\n\n",
                json!({
                    "type": "response.output_text.delta",
                    "delta": "本轮仅收到推理摘要，未产出可见内容。系统已自动重试一次，建议你直接重试或精简上下文后再试。\n"
                })
            );
            for output in transformer.transform_event(&synthetic_notice_delta) {
                if should_drop_duplicate_message_start(
                    &output,
                    &mut decision.sent_message_start_to_client,
                ) {
                    continue;
                }
                if chunk_is_message_stop(&output) {
                    decision.saw_message_stop = true;
                }
                if is_business_stream_output(&output) {
                    decision.emitted_business_event = true;
                }
                if is_tool_stream_output(&output) {
                    decision.emitted_tool_event = true;
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
                        "[Warning] #{} Client disconnected while sending empty-completion notice",
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
            decision.saw_response_failed,
            decision.saw_message_stop,
        );
        let stream_outcome = decision.stream_outcome();
        emit_stream_diag(
            &log_tx_clone,
            &logger_for_stream,
            format!(
                "[Stream] #{} stream_outcome={} saw_response_completed={} saw_response_failed={} saw_message_stop={} emitted_business_event={} final_fallback_emitted={}",
                request_id_for_stream,
                stream_outcome,
                decision.saw_response_completed,
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
        chunk_contains_sibling_tool_call_error, classify_connection_error, derive_stream_close_cause,
        disable_parallel_tool_calls_in_upstream_body, is_business_stream_output,
        extract_tool_leak_retry_signal, leaked_tool_text_retry_skip_reason,
        observe_upstream_chunk_events, resolve_effective_stream, resolve_upstream_url,
        should_suppress_premature_message_stop, sibling_tool_error_retry_skip_reason,
        ConnErrorClass, SseFrameParser, StreamEventCounters, StreamRuntimeOptions,
        UpstreamOperation,
    };
    use serde_json::json;

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
            enable_empty_completion_retry: true,
            empty_completion_retry_max_attempts: 1,
            enable_incomplete_stream_retry: true,
            incomplete_stream_retry_max_attempts: 1,
            enable_sibling_tool_error_retry: true,
        }
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
    fn test_resolve_effective_stream_forced_for_codex_when_accepts_sse() {
        let opts = stream_opts();
        assert!(resolve_effective_stream(
            false,
            "codex",
            Some("text/event-stream"),
            opts
        ));
    }

    #[test]
    fn test_resolve_effective_stream_respects_explicit_json_only() {
        let opts = stream_opts();
        assert!(!resolve_effective_stream(
            false,
            "codex",
            Some("application/json"),
            opts
        ));
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
                "dropped_incomplete_tool_json_fragments": 0
            }
        });

        let signal = extract_tool_leak_retry_signal(&summary).expect("should parse signal");
        assert_eq!(signal.dropped_leaked_marker_fragments, 2);
        assert_eq!(signal.dropped_raw_tool_json_fragments, 1);
        assert_eq!(signal.dropped_incomplete_tool_json_fragments, 0);
    }

    #[test]
    fn test_allow_leaked_tool_text_retry_requires_signal_and_clean_state() {
        let signal = Some(super::ToolLeakRetrySignal {
            dropped_leaked_marker_fragments: 1,
            dropped_raw_tool_json_fragments: 0,
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
            dropped_incomplete_tool_json_fragments: 0,
        });

        assert_eq!(
            leaked_tool_text_retry_skip_reason(&state, true, true, signal),
            Some("tool_event_emitted")
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
    fn test_observe_upstream_chunk_events_marks_flags_and_counts() {
        let mut saw_completed = false;
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
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(!saw_completed);
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
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(saw_completed);
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
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(saw_completed);
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
            &mut saw_failed,
            &mut saw_sibling,
            &mut upstream_error_event_type,
            &mut upstream_error_message,
            &mut upstream_error_code,
            &mut counters,
        );

        assert!(!saw_completed);
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
        let cause = derive_stream_close_cause(Some("client_disconnected"), false, false, false);
        assert_eq!(cause, "client_disconnected");
    }

    #[test]
    fn test_derive_stream_close_cause_completed_without_stop() {
        let cause = derive_stream_close_cause(None, true, false, false);
        assert_eq!(cause, "completed_without_message_stop");
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
            output, true, false, false
        ));
        assert!(!should_suppress_premature_message_stop(
            output, false, false, false
        ));
        assert!(!should_suppress_premature_message_stop(
            output, true, true, false
        ));
    }
}
