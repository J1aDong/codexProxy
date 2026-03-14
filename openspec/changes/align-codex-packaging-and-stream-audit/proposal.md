## Why

`codexProxy` 当前需要明确的不是“面向 codebuddy 的 prompt 清洗策略”，而是 **Anthropic Messages 与 OpenAI Chat Completions 在流式协议层的真实语义对齐**。已有实现已经能工作，但在 SSE 事件边界、thinking、tool calls、finish reason、usage、错误终止这些点上，仍需要严格对照 `deep-research-report.md` 和当前代码逐项审计，避免后续继续扩展时把协议细节越改越偏。

用户同时澄清了另一条清晰的责任边界：**把 `codexProxy` 发出的 OpenAI Chat 风格请求，对齐成 CodeBuddy CLI 风格，属于 `codebuddy2api` 的职责**，而不是 `codexProxy` 的 `openai chat` 转换层职责。`codexProxy` 里的 `codex` 转换层可以作为“如何组织 agent/system/tools”的参考，但不应把 codebuddy 风格包装直接落在本仓的 `openai chat` 能力里。

## What Changes

- 审计 `codexProxy` 中 `anthropic` 与 `openai chat` 的流式转换实现，逐项对照 `deep-research-report.md` 校验 SSE framing、事件顺序、thinking、tool calls、finish reason、usage 与错误终止语义。
- 为 `openai chat` 路径补充明确的行为边界：它在本仓中负责协议转换与流式语义对齐，**不负责**按 CodeBuddy CLI 风格重写 prompt/system/tool 包装。
- 为 `anthropic` passthrough 路径补充 SSE 完整性审计，确保 `event/data` 配对、注释心跳、终止事件与透传行为可验证。
- 补充回归要求与测试面，覆盖文本流、thinking block、工具调用增量、终止原因映射、usage 结束块与 passthrough SSE 完整性。

## Capabilities

### New Capabilities
- `chat-streaming-audit`: 为 `anthropic` 与 `openai chat` 的流式协议转换定义审计基线、行为边界与回归要求，覆盖事件边界、thinking、tool calls、finish reason、usage、错误终止，以及 vendor-specific prompt packaging 的职责边界。

### Modified Capabilities
- None.

## Impact

- `main/src/transform/openai.rs`
- `main/src/transform/anthropic.rs`
- `main/src/transform/mod.rs`
- `main/src/transform/codex/request.rs`（仅作为参考实现，不要求本 change 在此实现 codebuddy 风格包装）
- `main/tests/**` 与 `main/src/transform/codex/response/tests/**`
- 参考资料：`/Users/mr.j/Downloads/deep-research-report.md`
- 外部相关但不在本 change 内实现：`/Users/mr.j/myRoom/code/ai/MyProjects/codebuddy2api`
