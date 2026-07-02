## Why

当前 Codex Responses ↔ Anthropic Messages 的双向转换体验不好，根因是 `main/src/transform/codex/response.rs` 已膨胀为 **4931 行的单体转换器**：`TransformResponse` 结构体承载 **60+ 字段**，混合了流式状态机、tool 身份绑定、泄漏检测（`LeakDetector`）、plan 桥接（`PlanBridge`，含写文件副作用）、commentary 阶段重定向、Markdown Bash 拦截、RawToolJson 4 级风险评估（30+ 诊断计数器）等 6 类以上职责。

这些是 CLAUDE.md 第 9 节「黏连治理」要求做**机制级收敛**（状态机/通道隔离/事件边界）却退化为「补丁地狱」的产物：大量 `looks_like_*_fragment` / `strip_*_noise` 字符串规则与 `LEAKED_TOOL_MARKERS` 等脆弱匹配，每修一个 case 就新增一条规则与一个诊断计数器。tool 身份靠 6 个 `HashMap`（`output_index`/`item_id`/`call_id` 双向映射）+ 4 级回退查询维持，新增 case 难、回归频发。输入侧 `build_codex_unified_request` 同样把 skill bridge、worktree sanitize、system prompt 注入耦合在一起。

参考 Vercel AI SDK 的设计（已派发 3 个 subagent 深研）：它用**结构化 tagged union + 分层 + 物理隔离**的流式状态机取代字符串规则，用 `output_index` 单一索引取代多重身份映射，用 `call_id`/`item_id` 双 ID 体系严格区分。本次重构按此方向彻底重写转换层，把可观测性与治理从「字符串补丁」收敛到「类型驱动 + 事件边界」。

## What Changes

- **拆解单体 `TransformResponse`**：把 4931 行的 `codex/response.rs` 按职责拆分为独立模块——核心流式状态机、tool 调用拼装、reasoning 处理、文本/工具事件输出，各自有清晰边界与可独立测试的单元。
- **用结构化事件模型取代字符串规则**：用 serde tagged enum 严格建模 Codex Responses 的 `OutputItem` union 与 SSE 事件 union（含 `#[serde(other)]` 兜底未知事件），替换当前的 `serde_json::Value` + 大量 `looks_like_*` 字符串嗅探。
- **简化 tool 身份绑定**：参考 ai-sdk，用 `output_index` 单一索引维护 `ongoing_tool_calls`，区分 `call_id`（回传引用，↔ Anthropic `tool_use.id`）与 item `id`（fc_...），删除 4 级回退查询与 pending update backlog 队列。
- **reasoning 三态机**：引入 `Active`/`CanConclude`/`Concluded` 状态处理 `store=false` + `encrypted_content` 的多轮上下文，替换当前脆弱的 commentary phase 字符串检测。`encrypted_content` 透传（不伪造 Anthropic signature）。
- **隔离治理逻辑为独立通道**：`LeakDetector`、`PlanBridge`、`MarkdownBash` 等治理逻辑从核心状态机中剥离，成为可独立开关、可观测的 side-channel 钩子，不再污染主转换路径的字段与控制流。
- **清理诊断计数器**：30+ 个细粒度诊断字段收敛为结构化、分通道的诊断输出，遵循「新增样本 → 可复现 → 可回归 → 可观测」闭环。
- **保留 Anthropic 生命周期保证**：`message_start`/`content_block_*`/`message_delta`/`message_stop` 事件语义、stop_reason、usage、tool_use 闭环不变（见 Modified Capabilities）。
- **BREAKING**（内部实现层）：`ResponseTransformer` trait 的 `transform_line` 仍保留为入口契约，但 `TransformResponse` 的字段布局与内部方法签名重构；`take_diagnostics_summary` 的输出结构变更。这些是内部 API，不影响对外 HTTP 契约。

## Capabilities

### New Capabilities
- `codex-responses-event-model`: 用 serde tagged enum 严格建模 Codex Responses 的 OutputItem / SSE 事件 union，含未知事件兜底，作为转换层的类型契约。

### Modified Capabilities
- `codex-responses-transformer-alignment`: 重构 response transformer 的内部架构（拆分单体、tool 身份简化、reasoning 三态机、治理逻辑隔离），同时保留对外 Anthropic 生命周期保证（reasoning/tool use/usage/stop_reason 收口语义不变）。

## Impact

- **核心代码**：`main/src/transform/codex/response.rs`（拆分）、`main/src/transform/codex/backend.rs`、`main/src/transform/providers/mod.rs`（输入侧解耦）、`main/src/transform/unified.rs`（如需扩展协议无关类型）、`main/src/transform/mod.rs`（trait/上下文）。
- **测试**：现有 `main/src/transform/codex/response/tests/` 下 7 个测试文件（含 `tool_leak_stream` 2406 行、`upstream_event_compat` 1666 行）是回归基线，必须全绿；重构期间作为行为契约。
- **依赖**：不新增外部依赖（用现有 `serde`/`serde_json`/`tokio`/`futures`）。优先标准库与项目已有依赖。
- **对外契约**：`/v1/messages` 与 `/v1/messages/count_tokens` 的 HTTP/SSE 行为不变；配置热更新、负载均衡、failback 不受影响。
- **风险**：转换器是请求主路径核心，重构需在测试基线保护下分阶段进行，保持每步可编译可测试。
