## Why

当前 Anthropic ↔ OpenAI Chat Completions 转换链路已经能覆盖基础文本与部分工具流式场景，但与官方协议语义仍存在明显偏差，这些偏差已经会影响 SDK 兼容性、工具调用正确性和流式客户端稳定性。风险最高的问题集中在工具选择语义、stream 参数语义、工具参数增量拼接/收口、以及 Anthropic 特有内容语义在出口侧的丢失或降级。

现在推进这项变更的原因是：本次审计识别出的 P0/P1 问题大多发生在协议边界，而不是纯内部实现细节。如果继续保持现状，容易出现“表面可用、细节不兼容”的问题，进而导致客户端解析失败、工具调用错位、以及难以排查的流式异常。

同时，这次审计还识别出一批不应丢失的 P2 范围：包括 `Document` / unknown block 的更合理处理、system block fidelity、更广的多模态输入保真，以及 transform 与 `stream_decision` 二次裁剪层之间的端到端一致性。它们不要求在第一阶段立即实现，但应该一并写入本次变更，避免后续重复梳理。

## What Changes

- 补齐 Anthropic → OpenAI 请求构造中的高优先级兼容参数，重点包括工具选择语义和并行工具调用控制。
- 使上游流式行为尽量遵循调用方原始意图，而不是在会改变协议语义的地方无条件强制流式。
- 收紧流式 chunk 生命周期处理，确保仅在存在合法下游内容时才发出 `message_start`，并避免损坏的工具参数被静默降级为 `{}`。
- 扩展响应转换逻辑，覆盖更多与兼容性相关的 OpenAI 流式 delta，包括仍可能出现的旧式 function-call 语义和 refusal 语义。
- 统一 thinking 可见性策略，减少 Anthropic 特有输入内容在 OpenAI 出口侧被直接丢弃或粗暴降级的情况。
- 为本次审计识别出的 P0/P1 问题补充有针对性的回归测试。
- 把 P2 范围也纳入本次 change artifacts，明确记录 document/unknown block、system fidelity、更广多模态支持，以及 transform 与 `stream_decision` 的端到端一致性要求。
- 实施顺序保持分阶段：**先做 P0/P1，再做 P2**。

## Capabilities

### New Capabilities
- `chat-request-compatibility`: 约束 Anthropic → OpenAI 请求构造的兼容性要求，包括工具选择、stream 语义、关键请求参数保真，以及调用方意图保持。
- `chat-stream-compatibility`: 约束 OpenAI 流式 chunk → Anthropic SSE 的兼容性要求，包括事件生命周期、tool-call 增量、finish/usage 语义、错误处理，以及与二次裁剪层的协同行为。

### Modified Capabilities
- None.

## Delivery Phases

### Phase 1: P0 / P1（优先实施）
- 修复工具选择、并行工具控制、stream 语义偏差、过早 `message_start`、tool 参数静默吞错、关键流式 delta 漏处理、thinking 可见性等高优先级问题。

### Phase 2: P2（记录在案，后续实施）
- 提升 `Document` / unknown block 的结构化处理策略。
- 提升 system block fidelity 与更广输入语义保真。
- 扩展非 user 图像与更广多模态内容处理。
- 为 transform 层与 `stream_decision` 层补充端到端一致性回归测试。

## Impact

- 受影响代码：
  - `main/src/transform/openai.rs`
  - `main/src/transform/processor.rs`
  - `main/src/server.rs`
  - `main/src/server/stream_decision.rs`
  - 相关 transform / server streaming 测试
- 受影响 API：
  - Anthropic 风格 `/v1/messages` 在 OpenAI Chat Completions 上游下的兼容行为
  - OpenAI 上游请求构造中的 tool calling 与 streaming 语义
- 受影响系统：
  - 工具调用链路
  - 流式客户端 / SDK 消费端
  - 当前通过“上游流式 + 本地聚合”实现的非流请求路径
  - 后续 P2 阶段会触及的 document/system/multimodal 语义保真路径
- 本次变更预计不新增第三方依赖。
