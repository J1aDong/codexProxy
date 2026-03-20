## Why

本项目已经有 OpenAI Chat Completion 转换器，但当前实现把请求映射、SSE 状态机、上游 URL/鉴权差异和一些兼容分支集中在 `main/src/transform/openai.rs`，职责边界偏重，后续修补兼容问题时很难和参考实现逐项对齐。现在需要以 `/Users/mr.j/myRoom/code/ai/claude-code-proxy` 为参考，重新梳理 OpenAI Chat Completion 转换链路，让请求转换、响应转换和兼容策略的语义更稳定、更容易验证。

## What Changes

- 重构现有 OpenAI Chat Completion 转换器，按“请求转换 / 响应转换 / 上游请求构建”三个职责重新整理实现边界
- 对齐 `claude-code-proxy` 的核心转换语义：system 消息拼装、assistant/tool 消息映射、tool result 展平、tool choice 映射、finish reason 到 stop reason 的归一规则
- 收敛当前 `openai.rs` 中与转换无关的兼容分支，明确哪些属于协议转换、哪些属于上游适配
- 补齐围绕文本流、tool_calls 增量、usage 尾块、reasoning/refusal 文本的测试矩阵，保证重构前后可验证
- 保持现有 `openai` converter 对外入口不变，不新增新的配置类型或路由入口
- 本次变更只收敛 `openai` converter 内部实现边界，不调整 `codex` 转换层、`anthropic` 透传链路或 `gemini` 转换层

## Capabilities

### New Capabilities
- `openai-chat-transformer-alignment`: 定义本项目 Anthropic ↔ OpenAI Chat Completion 转换链路在请求映射、流式响应映射和上游适配边界上的对齐要求

### Modified Capabilities

## Impact

- 重点影响代码：`main/src/transform/openai.rs`, `main/src/transform/mod.rs`，以及可能承接拆分后的辅助模块/测试文件
- 重点影响行为：OpenAI Chat Completion 请求体生成、SSE 到 Anthropic 事件流转换、Azure/标准 OpenAI 端点构建、tool call/tool result 映射
- 不改变外部 API 入口、converter 名称或现有 slot 路由方式
- 不修改 `codex` 转换层、`anthropic` 透传链路或 `gemini` 转换层的行为边界
- 不引入新的外部依赖，优先复用现有 `serde_json`、`reqwest`、测试工具链
