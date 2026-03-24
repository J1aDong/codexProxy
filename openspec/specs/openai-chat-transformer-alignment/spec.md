# openai-chat-transformer-alignment Specification

## Purpose
TBD - created by archiving change refactor-openai-chat-transformer. Update Purpose after archive.
## Requirements
### Requirement: Refactor scope SHALL remain confined to the OpenAI transformer
系统在实施本次重构时，MUST 将变更范围限制在 `openai` converter 的内部实现与相关测试，不得改变 `codex` 转换层、`anthropic` 透传链路或 `gemini` 转换层的协议语义与行为边界。

#### Scenario: Update only OpenAI transformer internals
- **WHEN** 开发者根据本 change 实施重构
- **THEN** 代码改动 MUST 仅针对 OpenAI Chat Completion 转换链路及其测试，不得把其他 transformer 一并纳入重构范围

#### Scenario: Preserve non-OpenAI transformer behavior
- **WHEN** 重构完成并运行相关回归验证
- **THEN** `codex` 转换层、`anthropic` 透传链路和 `gemini` 转换层 MUST 保持既有行为边界不变

### Requirement: Request mapping SHALL separate OpenAI body semantics from transport adaptation
系统在把 Anthropic 请求转换为 OpenAI Chat Completion 请求时，MUST 先完成与协议语义相关的 body 映射，再处理 endpoint、鉴权头和 HTTP Accept 等传输层差异；请求体映射不得依赖具体上游是标准 OpenAI 还是 Azure OpenAI。

#### Scenario: Build request body before endpoint adaptation
- **WHEN** 系统收到包含 `system`、`messages`、`tools`、`tool_choice` 和采样参数的 Anthropic 请求
- **THEN** 系统 MUST 先得到与上游部署形态无关的 OpenAI body，再由独立的上游请求构建步骤补上 endpoint 与 header

#### Scenario: Preserve request semantics across different upstream styles
- **WHEN** 相同的 Anthropic 请求分别发往标准 OpenAI base URL 和 Azure OpenAI URL
- **THEN** 系统 MUST 生成语义等价的 OpenAI request body，且差异仅体现在 endpoint 与鉴权头等传输层字段

### Requirement: Request mapping SHALL normalize system, assistant, tool, and tool-result semantics consistently
系统 MUST 以稳定规则完成 system 消息拼装、assistant 文本与 tool call 映射，以及 tool result 展平；这些规则在重构后 MUST 保持可单独验证，并与参考项目的转换语义一致。

#### Scenario: Merge system prompt with custom injection prompt
- **WHEN** Anthropic 请求同时包含 `system` 内容和 `custom_injection_prompt`
- **THEN** 系统 MUST 以确定顺序将二者合并为首条 OpenAI `system` message，且不得生成空白 system message

#### Scenario: Convert assistant tool use and tool results
- **WHEN** 中间表示中包含 assistant 发起的工具调用以及后续工具结果
- **THEN** 系统 MUST 将其映射为 OpenAI `assistant.tool_calls` 与 `tool` role 消息，并保持 `tool_call_id` 关联关系不变

### Requirement: Upstream request construction SHALL be isolated and deterministic
系统 MUST 以独立且可测试的方式构建 OpenAI 上游请求，包括 endpoint 选择、Azure 与非 Azure 鉴权头差异，以及流式与非流式的 Accept 头语义；该步骤不得重新解释请求体业务语义。

#### Scenario: Build Azure request headers
- **WHEN** 目标地址为 Azure OpenAI Chat Completions endpoint
- **THEN** 系统 MUST 使用 Azure 所需的 endpoint 形式与 `api-key` 鉴权头，而不是 Bearer token 头

#### Scenario: Build standard OpenAI streaming request
- **WHEN** 目标地址为标准 OpenAI 或兼容 OpenAI 的 `/chat/completions` 端点且请求为流式
- **THEN** 系统 MUST 使用正确的 endpoint、`Authorization: Bearer` 头和 `text/event-stream` Accept 头

### Requirement: Streaming response conversion SHALL emit a legal Anthropic lifecycle and close only on terminal completion
系统在将 OpenAI Chat Completion SSE 转换为 Anthropic SSE 时，MUST 维持合法的消息生命周期顺序，并且 MUST 只在确认终止标记后发出最终 `message_delta` 与 `message_stop`，不得因提前收到 `finish_reason` 或 usage-only chunk 而过早收口。

#### Scenario: Finish reason arrives before DONE
- **WHEN** 上游 chunk 已包含 `finish_reason`，但后续仍可能发送 usage-only chunk 且尚未收到 `data: [DONE]`
- **THEN** 系统 MUST 记录 stop reason 所需状态，但 MUST 不提前发出最终消息结束事件

#### Scenario: Usage-only chunk arrives before terminal marker
- **WHEN** 上游在 `data: [DONE]` 之前发送仅包含 usage 的 chunk
- **THEN** 系统 MUST 更新 usage 统计并继续等待终止标记，而不是立即结束下游消息

### Requirement: Streaming response conversion SHALL aggregate tool-call deltas by index without breaking block order
系统 MUST 按 OpenAI `tool_calls[index]` 独立聚合每个工具调用的 `id`、`name` 和 `arguments` 增量，并在 Anthropic 事件流中保证文本块、thinking 块和 tool_use 块的开启与关闭顺序合法。

#### Scenario: Close text block before opening tool block
- **WHEN** 上游先发送文本增量，随后开始发送某个 `tool_calls[index]`
- **THEN** 系统 MUST 先结束当前文本内容块，再开启对应的 `tool_use` 内容块

#### Scenario: Interleave multiple tool-call indices
- **WHEN** 上游在同一响应中交错发送多个不同 `index` 的工具调用增量
- **THEN** 系统 MUST 以 `index` 为主键分别累积参数和块状态，且不得让不同工具调用的参数片段串线

### Requirement: Streaming response conversion SHALL preserve supported compatibility semantics
系统在完成职责重构后，MUST 保留当前已支持的兼容语义，包括 `reasoning_content` 的可见性控制、`refusal` 文本映射，以及旧式 `function_call` 增量的兼容归一；对这些语义的支持不得因向参考项目对齐而倒退。

#### Scenario: Visible thinking is disabled
- **WHEN** 当前请求上下文声明 `allow_visible_thinking=false` 且上游返回 `reasoning_content`
- **THEN** 系统 MUST 不向下游发出 thinking 相关内容块或 thinking delta

#### Scenario: Deprecated function_call delta is received
- **WHEN** 上游返回旧式 `function_call` 增量而不是 `tool_calls`
- **THEN** 系统 MUST 按统一工具调用路径归一处理，使下游仍能观察到合法的 tool-use 语义

### Requirement: OpenAI transformer SHALL isolate request mapping, upstream construction, and response transformation
系统在把 Anthropic 请求转换为 OpenAI Chat Completions 请求时，MUST 将 request body 语义映射、上游 endpoint/header 构建和 SSE 响应转换拆分为可独立测试的职责层，不得继续依赖单体实现中的隐式耦合。

#### Scenario: Same request targets different OpenAI upstream styles
- **WHEN** 相同的 Anthropic 请求分别发往标准 OpenAI 和兼容 OpenAI 的上游地址
- **THEN** 系统 MUST 生成语义等价的 request body，且差异仅体现在 endpoint 与 header 等 transport 字段

### Requirement: OpenAI request mapping SHALL normalize system, assistant, tool, and tool-result semantics deterministically
系统 MUST 以稳定规则完成 `system` 拼装、assistant 文本映射、tool call 映射和 tool result 展平，并使这些规则能够独立于传输层测试。

#### Scenario: Merge system content with custom injection prompt
- **WHEN** 请求同时包含 `system` 内容和 `custom_injection_prompt`
- **THEN** 系统 MUST 以确定顺序将其归并为 OpenAI `system` message，且不得生成空白 system message

#### Scenario: Convert assistant tool call and tool result
- **WHEN** 对话中出现 assistant 发起的工具调用及后续工具结果
- **THEN** 系统 MUST 将其映射为 OpenAI `assistant.tool_calls` 与 `tool` role 消息，并保持 `tool_call_id` 关联关系稳定

### Requirement: OpenAI upstream request builder SHALL handle endpoint and auth differences without reinterpreting body semantics
系统 MUST 通过独立的上游构建层处理标准 OpenAI、Azure OpenAI 与兼容 base URL 的 endpoint 与鉴权头差异，但不得在此阶段重新解释 `messages`、`tools`、`tool_choice` 或 system 语义。

#### Scenario: Target is Azure OpenAI
- **WHEN** 目标地址属于 Azure OpenAI Chat Completions 端点
- **THEN** 系统 MUST 使用 Azure 所需的 endpoint 形式与 `api-key` 鉴权头，而不是通用 Bearer token 头

### Requirement: OpenAI response transformer SHALL preserve a legal Anthropic event lifecycle until terminal completion
系统在将 OpenAI Chat Completions SSE 转回 Anthropic SSE 时，MUST 保持文本块、thinking 块、tool_use 块和最终 `message_stop` 的合法顺序，且 MUST 在收到终态标记后再完成收口。

#### Scenario: Finish reason arrives before DONE
- **WHEN** 上游 chunk 已包含 `finish_reason`，但 `data: [DONE]` 仍未到达
- **THEN** 系统 MUST 记录 stop reason 相关状态，但 MUST 不提前发出最终 `message_delta`/`message_stop`

#### Scenario: Multiple tool call indices are interleaved
- **WHEN** 上游交错发送多个 `tool_calls[index]` 的参数增量
- **THEN** 系统 MUST 按 `index` 独立聚合工具调用状态，且不得让不同工具调用的参数片段串线

