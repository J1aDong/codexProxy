## 1. Transformer 契约与 Anthropic baseline

- [x] 1.1 为 transformer 层新增 canonical baseline / backend contract 类型，并补对应单测锁定 Anthropic identity 与其他 provider override 角色
- [x] 1.2 新增 Anthropic identity baseline helper（request mapper / upstream request builder / passthrough response transformer），先写失败测试覆盖模型覆盖、header 归一和 SSE passthrough 语义
- [x] 1.3 将 `AnthropicBackend` 改为复用 baseline helper，保持现有 passthrough 行为不回退

## 2. Provider override boundary 收敛

- [x] 2.1 在 `TransformBackend` / `ResponseTransformer` 上暴露稳定 override 边界与 tool interception 入口，避免 provider-specific 逻辑继续混入主流程
- [x] 2.2 抽取最小共享 helper 骨架，收敛 canonical tool / system 相关的 provider-independent 逻辑
- [x] 2.3 让 `codex` / `openai` / `gemini` 复用新的契约类型与共享 helper，但不做大规模物理拆文件

## 3. Proxy-side tool interception 骨架

- [x] 3.1 新增 provider-independent 的 tool interception 类型、策略接口与 request-scoped router，并先写失败测试覆盖 `web_search` / `websearch` / `WebSearch` 归一化与 passthrough 判定
- [x] 3.2 为响应/编排层预留 normalized tool invocation 与 canonical tool result 入口，不接真实外部集成
- [x] 3.3 将现有 `codex` 的 `web_search` 特例改为复用共享 policy/helper，避免硬编码只停留在单个 provider 实现里

## 4. 回归验证

- [x] 4.1 补充 Anthropic passthrough baseline、backend contract、tool interception policy 的回归测试
- [x] 4.2 运行相关 `cargo test` 子集，确认 `anthropic` / `codex` / `openai` / `gemini` 的现有行为未回退
