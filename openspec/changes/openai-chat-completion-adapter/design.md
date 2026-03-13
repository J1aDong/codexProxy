## Context

Codex Proxy 当前已实现三种上游协议转换：
1. **Codex Responses**（`codex.rs`）：Anthropic → Codex Responses API
2. **Gemini**（`gemini.rs`）：Anthropic → Google Gemini API
3. **Anthropic**（`anthropic.rs`）：透传模式

新增 OpenAI Chat Completion 转换层需遵循现有的 `TransformBackend` trait 架构，复用 `MessageProcessor` 的消息预处理逻辑，并实现独立的 SSE 响应转换器。

**约束条件**：
- 必须保持与现有 `TransformBackend` trait 的兼容性
- SSE 流式转换需处理 OpenAI 的增量 `tool_calls` 格式（带 index）
- OpenAI 的 `tool_calls` 在流式响应中分多次发送，需累加解析

## Goals / Non-Goals

**Goals:**
- 实现完整的 Anthropic Messages → OpenAI Chat Completion 请求转换
- 实现 OpenAI Chat Completion SSE → Anthropic SSE 响应转换
- 支持 tool_use / tool_result 的双向映射
- 支持图片输入的格式转换
- 与现有负载均衡机制集成

**Non-Goals:**
- 不支持 OpenAI 特有的 `function_call` 旧格式（已弃用）
- 不支持 `audio` 输入（Claude Code 不使用）
- 不支持 `logprobs`、`stream_options` 等高级参数透传
- 不改变现有 slot 级别的路由策略

## Decisions

### D1: 模块结构与命名

**决策**：创建 `main/src/transform/openai.rs`，实现 `OpenAIChatBackend` 和 `OpenAIChatResponseTransformer`。

**理由**：
- 与 `gemini.rs` 结构一致，降低认知负担
- 命名为 `openai` 而非 `openai-chat`，因为未来可能扩展 Embedding/Completion 等端点

### D2: 消息转换策略

**决策**：复用 `MessageProcessor::transform_messages()` 的输出，再进行 OpenAI 格式映射。

**理由**：
- `MessageProcessor` 已处理图片解析、skill 提取等复杂逻辑
- 其输出的 Codex 风格格式（`type: "message"`, `type: "function_call"` 等）可被 OpenAI 转换层消费
- 避免重复实现图片 base64 处理

**转换映射**：
| Anthropic/Codex 输入格式 | OpenAI Chat 格式 |
|--------------------------|------------------|
| `type: "message", role: "user", content: [...]` | `{"role": "user", "content": [...]}` |
| `type: "message", role: "assistant", content: [...]` | `{"role": "assistant", "content": "...", "tool_calls": [...]}` |
| `type: "function_call"` | `{"role": "assistant", "tool_calls": [{"id": ..., "function": {...}}]}` |
| `type: "function_call_output"` | `{"role": "tool", "tool_call_id": ..., "content": "..."}` |
| `type: "input_text"` | `{"type": "text", "text": "..."}` |
| `type: "input_image", image_url: "..."` | `{"type": "image_url", "image_url": {"url": "..."}}` |
| `type: "thinking"` | （OpenAI 无原生支持，转为 reasoning_content 或跳过）|

### D3: Tools 转换

**决策**：Anthropic `tools` → OpenAI `tools`，schema 结构基本一致。

**转换映射**：
```json
// Anthropic
{"name": "get_weather", "description": "...", "input_schema": {...}}

// OpenAI
{"type": "function", "function": {"name": "get_weather", "description": "...", "parameters": {...}}}
```

### D4: SSE 响应转换策略

**决策**：实现状态机处理 OpenAI 流式 `tool_calls`。

**OpenAI 流式格式特点**：
- 每个 `tool_call` 带有 `index` 字段，按 index 累加
- `id` 和 `function.name` 只在首次 chunk 出现
- `function.arguments` 是 JSON 字符串，需流式拼接直到完整

**Anthropic SSE 事件序列**：
```
event: message_start
event: content_block_start (type: text)
event: content_block_delta (type: text_delta)
event: content_block_stop
event: content_block_start (type: tool_use)
event: content_block_delta (type: input_json_delta)
event: content_block_stop
event: message_delta
event: message_stop
```

### D5: finish_reason 映射

| OpenAI `finish_reason` | Anthropic `stop_reason` |
|------------------------|-------------------------|
| `stop` | `end_turn` |
| `tool_calls` | `tool_use` |
| `length` | `max_tokens` |
| `content_filter` | `stop_sequence`（或 error）|

### D6: URL 端点构建

**决策**：支持灵活的 URL 模板。

- 标准 OpenAI：`https://api.openai.com/v1/chat/completions`
- Azure OpenAI：`https://{resource}.openai.azure.com/openai/deployments/{deployment}/chat/completions?api-version=...`
- 第三方兼容：用户自定义 base URL + `/chat/completions`

## Risks / Trade-offs

### R1: Thinking/Reasoning 块兼容性
- **风险**：OpenAI 无原生 `thinking` block，部分模型返回 `reasoning_content`
- **缓解**：
  - 对于支持 `reasoning_content` 的模型（如 DeepSeek），映射为 thinking delta
  - 对于不支持的模型，将 thinking 块合并到 text 或静默丢弃

### R2: 流式 tool_calls 解析错误
- **风险**：JSON 字符串流式拼接可能因格式错误导致解析失败
- **缓解**：
  - 在 `content_block_stop` 时检测 JSON 完整性
  - 不完整时记录警告但不中断流

### R3: System Message 模式差异
- **风险**：部分模型不支持 `system` role，需要 `developer` role 或内联到首条 user message
- **缓解**：支持配置 `system_message_mode`（参考 vercel-ai 实现）

### R4: 多 tool_calls 交错
- **风险**：OpenAI 可能交错发送多个 tool_call 的 arguments chunks
- **缓解**：使用 `Vec<Option<ToolCallState>>` 按 index 管理每个 tool call 的状态

## Open Questions

1. **是否支持 `reasoning_effort` 参数？** OpenAI o-series 使用 `reasoning_effort`，Anthropic 无对应。建议：透传或忽略，不阻塞请求。
2. **`stream_options.include_usage` 是否强制启用？** 是，需要 usage 信息用于日志和监控。
