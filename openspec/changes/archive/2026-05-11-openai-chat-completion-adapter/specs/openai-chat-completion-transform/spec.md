## ADDED Requirements

### Requirement: OpenAI Chat Completion 请求转换

系统 SHALL 将 Anthropic Messages API 请求转换为 OpenAI Chat Completion API 请求格式。

#### Scenario: 基本文本消息转换
- **WHEN** 收到 Anthropic 请求，包含 user 和 assistant 消息
- **THEN** 系统生成 OpenAI 格式的 messages 数组，角色和内容正确映射

#### Scenario: System 消息转换
- **WHEN** Anthropic 请求包含 `system` 字段
- **THEN** 系统生成 OpenAI 格式的 `{"role": "system", "content": "..."}` 消息

#### Scenario: 图片输入转换
- **WHEN** Anthropic 请求包含 `image` 类型的 content block
- **THEN** 系统生成 OpenAI 格式的 `{"type": "image_url", "image_url": {"url": "..."}}`

#### Scenario: Tools 定义转换
- **WHEN** Anthropic 请求包含 `tools` 数组
- **THEN** 系统生成 OpenAI 格式的 `tools` 数组，`input_schema` 映射为 `parameters`

### Requirement: Tool Use 消息转换

系统 SHALL 正确转换 tool_use 和 tool_result 消息。

#### Scenario: Assistant tool_use 转换
- **WHEN** Anthropic 消息包含 `tool_use` content block
- **THEN** 系统生成 OpenAI 格式的 `tool_calls` 数组，包含 `id`、`type: "function"` 和 `function` 对象

#### Scenario: User tool_result 转换
- **WHEN** Anthropic 消息包含 `tool_result` content block
- **THEN** 系统生成 OpenAI 格式的 `{"role": "tool", "tool_call_id": "...", "content": "..."}`

### Requirement: SSE 流式响应转换

系统 SHALL 将 OpenAI Chat Completion SSE 流式响应转换为 Anthropic SSE 格式。

#### Scenario: 文本内容流式转换
- **WHEN** OpenAI 返回包含 `delta.content` 的 SSE chunk
- **THEN** 系统输出 Anthropic 格式的 `content_block_start`（type: text）、`content_block_delta`（type: text_delta）、`content_block_stop` 事件序列

#### Scenario: Tool Calls 流式转换
- **WHEN** OpenAI 返回包含 `delta.tool_calls` 的 SSE chunks
- **THEN** 系统正确累加 arguments，输出 `content_block_start`（type: tool_use）、`content_block_delta`（type: input_json_delta）、`content_block_stop` 事件序列

#### Scenario: 多个 Tool Calls 处理
- **WHEN** OpenAI 返回多个 tool_calls（不同 index）
- **THEN** 系统按 index 分别管理每个 tool call 的状态，独立输出各自的 content block 事件

#### Scenario: Finish Reason 映射
- **WHEN** OpenAI 返回 `finish_reason: "stop"`
- **THEN** 系统输出 `message_delta` 包含 `stop_reason: "end_turn"`

#### Scenario: Finish Reason Tool Use 映射
- **WHEN** OpenAI 返回 `finish_reason: "tool_calls"`
- **THEN** 系统输出 `message_delta` 包含 `stop_reason: "tool_use"`

### Requirement: HTTP 请求构建

系统 SHALL 构建正确的 HTTP 请求发送给 OpenAI 兼容的上游。

#### Scenario: 标准 OpenAI 端点
- **WHEN** converter 为 `openai` 且 target_url 为 `https://api.openai.com`
- **THEN** 系统发送 POST 请求到 `https://api.openai.com/v1/chat/completions`，包含 `Authorization: Bearer {api_key}` header

#### Scenario: 自定义 Base URL
- **WHEN** target_url 为自定义地址（如 `https://api.deepseek.com`）
- **THEN** 系统发送 POST 请求到 `{target_url}/v1/chat/completions` 或用户配置的完整路径

#### Scenario: Azure OpenAI 端点
- **WHEN** target_url 包含 Azure OpenAI 格式路径
- **THEN** 系统发送请求到正确的 Azure 端点，包含 `api-key` header

### Requirement: 参数映射

系统 SHALL 正确映射 Anthropic 和 OpenAI 的参数。

#### Scenario: max_tokens 映射
- **WHEN** Anthropic 请求包含 `max_tokens`
- **THEN** OpenAI 请求包含 `max_tokens`（或 `max_completion_tokens` 对于 reasoning 模型）

#### Scenario: temperature 和 top_p 映射
- **WHEN** Anthropic 请求包含 `temperature` 和/或 `top_p`
- **THEN** OpenAI 请求包含对应的参数

#### Scenario: stop_sequences 映射
- **WHEN** Anthropic 请求包含 `stop_sequences`
- **THEN** OpenAI 请求包含 `stop` 参数

#### Scenario: stream 强制启用
- **WHEN** 转换为 OpenAI 格式
- **THEN** 请求中 `stream` 字段设置为 `true`，`stream_options.include_usage` 设置为 `true`

### Requirement: Thinking/Reasoning 块处理

系统 SHALL 处理 OpenAI 响应中的 reasoning 内容。

#### Scenario: reasoning_content 处理
- **WHEN** OpenAI 返回 `reasoning_content` 字段（如 DeepSeek）
- **THEN** 系统输出 Anthropic 格式的 `thinking` content block

#### Scenario: 无 reasoning 支持
- **WHEN** OpenAI 模型不返回 reasoning 信息
- **THEN** 系统正常处理文本和 tool 内容，不输出 thinking block

### Requirement: 错误处理

系统 SHALL 正确处理 OpenAI API 返回的错误。

#### Scenario: API 错误响应
- **WHEN** OpenAI 返回非 2xx 状态码或 error 响应体
- **THEN** 系统将错误转换为 Anthropic 兼容的错误格式返回给客户端

#### Scenario: 流式解析错误
- **WHEN** SSE chunk JSON 解析失败
- **THEN** 系统记录警告并继续处理后续 chunk，不中断整个流
