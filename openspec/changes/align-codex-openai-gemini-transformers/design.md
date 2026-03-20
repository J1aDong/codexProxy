## Context

当前仓库的 transformer 组织方式并不统一：
- `main/src/transform/codex/backend.rs` 已经把入口拆到 backend/request/response，但 `main/src/transform/codex/request.rs` 仍承载大量请求增强、prompt 注入、tool 映射与兼容逻辑；
- `main/src/transform/openai.rs` 仍是集中式实现，同时承担 request mapping、endpoint/header 构建和 SSE 状态机；
- `main/src/transform/gemini.rs` 也是单模块结构，缺少和 Codex/OpenAI 同层级的职责切面；
- `main/src/transform/anthropic.rs` 则是相对简单且稳定的透传实现，本次不应触碰。

参考项目 `/Users/mr.j/myRoom/code/ai/proxy/claude-code-hub` 的方向更接近“请求体尽量透传，只在必要处做 override/rectifier，再由独立 forwarder 处理 endpoint 与 headers”。从现有证据看，参考项目对 Codex `instructions` 的旧策略字段已经废弃，运行时默认不再额外读取该配置；对 `/v1/responses` 主要只做 `input` rectifier 和 provider override。而当前仓库 Codex 路径仍会在特定条件下向 body 注入静态 `CODEX_INSTRUCTIONS`（`main/src/transform/codex/request.rs:1953`），这正是本次要重点对齐和审计的差异。

用户已明确接受这是新分支上的破坏性重构，因此本次设计可以优先追求职责清晰、行为可测和与参考项目语义对齐，而不是保守维持所有历史内部约定。

## Goals / Non-Goals

**Goals:**
- 将 `codex`、`openai`、`gemini` 三条非 Anthropic 转换链统一收敛为清晰的三层职责：request mapping / upstream request builder / response transformer
- 对齐参考项目的核心方向：请求体透传优先、显式 override/rectifier、有边界的上游适配，而不是在主路径里混入隐藏默认行为
- 重构 Codex 请求增强路径，重点梳理 `instructions` 与默认提示词策略，使其可审计、可测试、可显式控制
- 为 OpenAI Chat 与 Gemini 转换器建立和 Codex 同等级别的内部边界与回归测试矩阵
- 保持 `anthropic` 透传链路和外部 transformer 路由入口不变

**Non-Goals:**
- 不重构 `anthropic` passthrough 的协议行为或其 SSE 透传方式
- 不新增新的 converter 类型、路由入口、配置页面或上游服务种类
- 不把所有 transformer 强行合并成一个共享超抽象层
- 不顺手改造负载均衡、鉴权系统、MCP 透传、前端配置结构
- 不要求完全照搬参考项目的文件结构，只要求职责边界和关键语义对齐

## Decisions

### D1: 非 Anthropic transformer 统一收敛为“三层职责，单入口后端”模式
**Decision**: `CodexBackend`、`OpenAIChatBackend`、`GeminiBackend` 继续保留为 `TransformBackend` 的唯一入口，但内部实现统一按三层组织：
1. request mapping：只负责把 Anthropic 风格输入映射到目标上游 body；
2. upstream request builder：只负责 endpoint、headers、鉴权与 Accept/Content-Type；
3. response transformer：只负责把上游返回（尤其流式事件）归一成 Anthropic 输出。

**Why**: 这和参考项目中“请求语义”和“转发语义”分离的方向一致，也能让 OpenAI/Gemini 与 Codex 的实现边界统一，便于逐条对齐与测试。

**Alternatives considered**:
- 继续允许三种 transformer 各自保持不同风格：改动小，但后续维护成本持续偏高。
- 做一个所有 transformer 共享的大一统抽象层：理论更整齐，但本次 blast radius 过大。

### D2: Codex 默认走“透传优先”，移除主路径上的隐藏静态 instructions 注入
**Decision**: Codex 请求构建默认以显式请求上下文为准，优先透传调用方已经给出的 system / instructions 语义；当前基于 `static_instructions_applied` 的静态 `CODEX_INSTRUCTIONS` 主路径注入不再作为默认行为，而应被显式化、可测试化，必要时仅作为明确兼容分支保留。

**Why**: 用户已明确指出参考项目“压根就不传 codex 默认提示词”，而当前仓库的静态注入会让行为难以解释，也与参考项目“只做小型 rectifier / override”的思路不一致。

**Alternatives considered**:
- 维持当前静态注入逻辑不变：最保守，但无法解决本次最核心的对齐问题。
- 完全删除所有 Codex 请求增强：风险过高，可能误伤计划模式、工具状态与已有兼容逻辑。

### D3: 保留 MessageProcessor 作为薄预处理层，但不把跨 transformer 抽象继续上提
**Decision**: `MessageProcessor` 仍可用于图像、tool state、少量历史兼容等共享预处理，但每个 transformer 的 request mapping 仍独立拥有自己的目标协议映射规则。

**Why**: 这样既能避免重复实现底层消息清洗，又不会把 Codex/OpenAI/Gemini 的协议差异强行混成一个共享映射层。

**Alternatives considered**:
- 完全禁止共享预处理：会制造不必要的重复逻辑。
- 把 request mapping 全部抽成共享中间表示层：过度设计，而且不利于保留各上游的差异语义。

### D4: OpenAI Chat 与 Gemini 参照 Codex 的层次化结构重组，但不要求完全相同的文件切分
**Decision**: OpenAI 与 Gemini 都要达到“请求映射 / 上游构建 / 响应转换可独立阅读和单测”的效果；实现上可拆文件，也可先在单文件内完成明确分层，只要最终边界稳定、测试可挂靠。

**Why**: 本次目标是职责对齐和回归可测，不是为了机械追求文件数一致。

**Alternatives considered**:
- 强制每个 transformer 都拆成 backend/request/response 三个物理文件：一致性最好，但不一定是最小改动。
- 完全保留 OpenAI/Gemini 单文件：无法真正解决职责耦合问题。

### D5: 上游 request builder 不再重新解释业务语义，只处理 transport concern
**Decision**: 各 transformer 的 upstream builder 只负责 URL 解析、header 选择、API key/Bearer 差异、stream/non-stream Accept 等传输层问题，不再混入 system 拼接、tool choice 归一、默认提示词注入等业务语义。

**Why**: 这是参考项目 forwarder 设计最有价值的地方：上游适配层只做 transport，不再承担消息内容策略。

**Alternatives considered**:
- 继续把语义和 transport 混写：短期省事，但很难定位问题来源。

### D6: 响应转换以“保留现有已验证能力 + 向参考项目语义对齐”为准
**Decision**: OpenAI/Gemini/Codex 的 response transformer 都以稳定生命周期和可测试状态机为目标；对齐参考项目的核心语义，但不回退当前仓库已经支持且更强的兼容路径，除非它们正是本次决定要移除的隐藏行为。

**Why**: 用户要的是“参考这个项目重构”，不是“把当前能力机械降级到和参考项目完全一样”。

**Alternatives considered**:
- 逐行仿写参考项目：容易丢失当前仓库已验证的兼容收益。
- 完全忽略参考项目：无法满足本次 change 的目标。

## Risks / Trade-offs

- [Codex 默认提示词策略变化会带来行为差异] → 通过 spec 明确“透传优先”的新契约，并为有/无 system、有/无 custom prompt、plan mode 等场景补测试。
- [三条链同时重构，回归面较大] → 先按 transformer 分层锁住 request/response 行为，再分别补 request builder 和流式回归测试。
- [OpenAI/Gemini 为了追求统一而过度抽象] → 仅统一职责边界，不强制统一所有内部 helper。
- [共享预处理与各家协议映射边界再次混淆] → 明确 MessageProcessor 只做薄预处理，协议语义仍归各自 request mapper 所有。
- [破坏性调整影响已有隐式依赖] → 由于用户已在独立分支上授权，可优先收敛到清晰契约；若确有必要，再以显式兼容开关或独立回归测试说明。

## Migration Plan

1. 先为 `codex`、`openai`、`gemini` 三条链分别补齐当前行为测试，特别是 Codex `instructions`、OpenAI SSE、Gemini request/response 映射。
2. 在保持外部 `TransformBackend` 入口不变的前提下，逐条将三者内部逻辑拆成 request mapping / upstream builder / response transformer。
3. 将 Codex 默认提示词与 `instructions` 逻辑调整为透传优先，并通过测试锁定新的破坏性契约。
4. 对 OpenAI 与 Gemini 完成同层级重构后，运行 transformer 相关测试矩阵，确认 `anthropic` 路径未受影响。
5. 如果某条链出现不可接受回归，可单独回滚对应 transformer 内部实现，因为外部入口和 converter 名称保持不变。

## Open Questions

1. Codex 兼容分支里，是否还需要保留一个“显式开启时才注入官方 instructions”的后门，还是直接彻底去掉默认静态注入？默认建议：先去掉主路径默认注入，只在确有兼容性证据时再显式恢复。
2. Gemini 是否需要补一个和 OpenAI/Codex 等价的非流式 response normalizer 边界，还是维持当前以流式优先的实现即可？默认建议：两者都补，避免再次出现单模块混写。
3. OpenAI/Gemini 是否最终物理拆文件，还是仅在单文件内完成职责分层？默认建议：先以最小可读边界为准，文件拆分由实现复杂度决定。
