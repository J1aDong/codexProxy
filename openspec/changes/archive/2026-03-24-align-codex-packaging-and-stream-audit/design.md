## Context

当前 `codexProxy` 里已经存在三条相关链路：

- `main/src/transform/openai.rs` 负责把 OpenAI Chat Completions 风格流式响应转换成 Anthropic 风格的 `message_start / content_block_* / message_delta / message_stop` 事件序列，并处理 thinking、tool calls、finish reason 等状态机逻辑。
- `main/src/transform/anthropic.rs` 目前主要做 passthrough，保留上游 Anthropic SSE 的 `event/data` 语义与注释心跳。
- `main/src/transform/codex/request.rs` 已经实现了 Codex agent / passthrough 请求增强判定，能体现 Codex CLI 风格的 prompt 与 tool 组织思路，但它不是本 change 要直接扩展成 codebuddy 包装层的目标模块。

用户进一步澄清了项目边界：

1. **`codexProxy` 的 `openai chat` 层不需要清洗成 CodeBuddy 样式**；
2. `codexProxy` 在本 change 中只需要聚焦 **Anthropic ↔ OpenAI Chat 的流式转换语义** 是否正确；
3. **把 `codexProxy` 发出的 OpenAI Chat 请求，对齐到 CodeBuddy CLI 风格，属于 `codebuddy2api` 的职责**，可参考 `codexProxy` 里 Codex 转换层思路，但不应在本 openspec 中落成 `codexProxy` 的 requirement。

因此，这个 change 虽然名称沿用了 `align-codex-packaging-and-stream-audit`，但实际范围应收窄为：
- 在 `codexProxy` 中审计并必要时修正流式协议转换；
- 明确排除 codebuddy 风格包装改造；
- 把 `codebuddy2api` 兼容包装仅作为外部背景和责任边界说明。

## Goals / Non-Goals

**Goals:**
- 对照 `deep-research-report.md` 审计 `anthropic` 与 `openai chat` 当前流式转换实现。
- 明确 `openai chat` 路径在本仓中的职责边界：做协议转换，不做下游厂商风格 prompt 重写。
- 为文本流、thinking、tool calls、finish reason、usage、错误终止、Anthropic passthrough SSE 完整性建立回归要求。
- 让后续实现可测试、可回归、可解释。

**Non-Goals:**
- 本 change 不在 `codexProxy` 内实现 CodeBuddy CLI 风格 prompt/system/tools 重排。
- 本 change 不把 `codebuddy2api` 的包装逻辑迁入 `codexProxy`。
- 本 change 不引入新的上游厂商协议支持。
- 本 change 不重写整套通用流式引擎，只审计并修正当前 `anthropic` / `openai chat` 路径。

## Decisions

### 1. 将变更范围收敛为“流式协议审计”，而不是“codebuddy 兼容包装”
- **Decision:** proposal/spec/tasks 只保留 `chat-streaming-audit` capability，不再为 `codexProxy` 定义 `upstream-compatible-packaging` 能力。
- **Why:** 用户已经明确 codebuddy 风格包装属于 `codebuddy2api`，把它继续保留在 `codexProxy` 的 spec 中会导致仓库职责混淆。
- **Alternatives considered:**
  - 继续在 `codexProxy` 里定义上游兼容包装：与用户澄清的责任边界冲突。

### 2. `codex/request.rs` 只作为外部参考实现
- **Decision:** `codex/request.rs` 可以在 design 中作为参考来源，用来理解 Codex CLI 如何组织 system/tools/context，但本 change 不要求在 `openai chat` 路径复用其包装策略来生成 codebuddy 风格请求。
- **Why:** 用户希望保留“参考 codex 转换层思路”这层联系，但不希望把它写成本仓必须实现的 packaging capability。
- **Alternatives considered:**
  - 完全忽略 codex 层：会丢掉有价值的参考背景。
  - 直接把 codex 包装能力搬进 `openai chat`：越界。

### 3. 流式审计以“研究报告映射表 + 当前代码”为准
- **Decision:** 审计要求围绕 `main/src/transform/openai.rs` 与 `main/src/transform/anthropic.rs` 的现有行为展开，重点核对 SSE 事件边界、thinking、tool call 增量、finish reason / stop reason 映射、usage 结束块与错误终止。
- **Why:** 当前代码已经有状态机实现，问题在于是否与报告定义逐项对齐，而不是是否存在一套完全不同的架构。
- **Alternatives considered:**
  - 直接重写双向流式转换层：成本高，且与本轮“先审计、后最小修正”的目标不匹配。

### 4. 将“职责边界”写进 spec
- **Decision:** `chat-streaming-audit` spec 中增加 requirement，明确本仓 `openai chat` 转换不得引入面向特定下游厂商的 prompt 包装重写。
- **Why:** 如果只在对话里说明，不写进 spec，后续很容易再次把 `codebuddy2api` 的工作误塞回 `codexProxy`。
- **Alternatives considered:**
  - 只在 design 提醒：约束力不够。

## Risks / Trade-offs

- **[Risk]** 只做协议审计而不碰下游包装，用户短期可能仍需要在 `codebuddy2api` 继续调兼容包装。
  **Mitigation:** 在文档中明确这是有意的职责拆分，而不是遗漏。

- **[Risk]** 研究报告中的理论映射与当前上游真实行为存在偏差。
  **Mitigation:** 审计要求必须结合当前实现与回归测试，不把报告内容直接等同为实现真相。

- **[Risk]** `anthropic` passthrough 看似简单，但隐藏的 SSE framing / 注释心跳问题容易被忽视。
  **Mitigation:** 在 spec 和 tasks 中单独列出 Anthropic passthrough SSE integrity。
