## Context

当前仓库通过 `main/src/transform/openai.rs` 将 Anthropic 风格请求转换为 OpenAI Chat Completions 上游请求，并将 OpenAI 流式 chunk 转回 Anthropic SSE。主流程之外，还存在 `main/src/transform/processor.rs` 负责中间消息规范化，以及 `main/src/server.rs` / `main/src/server/stream_decision.rs` 对流式输出进行聚合、裁剪、重试与回退控制。

这次变更是一个典型的跨模块协议边界修复：问题不只在字段少映射，而是同时涉及请求构造、内容语义保真、流式生命周期、错误收口策略和测试覆盖。约束条件是：
- 先解决 P0/P1，并把 P2 作为第二阶段一并记录；
- 不引入额外依赖；
- 尽量不改变已对齐的基础文本/工具流路径；
- 保持 Anthropic 下游接口的兼容性，同时减少对 OpenAI 上游语义的扭曲。

## Goals / Non-Goals

**Goals:**
- 让 Anthropic → OpenAI 请求构造保留高优先级调用意图，至少补齐 `tool_choice`、`parallel_tool_calls`、stream 语义控制，以及其他已识别的 P1 参数策略。
- 让 OpenAI 流式 chunk → Anthropic SSE 的事件生命周期更严格，避免“无内容先发 message_start”与“损坏参数静默变空对象”等问题。
- 补齐对关键流式 delta 的处理策略，包括兼容旧式 `function_call`、处理 `refusal`、统一 thinking 可见性控制。
- 为本次修复增加回归测试，覆盖 P0/P1 审计结论中的高风险路径。
- 把 P2 范围也沉淀到本次 spec / tasks 中，明确第二阶段的目标、边界和实施顺序。

**Non-Goals:**
- 不在第一实施阶段中完整保真所有 Anthropic 特有内容块，例如 document 的结构化透传。
- 不在第一实施阶段中系统性重做全部 multimodal 语义。
- 不重构整个 transform 架构；只在现有边界上做兼容性收敛与防错增强。
- 不要求在当前阶段一次性完成所有 P2 增强项。

## Decisions

### 1. 将本次变更拆成“请求兼容性”和“流式兼容性”两条 capability
- **决策**：分别用 `chat-request-compatibility` 与 `chat-stream-compatibility` 两份 spec 约束行为。
- **原因**：问题来源虽集中在一个 backend，但失配点天然分成“请求进入上游之前”和“流式从上游回落到下游之后”两类，拆开更利于测试与验收。
- **备选方案**：使用单一 capability 覆盖全部协议行为。
- **不选原因**：会让 tasks 和回归测试颗粒度过粗，难以明确 P0/P1 的先后顺序。

### 2. 保留“上游流式 + 本地聚合”的总体模式，但让是否流式更尊重调用方语义
- **决策**：不推翻当前通过上游流再聚合非流响应的总体实现，但要求 backend 不再在协议层面无条件覆盖调用方 stream 意图。
- **原因**：当前服务层已有聚合与回退机制，完全改为双路径（真正非流 + 真流）成本较高；但继续无条件强制 stream 会放大兼容性风险。
- **备选方案**：维持现状，始终对 OpenAI 上游使用 stream。
- **不选原因**：这正是审计出的明确协议偏差之一。

### 3. 工具调用兼容优先于更广泛内容保真
- **决策**：P0/P1 优先修工具选择、并行工具控制、tool-call delta 收口和 tool 参数错误处理；document、system fidelity 与更广义 multimodal fidelity 作为 P2 第二阶段推进。
- **原因**：工具调用是最容易直接破坏 SDK/客户端行为的部分，且外部兼容层事故也主要集中在这里。
- **备选方案**：同步推进 thinking/document/multimodal 的更完整保真。
- **不选原因**：范围会明显膨胀，不利于先收敛高优先级风险。

### 4. 对损坏的工具参数采用“显式失败或显式标记”而不是静默兜底为空对象
- **决策**：在非流聚合和必要的流式收口路径中，避免把损坏的 tool 参数 JSON 静默收口成 `{}`。
- **原因**：静默吞错会让协议问题伪装成业务正常，增加排查成本。
- **备选方案**：维持 `{}` 兜底，保证接口总能返回结构合法 JSON。
- **不选原因**：这会隐藏真实协议错误，属于错误的成功。

### 5. thinking 输出必须受请求级策略控制
- **决策**：`allow_visible_thinking` 在 OpenAI backend 的 response transformer 中必须真正生效。
- **原因**：当前实现会把 `reasoning_content` 直接转成 Anthropic `thinking_delta`，但并未遵守请求级可见性策略。
- **备选方案**：继续视 `reasoning_content` 为普通增强信息，默认透出。
- **不选原因**：这与 Anthropic 请求级 thinking 语义不一致。

### 5.1 Phase 1 对 thinking 完整性的收口边界
- **决策**：Phase 1 仅保证 `thinking_delta` 与 `stop_sequence` 的下游行为；`signature_delta` 暂不在 OpenAI Chat Completions 路径中实现。
- **原因**：当前仓库内没有稳定、已验证的 OpenAI chat chunk 字段可无歧义映射为 Anthropic `signature_delta`，贸然定义项目内私有字段会扩大协议歧义。
- **备选方案**：约定一个项目内字段（如 `signature` / `reasoning_signature`）并桥接成 `signature_delta`。
- **不选原因**：这会引入非标准兼容约定，不适合作为当前 Phase 1 的默认行为。

### 6. 用同一份 change 记录两阶段计划，但实现时严格分阶段推进
- **决策**：保留现有 change 名称与目录，在同一套 artifacts 中同时记录 P0/P1 与 P2；其中 P0/P1 为当前实现范围，P2 为后续阶段。
- **原因**：P2 是本次审计明确识别出的后续工作，和前一阶段共享上下文与能力边界，写在同一个 change 里更利于后续承接。
- **备选方案**：把 P2 另起一份 change。
- **不选原因**：当前信息尚未过期，拆成新 change 会增加文档搬运和追踪成本。

## Risks / Trade-offs

- **[风险] 修正 stream 语义后，可能影响当前依赖“总是走上游流式”的非流路径行为** → **缓解**：保留本地聚合模式，但让 stream 选择逻辑可控，并补充非流聚合回归测试。
- **[风险] 补 `tool_choice` / `parallel_tool_calls` 后，不同 OpenAI 兼容上游支持程度不一** → **缓解**：优先对官方/主流语义对齐；对不支持参数的上游保留清晰降级策略或显式日志。
- **[风险] 增加对 deprecated `function_call` 的兼容会让逻辑更复杂** → **缓解**：仅在流式接收层做兼容分支，不扩散到整体数据模型。
- **[风险] 严格化错误处理后，某些过去“看似成功”的请求会变成显式失败** → **缓解**：这是期望行为，配合测试和日志让失败可观测。
- **[风险] thinking 可见性收紧可能改变当前部分调用的输出内容** → **缓解**：以请求级语义为准，并在设计中明确这是修正协议偏差而非新增限制。
- **[风险] 在同一份 change 中记录 P2，可能让后续实施范围膨胀** → **缓解**：在 tasks 中显式区分 Phase 1 和 Phase 2，执行时先完成 P0/P1。

## Migration Plan

- 第一步：补齐 spec，锁定请求与流式兼容性要求，并记录 P2 第二阶段目标。
- 第二步：在 `openai.rs` 实现 P0/P1 请求参数映射与流式生命周期修正。
- 第三步：在 `server.rs` / 聚合路径修正损坏 tool 参数的错误收口策略。
- 第四步：补齐 transform / server 相关 P0/P1 回归测试。
- 第五步：以现有客户端路径验证基础文本、单工具、多工具、usage-only chunk、非流聚合与错误流。
- 第六步：在第二阶段推进 P2，包括 document/unknown block 策略、system fidelity、更广多模态支持，以及 transform 与 `stream_decision` 的端到端测试。
- 回滚策略：本次变更不涉及数据迁移，若出现回归，可直接回滚代码变更；必要时保留最小粒度提交以便快速回退。

## Open Questions

- 对于不支持 `tool_choice` / `parallel_tool_calls` 的 OpenAI 兼容上游，是选择静默省略、显式报错，还是按可配置降级处理？
- 对 `refusal` delta 的 Anthropic 下游表示，是否只映射为 stop reason，还是还需要补充内容块级语义？
- 对 `Document` 等 P2 内容块，后续是定义结构化降级策略，还是明确标记为当前不支持？
- P2 阶段是否需要把 transform 与 `stream_decision` 的协同行为单独拆成更细的 capability 或独立测试套件？
