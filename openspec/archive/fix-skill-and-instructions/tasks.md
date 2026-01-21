# Implementation Tasks

**依赖**: 需要先完成 `context-injection` change

## 1. Skill 工具透传与参数归一
- [x] 1.1 保留 Skill tool_use，输出为 `function_call`
- [x] 1.2 记录 Skill tool_use 的 call_id 用于结果配对
- [x] 1.3 兼容 `{command}` 与 `{skill,args}` 形态，统一解析 skill name

## 2. Skill 结果提取与去重
- [x] 2.1 从 tool_result 中解析 `<command-name>` 与正文内容
- [x] 2.2 将正文包装为 `<skill>` 格式（path 取 Base Path，缺失则为 unknown）
- [x] 2.3 去重注入：同名 skill 在同一次请求中只注入一次
- [x] 2.4 仅包含“loading”的 tool_result 输出最小化或抑制

## 3. 输入一致性保障
- [x] 3.1 确保 `function_call` 与 `function_call_output` 成对出现
- [x] 3.2 Skill 结果不应产生“孤立输出”

## 4. 测试验证
- [x] 4.1 日志验证 Skill 调用与结果配对
- [x] 4.2 验证 `<skill>` 注入格式与去重行为
- [x] 4.3 端到端测试：Claude Code → Proxy → Codex
