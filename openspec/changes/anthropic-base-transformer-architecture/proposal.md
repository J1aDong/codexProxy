## Why

当前项目虽然已经有 `TransformBackend` / `ResponseTransformer` 这层抽象，但不同上游的实现深度不一致：`anthropic` 基本是透传实现，`codex` 已拆成 backend/request/response，`openai` 与 `gemini` 仍有较重的集中式逻辑，导致请求映射、上游 transport、SSE 响应归一和特例工具处理混在一起，扩展新上游或审计行为边界时成本很高。现在需要把“Anthropic 作为规范内部模型、上游按钩子 override”这套架构意图固化成明确契约，为后续继续扩展 Codex / OpenAI / Gemini 甚至代理侧工具拦截提供稳定骨架。

## What Changes

- 将 Anthropic Messages 结构正式定义为 transformer 层的 canonical request/response model，作为所有上游转换的统一内部基准
- 新增基于 `TransformBackend` 的模板方法式约束：把上游转换统一拆成 request mapping、upstream request builder、response transformer 三段职责
- **BREAKING** 收敛非 Anthropic transformer 的内部扩展点，明确哪些行为属于透传、哪些属于 override 钩子、哪些属于共享工具类，避免继续在主流程中混写 provider-specific 分支
- 为 `anthropic` passthrough 明确 identity-style 契约：请求体与 SSE 事件尽量原样透传，只在必要场景做模型覆盖与 header 归一
- 为 `codex`、`openai`、`gemini` 明确 provider override 契约：允许只重写 request/response 某些钩子，而不是重复实现整条转发管线
- 新增代理侧工具拦截能力的规范入口，允许像 `web_search` / `websearch` 这类工具在 transformer 或独立 router 中被特殊接管，并在代理内部完成 tool call -> tool result 闭环
- 抽离共享工具职责边界，明确例如 SSE frame 解析、消息块清洗、工具调用路由、usage 归一等逻辑应沉到无状态 helper / utility，而不是散落在具体 provider backend 中

## Capabilities

### New Capabilities
- `transformer-backend-architecture`: 定义 Anthropic 作为 canonical model 的 transformer 分层契约，以及 passthrough / override / utility 的职责边界
- `proxy-side-tool-interception`: 定义代理侧拦截特定工具调用并在服务端完成 tool execution 闭环的行为契约

### Modified Capabilities

## Impact

- 重点影响代码：`main/src/transform/mod.rs`, `main/src/transform/anthropic.rs`, `main/src/transform/codex/backend.rs`, `main/src/transform/openai.rs`, `main/src/transform/gemini.rs`, `main/src/transform/processor.rs`，以及后续新增的 transformer utility / router 模块
- 重点影响行为：provider request/response override 方式、Anthropic passthrough 边界、上游 request builder 的职责范围、代理侧工具执行与二次补发模型请求的时机
- 不改变外部 `/v1/messages` / `/v1/messages/count_tokens` 入口，也不要求本次直接新增新的上游 provider；重点是先重构内部架构契约
- 不新增前端 UI 入口；若后续需要启停或配置代理侧工具拦截，应在后续 change 单独定义
