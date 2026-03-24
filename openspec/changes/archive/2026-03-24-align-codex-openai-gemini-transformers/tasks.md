## 1. Codex Transformer Alignment

- [x] 1.1 梳理 `main/src/transform/codex/request.rs` 中 request mapping、prompt/`instructions` 组装与 upstream transport 的职责边界
- [x] 1.2 将 Codex 默认提示词/`instructions` 路径改为透传优先，并为显式兼容注入分支补充可验证测试
- [x] 1.3 校准 Codex upstream request builder，只保留 endpoint/header/session 相关 transport concern
- [x] 1.4 验证 Codex response transformer 在 reasoning、tool use、usage 与终态收口上的既有语义未被破坏

## 2. OpenAI Transformer Alignment

- [x] 2.1 将 `main/src/transform/openai.rs` 拆分或分层为 request mapping、upstream builder、response transformer 三个清晰职责面
- [x] 2.2 固化 OpenAI request 侧的 system 合并、tool/tool-result 映射和 tool choice 归一规则，并补充单元测试
- [x] 2.3 校准标准 OpenAI、Azure OpenAI 与兼容 base URL 的 endpoint/header 构建行为
- [x] 2.4 为 OpenAI SSE 增加文本流、usage-only chunk、`[DONE]` 收口和多工具交错的回归测试

## 3. Gemini Transformer Alignment

- [x] 3.1 将 `main/src/transform/gemini.rs` 收敛为与其他非 Anthropic transformer 一致的分层结构
- [x] 3.2 固化 Gemini `systemInstruction`、`contents`、图像/文本 parts 和工具相关映射规则，并补充测试
- [x] 3.3 校准官方 Gemini 与 Gemini CLI 风格接口的 endpoint/header 构建差异
- [x] 3.4 为 Gemini response 归一补充 finish reason、usage 变体和包装结构的回归测试

## 4. Cross-Transformer Verification

- [x] 4.1 运行 `codex`、`openai`、`gemini` 相关测试并修复职责重构引入的回归
- [x] 4.2 验证 `anthropic` passthrough 链路与外部 converter 路由入口未被本次重构改变
- [x] 4.3 复核非 Anthropic transformer 的共享预处理边界，确保未引入新的跨 transformer 隐式耦合
