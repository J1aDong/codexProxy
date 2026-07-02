//! Reasoning 三态机（D3）
//!
//! 处理 Codex Responses reasoning summary part 的生命周期，正确处理 `store=false` 时
//! `encrypted_content` 在 `output_item.done` 才到达的时序。
//!
//! 三态：
//! - `Active`：正在接收 summary delta
//! - `CanConclude`：已收到 `part.done`，但等待 `output_item.done` 携带 `encrypted_content`（仅 `store=false`）
//! - `Concluded`：可安全结束 reasoning 块
//!
//! 接入主路径前 allow(dead_code)。

#![allow(dead_code)]

use std::collections::HashMap;

/// 单个 reasoning summary part 的状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryPartState {
    /// 正在接收 delta
    Active,
    /// `part.done` 已到，但等 `output_item.done` 的 encrypted_content（store=false）
    CanConclude,
    /// 已结束
    Concluded,
}

/// 一个 reasoning item（按 item_id 聚合）的 summary part 状态
#[derive(Debug, Clone, Default)]
pub struct ActiveReasoning {
    /// summary_index -> 状态
    pub parts: HashMap<u32, SummaryPartState>,
    /// 累积的 summary 文本（按 summary_index 顺序拼接）
    pub summary_text: String,
    /// encrypted_content（在 output_item.done 时回填）
    pub encrypted_content: Option<String>,
    /// 是否已对外发出 thinking 块开始事件
    pub block_started: bool,
    /// 对外的 content_block index（Anthropic 侧）
    pub block_index: Option<usize>,
}

impl ActiveReasoning {
    pub fn new() -> Self {
        Self::default()
    }

    /// 标记某个 summary_index 为 Active（part.added 时调用）
    pub fn activate_part(&mut self, summary_index: u32) {
        self.parts.entry(summary_index).or_insert(SummaryPartState::Active);
    }

    /// 追加 summary delta 文本
    pub fn append_summary_delta(&mut self, delta: &str) {
        self.summary_text.push_str(delta);
    }

    /// `reasoning_summary_part.done` 时调用。
    ///
    /// `store=false` 时置 `CanConclude`（等 output_item.done）；
    /// `store=true` 时直接 `Concluded`。
    pub fn on_part_done(&mut self, summary_index: u32, store_enabled: bool) {
        let next = if store_enabled {
            SummaryPartState::Concluded
        } else {
            SummaryPartState::CanConclude
        };
        self.parts.insert(summary_index, next);
    }

    /// `output_item.done` 时调用：回填 encrypted_content，并把所有 Active/CanConclude 置为 Concluded。
    pub fn conclude_with(&mut self, encrypted_content: Option<String>) {
        if encrypted_content.is_some() {
            self.encrypted_content = encrypted_content;
        }
        for state in self.parts.values_mut() {
            *state = SummaryPartState::Concluded;
        }
    }

    /// 是否所有 part 都已 Concluded（可安全结束 thinking 块）
    pub fn is_concluded(&self) -> bool {
        !self.parts.is_empty() && self.parts.values().all(|s| *s == SummaryPartState::Concluded)
    }

    /// 是否有未结束的 part（Active 或 CanConclude）
    pub fn has_pending(&self) -> bool {
        self.parts.values().any(|s| *s != SummaryPartState::Concluded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_true_concludes_on_part_done() {
        let mut r = ActiveReasoning::new();
        r.activate_part(0);
        assert!(r.has_pending());
        r.on_part_done(0, true);
        assert!(r.is_concluded());
        assert!(!r.has_pending());
    }

    #[test]
    fn store_false_waits_for_output_item_done() {
        let mut r = ActiveReasoning::new();
        r.activate_part(0);
        r.on_part_done(0, false); // store=false
        assert!(!r.is_concluded()); // 仍 CanConclude
        assert!(r.has_pending());
        r.conclude_with(Some("enc".into()));
        assert!(r.is_concluded()); // 现在 Concluded
        assert_eq!(r.encrypted_content.as_deref(), Some("enc"));
    }

    #[test]
    fn multiple_parts_all_conclude_on_output_item_done() {
        let mut r = ActiveReasoning::new();
        r.activate_part(0);
        r.activate_part(1);
        r.on_part_done(0, false);
        r.on_part_done(1, false);
        // 两个都 CanConclude
        r.conclude_with(None);
        assert!(r.is_concluded());
    }

    #[test]
    fn encrypted_content_backfilled() {
        let mut r = ActiveReasoning::new();
        r.activate_part(0);
        r.on_part_done(0, false);
        r.conclude_with(Some("encrypted_payload".into()));
        assert_eq!(r.encrypted_content.as_deref(), Some("encrypted_payload"));
    }
}
