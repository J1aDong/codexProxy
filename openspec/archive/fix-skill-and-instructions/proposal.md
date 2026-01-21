# Change: Skill 工具调用处理（透传 + 去重注入）

## Why

Claude Code 的 `Skill` 工具是一个特殊工具，用于加载并注入 `SKILL.md` 中的指令。当前 proxy 在消息转换时会跳过 Skill tool_use，导致出现 `function_call_output` 没有对应 `function_call` 的不一致输入；同时 Skill 结果的“加载提示”和“正文内容”混杂，容易造成重复与卡顿。

**依赖**：此 change 依赖 `context-injection` change 完成后再实施。

## What Changes

### Skill 工具调用与结果转换

基于 Claude Code 的实际行为与 docs 研究，采用**透传 + 去重注入**策略：

1. **保留 Skill tool_use**：不再跳过 Skill 工具调用，确保 `function_call` 与 `function_call_output` 成对出现
2. **参数归一化**：兼容 `{command}` 与 `{skill,args}` 两种输入形态，保证 Codex 工具调用一致
3. **从 tool_result 提取内容**：不从磁盘读取 SKILL.md，直接解析 tool_result 中的 `<command-name>` 与正文
4. **包装为 `<skill>` 格式并去重注入**：
   ```xml
   <skill>
   <name>{skill-name}</name>
   <path>{base-path-or-unknown}</path>
   {skill-content}
   </skill>
   ```
5. **最小化重复输出**：对仅包含“loading”的 tool_result 进行抑制或最小化输出，避免冗余与不一致

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

- **Affected code**: `main/src/transform.rs`
- **Dependencies**: `context-injection` change

## Risks

1. **兼容性风险**：Skill 结果格式不稳定时可能无法正确提取（需要回退策略）
2. **行为风险**：过度去重可能导致上下文缺失（需只去重重复内容）
3. **一致性风险**：必须保证 `function_call` 与 `function_call_output` 的配对关系
