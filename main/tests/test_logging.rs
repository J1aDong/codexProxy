use codex_proxy_core::{
    get_reasoning_effort, is_debug_log_enabled, set_debug_log, AppLogger, ReasoningEffort,
    ReasoningEffortMapping, TransformRequest,
};
use serde_json::json;
use std::fs;

#[test]
fn test_debug_log_control() {
    // 测试日志开关
    set_debug_log(true);
    assert!(is_debug_log_enabled());

    set_debug_log(false);
    assert!(!is_debug_log_enabled());

    set_debug_log(true);
    assert!(is_debug_log_enabled());

    println!("✅ 日志开关测试通过");
}

#[test]
fn test_app_logger() {
    // 清理测试目录
    let test_log_dir = "/tmp/codex_proxy_test_logs";
    let _ = fs::remove_dir_all(test_log_dir);

    // 强制开启日志
    set_debug_log(true);

    // 创建 AppLogger
    let logger = AppLogger::init(Some(test_log_dir));

    // 写入日志
    logger.log("这是第一条测试日志");
    logger.log("这是第二条测试日志");
    logger.log("🖼️ [Image] base64 data: iVBORw0KGgo... (len=12345)");

    // 检查日志文件
    let log_path = logger.log_path();
    println!("日志文件路径: {:?}", log_path);

    assert!(log_path.exists(), "日志文件应该存在");

    let content = fs::read_to_string(log_path).expect("应该能读取日志文件");
    println!("日志内容:\n{}", content);

    assert!(content.contains("这是第一条测试日志"));
    assert!(content.contains("这是第二条测试日志"));
    assert!(content.contains("🖼️ [Image]"));

    // 清理
    let _ = fs::remove_dir_all(test_log_dir);

    println!("✅ AppLogger 测试通过");
}

#[test]
fn test_transform_with_logging() {
    // 强制开启日志
    set_debug_log(true);

    // 创建 broadcast channel
    let (log_tx, mut log_rx) = tokio::sync::broadcast::channel::<String>(256);

    // 构造测试请求 - 包含图片
    let request_json = json!({
        "model": "claude-3-opus-20240229",
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "请描述这张图片"
                    },
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                        }
                    }
                ]
            }
        ],
        "system": "你是一个有帮助的助手",
        "tools": [
            {
                "name": "Read",
                "description": "读取文件",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }
        ],
        "stream": true
    });

    // 解析请求
    let anthropic_body: codex_proxy_core::AnthropicRequest =
        serde_json::from_value(request_json).expect("应该能解析请求");

    // 执行转换
    let reasoning_mapping = ReasoningEffortMapping::default();
    let (codex_body, session_id) = TransformRequest::transform(
        &anthropic_body,
        Some(&log_tx),
        &reasoning_mapping,
        "",
        "gpt-5.3-codex",
    );

    println!("Session ID: {}", session_id);
    println!(
        "Codex Body: {}",
        serde_json::to_string_pretty(&codex_body).unwrap()
    );

    // 收集 broadcast 日志
    let mut broadcast_logs = Vec::new();
    while let Ok(msg) = log_rx.try_recv() {
        println!("[Broadcast] {}", msg);
        broadcast_logs.push(msg);
    }

    // 验证 broadcast 日志
    assert!(!broadcast_logs.is_empty(), "应该有 broadcast 日志");

    let all_logs = broadcast_logs.join("\n");
    assert!(
        all_logs.contains("[Transform] Session"),
        "应该包含 Session 日志"
    );
    assert!(
        all_logs.contains("[Transform]") && all_logs.contains("→"),
        "应该包含 Model 日志"
    );
    assert!(all_logs.contains("[Messages]"), "应该包含 Messages 日志");

    println!("✅ Transform 日志测试通过");
}

#[test]
fn test_image_in_message() {
    // 强制开启日志
    set_debug_log(true);

    let (log_tx, mut log_rx) = tokio::sync::broadcast::channel::<String>(256);

    // 测试各种图片格式
    let request_json = json!({
        "model": "claude-3-opus",
        "messages": [
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "图片1" },
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk"
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "图片2 - URL" },
                    {
                        "type": "image",
                        "source": {
                            "url": "https://example.com/image.png"
                        }
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "图片3 - image_url" },
                    {
                        "type": "image_url",
                        "image_url": { "url": "https://example.com/another.png" }
                    }
                ]
            }
        ],
        "stream": true
    });

    let anthropic_body: codex_proxy_core::AnthropicRequest =
        serde_json::from_value(request_json).expect("应该能解析请求");

    let reasoning_mapping = ReasoningEffortMapping::default();
    let (_codex_body, _session_id) = TransformRequest::transform(
        &anthropic_body,
        Some(&log_tx),
        &reasoning_mapping,
        "",
        "gpt-5.3-codex",
    );

    // 收集日志
    let mut logs = Vec::new();
    while let Ok(msg) = log_rx.try_recv() {
        println!("{}", msg);
        logs.push(msg);
    }

    let all_logs = logs.join("\n");

    // 验证图片日志
    assert!(all_logs.contains("🖼️"), "应该包含图片日志");
    assert!(
        all_logs.contains("Image base64") || all_logs.contains("Image source"),
        "应该包含图片详情"
    );

    println!("✅ 图片消息日志测试通过");
}

#[test]
fn test_reasoning_effort_mapping() {
    let default_mapping = ReasoningEffortMapping::default();
    assert_eq!(default_mapping.opus, ReasoningEffort::Xhigh);
    assert_eq!(default_mapping.sonnet, ReasoningEffort::Medium);
    assert_eq!(default_mapping.haiku, ReasoningEffort::Low);

    assert_eq!(
        get_reasoning_effort("claude-3-opus-20240229", &default_mapping),
        ReasoningEffort::Xhigh
    );
    assert_eq!(
        get_reasoning_effort("claude-sonnet-4-20250514", &default_mapping),
        ReasoningEffort::Medium
    );
    assert_eq!(
        get_reasoning_effort("claude-3-5-sonnet-20241022", &default_mapping),
        ReasoningEffort::Medium
    );
    assert_eq!(
        get_reasoning_effort("claude-3-haiku-20240307", &default_mapping),
        ReasoningEffort::Low
    );

    let custom_mapping = ReasoningEffortMapping {
        opus: ReasoningEffort::High,
        sonnet: ReasoningEffort::Low,
        haiku: ReasoningEffort::Medium,
    };
    assert_eq!(
        get_reasoning_effort("claude-3-opus-20240229", &custom_mapping),
        ReasoningEffort::High
    );
    assert_eq!(
        get_reasoning_effort("claude-sonnet-4-20250514", &custom_mapping),
        ReasoningEffort::Low
    );
    assert_eq!(
        get_reasoning_effort("claude-3-haiku-20240307", &custom_mapping),
        ReasoningEffort::Medium
    );

    assert_eq!(
        get_reasoning_effort("unknown-model", &default_mapping),
        ReasoningEffort::Medium
    );

    assert_eq!(ReasoningEffort::Low.as_str(), "low");
    assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
    assert_eq!(ReasoningEffort::High.as_str(), "high");
    assert_eq!(ReasoningEffort::Xhigh.as_str(), "xhigh");

    println!("✅ Reasoning effort mapping 测试通过");
}

#[test]
fn test_transform_with_custom_reasoning_effort() {
    set_debug_log(true);

    let (log_tx, _log_rx) = tokio::sync::broadcast::channel::<String>(256);

    let opus_request = json!({
        "model": "claude-3-opus-20240229",
        "messages": [{ "role": "user", "content": "Hello" }],
        "stream": true
    });

    let anthropic_body: codex_proxy_core::AnthropicRequest =
        serde_json::from_value(opus_request).expect("应该能解析请求");

    let default_mapping = ReasoningEffortMapping::default();
    let (codex_body, _) = TransformRequest::transform(
        &anthropic_body,
        Some(&log_tx),
        &default_mapping,
        "",
        "gpt-5.3-codex",
    );

    let body_str = serde_json::to_string(&codex_body).unwrap();
    assert!(
        body_str.contains("\"reasoning\""),
        "Should contain reasoning field"
    );
    assert!(
        body_str.contains("\"effort\":\"xhigh\""),
        "Opus should have xhigh effort"
    );

    let sonnet_request = json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{ "role": "user", "content": "Hello" }],
        "stream": true
    });

    let anthropic_body: codex_proxy_core::AnthropicRequest =
        serde_json::from_value(sonnet_request).expect("应该能解析请求");

    let (codex_body, _) = TransformRequest::transform(
        &anthropic_body,
        Some(&log_tx),
        &default_mapping,
        "",
        "gpt-5.3-codex",
    );

    let body_str = serde_json::to_string(&codex_body).unwrap();
    assert!(
        body_str.contains("\"effort\":\"medium\""),
        "Sonnet should have medium effort"
    );

    let haiku_request = json!({
        "model": "claude-3-haiku-20240307",
        "messages": [{ "role": "user", "content": "Hello" }],
        "stream": true
    });

    let anthropic_body: codex_proxy_core::AnthropicRequest =
        serde_json::from_value(haiku_request).expect("应该能解析请求");

    let (codex_body, _) = TransformRequest::transform(
        &anthropic_body,
        Some(&log_tx),
        &default_mapping,
        "",
        "gpt-5.3-codex",
    );

    let body_str = serde_json::to_string(&codex_body).unwrap();
    assert!(
        body_str.contains("\"effort\":\"low\""),
        "Haiku should have low effort"
    );

    println!("✅ Transform with custom reasoning effort 测试通过");
}
