//! Background agent lifecycle inspectors（D4 治理 side-channel）
//!
//! 从 `response.rs` 剥离的后台任务生命周期治理纯函数：检测 `<task-notification>` 等
//! 文本片段，生成进度消息（作为 thinking_delta 输出）。纯函数，无状态、无副作用。

/// 从 XML 片段中提取标签 body
pub fn extract_xml_tag_body<'a>(fragment: &'a str, tag: &str) -> Option<&'a str> {
    let start_marker = format!("<{tag}>");
    let end_marker = format!("</{tag}>");
    let start = fragment.find(start_marker.as_str())? + start_marker.len();
    let end = fragment[start..].find(end_marker.as_str())? + start;
    Some(fragment[start..end].trim())
}

/// 压缩任务完成摘要
pub fn compact_task_completion_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    if let Some(rest) = trimmed.strip_prefix("Agent \"") {
        if let Some(end_quote) = rest.find('"') {
            let name = rest[..end_quote].trim();
            if !name.is_empty() {
                return format!("后台 explorer 已完成：{name}…");
            }
        }
    }

    if trimmed.is_empty() {
        "后台任务已完成，正在汇总结果…".to_string()
    } else {
        format!("后台任务已完成：{trimmed}…")
    }
}

/// 检测后台任务生命周期文本片段，生成进度消息
pub fn build_task_lifecycle_progress_message(fragment: &str) -> Option<String> {
    let trimmed = fragment.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("<retrieval_status>") {
        let status = extract_xml_tag_body(trimmed, "retrieval_status")?;
        return Some(match status {
            "timeout" | "running" => "某个 explorer 仍在运行，我继续等待结果…".to_string(),
            other if !other.is_empty() => format!("后台任务状态更新：{other}…"),
            _ => return None,
        });
    }

    if trimmed.starts_with("<task-notification>") {
        let status = extract_xml_tag_body(trimmed, "status").unwrap_or("");
        let summary = extract_xml_tag_body(trimmed, "summary").unwrap_or("");
        return Some(match status {
            "completed" => compact_task_completion_summary(summary),
            "failed" => {
                if summary.trim().is_empty() {
                    "后台任务执行失败，我继续处理剩余结果…".to_string()
                } else {
                    format!("后台任务失败：{}…", summary.trim())
                }
            }
            _ => {
                if summary.trim().is_empty() {
                    "收到后台任务进度更新…".to_string()
                } else {
                    format!("后台任务进度更新：{}…", summary.trim())
                }
            }
        });
    }

    if trimmed.starts_with("Task is still running") {
        return Some("某个 explorer 仍在运行，我继续等待结果…".to_string());
    }

    if trimmed.starts_with("No task output available") {
        return Some("后台任务暂时还没有新输出，我继续等待…".to_string());
    }

    if trimmed.starts_with("Error: No task found with ID:") {
        return Some("某个后台任务已结束或状态失效，我继续汇总现有结果…".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_notification_completed() {
        let msg = build_task_lifecycle_progress_message(
            "<task-notification><status>completed</status><summary>Agent \"explorer\" done</summary></task-notification>",
        );
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("后台 explorer 已完成：explorer"));
    }

    #[test]
    fn task_notification_failed() {
        let msg = build_task_lifecycle_progress_message(
            "<task-notification><status>failed</status><summary>oops</summary></task-notification>",
        );
        assert_eq!(msg.as_deref(), Some("后台任务失败：oops…"));
    }

    #[test]
    fn task_still_running_text() {
        let msg = build_task_lifecycle_progress_message("Task is still running");
        assert_eq!(msg.as_deref(), Some("某个 explorer 仍在运行，我继续等待结果…"));
    }

    #[test]
    fn non_task_text_returns_none() {
        assert!(build_task_lifecycle_progress_message("普通文本").is_none());
    }
}
