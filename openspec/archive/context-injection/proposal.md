# Change: 上下文注入优化

## Why

当前 `codex-proxy-anthropic.js` 在处理 Claude Code 的 `system` 字段时，简单地包装在 `<system_context>` 标签中，与 Codex CLI 原生的上下文注入格式不一致：

1. **格式不一致**：Codex CLI 使用 `# AGENTS.md instructions for {cwd}` + `<INSTRUCTIONS>` 标签
2. **缺少环境上下文**：Codex CLI 会注入 `<environment_context>` 包含 cwd、sandbox_mode 等信息
3. **消息顺序不正确**：Codex CLI 有特定的 input 消息顺序要求

## What Changes

### 1. AGENTS.md 格式转换
- 将 Claude Code 的 `system` 字段转换为 Codex 的 AGENTS.md 格式
- 使用 `# AGENTS.md instructions for {cwd}` 作为标题
- 包装在 `<INSTRUCTIONS>...</INSTRUCTIONS>` 标签中

### 2. environment_context 注入
- 构造标准的 `<environment_context>` XML 消息
- 包含 cwd、approval_policy、sandbox_mode、network_access、shell 等字段

### 3. 消息顺序规范化
确保 input 消息顺序：
1. TEMPLATE.input[0]（Codex 签名）
2. AGENTS.md 内容
3. environment_context
4. 用户对话消息

## Impact

- **Affected code**: `codex-proxy-anthropic.js` 的 `transformRequest()` 函数
- **No breaking changes**: 向后兼容现有请求

## Risks

1. **格式风险**：需要确保转换后的格式与 Codex 原生格式一致
2. **兼容性风险**：修改可能影响现有的正常请求（需要测试验证）
