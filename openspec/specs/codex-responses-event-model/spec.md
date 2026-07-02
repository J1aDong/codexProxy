# codex-responses-event-model Specification

## Purpose
TBD - created by archiving change refactor-codex-anthropic-transform. Update Purpose after archive.
## Requirements
### Requirement: 转换层 SHALL 用结构化类型建模 Codex Responses 的 OutputItem union
系统在解析 Codex Responses API 的非流式响应体与流式 SSE 事件时，MUST 用 serde tagged enum（基于 `type` 字段判别）严格建模 `OutputItem` union（`message` / `function_call` / `function_call_output` / `reasoning` / 未知），不得继续依赖 `serde_json::Value` + 运行时字符串嗅探来判别 item 类型。

#### Scenario: 解析已知类型的 output item
- **WHEN** Codex 上游返回一个 `type: "function_call"` 的 output item（含 `id`/`call_id`/`name`/`arguments`）
- **THEN** 系统 MUST 将其反序列化为对应的强类型变体，字段可直接通过类型安全访问，无需二次 `as_str()`/`as_object()` 拆解

#### Scenario: 解析未知的 output item 类型
- **WHEN** Codex 上游返回一个 `type` 值不在已建模列表内的 output item（例如上游新增了某种 item 类型）
- **THEN** 系统 MUST 将其归入 `Unknown` 变体（保留原始 `type` 字符串）而不是解析失败或静默丢弃，并且转换流程继续不中断

### Requirement: 转换层 SHALL 用结构化类型建模 Codex Responses 的流式 SSE 事件 union
系统解析 Codex 流式响应时，MUST 用 serde tagged enum 严格建模 SSE 事件 union（`response.created` / `response.in_progress` / `response.output_item.added` / `response.output_item.done` / `response.content_part.added` / `response.content_part.done` / `response.output_text.delta` / `response.output_text.done` / `response.refusal.delta` / `response.refusal.done` / `response.function_call_arguments.delta` / `response.function_call_arguments.done` / `response.reasoning_summary_text.delta` / `response.reasoning_summary_text.done` / `response.reasoning_summary_part.added` / `response.reasoning_summary_part.done` / `response.completed` / `response.failed` / `response.incomplete` / `error` / 未知），并对未知事件类型兜底。

#### Scenario: 解析已知类型的流式事件
- **WHEN** 上游 SSE 推送一条 `event: response.function_call_arguments.delta` 且 data 含合法 JSON
- **THEN** 系统 MUST 将其反序列化为对应强类型事件变体（携带 `output_index` / `item_id` / `delta` 等字段），供状态机按类型分发处理

#### Scenario: 解析未知类型的流式事件
- **WHEN** 上游 SSE 推送一条 `event` 值不在已建模列表内的事件
- **THEN** 系统 MUST 将其归入 `Unknown` 变体（保留原始 `type` 与 raw data），不中断流式转换，并记录该未知事件类型以便后续扩展

#### Scenario: 解析无法识别为 JSON 的 data
- **WHEN** 上游 SSE 的 data 字段不是合法 JSON（例如 `[DONE]` 或空）
- **THEN** 系统 MUST 安全跳过该帧而不 panic，流式转换继续

### Requirement: 事件模型 SHALL 区分 function_call 的 call_id 与 item id
系统建模 `function_call` output item 及其流式事件时，MUST 同时保留 `call_id`（`call_` 前缀，用于回传 `function_call_output` 的引用键）与 item `id`（`fc_` 前缀，item 自身标识）两个独立字段，不得合并或混淆。

#### Scenario: 回传 tool result 时使用 call_id 引用
- **WHEN** 系统将 Anthropic `tool_result` 转换为 Codex `function_call_output` input item
- **THEN** 该 item 的 `call_id` MUST 与原 `function_call` 的 `call_id` 一致（对应 Anthropic 的 `tool_use.id`），而不得使用 item `id`

#### Scenario: Anthropic tool_use.id 与 Codex call_id 对应
- **WHEN** 系统在 Codex `function_call` 与 Anthropic `tool_use` 之间建立映射
- **THEN** Anthropic 的 `tool_use.id` MUST 对应 Codex 的 `function_call.call_id`（而非 item `id`），以保证双向 tool 闭环一致

