# Implementation Tasks

## 1. 分析与设计
- [ ] 1.1 详细分析 Codex CLI Skill 的实际实现机制
- [ ] 1.2 确定 Skill 转换的最终方案（方案 C：上下文注入模拟）
- [ ] 1.3 设计 instructions 注入的优化方案

## 2. 上下文注入转换
- [ ] 2.1 修改 `system` 字段处理，转换为 AGENTS.md 格式
  - 使用 `# AGENTS.md instructions for {cwd}` 标题
  - 包装在 `<INSTRUCTIONS>...</INSTRUCTIONS>` 标签中
- [ ] 2.2 添加 `<environment_context>` 注入
  - 从请求中提取 cwd、approval_policy 等信息
  - 构造标准的 environment_context XML
- [ ] 2.3 确保 input 消息顺序正确：
  1. AGENTS.md 内容
  2. environment_context
  3. 用户消息
  4. Skill 内容（如有）

## 3. Skill 工具处理
- [ ] 3.1 在 proxy 中识别 Claude Code 的 `Skill` 工具调用
- [ ] 3.2 实现 Skill 文件读取逻辑
  - 解析 skill 参数获取文件路径
  - 添加路径安全验证（防止路径遍历）
  - 读取 SKILL.md 文件内容
- [ ] 3.3 将 Skill 内容包装为 `<skill>` 格式
  ```xml
  <skill>
  <name>{skill-name}</name>
  <path>{file-path}</path>
  {skill-content}
  </skill>
  ```
- [ ] 3.4 将包装后的内容作为 user message 注入到 input 数组
- [ ] 3.5 返回成功响应给 Claude Code（模拟工具调用成功）

## 4. Instructions 字段处理
- [ ] 4.1 保持 `instructions` 字段使用 Codex 标准模板
- [ ] 4.2 所有自定义内容通过 `input` 注入
- [ ] 4.3 测试与 Codex 后端的兼容性

## 5. 测试验证
- [ ] 5.1 编写 AGENTS.md 格式转换的测试用例
- [ ] 5.2 编写 environment_context 注入的测试用例
- [ ] 5.3 编写 Skill 调用的测试用例
- [ ] 5.4 端到端测试：Claude Code → Proxy → Codex
- [ ] 5.5 回归测试：确保现有功能不受影响

## 6. 文档更新
- [ ] 6.1 更新 API-FORMAT-REFERENCE.md
- [ ] 6.2 更新 README.md 说明
- [ ] 6.3 添加 Skill 配置说明
