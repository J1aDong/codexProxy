## 1. 模块结构创建

- [x] 1.1 创建 `main/src/transform/openai.rs` 文件骨架
- [x] 1.2 在 `main/src/transform/mod.rs` 中添加 `pub mod openai;` 并导出 `OpenAIChatBackend`

## 2. 请求转换实现

- [x] 2.1 实现 `OpenAIChatBackend::transform_request()` 方法
- [x] 2.2 实现 `convert_content_block()` - 将 Codex 格式 content block 转为 OpenAI 格式
- [x] 2.3 实现 `build_messages()` - 构建完整的 OpenAI messages 数组
- [x] 2.4 实现 `convert_tools()` - Anthropic tools → OpenAI tools 转换
- [x] 2.5 实现 `build_system_message()` - 处理 system 消息映射
- [x] 2.6 参数映射：max_tokens, temperature, top_p, stop 等

## 3. HTTP 请求构建

- [x] 3.1 实现 `OpenAIChatBackend::build_upstream_request()` 方法
- [x] 3.2 支持标准 OpenAI 端点 URL 构建
- [x] 3.3 支持自定义 base URL 和 Azure OpenAI 格式

## 4. SSE 响应转换器实现

- [x] 4.1 创建 `OpenAIChatResponseTransformer` 结构体，包含状态管理字段
- [x] 4.2 实现 `ResponseTransformer` trait 的 `transform_line()` 方法
- [x] 4.3 实现 `ensure_message_start()` - 发送 message_start 事件
- [x] 4.4 实现文本内容转换：delta.content → text_delta
- [x] 4.5 实现 tool_calls 状态管理：按 index 累加 arguments
- [x] 4.6 实现 tool_use block 的 start/delta/stop 事件生成
- [x] 4.7 实现 `finish_reason` 映射到 `stop_reason`
- [x] 4.8 实现 reasoning_content → thinking block 转换（可选）
- [x] 4.9 实现 `message_stop` 事件生成

## 5. 服务集成

- [x] 5.1 在 `server.rs` 的 converter 匹配逻辑中添加 `openai` 分支
- [x] 5.2 更新 URL 路由逻辑，支持 `/chat/completions` 端点
- [x] 5.3 确保负载均衡器正确识别 `openai` converter 类型

## 6. 测试与验证

- [x] 6.1 编写单元测试：请求转换正确性
- [x] 6.2 编写单元测试：SSE 响应转换正确性（参考 gemini.rs 测试）
- [x] 6.3 编写单元测试：多 tool_calls 流式累加
- [ ] 6.4 手动测试：连接 OpenAI API 验证完整流程
- [ ] 6.5 手动测试：连接第三方兼容 API（如 DeepSeek）验证

## 7. 文档与配置

- [x] 7.1 更新 CLAUDE.md 添加 OpenAI 转换层说明
- [x] 7.2 确保配置文件支持 `openai` converter 类型配置
