# Codex Proxy - OpenCode 中转配置

本项目提供 OpenAI Chat Completions 格式与 Codex Responses API 格式之间的转换，使 OpenCode 能够使用 aicodemirror 的 Codex API。

目前仅支持 aicodemirror。

## 功能特性

✅ **完整功能支持**
- 文本对话流式响应
- 工具调用（shell_command、apply_patch、view_image、MCP 等）
- 图片支持
- 推理强度配置（reasoning_effort）
- 工具结果反馈循环

## 快速开始

### 1. 启动 Anthropic 适配器

```bash
cd /Users/mr.j/myRoom/code/ai/codexProxy
node codex-proxy-anthropic.js
```

### 2. 配置 Claude Code 环境

Claude Code 配置文件路径：`/Users/mr.j/.claude/settings.json`

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://localhost:8889",
    "ANTHROPIC_AUTH_TOKEN": "sk-ant-api03-你的API密钥"
  },
  "forceLoginMethod": "claudeai",
  "permissions": {
    "allow": [],
    "deny": []
  }
}
```

代理服务器默认监听 `http://localhost:8889`

## 完整用例

OpenCode 配置：
```json
{
  "provider": {
    "localproxyCodex": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "本地中转Codex",
      "options": {
        "baseURL": "http://localhost:8889/v1",
        "apiKey": "sk-ant-api03-你的API密钥"
      },
      "models": {
        "gpt-5.2-codex": {
          "name": "codex5.2",
          "limit": {
            "context": 200000,
            "output": 8192
          }
        }
      }
    }
  },
  "model": "localproxyCodex/gpt-5.2-codex"
}
```

### 3. 运行测试

```bash
python3 codex-proxy-test.py
```

测试通过后会显示：
```
================================================================================
Codex Proxy Test Report
================================================================================

✅ PASS - Basic Text Streaming
✅ PASS - Tool: shell_command
✅ PASS - Tool: view_image
✅ PASS - Tool: apply_patch
✅ PASS - Tool: list_mcp_resources
✅ PASS - Tool: update_plan
✅ PASS - Multiple Tools
✅ PASS - Tool Result Feedback

================================================================================
Summary: 8/8 tests passed
================================================================================
```

## API 格式对比

### OpenAI Chat Completions → Codex

| OpenAI 参数 | Codex 参数 | 说明 |
|------------|-----------|------|
| `messages` | `input` | 消息内容 |
| `system` | `instructions` | 系统提示 |
| `tools` | `tools` | 工具定义 |

响应格式：
```json
{"id":"chatcmpl-xxx","object":"chat.completion.chunk",...}
```

### Anthropic Messages → Codex

| Anthropic 参数 | Codex 参数 | 说明 |
|---------------|-----------|------|
| `messages` | `input` | 消息内容 |
| `system` | `instructions` | 系统提示 |
| `tools` | `tools` | 工具定义 |

响应格式：
```json
{"type":"content_block_delta","delta":{"type":"text_delta","text":"..."}}
```

### Anthropic 响应示例

```json
{
  "type": "content_block_delta",
  "delta": { "type": "text_delta", "text": "Hello!" },
  "index": 0
}
```

工具调用：
```json
{
  "type": "content_block_start",
  "content_block": {
    "type": "tool_use",
    "id": "tool_123",
    "name": "shell_command",
    "input": {}
  }
}
```

消息结束：
```json
{
  "type": "message_delta",
  "delta": { "stop_reason": "end_turn" }
}
```

## 支持的工具

| 工具名称 | 类型 | 说明 |
|----------|------|------|
| `shell_command` | function | 执行 Shell 命令 |
| `apply_patch` | custom | 文件编辑（FREEFORM） |
| `view_image` | function | 查看本地图片 |
| `list_mcp_resources` | function | 列出 MCP 资源 |
| `list_mcp_resource_templates` | function | 列出 MCP 资源模板 |
| `read_mcp_resource` | function | 读取 MCP 资源 |
| `update_plan` | function | 更新任务计划 |

## 消息格式

### 输入消息格式

```json
{
  "model": "gpt-5.2-codex",
  "messages": [
    {
      "role": "user",
      "content": "Hello, Codex!"
    }
  ],
  "stream": true
}
```

### 带工具的请求

```json
{
  "model": "gpt-5.2-codex",
  "messages": [
    {
      "role": "user",
      "content": "Run echo hello"
    }
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "shell_command",
        "description": "Runs a shell command",
        "parameters": {
          "type": "object",
          "properties": {
            "command": {
              "type": "string"
            }
          },
          "required": ["command"]
        }
      }
    }
  ],
  "stream": true
}
```

### 工具结果反馈

```json
{
  "messages": [
    {"role": "user", "content": "Run echo hello"},
    {
      "role": "tool",
      "tool_call_id": "call_xxx",
      "content": "hello"
    }
  ]
}
```

## 故障排除

### 问题：工具调用不触发

**原因**：工具定义格式不完整

**解决**：确保使用 Codex 模板中的完整工具定义，或让代理自动使用模板工具

### 问题：图片无法识别

**原因**：图片 URL 格式不支持

**解决**：OpenCode 使用 `file://` 协议时，需要确保代理正确转换

### 问题：推理强度不生效

**原因**：参数名称不匹配

**解决**：使用 `reasoning_effort` 参数（不是 `reasoning_effort`）

### 问题：400 错误 "Instructions are not valid"

**原因**：instructions 格式不正确

**解决**：代理会自动使用 Codex CLI 的完整 instructions 模板

## 背景知识

### OpenAI Chat Completions API

```json
{
  "model": "gpt-4",
  "messages": [...],
  "tools": [...]
}
```

### Codex Responses API

```json
{
  "model": "gpt-5.2-codex",
  "instructions": "You are Codex...",
  "input": [...],
  "tools": [...],
  "reasoning": {...}
}
```

代理服务器负责这两种格式之间的双向转换。

## API 端点

- **代理服务器**: `http://localhost:8889/v1/chat/completions`
- **Codex API**: `https://api.aicodemirror.com/api/codex/backend-api/codex/responses`

## 许可证

本项目仅供学习和研究使用。

## 参考

- [OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat)
- [Codex CLI 源码](https://github.com/openai/codex)
- [aicodemirror](https://aicodemirror.com)
