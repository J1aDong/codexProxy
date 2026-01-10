# Change: 修复 Skill 工具调用和 CLAUDE.md 指令传递

## Why

当前 `codex-proxy-anthropic.js` 在将 Claude Code 请求转换为 Codex Responses API 时存在两个关键问题：

1. **Skill 工具无法正常工作**：Claude Code 的 `Skill` 工具是一个特殊工具，用于读取和执行 `SKILL.md` 文件中的指令。当前 proxy 只是简单地将工具定义转换格式，但 Codex CLI 后端没有原生的 Skill 概念，导致 Skill 调用失败或被忽略。

2. **CLAUDE.md/AGENTS.md 指令传递不完整**：Claude Code 通过 `system` 字段传递用户的自定义指令（来自 CLAUDE.md），但当前 proxy 的处理方式可能导致这些指令被截断或格式错误。

## What Changes

### 1. Skill 工具处理增强
- 识别 `Skill` 工具调用，将其转换为等效的文件读取操作
- 在 proxy 层面模拟 Skill 行为：读取 SKILL.md 文件内容并注入到上下文
- 或者：将 Skill 工具调用转换为 `shell_command` 调用（`cat SKILL.md`）

### 2. Instructions 传递优化
- 优化 `system` 字段到 `input` 的注入方式
- 确保 CLAUDE.md 内容完整传递且格式正确
- 保持与 Codex 后端的 `instructions` 校验兼容

### 3. 工具转换增强
- 特殊处理 Claude Code 特有的工具类型
- 添加工具调用结果的格式转换

## Impact

- **Affected specs**: protocol-conversion (新建)
- **Affected code**:
  - `codex-proxy-anthropic.js` - 主要修改
  - 可能需要新增 skill 处理模块

## Technical Analysis

### 🔍 Codex CLI Skill 机制分析（基于实际日志）

通过分析 Codex CLI 的实际 API 调用日志，发现 **Skill 不是工具调用，而是上下文注入机制**：

#### Codex CLI 的 Skill 实现方式

1. **Skill 列表在 AGENTS.md 中声明**：
   ```
   ## Skills
   A skill is a set of local instructions to follow that is stored in a `SKILL.md` file.
   ### Available skills
   - create-plan: Create a concise plan. (file: /Users/mr.j/.codex/skills/create-plan/SKILL.md)
   - pdf-text-to-markdown: Extract plain text from PDFs... (file: ...)
   ```

2. **用户触发 Skill**：使用 `$skill-name` 或直接 `skill-name`

3. **Codex CLI 客户端处理**：
   - 识别用户消息中的 skill 触发
   - 读取对应的 SKILL.md 文件
   - 将内容包装在 `<skill>` 标签中
   - 作为额外的 user message 注入到 input 数组

#### 实际请求结构

```json
{
  "model": "gpt-5.2-codex",
  "instructions": "You are Codex, based on GPT-5...",
  "input": [
    {
      "type": "message",
      "role": "user",
      "content": [{ "type": "input_text", "text": "# AGENTS.md instructions...\n## Skills\n..." }]
    },
    {
      "type": "message",
      "role": "user",
      "content": [{ "type": "input_text", "text": "<environment_context>\n  <cwd>/path/to/project</cwd>\n  <approval_policy>on-request</approval_policy>\n  ...</environment_context>" }]
    },
    {
      "type": "message",
      "role": "user",
      "content": [{ "type": "input_text", "text": "$create-plan 告诉我怎么用claude code来调用codex api" }]
    },
    {
      "type": "message",
      "role": "user",
      "content": [{ "type": "input_text", "text": "<skill>\n<name>create-plan</name>\n<path>/Users/mr.j/.codex/skills/create-plan/SKILL.md</path>\n---\nname: create-plan\ndescription: Create a concise plan...\n---\n\n# Create Plan\n\n## Goal\n..." }]
    }
  ],
  "tools": [...]
}
```

### 当前 Proxy 实现分析

**codex-proxy-anthropic.js 关键代码：**

```javascript
// 第 345-355 行：system 字段处理
if (anthropicBody.system) {
  console.log("📝 Injecting Claude system context (" + anthropicBody.system.length + " chars)");
  finalInput.push({
    type: "message",
    role: "user",
    content: [{
      type: "input_text",
      text: `<system_context>\n${anthropicBody.system}\n</system_context>`
    }]
  });
}
```

**问题：**
1. `system` 内容被包装在 `<system_context>` 标签中，与 Codex 原生格式不一致
2. Claude Code 的 Skill 工具定义被原样转换，但 Codex 后端不理解 Skill 语义
3. 缺少 `<environment_context>` 注入
4. 缺少 Skill 内容的 `<skill>` 标签包装

### 解决方案

**方案 A：Proxy 层 Skill 模拟**（原方案，已废弃）
- ~~在 proxy 中拦截 Skill 工具调用~~
- ~~读取对应的 SKILL.md 文件~~
- ~~将内容作为 function_call_output 返回~~

**方案 B：转换为等效操作**（原方案，已废弃）
- ~~将 Skill 调用转换为 shell_command（cat 文件）~~
- ~~让 Codex 后端执行实际的文件读取~~

**方案 C：上下文注入模拟**（推荐 ✅）

基于 Codex CLI 的实际实现，在 proxy 中模拟相同的上下文注入机制：

1. **CLAUDE.md → AGENTS.md 格式转换**：
   - 将 Claude Code 的 `system` 字段内容转换为 Codex 的 AGENTS.md 格式
   - 使用 `# AGENTS.md instructions for {cwd}` 作为标题
   - 包装在 `<INSTRUCTIONS>...</INSTRUCTIONS>` 标签中

2. **environment_context 注入**：
   - 从请求中提取或构造环境上下文
   - 注入 `<environment_context>` 消息

3. **Skill 工具调用转换**：
   - 当 Claude Code 调用 `Skill` 工具时，proxy 拦截
   - 读取指定的 SKILL.md 文件
   - 将内容包装在 `<skill>` 标签中
   - 作为 user message 注入到 input 数组
   - 返回成功响应给 Claude Code

4. **保持 instructions 字段不变**：
   - Codex 后端校验 `instructions` 必须与模板一致
   - 所有自定义内容通过 `input` 注入

## Risks

1. **兼容性风险**：修改可能影响现有的正常请求
2. **性能风险**：Skill 文件读取可能增加延迟
3. **安全风险**：Skill 文件路径需要验证，防止路径遍历攻击
4. **格式风险**：需要确保转换后的格式与 Codex 原生格式完全一致

## Migration

- 无破坏性变更
- 向后兼容现有请求格式
- 新增 Skill 处理逻辑为可选功能
