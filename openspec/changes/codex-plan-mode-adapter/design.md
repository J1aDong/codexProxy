## Context

当前 `main/src/transform/codex/request.rs` 只把 Codex 请求增强分成 `Agent` 与 `Passthrough` 两类；`main/src/transform/codex/response.rs` 虽然已经有较多文本清洗与泄漏抑制逻辑，但并没有识别或剥离 `<proposed_plan>...</proposed_plan>` 这一类 plan 包装标记。

与此相对，codex-cli 的 plan mode 是一套更结构化的协作协议，包含 plan 增量、执行审批与用户输入等事件。当前 codexProxy 并没有实现这套状态机，而是把 Claude Code plan mode 的大部分语义停留在提示词、schema 与普通文本层面。用户当前需要的不是完整协议复刻，而是先补一个最小适配层，让 Claude Code 经由 Codex API 时，能先稳定返回计划，再等待用户决定是否执行。

本次变更跨越 request augmentation 与 response SSE hygiene 两侧，因此需要先把边界讲清楚：只做方案 A 的最小 plan adapter，不引入新的服务层状态机，也不改现有 server 路由结构。

## Goals / Non-Goals

**Goals:**
- 为 Codex request augmentation 增加独立的 `Plan` 路径，用于承载 Claude Code 的 plan mode 请求
- 使用强信号识别 plan 请求，包括 `metadata.plan_mode`、`tool_choice.name == "ExitPlanMode"` 以及 `plan_approval_response` 相关信号
- 让 plan 请求的 outbound Codex 输入优先表达“先提出计划、等待确认”的约束，而不是直接沿用通用 agent 增强
- 在 Codex SSE → Anthropic SSE 过程中剥离 `<proposed_plan>` 可见包装标签，但保留计划正文
- 为 request / response 两侧补回归测试，确保 plan 行为增强同时不破坏非 plan 流量

**Non-Goals:**
- 不实现 codex-cli 的完整结构化 plan/approval 状态机
- 不新增独立的服务端会话状态持久化或执行阶段调度器
- 不修改 `server.rs` 的 backend 选择与整体请求路由模型
- 不在本次变更中完整建模 Claude Code 的 `context_management` 字段
- 不尝试一次性实现与 codex-cli 完全等价的协议级 plan 事件桥接

## Decisions

### 1. 将 plan 作为独立 augmentation mode，而不是继续塞进 Agent

**决定**：为 `RequestAugmentationMode` 新增 `Plan` 变体，并在 `decide_request_augmentation` 中单独判定。

**理由**：plan 模式与通用 agent 模式的目标不同。agent 偏向“可执行代理”，plan 偏向“先给方案再等待确认”。如果继续复用 `Agent`，plan 语义会继续埋在散落的提示词与启发式逻辑里，难以测试与扩展。

**备选方案**：继续沿用 `Agent`，只补更多 prompt 规则。
**不采用原因**：语义混杂，后续很难继续补 approval 相关能力，也不利于 regression test 明确表达行为。

### 2. 只用强信号识别 plan 请求，避免过宽文本猜测

**决定**：plan 判定优先依赖明确且高置信的信号：`metadata.plan_mode == true`、`tool_choice.name == "ExitPlanMode"`、请求文本或工具 schema 中出现 `plan_approval_response`。

**理由**：用户普通提到“plan”或“先想想”并不一定意味着要进入 Claude Code plan mode；过宽的文本匹配会误伤正常请求。

**备选方案**：只要用户文本包含“plan/计划”就视为 plan mode。
**不采用原因**：误判概率高，会把普通问答或 agent 请求错误引入 plan 路径。

### 3. 方案 A 采用 prompt-layer adapter，而不是完整协议状态机

**决定**：保持上游仍是普通 Codex `/v1/responses` 调用，在 request/response 转换层做最小 plan 适配。

**理由**：当前用户目标是先得到计划、再自己决定是否执行；实现完整 codex-cli plan 协议需要更大的跨层改造，包括结构化 plan delta、审批事件与执行阶段状态管理，超出本次最小变更范围。

**备选方案**：直接在 proxy 内复刻 codex-cli plan/approval state machine。
**不采用原因**：改动面过大，验证成本高，也不符合这次先落地最小可用能力的目标。

### 4. response 侧只剥离 `<proposed_plan>` 包装，不吞掉计划正文

**决定**：在 `handle_text_fragment` / `flush_text_carryover` 所在的文本 hygiene 流程里，把 `<proposed_plan>` 与 `</proposed_plan>` 视为不可见包装标记，正文继续按普通文本流出。

**理由**：用户需要看到计划内容本身，真正该隐藏的是 wrapper tag，而不是 plan body。

**备选方案**：整段 `<proposed_plan>...</proposed_plan>` 全部抑制。
**不采用原因**：会把最重要的计划正文也隐藏掉，违背这次改动目标。

### 5. 先修测试编译面阻塞，再进入真正 red/green

**决定**：把缺失导入、测试初始化字段不全等问题视为测试地基修正，先补到可运行，再继续 plan adapter 的 red/green。

**理由**：当前已有编译阻塞（例如 `collect_request_text_corpus` 测试导入、`TransformContext` 缺字段），不先清理就无法准确观察 plan 相关测试的失败原因。

## Risks / Trade-offs

- **[误判 plan 请求]** → 只使用强信号，不靠宽泛自然语言匹配；并为非 plan 请求保留回归测试
- **[与 codex-cli 仍有语义差距]** → 在 proposal / design 中明确本次仅为方案 A，后续如需完整审批协议再单独扩展
- **[wrapper 标签被拆跨 chunk，剥离不完整]** → 在现有 `text_carryover` 机制上实现 carryover-aware stripping，并补 chunk-split 测试
- **[新增 hygiene 逻辑误伤普通文本]** → 限制只处理精确的 `<proposed_plan>` 包装标记，保持其他文本沿用现有逻辑

## Migration Plan

- 先补并运行 request / response 两侧定向红测
- 在 `request.rs` 引入 `Plan` augmentation 与强信号检测
- 在 `response.rs` 增加 `<proposed_plan>` wrapper stripping
- 跑 plan 相关定向测试与 codex transform 回归子集
- 若行为不符预期，直接回滚该 change 涉及的 request/response 改动即可，无需数据迁移

## Open Questions

- 后续是否需要把 `plan_approval_response` 进一步桥接为更接近 codex-cli 的结构化审批事件，而不只是保留在文本/提示层
- Claude Code 的 `context_management` 字段是否应在后续单独建模进 `AnthropicRequest`，以减少 plan mode 判断对文本语料的依赖
