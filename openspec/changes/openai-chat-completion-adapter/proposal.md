## Why

当前 Codex Proxy 支持 Codex Responses、Gemini 和 Anthropic 透传三种上游协议，但缺少对 **OpenAI Chat Completion API** 的兼容支持。这意味着无法接入 OpenAI 官方 API、Azure OpenAI 或其他兼容 OpenAI Chat Completion 格式的第三方服务（如 Groq、DeepSeek、Moonshot 等）。添加此转换层可扩展代理的兼容性范围，让 Claude Code 无缝使用更多模型供应商。

## What Changes

- **新增 OpenAI Chat Completion 转换后端**：实现 `TransformBackend` trait，将 Anthropic Messages 协议转换为 OpenAI Chat Completion 协议
- **请求体转换**：
  - `messages` 数组转换：Anthropic content blocks → OpenAI message content
  - `tools` 转换：Anthropic tool schema → OpenAI function schema
  - 参数映射：`max_tokens`、`temperature`、`top_p`、`stop_sequences` 等
  - 图片输入：Anthropic image block → OpenAI `image_url` format
  - System message：Anthropic `system` 字段 → OpenAI `system` role message
- **SSE 流式响应转换**：
  - OpenAI `data: {...}` 格式 → Anthropic SSE 事件格式
  - `content_block_start`、`content_block_delta`、`content_block_stop` 事件生成
  - `tool_calls` 流式增量解析与 Anthropic `tool_use` block 映射
  - `finish_reason` 映射到 Anthropic `stop_reason`
- **URL 路由规范**：支持 `/chat/completions` 端点路径
- **负载均衡集成**：OpenAI 端点可纳入现有 slot 级别的负载均衡和故障切换机制

## Capabilities

### New Capabilities

- `openai-chat-completion-transform`: Anthropic Messages ↔ OpenAI Chat Completion 双向协议转换能力，包括请求体转换和 SSE 流式响应转换

### Modified Capabilities

- `load-balancer-routing`: 扩展以支持 `openai` 类型的 converter，在端点配置中识别和处理 OpenAI Chat Completion 上游

## Impact

- **新增文件**：`main/src/transform/openai.rs`（参考 `gemini.rs` 结构）
- **修改文件**：
  - `main/src/transform/mod.rs`：导出新模块和类型
  - `main/src/server.rs`：在 converter 匹配逻辑中添加 `openai` 分支
  - `main/src/load_balancer/mod.rs`：端点类型扩展（如有必要）
  - 配置相关：支持 `openai` converter 类型配置
- **依赖**：无新增外部依赖，复用现有 `serde_json`、`reqwest` 等
- **兼容性**：完全向后兼容，新功能为可选扩展
