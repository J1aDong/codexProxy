# Implementation Tasks

**依赖**: 需要先完成 `context-injection` change

## 1. Skill 工具识别
- [ ] 1.1 在 proxy 中识别 Claude Code 的 `Skill` 工具调用
- [ ] 1.2 解析 skill 参数（skill name、args）

## 2. Skill 文件读取
- [ ] 2.1 根据 skill name 定位 SKILL.md 文件路径
- [ ] 2.2 添加路径安全验证（防止路径遍历攻击）
- [ ] 2.3 读取 SKILL.md 文件内容

## 3. Skill 内容注入
- [ ] 3.1 将 Skill 内容包装为 `<skill>` 格式
  ```xml
  <skill>
  <name>{skill-name}</name>
  <path>{file-path}</path>
  {skill-content}
  </skill>
  ```
- [ ] 3.2 将包装后的内容作为 user message 注入到 input 数组
- [ ] 3.3 返回成功响应给 Claude Code（模拟工具调用成功）

## 4. 测试验证
- [ ] 4.1 测试 Skill 工具调用识别
- [ ] 4.2 测试 Skill 文件读取
- [ ] 4.3 测试 Skill 内容注入格式
- [ ] 4.4 端到端测试：Claude Code → Proxy → Codex
