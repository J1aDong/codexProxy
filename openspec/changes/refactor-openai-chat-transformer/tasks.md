## 1. Request Mapping Refactor

- [x] 1.1 提取仅服务于 `openai` converter 的 request mapping 逻辑，保持 `OpenAIChatBackend` 外部入口不变
- [x] 1.2 固化 `system` 与 `custom_injection_prompt` 的合并顺序，并补充对应单元测试
- [x] 1.3 梳理 assistant/tool/tool-result 映射路径，确保 `tool_call_id` 关联关系稳定
- [x] 1.4 校准 `tools`、`tool_choice`、`parallel_tool_calls` 与采样参数的请求侧映射行为

## 2. Upstream Request Builder Isolation

- [x] 2.1 将 endpoint 构建与 HTTP header 选择收敛到独立的上游请求构建层
- [x] 2.2 明确标准 OpenAI、Azure OpenAI 与兼容 base URL 的 endpoint 拼接规则
- [x] 2.3 校准 Azure `api-key` 与非 Azure `Authorization: Bearer` 的鉴权头行为
- [x] 2.4 为流式与非流式请求补充 Accept / Content-Type 相关测试

## 3. Streaming Response Refactor

- [x] 3.1 将 OpenAI SSE 状态聚合与 Anthropic SSE 事件发射拆分为清晰的内部步骤
- [x] 3.2 保持 `finish_reason`、usage-only chunk 与 `[DONE]` 的终态收口规则一致
- [x] 3.3 校准文本块、thinking 块与 `tool_use` 块的开启/关闭顺序
- [x] 3.4 按 `tool_calls[index]` 聚合多个工具调用增量，并保留 `function_call` 兼容路径
- [x] 3.5 保持 `reasoning_content`、`refusal` 与 `allow_visible_thinking` 的现有兼容语义不回退

## 4. Regression Coverage

- [x] 4.1 为 request mapping 增加 system 合并、tool 映射与参数归一测试
- [x] 4.2 为 upstream request builder 增加标准 OpenAI / Azure / 自定义 base URL 测试
- [x] 4.3 为 streaming response 增加文本流、usage-only chunk、`[DONE]` 收口测试
- [x] 4.4 为多工具交错、文本转 tool、thinking 可见性与 `function_call` 兼容增加回归测试
- [x] 4.5 运行相关测试并修复重构引入的行为回归
- [x] 4.6 验证本次 change 未改变 `codex` 转换层、`anthropic` 透传链路与 `gemini` 转换层的既有行为边界
