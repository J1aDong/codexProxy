use crate::logger::AppLogger;
use crate::models::{AnthropicRequest, CodexModelMapping, ContentBlock, GeminiReasoningEffortMapping, Message, MessageContent, ReasoningEffort, ReasoningEffortMapping};
use crate::transform::{CodexBackend, GeminiBackend, TransformBackend, TransformContext};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

pub struct ProxyServer {
    port: u16,
    target_url: String,
    api_key: Option<String>,
    reasoning_mapping: ReasoningEffortMapping,
    skill_injection_prompt: String,
    converter: String,
    codex_model: String,
    codex_model_mapping: CodexModelMapping,
    gemini_reasoning_effort: GeminiReasoningEffortMapping,
    max_concurrency: u32,
    ignore_probe_requests: bool,
    allow_count_tokens_fallback_estimate: bool,
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

fn sorted_object_keys(value: &Value) -> Vec<String> {
    let mut keys = value
        .as_object()
        .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    keys
}

fn summarize_message_content_block(block: &Value) -> String {
    let block_type = block
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let keys = sorted_object_keys(block).join(",");
    format!("{}<{}>", block_type, keys)
}

fn summarize_codex_input_item(index: usize, item: &Value) -> String {
    let item_type = item
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
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

fn extract_cooldown_info(status: u16, error_text: &str, retry_after_header: &str, default_model: &str) -> Option<(String, u64, String)> {
    if status != StatusCode::TOO_MANY_REQUESTS.as_u16() {
        return None;
    }

    let retry_after_secs = parse_seconds_str(retry_after_header);
    let parsed = serde_json::from_str::<Value>(error_text).ok();
    let error_obj = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .or(parsed.as_ref());

    let code = error_obj
        .and_then(|value| value.get("code"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

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

fn get_active_cooldown_seconds(cooldowns: &Arc<Mutex<HashMap<String, Instant>>>, model: &str) -> Option<u64> {
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
        map.insert(model.to_string(), Instant::now() + Duration::from_secs(seconds));
    }
}

fn strip_query(url: String) -> String {
    if let Some((head, _)) = url.split_once('?') {
        head.to_string()
    } else {
        url
    }
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
                        ContentBlock::Thinking { thinking, .. } => chars += thinking.chars().count(),
                        ContentBlock::ToolUse { name, input, .. } => {
                            chars += name.chars().count();
                            chars += serde_json::to_string(input).unwrap_or_default().chars().count();
                        }
                        ContentBlock::ToolResult { content, .. } => {
                            chars += content
                                .as_ref()
                                .map(|v| serde_json::to_string(v).unwrap_or_default().chars().count())
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
        chars += serde_json::to_string(tools).unwrap_or_default().chars().count();
    }

    ((chars as f64) / 4.0).ceil() as u64
}

impl ProxyServer {
    pub fn new(port: u16, target_url: String, api_key: Option<String>) -> Self {
        Self {
            port,
            target_url,
            api_key,
            reasoning_mapping: ReasoningEffortMapping::default(),
            skill_injection_prompt: String::new(),
            converter: "codex".to_string(),
            codex_model: "gpt-5.3-codex".to_string(),
            codex_model_mapping: CodexModelMapping::default(),
            gemini_reasoning_effort: GeminiReasoningEffortMapping::default(),
            max_concurrency: 0,
            ignore_probe_requests: false,
            allow_count_tokens_fallback_estimate: true,
        }
    }

    pub fn with_reasoning_mapping(mut self, mapping: ReasoningEffortMapping) -> Self {
        self.reasoning_mapping = mapping;
        self
    }

    pub fn with_skill_injection_prompt(mut self, prompt: String) -> Self {
        self.skill_injection_prompt = prompt;
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



    pub fn with_gemini_reasoning_effort(mut self, effort: GeminiReasoningEffortMapping) -> Self {
        self.gemini_reasoning_effort = effort;
        self
    }

    pub fn with_max_concurrency(mut self, max: u32) -> Self {
        self.max_concurrency = max;
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

    /// Start the proxy server and return a shutdown sender + JoinHandle
    /// Send () to the returned sender to stop the server
    pub async fn start(
        &self,
        log_tx: broadcast::Sender<String>,
    ) -> Result<(broadcast::Sender<()>, tokio::task::JoinHandle<()>), Box<dyn std::error::Error + Send + Sync>> {
        // 初始化全局日志记录器
        let logger = AppLogger::init(Some("logs"));
        logger.log("=== Codex Proxy Started ===");

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr).await?;

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_tx_clone = shutdown_tx.clone();

        let target_url = Arc::new(self.target_url.clone());
        let api_key = Arc::new(self.api_key.clone());
        let ignore_probe_requests = self.ignore_probe_requests;
        let allow_count_tokens_fallback_estimate = self.allow_count_tokens_fallback_estimate;
        let model_cooldowns: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));

        // 构建共享的 TransformContext
        let ctx = Arc::new(TransformContext {
            reasoning_mapping: self.reasoning_mapping.clone(),
            codex_model_mapping: self.codex_model_mapping.clone(),
            skill_injection_prompt: self.skill_injection_prompt.clone(),
            converter: self.converter.clone(),
            codex_model: self.codex_model.clone(),
            gemini_reasoning_effort: self.gemini_reasoning_effort.clone(),
        });

        // 按 converter 选择后端，默认走 Codex
        let backend: Arc<dyn TransformBackend> = if self.converter.eq_ignore_ascii_case("gemini") {
            Arc::new(GeminiBackend)
        } else {
            Arc::new(CodexBackend)
        };

        // 并发控制：0 = 不限制
        let semaphore: Option<Arc<Semaphore>> = if self.max_concurrency > 0 {
            let _ = log_tx.send(format!("[System] Max concurrency: {}", self.max_concurrency));
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

        let _ = log_tx.send(format!(
            "[System] Init success: Codex Proxy (Rust) listening on http://localhost:{}",
            self.port
        ));
        let _ = log_tx.send(format!("[System] Target: {}", self.target_url));
        logger.log(&format!("Listening on http://localhost:{}", self.port));
        logger.log(&format!("Target: {}", self.target_url));

        // Spawn the server loop in a separate task
        let handle = tokio::spawn(async move {
            let mut conn_tasks = tokio::task::JoinSet::new();

            loop {
                let mut shutdown_rx = shutdown_tx_clone.subscribe();

                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _)) => {
                                let io = TokioIo::new(stream);
                                let target_url = Arc::clone(&target_url);
                                let api_key = Arc::clone(&api_key);
                                let ctx = Arc::clone(&ctx);
                                let backend = Arc::clone(&backend);
                                let http_client = Arc::clone(&http_client);
                                let semaphore = semaphore.clone();
                                let ignore_probe_requests = ignore_probe_requests;
                                let allow_count_tokens_fallback_estimate = allow_count_tokens_fallback_estimate;
                                let model_cooldowns = Arc::clone(&model_cooldowns);
                                let log_tx = log_tx.clone();

                                conn_tasks.spawn(async move {
                                    let service = service_fn(move |req| {
                                        handle_request(
                                            req,
                                            Arc::clone(&target_url),
                                            Arc::clone(&api_key),
                                            Arc::clone(&ctx),
                                            Arc::clone(&backend),
                                            Arc::clone(&http_client),
                                            semaphore.clone(),
                                            ignore_probe_requests,
                                            allow_count_tokens_fallback_estimate,
                                            Arc::clone(&model_cooldowns),
                                            log_tx.clone(),
                                        )
                                    });

                                    if let Err(e) = http1::Builder::new()
                                        .serve_connection(io, service)
                                        .await
                                    {
                                        eprintln!("Connection error: {}", e);
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

        Ok((shutdown_tx, handle))
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    target_url: Arc<String>,
    api_key: Arc<Option<String>>,
    ctx: Arc<TransformContext>,
    backend: Arc<dyn TransformBackend>,
    http_client: Arc<reqwest::Client>,
    semaphore: Option<Arc<Semaphore>>,
    ignore_probe_requests: bool,
    allow_count_tokens_fallback_estimate: bool,
    model_cooldowns: Arc<Mutex<HashMap<String, Instant>>>,
    log_tx: broadcast::Sender<String>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let path = req.uri().path();
    let normalized_path = path.trim_end_matches('/');
    let is_messages = normalized_path == "/messages" || normalized_path == "/v1/messages";
    let is_count_tokens = normalized_path == "/messages/count_tokens" || normalized_path == "/v1/messages/count_tokens";
    let request_id: String = Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect();

    // 只处理 POST /messages、/v1/messages、/messages/count_tokens、/v1/messages/count_tokens
    if req.method() != Method::POST || (!is_messages && !is_count_tokens) {
        let _ = log_tx.send(format!("[Debug] Ignored {} request to {}", req.method(), path));
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(full_body(
                json!({"error": {"type": "not_found", "message": "Not found"}}).to_string(),
            ))
            .unwrap());
    }

    let _ = log_tx.send(format!("[System] Processing #{} {} {}", request_id, req.method(), path));

    // 并发控制：获取许可证，FIFO 排队
    let permit: Option<OwnedSemaphorePermit> = if let Some(ref sem) = semaphore {
        let _ = log_tx.send(format!(
            "[System] #{} waiting for concurrency permit (available: {})",
            request_id,
            sem.available_permits(),
        ));
        Some(Arc::clone(sem).acquire_owned().await.expect("semaphore closed"))
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

    // 确定最终使用的 API key
    let final_api_key = if let Some(ref key) = *api_key {
        // 环境变量配置的 key 优先
        Some(key.clone())
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
                json!({"error": {"type": "unauthorized", "message": "Missing API key"}}).to_string(),
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
                    json!({"error": {"message": format!("Failed to read body: {}", e)}}).to_string(),
                ))
                .unwrap());
        }
    };

    // 解析 Anthropic 请求
    let anthropic_body: AnthropicRequest = match serde_json::from_slice(&body_bytes) {
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
                anthropic_body.tools.as_ref().map(|tools| tools.len()).unwrap_or(0),
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

    let model_name = if ctx.converter.eq_ignore_ascii_case("gemini") {
        // Map Anthropic model to Gemini model using reasoning effort configuration
        let input_model = anthropic_body.model.as_deref().unwrap_or("claude-3-5-sonnet-20240620");
        let effort = crate::models::get_reasoning_effort(
            input_model,
            &ctx.reasoning_mapping
        );

        if let Some(ref l) = AppLogger::get() {
            l.log(&format!("[Debug] Mapping Gemini - Input: {}, Effort: {:?}", input_model, effort));
            l.log(&format!("[Debug] Current Gemini Config: {:?}", ctx.gemini_reasoning_effort));
        }

        match effort {
            ReasoningEffort::Xhigh => {
                let msg = format!("[Debug] Mapping to Opus (Xhigh) -> {}", ctx.gemini_reasoning_effort.opus);
                let _ = log_tx.send(msg.clone());
                if let Some(ref l) = AppLogger::get() { l.log(&msg); }
                ctx.gemini_reasoning_effort.opus.clone()
            },
            ReasoningEffort::High | ReasoningEffort::Medium => {
                let msg = format!("[Debug] Mapping to Sonnet (High/Medium) -> {}", ctx.gemini_reasoning_effort.sonnet);
                let _ = log_tx.send(msg.clone());
                if let Some(ref l) = AppLogger::get() { l.log(&msg); }
                ctx.gemini_reasoning_effort.sonnet.clone()
            },
            ReasoningEffort::Low => {
                let msg = format!("[Debug] Mapping to Haiku (Low) -> {}", ctx.gemini_reasoning_effort.haiku);
                let _ = log_tx.send(msg.clone());
                if let Some(ref l) = AppLogger::get() { l.log(&msg); }
                ctx.gemini_reasoning_effort.haiku.clone()
            },
        }
    } else {
        let input_model = anthropic_body.model.as_deref().unwrap_or("claude-3-5-sonnet-20240620");
        let effort = crate::models::get_reasoning_effort(input_model, &ctx.reasoning_mapping);
        match effort {
            ReasoningEffort::Xhigh => ctx.codex_model_mapping.opus.clone(),
            ReasoningEffort::High | ReasoningEffort::Medium => ctx.codex_model_mapping.sonnet.clone(),
            ReasoningEffort::Low => ctx.codex_model_mapping.haiku.clone(),
        }
    };
    let input_model = anthropic_body.model.as_deref().unwrap_or("claude-3-5-sonnet-20240620");
    if let Some(family) = detect_model_family(input_model) {
        let _ = log_tx.send(format!("[Stat] model_request:{}", family));
    }

    let display_summary = summarize_request_messages(&anthropic_body.messages);
    let tool_count = anthropic_body.tools.as_ref().map(|tools| tools.len()).unwrap_or(0);
    let system_chars = anthropic_body
        .system
        .as_ref()
        .map(|system| system.to_string().chars().count())
        .unwrap_or(0);

    let _ = log_tx.send(format!(
        "[Req] #{} in={} out={} msgs={} stream={} tools={} system_chars={} summary={}",
        request_id,
        input_model,
        model_name,
        anthropic_body.messages.len(),
        anthropic_body.stream,
        tool_count,
        system_chars,
        display_summary,
    ));

    if is_count_tokens {
        let _ = log_tx.send(format!(
            "[Req] #{} mode=count_tokens converter={} in={} out={}",
            request_id,
            ctx.converter,
            input_model,
            model_name,
        ));

        let mut token_count: Option<u64> = None;
        let mut upstream_status: Option<u16> = None;
        let mut source = "estimate".to_string();

        if ctx.converter.eq_ignore_ascii_case("gemini") {
            let endpoint = build_gemini_count_tokens_endpoint(&target_url, &model_name);
            let (messages, _) = crate::transform::MessageProcessor::transform_messages(&anthropic_body.messages, Some(&log_tx));
            let contents = GeminiBackend::build_contents_for_count(&messages);
            let body = json!({ "contents": contents });

            let response = http_client
                .post(endpoint)
                .header("Content-Type", "application/json")
                .header("x-goog-api-key", &final_api_key)
                .header("Authorization", format!("Bearer {}", &final_api_key))
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
        } else {
            let endpoint = build_codex_input_tokens_endpoint(&target_url);
            let (codex_body, _) = backend.transform_request(
                &anthropic_body,
                Some(&log_tx),
                &ctx,
                Some(model_name.clone()),
            );

            let response = http_client
                .post(endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", &final_api_key))
                .header("x-api-key", &final_api_key)
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

        let input_tokens = if let Some(tokens) = token_count {
            tokens
        } else if allow_count_tokens_fallback_estimate {
            source = "estimate".to_string();
            estimate_input_tokens(&anthropic_body)
        } else {
            let _ = log_tx.send(format!(
                "[Tokens] #{} failed upstream_status={} fallback=disabled",
                request_id,
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
            "[Tokens] #{} input_tokens={} source={} upstream_status={}",
            request_id,
            input_tokens,
            source,
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

    if let Some(remaining_secs) = get_active_cooldown_seconds(&model_cooldowns, &model_name) {
        let _ = log_tx.send(format!(
            "[RateLimit] #{} local_cooldown model={} retry_after={}s in={} out={} msgs={} summary={}",
            request_id,
            model_name,
            remaining_secs,
            input_model,
            model_name,
            anthropic_body.messages.len(),
            display_summary,
        ));

        let payload = json!({
            "error": {
                "type": "rate_limit_error",
                "source": "local_cooldown",
                "model": model_name,
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

    // 通过 trait 转换请求
    let (codex_body, session_id) = backend.transform_request(
        &anthropic_body,
        Some(&log_tx),
        &ctx,
        Some(model_name.clone()),
    );

    if let Some(input_summary) = summarize_codex_payload(&codex_body) {
        let top_keys = sorted_object_keys(&codex_body).join(",");
        let input_items = codex_body
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
    let model = model_name;

    // 获取全局日志记录器
    let logger = AppLogger::get();

    // 记录原始 Anthropic 请求到日志文件
    if let Some(ref l) = logger {
        l.log_anthropic_request(&body_bytes);
    }

    // 记录转换后的 Codex 请求（curl 格式）
    if let Some(ref l) = logger {
        let headers = vec![
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer <API_KEY>"),
            ("User-Agent", "Anthropic-Node/0.3.4"),
            ("x-anthropic-version", &anthropic_version),
            ("Accept", "text/event-stream"),
            ("session_id", &session_id),
        ];
        l.log_curl_request("POST", &target_url, &headers, &codex_body);
    }

    // 通过 trait 构建上游请求
    let upstream_req = backend.build_upstream_request(
        &http_client,
        &target_url,
        &final_api_key,
        &codex_body,
        &session_id,
        &anthropic_version,
    );

    let upstream_req = if let Some(beta) = &anthropic_beta {
        upstream_req.header("anthropic-beta", beta)
    } else {
        upstream_req
    };

    // 发送到目标服务器
    let response = match upstream_req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let _ = log_tx.send(format!("[Error] Request failed: {}", e));
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

        if let Some((cooldown_model, cooldown_secs, reason)) = extract_cooldown_info(status, &error_text, &retry_after, &model) {
            set_model_cooldown(&model_cooldowns, &cooldown_model, cooldown_secs);
            let _ = log_tx.send(format!(
                "[RateLimit] #{} upstream=429 reason={} model={} retry_after={}s in={} out={} msgs={} summary={}",
                request_id,
                reason,
                cooldown_model,
                cooldown_secs,
                input_model,
                model,
                anthropic_body.messages.len(),
                display_summary,
            ));
        }

        let _ = log_tx.send(format!("[Error] #{} Upstream returned {}: {}", request_id, status, error_text));

        // 记录错误响应到日志文件
        if let Some(ref l) = logger {
            l.log_upstream_response(status, &error_text);
        }

        return Ok(Response::builder()
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY))
            .header("Content-Type", "application/json")
            .body(full_body(error_text))
            .unwrap());
    }

    let _ = log_tx.send(format!(
        "[System] #{} Request transformed and forwarding to Codex Responses API",
        request_id
    ));

    let upstream_status = response.status().as_u16();

    // 使用 channel 进行流式响应
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, Infallible>>(256);

    // 通过 trait 创建响应转换器
    let mut transformer = backend.create_response_transformer(&model);

    let log_tx_clone = log_tx.clone();
    let request_id_for_stream = request_id.clone();
    let permit_for_stream = permit;
    tokio::spawn(async move {
        let _permit_guard = permit_for_stream;
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        loop {
            // 添加 300 秒读取超时
            match tokio::time::timeout(std::time::Duration::from_secs(300), stream.next()).await {
                Ok(Some(chunk_result)) => {
                    match chunk_result {
                        Ok(chunk) => {
                            buffer.push_str(&String::from_utf8_lossy(&chunk));

                            // 按行处理
                            while let Some(pos) = buffer.find('\n') {
                                let line = buffer[..pos].to_string();
                                buffer = buffer[pos + 1..].to_string();

                                if line.trim().is_empty() {
                                    continue;
                                }

                                // 记录上游原始响应
                                if let Some(ref l) = AppLogger::get() {
                                    l.log_upstream_response(upstream_status, &line);
                                }

                                for output in transformer.transform_line(&line) {
                                    // 记录转换后的响应
                                    if let Some(ref l) = AppLogger::get() {
                                        l.log_anthropic_response(&output);
                                    }
                                    if tx.send(Ok(Frame::data(Bytes::from(output)))).await.is_err() {
                                        let _ = log_tx_clone.send(format!(
                                            "[Warning] #{} Client disconnected, stopping stream",
                                            request_id_for_stream
                                        ));
                                        return; // 客户端断开，停止处理
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = log_tx_clone.send(format!("[Error] #{} Stream error: {}", request_id_for_stream, e));
                            break;
                        }
                    }
                }
                Ok(None) => break, // 流结束
                Err(_) => {
                    let _ = log_tx_clone.send(format!(
                        "[Error] #{} Upstream read timeout (300s)",
                        request_id_for_stream
                    ));
                    break;
                }
            }
        }

        // 处理剩余的 buffer
        if !buffer.trim().is_empty() {
            // 记录上游原始响应
            if let Some(ref l) = AppLogger::get() {
                l.log_upstream_response(upstream_status, &buffer);
            }

            for output in transformer.transform_line(&buffer) {
                // 记录转换后的响应
                if let Some(ref l) = AppLogger::get() {
                    l.log_anthropic_response(&output);
                }
                if tx.send(Ok(Frame::data(Bytes::from(output)))).await.is_err() {
                    let _ = log_tx_clone.send(format!(
                        "[Warning] #{} Client disconnected during flush",
                        request_id_for_stream
                    ));
                    return;
                }
            }
        }

        // 记录完成
        if let Some(ref l) = AppLogger::get() {
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

fn full_body(s: String) -> BoxBody<Bytes, Infallible> {
    BoxBody::new(Full::new(Bytes::from(s)).map_err(|_: Infallible| unreachable!()))
}
