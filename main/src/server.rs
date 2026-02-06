use crate::transform::{AnthropicRequest, AppLogger, ReasoningEffortMapping, TransformRequest, TransformResponse};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

pub struct ProxyServer {
    port: u16,
    target_url: String,
    api_key: Option<String>,
    reasoning_mapping: ReasoningEffortMapping,
    skill_injection_prompt: String,
    codex_model: String,
}

impl ProxyServer {
    pub fn new(port: u16, target_url: String, api_key: Option<String>) -> Self {
        Self {
            port,
            target_url,
            api_key,
            reasoning_mapping: ReasoningEffortMapping::default(),
            skill_injection_prompt: String::new(),
            codex_model: "gpt-5.3-codex".to_string(),
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

    pub fn with_codex_model(mut self, model: String) -> Self {
        self.codex_model = model;
        self
    }

    /// Start the proxy server and return a shutdown sender
    /// Send () to the returned sender to stop the server
    pub async fn start(
        &self,
        log_tx: broadcast::Sender<String>,
    ) -> Result<broadcast::Sender<()>, Box<dyn std::error::Error + Send + Sync>> {
        // 初始化全局日志记录器
        let logger = AppLogger::init(Some("logs"));
        logger.log("=== Codex Proxy Started ===");

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr).await?;

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_tx_clone = shutdown_tx.clone();

        let target_url = Arc::new(self.target_url.clone());
        let api_key = Arc::new(self.api_key.clone());
        let reasoning_mapping = Arc::new(self.reasoning_mapping.clone());
        let skill_injection_prompt = Arc::new(self.skill_injection_prompt.clone());
        let codex_model = Arc::new(self.codex_model.clone());

        let _ = log_tx.send(format!(
            "🚀 Codex Proxy (Rust) listening on http://localhost:{}",
            self.port
        ));
        let _ = log_tx.send(format!("🎯 Target: {}", self.target_url));
        logger.log(&format!("Listening on http://localhost:{}", self.port));
        logger.log(&format!("Target: {}", self.target_url));

        // Spawn the server loop in a separate task
        tokio::spawn(async move {
            loop {
                let mut shutdown_rx = shutdown_tx_clone.subscribe();

                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _)) => {
                                let io = TokioIo::new(stream);
                                let target_url = Arc::clone(&target_url);
                                let api_key = Arc::clone(&api_key);
                                let reasoning_mapping = Arc::clone(&reasoning_mapping);
                                let skill_injection_prompt = Arc::clone(&skill_injection_prompt);
                                let codex_model = Arc::clone(&codex_model);
                                let log_tx = log_tx.clone();

                                tokio::spawn(async move {
                                    let service = service_fn(move |req| {
                                        handle_request(
                                            req,
                                            Arc::clone(&target_url),
                                            Arc::clone(&api_key),
                                            Arc::clone(&reasoning_mapping),
                                            Arc::clone(&skill_injection_prompt),
                                            Arc::clone(&codex_model),
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
                        let _ = log_tx.send("[System] Proxy server shutting down...".to_string());
                        break;
                    }
                }
            }
        });

        Ok(shutdown_tx)
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    target_url: Arc<String>,
    api_key: Arc<Option<String>>,
    reasoning_mapping: Arc<ReasoningEffortMapping>,
    skill_injection_prompt: Arc<String>,
    codex_model: Arc<String>,
    log_tx: broadcast::Sender<String>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let path = req.uri().path();

    // 只处理 POST /messages 或 /v1/messages
    if req.method() != Method::POST || !path.contains("/messages") {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(full_body(
                json!({"error": {"type": "not_found", "message": "Not found"}}).to_string(),
            ))
            .unwrap());
    }

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

    let _ = log_tx.send(format!(
        "📥 Anthropic Request: model={:?}, messages={}, tools={}",
        anthropic_body.model,
        anthropic_body.messages.len(),
        anthropic_body.tools.as_ref().map(|t| t.len()).unwrap_or(0)
    ));

    // 转换请求
    let (codex_body, session_id) = TransformRequest::transform(
        &anthropic_body,
        Some(&log_tx),
        &reasoning_mapping,
        &skill_injection_prompt,
    );
    let model = anthropic_body
        .model
        .clone()
        .unwrap_or_else(|| codex_model.as_ref().clone());

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

    // 发送到目标服务器
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let response = match client
        .post(&*target_url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", final_api_key))
        .header("x-api-key", &final_api_key)
        .header("User-Agent", "Anthropic-Node/0.3.4")
        .header("x-anthropic-version", &anthropic_version)
        .header("originator", "codex_cli_rs")
        .header("Accept", "text/event-stream")
        .header("conversation_id", &session_id)
        .header("session_id", &session_id)
        .body(codex_body.to_string())
        .send()
        .await
    {
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
        let status = response.status().as_u16();
        let error_text = response.text().await.unwrap_or_default();
        let _ = log_tx.send(format!("[Error] Upstream returned {}: {}", status, error_text));

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

    let _ = log_tx.send("[✅] Anthropic Messages → Codex Responses API".to_string());

    let upstream_status = response.status().as_u16();

    // 使用 channel 进行流式响应
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, Infallible>>(256);

    let log_tx_clone = log_tx.clone();
    tokio::spawn(async move {
        let mut transformer = TransformResponse::new(&model);
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
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
                            let _ = tx.send(Ok(Frame::data(Bytes::from(output)))).await;
                        }
                    }
                }
                Err(e) => {
                    let _ = log_tx_clone.send(format!("[Error] Stream error: {}", e));
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
                let _ = tx.send(Ok(Frame::data(Bytes::from(output)))).await;
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
