## MODIFIED Requirements

### Requirement: Codex response transformation SHALL keep the existing Anthropic lifecycle guarantees
系统在重构 Codex response transformer 时，MUST 继续输出合法的 Anthropic 生命周期事件，并保留当前已验证的 reasoning、tool use、usage 与终态收口语义，除非 spec 明确要求移除某条行为。重构仅作用于转换器**内部架构**（模块拆分、状态机重组、治理逻辑隔离），不得改变对外 SSE 事件序列与字段语义。

#### Scenario: Response stream contains reasoning and usage tail data
- **WHEN** Codex 上游流中包含 reasoning 片段、tool 调用以及最终 usage 信息
- **THEN** 系统 MUST 继续将其转换为合法的 Anthropic 内容块与结束事件，不得因为 request 侧重构而破坏响应生命周期

#### Scenario: Internal refactor must not alter external SSE contract
- **WHEN** 转换器内部从单体结构重组为多模块状态机、或 tool 身份绑定从多重映射改为单一索引
- **THEN** 对外输出的 `message_start` / `content_block_start` / `content_block_delta` / `content_block_stop` / `message_delta` / `message_stop` 事件序列与 `stop_reason` / `usage` 语义 MUST 与重构前保持一致，现有回归测试全绿

## ADDED Requirements

### Requirement: Codex response transformer SHALL track tool calls by a single output_index index
系统在流式拼装 Codex `function_call` 时，MUST 以 `output_index` 作为 `ongoing_tool_calls` 的主索引（delta 事件必带该字段），并通过 `call_id` 与 Anthropic `tool_use.id` 建立对应，不得再维护 `output_index` / `item_id` / `call_id` 之间的多重双向 HashMap 与多级回退查询。

#### Scenario: Function call argument deltas accumulate by output_index
- **WHEN** 上游连续推送同一 `output_index` 的多条 `response.function_call_arguments.delta`
- **THEN** 系统 MUST 将这些增量追加到该 `output_index` 对应的唯一 ongoing tool call 上，并在 `output_item.done` 时输出最终 `tool_use` 块

#### Scenario: Function call bound without multi-level fallback
- **WHEN** 一个 `function_call` 的 delta 事件只携带 `output_index` 而缺少 `item_id` 或 `call_id`
- **THEN** 系统 MUST 仍能通过 `output_index` 唯一定位 ongoing tool call，不依赖多级回退查询或 pending update backlog 队列

### Requirement: Codex response transformer SHALL model reasoning with a three-state lifecycle
系统在流式处理 Codex reasoning（`reasoning_summary_part.added` / `reasoning_summary_text.delta` / `reasoning_summary_part.done` / `output_item.done`）时，MUST 用三态生命周期（`Active` / `CanConclude` / `Concluded`）管理每个 reasoning summary part，以正确处理 `encrypted_content` 在 `output_item.done` 才到达的多轮场景。

#### Scenario: Reasoning part done before encrypted_content arrives
- **WHEN** 上游在 `store=false` 场景下先推送 `reasoning_summary_part.done`，随后才在 `output_item.done` 携带 `encrypted_content`
- **THEN** 系统 MUST 在 `part.done` 时将状态置为 `CanConclude` 而非立即结束，等到 `output_item.done` 时才转为 `Concluded` 并输出 Anthropic `thinking` 块的结束事件

#### Scenario: Reasoning encrypted_content is passed through without forging signature
- **WHEN** Codex reasoning item 携带 `encrypted_content`（加密推理上下文）
- **THEN** 系统 MUST 将其透传保留，不得伪造为 Anthropic `signature`（二者语义不对等），以避免多轮 reasoning 上下文丢失

### Requirement: Codex response transformer SHALL isolate governance logic into independent side-channels
系统 MUST 将泄漏检测（leak marker 抑制）、plan 桥接（proposed_plan 捕获与文件写入）、commentary 阶段重定向、Markdown Bash 拦截、RawTool JSON 评估等治理逻辑从核心流式状态机中剥离，作为独立、可单独开关与可观测的 side-channel 钩子，不得在核心状态机结构体中直接承载这些字段与控制流分支。

#### Scenario: Core state machine is free of governance branches
- **WHEN** 审查重构后的核心流式状态机结构体字段与事件分发逻辑
- **THEN** 不得包含 `LeakDetector` / `PlanBridge` / `CommentaryPhase` / `MarkdownBash` / `RawToolJson` 等治理专用的内联字段与字符串嗅探分支，这些 MUST 通过显式钩子接口注入

#### Scenario: Governance channels are independently observable
- **WHEN** 某条治理规则触发（例如丢弃了一段疑似泄漏的 tool JSON 片段）
- **THEN** 系统 MUST 通过结构化、分通道的诊断输出记录该事件，而不是依赖 30+ 个细粒度平铺计数器中的一个

### Requirement: Codex response transformer SHALL preserve diagnostics via structured per-channel output
系统 MUST 用结构化、分通道的诊断输出取代当前的 30+ 平铺细粒度计数器，每个治理通道（leak / plan / commentary / raw-tool-json 等）独立汇总其活动，并可通过 `take_diagnostics_summary` 导出。

#### Scenario: Diagnostics summary is structured by channel
- **WHEN** 转换器在 `take_diagnostics_summary` 中导出诊断摘要
- **THEN** 输出 MUST 按 channel 分组（如 `{ "leak": {...}, "plan_bridge": {...}, "raw_tool_json": {...} }`），而不是平铺的 30+ 字段
