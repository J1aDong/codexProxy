## Context

当前 `main/src/transform/openai.rs` 同时承担了三类职责：
1. 将 Anthropic Messages 风格输入映射为 OpenAI Chat Completion 请求；
2. 处理标准 OpenAI、Azure OpenAI 与兼容 base URL 的上游请求构建；
3. 将 OpenAI SSE 增量转换为 Anthropic SSE 事件流。

这条链路已经可用，但代码集中度较高，导致两个问题：
- 当协议兼容问题出现时，很难判断缺陷来自“请求映射”还是“流式收尾/状态机”；
- 与参考项目 `/Users/mr.j/myRoom/code/ai/claude-code-proxy` 对齐时，只能做整文件对照，无法逐层验证 system、tool、finish reason、usage 等关键语义。

参考项目虽然是 Python 实现，但它把 `request_converter.py`、`response_converter.py` 与 API 调用边界拆开，表达出了更清晰的职责模型。本次设计要吸收这种职责拆分方式，同时保留 codexProxy 当前已经正确处理的细节，例如 usage-only chunk 直到 `data: [DONE]` 再收口，以及 `allow_visible_thinking` 控制。

## Goals / Non-Goals

**Goals:**
- 在不改变 `openai` converter 对外入口的前提下，重构 OpenAI Chat Completion 转换实现的内部职责边界
- 让请求映射、上游请求构建、SSE 响应转换三部分可以独立阅读、测试和回归
- 对齐参考项目的核心语义：system 拼装、assistant/tool/tool_result 映射、tool choice 归一、finish reason → stop reason 映射
- 为流式文本、tool_calls 增量、usage-only chunk、reasoning/refusal 等场景建立稳定测试矩阵

**Non-Goals:**
- 不新增新的 converter 类型、路由入口或配置模型
- 不把实现完全改写成与参考项目同样的文件结构，只要求职责边界对齐
- 不扩展到 OpenAI 其他端点（responses、embeddings、audio）
- 不顺手调整负载均衡、鉴权或非 OpenAI transformer 的行为
- 不修改 `codex` 转换层、`anthropic` 透传链路或 `gemini` 转换层

## Decisions

### D1: 保留 `OpenAIChatBackend` 作为唯一外部入口，内部拆成三个职责层
**Decision**: `TransformBackend` 入口继续保留在 `OpenAIChatBackend`，但内部逻辑拆分为：
- request mapping：只负责 Anthropic → OpenAI body 语义映射；
- upstream request builder：只负责 endpoint、鉴权头和 Accept/Content-Type；
- streaming response transformer：只负责 OpenAI SSE → Anthropic SSE 状态机。

**Why**: 这样可以对齐参考项目的 request/response converter 模式，同时避免影响 `server.rs`、`transform/mod.rs` 的现有调用面。

**Alternatives considered**:
- 直接维持单文件大实现：改动最少，但后续继续对齐参考实现时认知成本高。
- 完全拆成多个公开模块：结构更纯，但会扩大本次 refactor 的 blast radius。

### D2: 请求转换不绑定特定共享预处理层
**Decision**: 请求侧只要求完成 Anthropic Messages 到 OpenAI `messages` / `tools` / `tool_choice` 的语义映射。实现上可以复用现有内部 helper 或预处理结果，但前提是不得改变 `codex` 转换层、`anthropic` 透传链路或 `gemini` 转换层的行为边界。

**Why**: 本次 change 的目标是收敛 `openai` converter 的内部职责，而不是重构共享消息预处理层。把是否复用 `MessageProcessor` 视为实现细节，可以避免 design 文档把改动范围误导到其他 transformer。

**Alternatives considered**:
- 强制要求复用 `MessageProcessor::transform_messages()`：可以减少重复逻辑，但会把设计错误地绑定到共享中间层实现。
- 完全禁止复用现有 helper：范围最保守，但可能引入不必要的重复代码。

### D3: 将 system 拼装与自定义注入提示视为 request mapping 的一部分，并显式测试顺序与合并策略
**Decision**: `system` 与 `ctx.custom_injection_prompt` 的合并继续在请求转换阶段完成，统一产出首条 OpenAI `system` message。

**Why**: 它属于协议级语义而不是传输级适配；参考项目也把 system 处理放在请求转换器中。重构后需要把“原始 system 在前，custom injection 在后，空文本不输出”作为固定契约。

**Alternatives considered**:
- 在上游请求构建阶段拼接 system：职责错误，且不利于 body 单测。
- 把 custom injection 混入首条 user message：会改变语义边界。

### D4: 流式响应转换继续以 `[DONE]` 为唯一终态收口点，保留 usage-only chunk 语义
**Decision**: 即便参考项目在收到 `finish_reason` 后就准备 stop reason，本项目仍保持“记录 finish reason/usage，但只有在 `data: [DONE]` 后发出最终 `message_delta`/`message_stop`”的规则。

**Why**: 当前实现已经显式修复了 include_usage 场景下 usage 尾块先于 `[DONE]` 到来的问题，这一点比参考项目更稳，不能在对齐时倒退。

**Alternatives considered**:
- 在首次出现 `finish_reason` 时立刻收口：实现更简单，但会让 usage 统计丢失或生命周期过早结束。

### D5: tool call 状态聚合继续以 `index` 为主键，但将“状态更新”和“Anthropic 事件发射”分开
**Decision**: 对每个 `tool_calls[index]`，先做增量归并（id/name/arguments），再判断是否需要发出 `content_block_start` / `input_json_delta` / `content_block_stop`。

**Why**: 参考项目的核心经验是按 index 聚合；本项目当前也这样做，但状态更新和事件发射耦合较深。拆开后可更容易验证“文本块先 stop，再开启 tool 块”“交错多个 tool call 不串线”。

**Alternatives considered**:
- 维持现有写法：功能可用，但不利于细粒度单测。
- 在每次参数增量都尝试完整 JSON 解析后再发事件：会引入不必要的时序耦合。

### D6: 参考项目中的 response converter 作为语义基线，而不是逐行移植目标
**Decision**: 对齐参考项目中已验证的语义边界，如 assistant/tool/tool_result 的转换方式、finish reason 映射和 tool result 展平；但保留本项目已有且更强的兼容行为，如 `reasoning_content`、`refusal`、`function_call` 兼容路径和 usage-only chunk 收尾。

**Why**: 用户要求“对齐 claude-code-proxy”，重点是语义和职责，而不是回退已有能力。

**Alternatives considered**:
- 逐行照搬参考项目：无法利用 Rust 侧已有更完善的流式兼容逻辑。
- 完全不参考：无法满足本次变更目标。

## Risks / Trade-offs

- [重构改变事件顺序] → 用事件序列测试覆盖 `text -> tool_use`、多个 tool call、`finish_reason` + `[DONE]` 的顺序约束。
- [职责拆分后出现重复转换逻辑] → 仅抽取最小辅助函数，不引入新的公共抽象层级。
- [参考项目与当前仓库语义不完全一致] → 以“参考项目的职责边界 + 当前仓库已验证的兼容行为”为最终准则，不做机械对齐。
- [Azure/自定义 base URL 回归] → 为 endpoint 构建和鉴权头补充独立测试，避免被请求映射改动连带破坏。

## Migration Plan

1. 先在现有 `openai.rs` 内完成逻辑分层或抽取最小私有 helper，保持外部 API 不变。
2. 为 request mapping、upstream builder、response transformer 分别补测试，确保行为先被锁定。
3. 在通过现有和新增测试后替换旧实现路径。
4. 若出现兼容性回归，可回滚到 refactor 前版本；因为不改入口和配置，回滚成本仅限于 transformer 实现文件。

## Open Questions

1. 是否需要在本次 refactor 中把 `openai.rs` 物理拆成多个文件，还是先保留单文件内部分层即可？默认建议先逻辑拆分，只有在实现明显变长时再拆文件。
2. 对于 `reasoning_content` 之外的其他供应商扩展字段，是否只要求“不破坏现有行为”，还是要在本次 spec 中明确更多映射规则？默认建议只锁定当前已支持路径。
3. 是否把请求侧的 `parallel_tool_calls` 和 `tool_choice` 规则进一步上提为共享 helper，供其他 transformer 复用？本次默认不做跨 transformer 抽象。
