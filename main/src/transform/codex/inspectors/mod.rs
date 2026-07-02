//! 治理 side-channel 钩子（D4）
//!
//! 把泄漏检测、plan 桥接、commentary 阶段、Markdown Bash 拦截、RawToolJson 评估等治理逻辑
//! 从核心流式状态机剥离为独立钩子。核心状态机只产生"候选输出"，钩子决定是否抑制/改写/重定向。
//!
//! 钩子接口（trait `ContentInspector`）供新 `StreamState` 在文本/工具事件输出前后挂载。
//! 接入主路径前 allow(dead_code)。

#![allow(dead_code)]

pub mod background_agent;
pub mod leak;

/// 文本片段的检查结果：核心状态机产出一个候选文本片段，钩子决定如何处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInspectionResult {
    /// 正常输出
    Emit,
    /// 抑制（不输出）
    Suppress,
    /// 重定向到 thinking 块（commentary 阶段用）
    RedirectToThinking,
    /// 清洗后输出（携带清洗后的文本）
    Sanitize(String),
}

/// tool JSON 片段的检查结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolJsonInspectionResult {
    /// 接受为正常文本
    Accept,
    /// 恢复为 tool call（携带 tool 名与参数 JSON）
    RecoverAsToolCall { name: String, arguments: String },
    /// 丢弃
    Drop,
}

/// plan 块的处理动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanBridgeAction {
    /// 无动作
    None,
    /// 捕获 plan body（携带内容）
    Capture(String),
}
