## Context

当前 `main/src/transform` 已经有 `TransformBackend`、`ResponseTransformer`、`TransformContext` 三个核心抽象，但它们更多是“最低限度可复用接口”，还没有把项目真正想要的架构意图固定下来。现在的状态是：
- `anthropic` 路径已经近似 passthrough，但仍是特例实现，没有被定义成所有 backend 的 identity baseline；
- `codex` 相对接近分层结构，不过 request 侧逻辑与 response 侧状态机之外，仍有大量 provider-specific 语义和 transport concern 混在同条链路里；
- `openai` 与 `gemini` 还保留较多集中式实现，和 `codex` 的演进方向不一致；
- 共享逻辑只部分沉淀在 `MessageProcessor`，SSE frame 解析、工具拦截、usage 归一等职责还没有统一归位。

用户希望从架构师视角明确一个基线：项目以 Anthropic 协议作为内部 canonical model；若上游本身就是 Anthropic，则实现一个几乎全透传的 `AnthropicTrans`；若是 Codex / OpenAI / Gemini，则在公共骨架之上只重写 request / response 等少量钩子，公共流程和工具路由能力尽量复用。这个方向与 Rust 更契合的落地方式不是经典继承，而是以 `TransformBackend` trait + 默认 helper / 组合式组件实现模板方法与策略模式的结合。

## Goals / Non-Goals

**Goals:**
- 固化 Anthropic request/response 作为 transformer 内部 canonical model 的架构约束
- 将 transformer 层收敛为 request mapping、upstream request builder、response transformer、shared utility / tool router 四个清晰职责面
- 明确 `anthropic` passthrough 的 identity contract，以及 `codex` / `openai` / `gemini` 的 override contract
- 为代理侧工具拦截预留标准入口，使 `websearch` 一类工具可以在代理内部闭环执行并重新喂回模型
- 保持现有外部 `/v1/messages`、`/count_tokens`、slot 路由、LB/failback 入口不变，让重构聚焦在 transformer 架构内部

**Non-Goals:**
- 本次不要求直接完成所有 transformer 的物理拆文件或一次性改完所有 provider 行为
- 不改变前端 Tauri 配置面板、上游 endpoint 配置格式或负载均衡算法
- 不把所有 provider 的协议差异抽象成一个过度统一的 mega-adapter
- 不在本次设计里承诺新增 websearch 等外部依赖的具体供应商实现，仅定义代理侧拦截契约和边界

## Decisions

### D1: 以 Anthropic 模型作为 transformer 内部唯一 canonical model
**Decision**: 所有进入 transformer 层的请求都视为 Anthropic canonical request，所有离开 transformer 层、回到客户端的流式与非流式响应都必须重新归一为 Anthropic canonical response。provider backend 不拥有新的“内部标准模型”，只能在进入上游前做 request override，在接收上游后做 response override。

**Why**: 当前项目的外部契约就是 Anthropic Messages；以此为内部通用语言，可以避免 `codex` / `openai` / `gemini` 各自维护不同中间结构，降低新 provider 接入与跨 provider 行为审计成本。

**Alternatives considered**:
- 为每个 provider 保留独立中间模型：实现自由度更高，但会持续放大分叉和测试矩阵。
- 单独发明一个自定义 canonical schema：理论更通用，但与现有外部协议脱节，迁移成本更高。

### D2: `TransformBackend` 继续作为统一入口，但内部按四段职责组合实现
**Decision**: 保留 `TransformBackend` 作为 server 调度层唯一认知的入口；在 backend 内部进一步收敛为四段职责：
1. request mapper：把 Anthropic body 映射成 provider request body；
2. upstream request builder：只管 URL、headers、auth、Accept/Content-Type 等 transport concern；
3. response transformer：把 provider 的 SSE/non-stream 输出归一回 Anthropic 事件/响应；
4. shared utilities / tool router：承载跨 backend 可复用的无状态工具与特定工具拦截能力。

**Why**: 这样能保留现有外部稳定接口，同时把“继承 BaseTrans 重写钩子”的意图在 Rust 中改写为“trait + 组合组件 + 默认 helper”。

**Alternatives considered**:
- 新建第二层 super trait 并强制所有 backend 全量重写：形式更统一，但对现有代码入侵太大。
- 继续维持现状，只靠代码约定区分职责：后续仍会回到 provider-specific 分支蔓延的问题。

### D3: `anthropic` backend 定义为 identity baseline，默认只允许最小 override
**Decision**: `AnthropicBackend` 被定义为 identity-style backend：默认 request body 直接序列化透传，response event frame 原样透传，允许的 override 仅限模型覆盖、header 归一、少量协议兼容字段修正。任何新增业务分支不得优先落到 `anthropic` passthrough 主路径。

**Why**: 这能让 `anthropic` 成为“什么都不额外做时系统应该怎样工作”的基线，也便于其他 backend 用它做行为回归参照。

**Alternatives considered**:
- 继续把 `anthropic` 视为普通 provider backend：缺少基线语义，不利于约束其他 provider 的 override 边界。
- 让 `anthropic` 也复用大量共享修正逻辑：会让 passthrough 失去 identity 特性。

### D4: 非 Anthropic backend 只允许在 request/response hook 层表达 provider 差异
**Decision**: `codex`、`openai`、`gemini` 等 backend 的 provider 差异主要落在 request mapper 与 response transformer；upstream request builder 不再承载 system prompt 注入、tool 语义变换、usage 修正等业务逻辑。共享业务能力若跨 provider 复用，应进入 utility 层，而不是继续塞回 transport builder。

**Why**: 用户想要的是“Codex 继承 base，只重写少量钩子”，对应到 Rust 就是 provider-specific 逻辑集中在少量 override 点，而不是每个 backend 都复制整条 pipeline。

**Alternatives considered**:
- 允许 request builder 混入 provider-specific 语义补丁：短期更省事，但会让 transport 与业务边界再次混淆。
- 把 request/response 全部抽成共享 mega-helper：会掩盖 provider 真实差异，不利于精确调试。

### D5: 工具拦截能力采用“router + policy”而不是直接写死在某个 transformer 里
**Decision**: 对 `websearch` / `web_search` 等工具的代理侧接管，不直接耦合到单个 provider transformer，而是定义独立的 Tool Interception Router：
- response transformer 负责识别并导出标准化 tool invocation；
- router 基于工具名和策略判断是否由代理侧接管；
- 被接管时，router 执行外部 API 调用并生成 canonical tool result；
- server / backend 再把 canonical tool result 回注到下一轮 provider request。

**Why**: 工具拦截是跨 provider 的能力，不应该成为 `CodexBackend` 或 `OpenAIBackend` 的专属副作用，否则未来 Gemini 或 Anthropic 也要重复实现同样逻辑。

**Alternatives considered**:
- 只在 Codex transformer 中硬编码 websearch：落地最快，但会快速形成新的 provider lock-in。
- 在 server 层完全不感知工具拦截，只让客户端执行：不满足“代理侧外挂能力”的目标。

### D6: 共享工具类保持无状态，状态机只留在 response transformer 和 orchestration 层
**Decision**: SSE frame 解析、消息块清洗、tool payload 标准化、usage 归一、system text flatten 等共享逻辑应尽量沉为无状态 helper；真正需要保留状态的部分只存在于 response transformer（流式状态机）和 tool orchestration（某个请求内的 tool loop）中。

**Why**: 共享工具一旦持有 provider-specific 状态，就会让不同 backend 难以预测地耦合。无状态 helper 更容易测试和复用。

**Alternatives considered**:
- 把状态机也下沉到公共 utility：复用度���似更高，但很容易让不同 provider 的流式生命周期被错误统一。

## Risks / Trade-offs

- [现有 provider 行为中有些隐式分支会被重新归位，可能引入行为差异] → 先用 spec 固化新契约，再在实现阶段补 request/response 回归测试，尤其覆盖 Anthropic passthrough、Codex request mapping、OpenAI/Gemini SSE 生命周期。
- [工具拦截引入二次模型调用与新的失败路径] → 通过 router policy 限定仅拦截白名单工具，并要求拦截失败时有明确的回退策略（回传原始 tool_use 或显式 tool_result 错误）。
- [为追求统一而过度抽象] → 只统一骨架和职责，不强求每个 provider 的所有 helper 名称、文件拆分完全一致。
- [canonical model 过度绑定 Anthropic，可能压制某些 provider 原生能力] → 允许 provider-specific metadata 通过 context / diagnostics 保留，但对客户端可见的主契约仍以 Anthropic 为准。

## Migration Plan

1. 先补一轮 architecture spec，把 Anthropic canonical model、passthrough baseline、override hook、tool interception 契约写成 requirement。
2. 在实现阶段为 `TransformBackend` 增补更明确的内部组织方式，例如 request mapper / request builder / response transformer / tool router 的模块或类型边界，但不先改 server 外部入口。
3. 先把 `anthropic`、`codex` 对齐到新边界，再逐步把 `openai`、`gemini` 收敛到同一模式。
4. 工具拦截入口先定义为可插拔路由，不默认开启所有工具；待核心 request/response 重构稳定后，再分 provider 接入实际 `websearch` 实现。
5. 若中途某个 backend 出现较大回归，可在不动外部 API 的前提下，回滚该 backend 的内部组织实现。

## Open Questions

1. `TransformBackend` 是否需要新增显式的默认 helper（例如 `transform_non_stream_response`、`export_tool_invocation`），还是先通过组合组件落地，保持 trait 最小化？
2. 代理侧工具拦截是否要在第一阶段直接进入 `server.rs` 主循环，还是先放在某个 backend orchestration helper 中试点？
3. 对于 provider 原生返回但 Anthropic 无法直接表达的附加元数据，最终是走 diagnostics、内部日志还是扩展字段透传？
