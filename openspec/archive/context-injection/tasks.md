# Implementation Tasks

## 1. AGENTS.md 格式转换
- [x] 1.1 修改 `transformRequest()` 中的 system 字段处理
- [x] 1.2 使用 `# AGENTS.md instructions for {cwd}` 标题格式
- [x] 1.3 包装内容在 `<INSTRUCTIONS>...</INSTRUCTIONS>` 标签中

## 2. environment_context 注入
- [x] 2.1 构造 `<environment_context>` XML 结构
- [x] 2.2 包含字段：cwd、approval_policy、sandbox_mode、network_access、shell
- [x] 2.3 作为独立的 user message 注入

## 3. 消息顺序规范化
- [x] 3.1 确保 input 数组顺序：
  1. TEMPLATE.input[0]（Codex 签名）
  2. AGENTS.md 内容
  3. environment_context
  4. 用户对话消息

## 4. 测试验证
- [x] 4.1 测试基本文本对话（单元测试通过）
- [x] 4.2 端到端测试（服务器日志确认上下文注入工作）
- [x] 4.3 请求转换验证通过

## 5. 辅助函数
- [x] 5.1 添加 `extractCwdFromMessages()` 从消息中提取工作目录
