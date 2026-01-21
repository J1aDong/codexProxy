# Tasks: Claude 模型 Reasoning Effort 映射

## 1. 后端核心逻辑 (Rust)

- [ ] 1.1 在 `main/src/transform.rs` 中定义 `ReasoningEffortMapping` 结构体
- [ ] 1.2 实现 `get_reasoning_effort()` 函数，根据模型名和映射配置返回对应的 effort 值
- [ ] 1.3 修改 `TransformRequest::transform()` 方法，接收 `ReasoningEffortMapping` 参数
- [ ] 1.4 更新 `reasoning` 字段生成逻辑，使用动态 effort 值替代硬编码的 `"auto"`

## 2. Tauri 配置管理

- [ ] 2.1 在 `fronted-tauri/src-tauri/src/proxy.rs` 中扩展 `ProxyConfig` 结构体，添加 `reasoning_effort_mapping` 字段
- [ ] 2.2 实现 `ReasoningEffortMapping` 的默认值逻辑
- [ ] 2.3 更新 `load_config()` 和 `save_config()` 函数，处理新字段
- [ ] 2.4 修改 `start_proxy()` 命令，将映射配置传递给代理服务器

## 3. 代理服务器集成

- [ ] 3.1 在 `main/src/server.rs` 中更新 `ProxyServer` 结构体，存储 `ReasoningEffortMapping`
- [ ] 3.2 修改 `ProxyServer::new()` 构造函数，接收映射配置
- [ ] 3.3 在请求处理中将映射配置传递给 `TransformRequest::transform()`

## 4. 前端 UI

- [ ] 4.1 在 `fronted-tauri/src/App.vue` 中扩展 `form` 响应式对象，添加 `reasoningEffortMapping` 字段
- [ ] 4.2 添加 reasoning effort 配置 UI 区域（三个下拉选择器）
- [ ] 4.3 添加中英文翻译文本
- [ ] 4.4 更新 `resetDefaults()` 函数，重置 reasoning effort 映射为默认值
- [ ] 4.5 更新 `toggleProxy()` 函数，传递映射配置

## 5. Node.js 版本同步（可选）

- [ ] 5.1 在 `codex-proxy-anthropic.js` 中添加相同的映射逻辑
- [ ] 5.2 支持通过环境变量配置映射

## 6. 测试验证

- [ ] 6.1 验证默认映射行为（opus→xhigh, sonnet→medium, haiku→low）
- [ ] 6.2 验证自定义映射配置生效
- [ ] 6.3 验证配置持久化和恢复
- [ ] 6.4 验证"恢复默认"功能
- [ ] 6.5 验证 UI 中英文显示正确
