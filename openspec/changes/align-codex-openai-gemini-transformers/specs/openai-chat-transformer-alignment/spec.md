## ADDED Requirements

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
