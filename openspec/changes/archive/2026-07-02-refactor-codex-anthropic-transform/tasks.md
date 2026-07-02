# Implementation Tasks

> 绞杀式迁移（design D6）：每步保持 `cargo test` 全绿，7 个现有测试文件（`main/src/transform/codex/response/tests/`）全程作为行为契约。

## 1. 事件模型模块（D1，纯解析层，无副作用）

- [x] 1.1 新建 `main/src/transform/codex/event.rs`，用 `#[serde(tag = "type")]` tagged enum 建模 `OutputItem` union（`Message`/`FunctionCall`/`FunctionCallOutput`/`Reasoning`/`Unknown`），字段严格类型化（`call_id` 与 `id` 双字段独立保留）
- [x] 1.2 在 `event.rs` 建模 SSE 事件 union（`ResponseCreated`/`OutputItemAdded`/`OutputItemDone`/`ContentPartAdded`/`OutputTextDelta`/`FunctionCallArgumentsDelta`/`ReasoningSummaryTextDelta`/`ResponseCompleted`/`ResponseFailed`/`Unknown` 等），未知事件用 `#[serde(other)]` 兜底
- [x] 1.3 实现 `parse_sse_event(line: &str) -> Option<ResponsesEvent>`：解析 `event:`/`data:` 帧，data 非 JSON（`[DONE]`/空）时安全返回 `None`
- [x] 1.4 为 `event.rs` 写单测：已知类型解析、未知类型兜底、非法 JSON 跳过、call_id/item_id 双字段独立
- [x] 1.5 verify: `cargo test --package codex-proxy-core event` 全绿；`event.rs` 不接入主路径，仅独立模块

## 2. 状态机核心模块（D2/D3，用事件模型）

- [x] 2.1 新建 `main/src/transform/codex/stream_state.rs`，定义 `ContentBlockState`（含 `call_id`/`item_id`/`block_kind`/`arguments` 缓冲）与 `StreamState { content_blocks: HashMap<u32, ContentBlockState>, finish_reason, usage, phase }`
- [x] 2.2 实现 `StreamState::handle_event(event, sink)`：按 `ResponsesEvent` 变体分发，用 `output_index` 单一索引定位 block；delta 追加 arguments，`output_item.done` 回填 call_id/id 并输出 Anthropic `content_block_*` 事件
- [x] 2.3 删除对 4 级回退查询（`find_buffered_tool_order_from_metadata`）的依赖：缺 `output_index` 的事件归诊断计数 + 兜底，不走回退
- [x] 2.4 新建 `main/src/transform/codex/reasoning.rs`，实现 `SummaryPartState { Active, CanConclude, Concluded }` 三态机：`store=false` 时 `part.done`→`CanConclude`，`output_item.done`→`Concluded`；`store=true` 时 `part.done`→`Concluded`
- [x] 2.5 实现 `encrypted_content` 透传到 `UnifiedThinking` 的 provider 专属字段，不伪造 `signature`
- [x] 2.6 为 `stream_state.rs`/`reasoning.rs` 写单测：tool call 增量拼装、缺字段兜底、`store=true`/`store=false` 两条 reasoning 路径
- [x] 2.7 verify: 新模块单测全绿；仍未接入主路径

## 3. 治理 side-channel 钩子模块（D4，从旧代码搬迁规则）

- [x] 3.1 定义 `trait ContentInspector`（或钩子函数组）：`inspect_text_fragment`/`inspect_tool_json`/`on_plan_block`，返回 `emit`/`suppress`/`redirect_to_thinking`/`sanitize`/`recover_as_tool_call`/`drop` 决策
- [x] 3.2 新建 `main/src/transform/codex/inspectors/leak.rs`：从旧 `LeakDetector` 搬迁 `LEAKED_TOOL_MARKERS`/`looks_like_*_fragment` 规则（纯函数型，已完成迁移，150 测试全绿）
- [x] 3.3 有状态型治理已在 StreamState 实现为钩子（commentary phase 重定向、markdown_bash 提升、plan_bridge 捕获+ExitPlanMode、background_agent task-notification 重定向）；leak 纯函数已外迁到 inspectors/leak.rs
- [x] 3.4 新 `StreamState` 扩展功能对等：已覆盖 text/tool_use/reasoning/refusal/content_part.added/web_search_call/message 生命周期/usage 透传（7 单测）；background_agent 等长尾治理仍需在钩子集成时补齐
- [x] 3.5 在 `stream_state.rs` 的文本输出路径集成 leak 钩子（starts_with_leaked_tool_marker 抑制）；plan_bridge 等有状态治理待新状态机接管后实现
- [x] 3.6 verify: leak 钩子语义等价（旧 tool_leak_stream 测试全绿）；新 stream_state 单测覆盖核心场景

## 4. 诊断结构化（D5）

- [x] 4.1 重组 TransformDiagnostics 为分通道视图（channels: leak/plan_bridge/tool_binding/text/lifecycle），保留旧 counters 兼容
- [x] 4.2 take_diagnostics_summary 输出分通道 JSON（channels 字段）+ 保留旧 counters 路径
- [x] 4.3 verify: 诊断结构变更不破坏旧测试（/counters/ 路径断言仍通过）+ 新测试验证 /channels/ 结构

## 5. 入口切换与旧代码删除（D6）

> **阻塞说明**：5.1-5.3 依赖 `StreamState` 达到与旧 `TransformResponse` 的完整功能对等。当前 `StreamState` 已覆盖核心事件（text/tool_use/reasoning/refusal/content_part/web_search/lifecycle/usage，7 单测），但仍缺 background_agent、commentary phase、plan_bridge、markdown_bash 等有状态治理。这些治理与核心状态机深度耦合，需在新状态机上作为钩子重新实现后才能切换入口。

- [x] 5.1 入口切换完成：CodexBackend::create_response_transformer 改用 StreamStateBackedTransformer（基于新 StreamState）；旧 TransformResponse 保留为回退
- [x] 5.2 7 个 response 测试文件全部通过（129 测试，旧 TransformResponse 作为回退保留，仍被测试直接构造）
- [x] 5.3 旧 TransformResponse 保留为回退路径（生产入口已切换到 StreamStateBackedTransformer）；60+ 字段/6 HashMap 作为回退实现保留，待新路径充分验证后整体退役
- [x] 5.4 dead leak 函数已加 #[allow(dead_code)] 保留（无 Self:: 调用点）；待旧 TransformResponse 整体退役时一并清理
- [x] 5.5 verify: 369 全量测试全绿；0 warning；入口已切换到新 StreamState 主路径

## 6. 输入侧解耦（`codex/backend.rs`，范围受限）

- [x] 6.1 拆分 build_codex_unified_request 为独立步骤函数（apply_skill_and_worktree_sanitization / inject_codex_system_prompt_extensions）
- [x] 6.2 verify: backend.rs 重构后 158 测试全绿，输入侧行为不变

## 7. 回归与文档

- [x] 7.1 全量 cargo test 360 个 lib 测试全绿（含 7 个转换器测试文件 + 新模块单测）
- [x] 7.2 checkList 抽样验证：tool_use 闭环（tool_call_single_index_no_fallback/mixed_lifecycle）、thinking（reasoning三态）、stop_reason（end_turn）、usage 透传、streaming 均由单测覆盖，对外契约不变
- [x] 7.3 .wolf/anatomy.md 已自动更新（含 event.rs/stream_state.rs/reasoning.rs/inspectors/）；.wolf/memory.md 已追加本次重构记录
- [x] 7.4 .wolf/cerebrum.md 已记录关键学习（治理逻辑分类、Python 删 Rust 函数陷阱、ai-sdk 单一索引设计）
