## Why

当前仓库的非 Anthropic 转换层已经分化成三套风格：`codex` 拆成 backend/request/response 但请求增强逻辑过重，`openai` 基本集中在单文件里，`gemini` 仍是单模块直写。这样一来，很难像参考项目 `/Users/mr.j/myRoom/code/ai/proxy/claude-code-hub` 一样按“请求语义 / 上游请求构建 / 响应转换”去逐层对齐和验证，尤其 Codex 默认提示词与 `instructions` 的处理路径现在仍带有隐式注入行为，和参考项目“请求体基本透传、只做少量 rectifier / override”的方向不一致。

现在需要把除 `anthropic` 之外的 `codex`、`openai`、`gemini` 三条转换链统一按参考项目思路重构，并允许做破坏性调整；重点是把 Codex 的默认提示词/`instructions` 策略梳理成可审计、可测试、尽量透传优先的行为，而不是继续依赖隐藏的静态注入分支。

## What Changes

- 重构 `codex`、`openai`、`gemini` 三个非 Anthropic 转换器的内部职责边界，对齐参考项目的“请求映射 / 上游请求构建 / 响应转换”分层
- **BREAKING** 调整 Codex 请求增强策略：重点审计并收敛默认提示词与 `instructions` 的隐式注入，优先对齐参考项目的透传语义
- 将 OpenAI Chat 与 Gemini 转换器从当前偏单体实现收敛为可独立测试的 request mapper、upstream builder、response transformer 三层
- 收敛三条非 Anthropic 链路中与协议转换无关的兼容分支，明确哪些属于消息预处理、哪些属于上游适配、哪些属于流式状态机
- 为 Codex `instructions`、OpenAI Chat 消息映射、Gemini `contents/systemInstruction` 映射以及三者的流式/非流式行为补齐回归测试矩阵
- 保持 `anthropic` 透传链路与外部 converter 路由入口不变，不新增新的 converter 类型或新 API 入口

## Capabilities

### New Capabilities
- `codex-responses-transformer-alignment`: 定义 Codex Responses 请求增强、`instructions` 传递、上游请求构建与响应转换的对齐要求
- `openai-chat-transformer-alignment`: 定义 Anthropic ↔ OpenAI Chat Completion 转换链路在请求映射、上游适配和 SSE 生命周期上的对齐要求
- `gemini-transformer-alignment`: 定义 Anthropic ↔ Gemini 转换链路在 `contents`/`systemInstruction` 映射、上游适配和响应归一上的对齐要求

### Modified Capabilities

## Impact

- 重点影响代码：`main/src/transform/codex/backend.rs`, `main/src/transform/codex/request.rs`, `main/src/transform/codex/response.rs`, `main/src/transform/openai.rs`, `main/src/transform/gemini.rs`, `main/src/transform/mod.rs`，以及相关测试文件
- 重点影响行为：Codex `instructions`/默认提示词策略、OpenAI Chat request/SSE 映射、Gemini request/response 映射、上游 endpoint 与 header 构建
- 允许破坏性调整非 Anthropic transformer 的内部行为契约，但不改变外部 API 路由入口、converter 名称或 `anthropic` passthrough 行为
- 不引入新的外部依赖，优先复用现有 `serde_json`、`reqwest`、测试工具链与必要的共享消息预处理逻辑
