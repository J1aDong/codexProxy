use codex_proxy_core::{
    get_reasoning_effort, is_debug_log_enabled, set_debug_log, AppLogger, ReasoningEffort,
    ReasoningEffortMapping,
};
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
