## Context

codexProxy 的核心路径是把 Anthropic Messages 请求转换为 Codex Responses API（OpenAI 原生 Responses 风格），再把 Codex 的流式响应转换回 Anthropic SSE。这条双向转换链路当前存在严重的可维护性问题：

**现状（基于源码审计）：**
- `main/src/transform/codex/response.rs` 已达 **4931 行**，`TransformResponse` 结构体承载 **60+ 字段**，混合 6 类以上职责：流式状态机、tool 身份绑定、泄漏检测（`LeakDetector`）、plan 桥接（`PlanBridge`，含写文件副作用）、commentary 阶段重定向、Markdown Bash 拦截、RawToolJson 4 级风险评估（30+ 诊断计数器）。
- tool 身份靠 6 个 `HashMap`（`tool_order_by_output_index` / `tool_order_by_item_id` / `tool_order_by_call_id` / `canonical_item_id_by_output_index` / `canonical_output_index_by_item_id` / `closed_tool_call_ids`）+ `find_buffered_tool_order_from_metadata` 的 4 级回退查询维持，外加 `pending_tool_argument_updates` backlog 队列与 64 上限 trim。
- 大量 `looks_like_*_fragment` / `strip_*_noise` / `LEAKED_TOOL_MARKERS` 字符串规则，每修一个 case 新增一条规则与一个诊断计数器——这正是 CLAUDE.md 第 9 节「黏连治理」要求做**机制级收敛**却退化为「补丁地狱」的产物。
- 输入侧 `build_codex_unified_request`（`codex/backend.rs`）同样把 skill bridge、worktree sanitize、system prompt 注入耦合在一起。

**参考依据：**
已派发 3 个 subagent 深研 Vercel AI SDK（`/Volumes/zhitai/code/ai/ai-sdk`）的 anthropic provider、responses API、统一抽象层实现。ai-sdk 的设计模式（结构化 tagged union + 分层 + 物理隔离的流式状态机）为本重构提供参照。

**约束：**
- `main/src/transform/unified.rs` 是协议无关层，不得混入任何单一上游私有字段（CLAUDE.md 第 10 节）。
- 对外 HTTP/SSE 契约不变（`/v1/messages` 行为、配置热更新、负载均衡、failback 不受影响）。
- 现有 7 个测试文件（`response/tests/`，含 `tool_leak_stream` 2406 行、`upstream_event_compat` 1666 行）是回归基线，必须全程保持全绿。

## Goals / Non-Goals

**Goals:**
- 把 4931 行单体转换器按职责拆分为有清晰边界的模块，每个模块可独立理解与测试。
- 用 serde tagged enum 严格建模 Codex Responses 的 OutputItem 与 SSE 事件 union（含未知事件兜底），替换 `Value` + 字符串嗅探。
- 用 `output_index` 单一索引 + `call_id`/`item_id` 双 ID 体系简化 tool 身份绑定，删除 4 级回退与 backlog 队列。
- 用三态机（Active/CanConclude/Concluded）处理 reasoning 与 `encrypted_content` 的多轮时序。
- 把 LeakDetector / PlanBridge / CommentaryPhase / MarkdownBash / RawToolJson 等治理逻辑从核心状态机剥离为独立 side-channel 钩子。
- 把 30+ 平铺诊断计数器收敛为结构化、分通道的诊断输出。
- 全程保持现有回归测试全绿，对外 Anthropic 生命周期保证不变。

**Non-Goals:**
- 不改 `unified.rs` 的协议无关抽象职责（除非转换层需要新增协议无关类型，且通过审查）。
- 不改 OpenAI Chat / Gemini / Anthropic 透传适配层（本次只聚焦 Codex Responses ↔ Anthropic）。
- 不改对外 HTTP 契约、负载均衡、failback、配置热更新逻辑。
- 不引入新的外部依赖（用现有 serde / serde_json / tokio / futures）。
- 不在本变更内「清理」现有治理规则的语义（如调整 leak 检测阈值）——只做架构隔离，不改治理策略本身。

## Decisions

> 依据：ai-sdk anthropic provider（`anthropic-api.ts` zod schema、`anthropic-language-model.ts` doStream）、ai-sdk responses（`openai-responses-language-model.ts`）、统一抽象层（V4 `LanguageModelV4Prompt` tagged parts、`provider-utils/streaming-tool-call-tracker.ts`）的源码研究，以及对 codexProxy `codex/response.rs` 的审计。

### D1：用 serde tagged enum 严格建模 Codex Responses 事件与 OutputItem（替换 `Value` + 字符串嗅探）

**决策**：新建 `main/src/transform/codex/event.rs`，用 `#[serde(tag = "type")]` 的 tagged enum 建模 `OutputItem` 与 SSE 事件 union，未知类型用 `#[serde(other)]` 兜底为 `Unknown`。

**为什么**：当前 `transform_line` 接收 `&str`，内部用 `serde_json::Value` + `data["type"].as_str()` + 大量 `looks_like_*` 嗅探判别，是字符串规则地狱的根源。ai-sdk 的 `anthropicChunkSchema`（zod discriminated union）与 `openai-responses-api.ts` 都用严格 schema 建模，未知事件走 `.loose().transform(...)` 兜底——在 Rust 里对应 `#[serde(other)]`。

**替代方案**：保留 `Value` 但抽 helper 函数判别类型。否决——不解决"类型不安全 + 散落嗅探"根因。

**迁移**：事件模型是纯解析层，无副作用，优先落地。`transform_line` 入口先解析成 `ResponsesEvent`，再交给状态机。

### D2：状态机用 `content_blocks: HashMap<index, Block>` 单一索引（替换 6 HashMap + 4 级回退）

**决策**：重构 `TransformResponse` 的 tool/text/reasoning 跟踪为单一 `content_blocks: HashMap<u32, ContentBlockState>`，以 `output_index`（Responses）/ `index`（block）为键，删除 `tool_order_by_output_index`/`tool_order_by_item_id`/`tool_order_by_call_id`/`canonical_*_by_*` 等 6 个映射与 `pending_tool_argument_updates` backlog。

**为什么**：ai-sdk 的 anthropic doStream 用 `contentBlocks: Record<number, ...>` 单一索引（`anthropic-language-model.ts:1488`），responses doStream 用 `ongoingToolCalls: Record<output_index, ...>`（`openai-responses-language-model.ts:1141`），`StreamingToolCallTracker` 用 `toolCalls[index]` 数组——三者都用单一索引，无任何回退查询。codexProxy 的 4 级回退（`find_buffered_tool_order_from_metadata`：call_id → output_index → item_id → 唯一工具）是历史补丁，ai-sdk 证明单一索引足够鲁棒。

**call_id/item_id 双 ID 处理**：`ContentBlockState` 内同时存 `call_id`（↔ Anthropic `tool_use.id`，回传引用键）与 `item_id`（fc_...，item 标识）。delta 事件只带 `output_index`，用 `output_index` 定位 block；`output_item.done` 携带完整 `call_id`/`id` 时回填到 block。回传 `function_call_output` 时取 block 的 `call_id`。

**替代方案**：保留双映射但合并为单一 `Identity` 结构。否决——仍是补丁，不如直接删。

### D3：reasoning 三态机（Active/CanConclude/Concluded）

**决策**：新建 `reasoning.rs` 模块，用 `SummaryPartState { Active, CanConclude, Concluded }` 管理 reasoning summary part 生命周期。`store=false` 时 `part.done` → `CanConclude`，`output_item.done` 携带 `encrypted_content` → `Concluded`；`store=true` 时 `part.done` 直接 `Concluded`。

**为什么**：ai-sdk responses doStream 的 `activeReasoning` 用三态处理 `encrypted_content` 在 `output_item.done` 才到达的时序（`openai-responses-language-model.ts:2075-2099`），避免 reasoning 提前结束导致多轮上下文断裂。当前 codexProxy 用 `had_reasoning_in_response` + commentary phase 字符串检测，脆弱且无法正确处理时序。

**encrypted_content 处理**：透传到 `UnifiedThinking` 的 provider 专属字段（不伪造 Anthropic `signature`）。ai-sdk 把 `encryptedContent` 存 `providerOptions.openai`（`convert-to-openai-responses-input.ts:536-638`），符合 CLAUDE.md 第 10 节"provider 专属能力不污染 unified 层"——codexProxy 的 `UnifiedThinking` 已有类似字段，透传即可。

**替代方案**：两态（Active/Concluded）。否决——`store=false` 时 `part.done` 早于 `encrypted_content`，两态会丢失上下文。

### D4：治理逻辑剥离为 side-channel 钩子（替换内联分支）

**决策**：把 `LeakDetector`/`PlanBridge`/`CommentaryPhase`/`MarkdownBash`/`RawToolJson` 从 `TransformResponse` 字段与 `transform_sse_line` 分支中剥离，定义为独立 `trait ContentInspector`（或一组钩子函数），在文本/工具事件输出前后挂载。核心状态机只产生"候选输出"，钩子决定是否抑制/改写/重定向。

**为什么**：当前 60+ 字段中过半属于治理逻辑，核心状态机被治理分支污染到难以理解。ai-sdk 的 doStream 核心只管 `contentBlocks`/`finishReason`/`usage`（约 10 个状态变量），治理完全不在主路径。CLAUDE.md 第 9 节要求"状态机/通道隔离/事件边界"——这正是把治理从主状态机隔离为独立通道。

**钩子粒度**：
- `inspect_text_fragment(fragment) -> TextInspectionResult { emit, suppress, redirect_to_thinking, sanitize }`
- `inspect_tool_json(fragment) -> ToolJsonInspectionResult { accept, recover_as_tool_call, drop, risk_tier }`
- `on_plan_block(body) -> PlanBridgeAction`（plan 桥接 + 文件写入副作用收敛在此）

**替代方案**：保留内联但抽函数。否决——字段与控制流仍在主结构体，不解决可读性与可测性。

**实施调整（apply 阶段发现）**：审计后发现治理逻辑分两类：
1. **纯函数型**（leak 检测的 ~30 个 `looks_like_*`/`strip_*`/`assess_*` 函数）：无 `&self` 状态依赖，可干净外迁到 `inspectors/leak.rs`。**已完成迁移**，`response.rs` 调用点全部改为 `leak::`，150 测试全绿。
2. **有状态型**（plan_bridge / commentary / markdown_bash / raw_tool_json）：方法依赖 `&mut self` 的核心状态机字段（`saw_tool_call`、`mark_tool_turn_open()`、`emit_serialized_tool_call()`、`BufferedToolCall`、`diagnostics`、`logger`），无法纯函数化外迁——强行剥离需传入大量核心上下文，反而更复杂且违背手术式修改原则。

因此 D4 的完整实现路径修正为：**纯函数型治理（leak）已外迁；有状态型治理不在旧 `TransformResponse` 里物理迁移，而是在任务组 5 让新 `StreamState` 从设计上集成钩子挂载点，治理逻辑作为新状态机的钩子实现**。这要求新 `StreamState` 先达到功能对等（web_search / background_agent / server_tool_use 等），是大工作量工作。

### D5：诊断输出结构化分通道（替换 30+ 平铺计数器）

**决策**：`TransformDiagnostics` 重组为 `Diagnostics { leak: LeakDiag, plan_bridge: PlanDiag, raw_tool_json: RawToolJsonDiag, commentary: CommentaryDiag, ... }`，每个通道独立 `has_activity()` + 序列化。`take_diagnostics_summary` 输出分通道 JSON。

**为什么**：当前 30+ 平铺字段 + 巨型 `has_activity()` OR 链（`response.rs:81-114`）不可维护。ai-sdk 的错误与元数据都是分通道结构化（`openai-responses-provider-metadata.ts` 按 itemId/encryptedContent/phase/annotations 分组）。

**替代方案**：保留平铺但用 macro 生成 `has_activity`。否决——治标不治本。

### D6：绞杀式迁移，旧路径保留至新路径全绿

**决策**：不一次性重写。分阶段：(1) 新建事件模型模块（纯解析，并行存在）；(2) 新建状态机模块（用事件模型，独立单测）；(3) 新建治理钩子模块（从旧代码搬迁规则）；(4) 在 `transform_line` 入口切换到新状态机；(5) 旧 `TransformResponse` 删除。每步 `cargo test` 全绿，7 个现有测试文件全程作为契约。

**为什么**：4931 行主路径重写风险极高。ai-sdk 的 provider 之间是物理隔离的独立类（无共享可变状态），但 codexProxy 是单转换器，必须用绞杀式迁移保证可回滚。

**替代方案**：一次性重写。否决——风险不可控，且无法保持测试全程绿。

## Risks / Trade-offs

- **[主路径回归风险]** 转换器是请求主路径核心，重构期间任何行为偏移都会影响真实流量。→ **Mitigation**：全程以 7 个现有测试文件为回归基线，每步保持 `cargo test` 全绿；采用「先并行新模块、再切换入口、最后删旧」的绞杀式迁移，旧路径保留至新路径验证通过。
- **[治理逻辑剥离可能改变触发时机]** 把 LeakDetector 等从内联分支改为钩子，可能改变其在事件流中的触发顺序或可见性。→ **Mitigation**：剥离时保持钩子挂载点与原分支位置等价；`tool_leak_stream` 测试（2406 行）作为治理行为契约，必须全绿。
- **[tool 身份简化可能丢失边界 case]** 单一 `output_index` 索引在某些上游异常事件序列下（如缺 output_index 的 delta）可能不如 4 级回退鲁棒。→ **Mitigation**：保留对缺字段事件的兜底（归 Unknown + 诊断），并以 `upstream_event_compat` 测试（1666 行）验证兼容性。
- **[reasoning 三态机复杂度]** 三态机比当前隐式处理更复杂，但这是正确处理 `encrypted_content` 时序的必要代价。→ **Mitigation**：状态机集中在一个模块，配针对性单测覆盖 `store=true`/`store=false` 两条路径。
- **[重构体量大]** 4931 行重写本身是高风险操作。→ **Mitigation**：分阶段提交，每阶段可独立编译可测试；优先拆分纯函数模块（事件模型、output 建模），再做有状态状态机迁移。
