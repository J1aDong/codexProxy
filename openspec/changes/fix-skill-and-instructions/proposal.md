# Change: Skill 工具调用处理

## Why

Claude Code 的 `Skill` 工具是一个特殊工具，用于读取和执行 `SKILL.md` 文件中的指令。当前 proxy 只是简单地将工具定义转换格式，但 Codex CLI 后端没有原生的 Skill 概念，导致 Skill 调用失败或被忽略。

**依赖**：此 change 依赖 `context-injection` change 完成后再实施。

## What Changes

### Skill 工具调用转换

基于 Codex CLI 的实际实现，Skill 不是工具调用，而是**上下文注入机制**：

1. **识别 Skill 工具调用**：在 proxy 中拦截 Claude Code 的 `Skill` 工具调用
2. **读取 SKILL.md 文件**：
   - 解析 skill 参数获取文件路径
   - 添加路径安全验证（防止路径遍历）
   - 读取 SKILL.md 文件内容
3. **包装为 `<skill>` 格式**：
   ```xml
   <skill>
   <name>{skill-name}</name>
   <path>{file-path}</path>
   {skill-content}
   </skill>
   ```
4. **注入到 input 数组**：将包装后的内容作为 user message 注入
5. **返回成功响应**：模拟工具调用成功，返回给 Claude Code

### Codex CLI 的 Skill 实际格式

```json
{
  "input": [
    // ... AGENTS.md, environment_context ...
    {
      "type": "message",
      "role": "user",
      "content": [{ "type": "input_text", "text": "$create-plan 告诉我怎么做" }]
    },
    {
      "type": "message",
      "role": "user",
      "content": [{ "type": "input_text", "text": "<skill>\n<name>create-plan</name>\n<path>/path/to/SKILL.md</path>\n---\nname: create-plan\n...\n</skill>" }]
    }
  ]
}
```

## Impact

- **Affected code**: `codex-proxy-anthropic.js`
- **Dependencies**: `context-injection` change

## Risks

1. **性能风险**：Skill 文件读取可能增加延迟
2. **安全风险**：Skill 文件路径需要验证，防止路径遍历攻击
3. **兼容性风险**：需要确保与 Claude Code 的 Skill 工具定义兼容
