## Why

当 Claude Code 通过 Codex Proxy 使用 OpenAI Chat Completions API 时，Anthropic 请求中的 `max_tokens` 值可能超过目标 API 的限制（如 8192），导致 `400 Invalid max_tokens value` 错误。不同模型和 API 提供商对 `max_tokens` 有不同的限制，需要在转换层进行适配。

此外，用户希望能够在 UI 中为每个 slot（Opus/Sonnet/Haiku）单独配置 `max_tokens` 限制，留空则使用 Claude Code 传入的值。

## What Changes

- 在 OpenAI Chat 转换器中添加 `max_tokens` 限制逻辑
- 支持按 slot（Opus/Sonnet/Haiku）配置最大输出 token 限制
- 当输入 `max_tokens` 超过限制时，自动裁剪到模型允许的最大值
- 在 UI 的 "OpenAI Chat 模型映射" 区域添加三个 `max_tokens` 输入框
- 配置跟随单个 endpoint 保存，类似模型映射的记住方式

## Capabilities

### New Capabilities

- `max-tokens-limiting`: 在 Anthropic → OpenAI 协议转换中，根据目标 slot 配置的 `max_tokens` 限制参数，避免超出 API 限制导致的错误

### Modified Capabilities

（无现有能力需要修改）

## Impact

- 影响后端文件：`main/src/transform/openai.rs`, `main/src/models/common.rs`, `main/src/transform/mod.rs`
- 影响前端文件：`fronted-tauri/src/types/configTypes.ts`, UI 组件
- 影响配置：`EndpointOption` 添加 `openaiMaxTokensMapping` 字段
- 兼容性：对现有行为向后兼容，留空则透传 Claude Code 传入的 `max_tokens`
