# chat-streaming-audit Specification

## Purpose
TBD - created by archiving change align-codex-packaging-and-stream-audit. Update Purpose after archive.
## Requirements
### Requirement: OpenAI chat streaming event parity
The OpenAI Chat backend SHALL emit Anthropic-compatible streaming events with correct event ordering, message lifecycle boundaries, and stop signaling.

#### Scenario: Text response emits Anthropic event lifecycle
- **WHEN** an OpenAI chat completion upstream emits text deltas
- **THEN** the transformer SHALL emit `message_start`, one or more `content_block_*` text events, a `message_delta` with mapped stop reason when available, and a terminating `message_stop`

### Requirement: Tool call streaming fidelity
The OpenAI Chat backend SHALL preserve streamed tool call semantics, including tool start boundaries, incremental argument delivery, and completion ordering across multiple tool calls.

#### Scenario: Tool arguments streamed incrementally
- **WHEN** an upstream OpenAI chat completion emits `tool_calls[].function.arguments` in multiple deltas
- **THEN** the transformer SHALL emit Anthropic-compatible incremental tool input events without losing order or merging separate tool calls incorrectly

#### Scenario: Multiple tool calls remain distinct
- **WHEN** an upstream OpenAI chat completion emits more than one tool call in the same response
- **THEN** the transformer SHALL maintain distinct content block indexes and stop boundaries for each tool call

### Requirement: Thinking and reasoning mapping audit
The OpenAI Chat backend SHALL define and test how reasoning or thinking-style upstream fields are represented in downstream Anthropic-compatible streams.

#### Scenario: Reasoning content opens thinking block
- **WHEN** the upstream response includes reasoning or thinking content that the backend chooses to expose
- **THEN** the transformer SHALL emit a consistent Anthropic-compatible thinking block lifecycle and corresponding regression tests SHALL cover the behavior

### Requirement: Finish reason and termination mapping audit
The streaming adapters SHALL document and test how upstream finish reasons map to downstream stop reasons or termination markers, including tool-use completion, normal stop, length limits, refusal/content-filter cases.

#### Scenario: Tool completion maps to tool stop reason
- **WHEN** an upstream OpenAI chat completion finishes with `tool_calls`
- **THEN** the downstream Anthropic-compatible stream SHALL map that completion to the corresponding tool-use stop semantics before `message_stop`

#### Scenario: Refusal-like termination is observable
- **WHEN** an upstream completion ends in a refusal or content-filter condition
- **THEN** the adapter SHALL produce a deterministic downstream termination signal and logs/tests SHALL make the mapping auditable

### Requirement: Anthropic passthrough SSE integrity
The Anthropic backend SHALL preserve SSE event/data pairing and comment-style keepalive lines without reordering or collapsing events.

#### Scenario: Passthrough preserves event/data association
- **WHEN** the upstream Anthropic stream emits named events followed by data lines
- **THEN** the passthrough transformer SHALL forward the same event/data pairing and preserve SSE framing boundaries

### Requirement: OpenAI chat transformation boundary
The OpenAI Chat transformation layer in `codexProxy` SHALL remain focused on protocol conversion and SHALL NOT introduce vendor-specific prompt packaging mutations for downstream adapters such as CodeBuddy.

#### Scenario: Downstream-specific packaging remains out of scope
- **WHEN** a downstream adapter requires CodeBuddy CLI-style prompt, system, or tool packaging
- **THEN** `codexProxy` SHALL leave that packaging responsibility to the downstream adapter layer rather than rewriting the OpenAI Chat transform to emit CodeBuddy-specific envelopes

### Requirement: Streaming regression coverage
The codebase SHALL include regression tests for the audited streaming behaviors so that future changes to packaging or protocol conversion cannot silently break event semantics.

#### Scenario: Regression tests cover audited mappings
- **WHEN** streaming conversion code changes in the future
- **THEN** the test suite SHALL include assertions for text event ordering, tool call argument streaming, thinking mapping, finish reason mapping, and passthrough SSE integrity

### Requirement: 仅在存在有效消息时发出消息生命周期事件
系统在将 OpenAI 流式 chunk 转换为 Anthropic SSE 时，MUST 仅在确认存在有效下游消息内容后才发出 `message_start`，并 MUST 保证生命周期事件顺序合法。

#### Scenario: 无 choices 的 JSON chunk
- **WHEN** 上游先返回可解析 JSON，但该 chunk 不包含有效 `choices[0]` 消息增量
- **THEN** 系统 MUST 不发出下游 `message_start`

#### Scenario: 正常文本流式输出
- **WHEN** 上游返回包含文本增量的有效 chunk
- **THEN** 系统 MUST 以合法顺序发出 `message_start`、内容块事件和最终消息收尾事件

### Requirement: 正确处理工具调用增量与参数收口
系统 MUST 按 OpenAI 流式 tool-call 的 `index` 维持稳定状态，并正确拼接参数增量；若参数在收口时损坏或不完整，系统 MUST 显式报告或进入已定义错误路径，而不是静默降级为 `{}`。

#### Scenario: 多工具并发增量
- **WHEN** 上游在同一响应中返回多个 `tool_calls` 且通过不同 `index` 交错发送参数片段
- **THEN** 系统 MUST 以 `index` 为主键独立聚合每个工具调用的参数与块状态

#### Scenario: 工具参数损坏
- **WHEN** 工具参数片段在收口阶段无法组成合法 JSON
- **THEN** 系统 MUST 不将其静默替换为空对象，而 MUST 进入显式错误或显式降级路径

### Requirement: 覆盖关键流式 delta 与结束语义
系统 MUST 处理影响兼容性的关键 OpenAI 流式 delta 与结束语义，包括 `tool_calls`、仍可能出现的 `function_call`、`refusal`、finish reason 和 usage-only chunk 规则。

#### Scenario: usage-only chunk 在 DONE 之前到达
- **WHEN** 上游因启用 usage 流式选项而在 `data: [DONE]` 之前返回一个空 `choices` 的 usage-only chunk
- **THEN** 系统 MUST 更新 usage 状态，但 MUST 不提前发出 `message_stop`

#### Scenario: 旧式 function_call 增量
- **WHEN** 上游返回 deprecated 但仍可能出现的 `function_call` 增量
- **THEN** 系统 MUST 按兼容策略将其转换为下游可理解的工具调用语义

#### Scenario: refusal 增量或内容过滤结束
- **WHEN** 上游通过 `refusal` delta 或相关 finish reason 表达拒绝/过滤结果
- **THEN** 系统 MUST 将该语义显式映射到下游，而不是无声忽略

### Requirement: thinking 输出遵守可见性与完整性策略
系统在输出 Anthropic 风格 thinking 相关事件时 MUST 遵守请求级可见性策略，并在支持的情况下保留必要的 thinking 完整性信息。

#### Scenario: 请求禁止可见 thinking
- **WHEN** 请求上下文声明不允许向下游暴露 thinking
- **THEN** 系统 MUST 不发出 thinking 相关内容块或增量

#### Scenario: thinking 完整性信息可用
- **WHEN** 上游提供可映射的 thinking 完整性附加信息
- **THEN** 系统 MUST 按已定义策略保留或转发对应语义

### Requirement: 在 P2 阶段保证 transform 与 stream_decision 的端到端一致性
系统在 P2 阶段 MUST 对 transformer 输出与 `stream_decision` 二次裁剪规则施加一致性约束，并通过端到端回归测试覆盖重复 start、过早 stop 和 stop 后业务输出等边界条件。

#### Scenario: 重复的 message_start
- **WHEN** transformer 或上游异常导致候选输出重复触发 `message_start`
- **THEN** 系统 MUST 保证下游观察到的生命周期事件仍然合法且唯一

#### Scenario: 过早的 message_stop
- **WHEN** 在内容块尚未完成或 usage / finish 语义尚未收口前出现 `message_stop`
- **THEN** 系统 MUST 阻止非法终态泄漏到下游

#### Scenario: message_stop 之后仍有业务输出
- **WHEN** transform 层或后续逻辑在 `message_stop` 之后继续生成业务事件
- **THEN** 系统 MUST 按一致策略拒绝或裁剪这些输出，并由测试覆盖该行为
