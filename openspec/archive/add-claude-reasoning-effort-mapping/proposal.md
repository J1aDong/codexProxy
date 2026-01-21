# Change: 添加 Claude 模型 reasoning_effort 映射配置

## Why

当前代码中所有 Claude 模型（opus、sonnet、haiku）都被强制转换为 `xhigh` reasoning effort。用户希望能够：
1. 根据模型类型自动映射不同的 reasoning effort 级别（opus→xhigh, sonnet→medium, haiku→low）
2. 在 UI 界面上配置和自定义这些映射关系
3. 持久化保存配置，下次启动时恢复
4. 提供"恢复默认"功能

## What Changes

### 后端 (Rust - main/src/transform.rs)
- **ADDED**: 新增 `ReasoningEffortMapping` 结构体，定义模型到 reasoning effort 的映射
- **MODIFIED**: `TransformRequest::transform()` 方法，根据配置动态设置 reasoning effort
- **ADDED**: 新增配置加载/保存逻辑

### 前端 (Vue - fronted-tauri/src/App.vue)
- **ADDED**: 新增 reasoning effort 映射配置 UI 组件
- **ADDED**: 下拉选择器允许用户自定义每个模型的 reasoning effort 级别
- **MODIFIED**: 配置持久化逻辑，包含 reasoning effort 映射

### Tauri 后端 (fronted-tauri/src-tauri/src/proxy.rs)
- **MODIFIED**: `ProxyConfig` 结构体，新增 `reasoning_effort_mapping` 字段
- **MODIFIED**: 配置加载/保存逻辑

## Impact

- **Affected specs**: 无现有 spec（新功能）
- **Affected code**:
  - `main/src/transform.rs` - 核心转换逻辑
  - `fronted-tauri/src/App.vue` - UI 界面
  - `fronted-tauri/src-tauri/src/proxy.rs` - Tauri 配置管理
  - `codex-proxy-anthropic.js` - Node.js 版本（可选同步）
- **Breaking changes**: 无（向后兼容，默认行为与现有一致）
