# API 格式参考文档

本文档记录 **Anthropic Messages API** 和 **OpenAI Codex CLI Responses API** 的传输格式，供 `codex-proxy-anthropic.js` 转换使用。

---

## 目录

1. [Anthropic Messages API](#1-anthropic-messages-api)
2. [Codex CLI Responses API](#2-codex-cli-responses-api)
3. [格式映射对照表](#3-格式映射对照表)
4. [Streaming 事件映射](#4-streaming-事件映射)

---

## 1. Anthropic Messages API

> 官方文档: [platform.claude.com/docs/en/api/messages](https://platform.claude.com/docs/en/api/messages)

### 1.1 请求格式

```json
{
  "model": "claude-sonnet-4-5",
  "max_tokens": 1024,
  "system": "You are a helpful assistant.",
  "messages": [
    {
      "role": "user",
      "content": "Hello"
    },
    {
      "role": "assistant",
      "content": "Hi there!"
    },
    {
      "role": "user",
      "content": [
        { "type": "text", "text": "What's in this image?" },
        {
          "type": "image",
          "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "base64_encoded_data..."
          }
        }
      ]
    }
  ],
  "tools": [
    {
      "name": "get_weather",
      "description": "Get the current weather",
      "input_schema": {
        "type": "object",
        "properties": {
          "location": { "type": "string" }
        },
        "required": ["location"]
      }
    }
  ],
  "tool_choice": { "type": "auto" },
  "stream": true
}
```

### 1.2 消息内容类型 (Content Blocks)

| 类型 | 结构 | 说明 |
|------|------|------|
| `text` | `{ "type": "text", "text": "..." }` | 文本内容 |
| `image` | `{ "type": "image", "source": {...} }` | 图片 (base64/url) |
| `tool_use` | `{ "type": "tool_use", "id": "...", "name": "...", "input": {...} }` | 工具调用 (模型输出) |
| `tool_result` | `{ "type": "tool_result", "tool_use_id": "...", "content": "..." }` | 工具结果 (用户输入) |

### 1.3 工具定义格式

```json
{
  "name": "function_name",
  "description": "Function description",
  "input_schema": {
    "type": "object",
    "properties": {
      "param1": { "type": "string", "description": "..." }
    },
    "required": ["param1"]
  }
}
```

### 1.4 Streaming 事件类型

> 官方文档: [platform.claude.com/docs/en/api/messages-streaming](https://platform.claude.com/docs/en/api/messages-streaming)

**事件流程:**
```
message_start → content_block_start → content_block_delta* → content_block_stop → message_delta → message_stop
```

#### 事件详解

| 事件名 | 数据结构 | 说明 |
|--------|----------|------|
| `message_start` | `{ "type": "message_start", "message": {...} }` | 消息开始，包含空 content |
| `content_block_start` | `{ "type": "content_block_start", "index": 0, "content_block": {...} }` | 内容块开始 |
| `content_block_delta` | `{ "type": "content_block_delta", "index": 0, "delta": {...} }` | 内容增量 |
| `content_block_stop` | `{ "type": "content_block_stop", "index": 0 }` | 内容块结束 |
| `message_delta` | `{ "type": "message_delta", "delta": {...}, "usage": {...} }` | 消息增量 (stop_reason) |
| `message_stop` | `{ "type": "message_stop" }` | 消息结束 |
| `ping` | `{ "type": "ping" }` | 心跳 |
| `error` | `{ "type": "error", "error": {...} }` | 错误 |

#### Delta 类型

```json
// 文本增量
{ "type": "text_delta", "text": "Hello" }

// 工具调用参数增量 (partial JSON string)
{ "type": "input_json_delta", "partial_json": "{\"location\":" }

// 思考增量 (extended thinking)
{ "type": "thinking_delta", "thinking": "Let me think..." }

// 签名增量 (thinking 结束前)
{ "type": "signature_delta", "signature": "EqQBCgIYAhIM..." }
```

#### 完整 Streaming 示例

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_01...","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-5","stop_reason":null,"usage":{"input_tokens":25,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}

event: message_stop
data: {"type":"message_stop"}
```

#### 工具调用 Streaming 示例

```
event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01...","name":"get_weather","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"location\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"San Francisco\""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":89}}

event: message_stop
data: {"type":"message_stop"}
```

### 1.5 Stop Reason 值

| 值 | 说明 |
|----|------|
| `end_turn` | 正常结束 |
| `tool_use` | 需要执行工具调用 |
| `max_tokens` | 达到 token 上限 |
| `stop_sequence` | 遇到停止序列 |

---

## 2. Codex CLI Responses API

> 基于 OpenAI Responses API，专为 Codex CLI 设计
> GitHub: [github.com/openai/codex](https://github.com/openai/codex)

### 2.1 请求格式

```json
{
  "model": "gpt-5.2-codex",
  "instructions": "System prompt here...",
  "input": [
    {
      "type": "message",
      "role": "user",
      "content": [
        { "type": "input_text", "text": "Hello" }
      ]
    },
    {
      "type": "message",
      "role": "assistant",
      "content": [
        { "type": "output_text", "text": "Hi there!" }
      ]
    },
    {
      "type": "function_call_output",
      "call_id": "call_xxx",
      "output": "Tool result here"
    }
  ],
  "tools": [
    {
      "type": "function",
      "name": "shell_command",
      "description": "Run a shell command",
      "strict": false,
      "parameters": {
        "type": "object",
        "properties": {
          "command": { "type": "string" }
        },
        "required": ["command"]
      }
    },
    {
      "type": "custom",
      "name": "apply_patch",
      "description": "Apply a patch to files",
      "format": {
        "type": "grammar",
        "syntax": "lark",
        "definition": "..."
      }
    }
  ],
  "tool_choice": "auto",
  "parallel_tool_calls": true,
  "reasoning": {
    "effort": "medium",
    "summary": "auto"
  },
  "store": false,
  "stream": true,
  "include": ["reasoning.encrypted_content"],
  "prompt_cache_key": "uuid-here"
}
```

### 2.2 Input 项类型

| 类型 | 结构 | 说明 |
|------|------|------|
| `message` | `{ "type": "message", "role": "user/assistant", "content": [...] }` | 消息 |
| `function_call_output` | `{ "type": "function_call_output", "call_id": "...", "output": "..." }` | 工具调用结果 |

### 2.3 Content Block 类型

| 类型 | 说明 | 使用场景 |
|------|------|----------|
| `input_text` | 用户输入文本 | role=user |
| `output_text` | 模型输出文本 | role=assistant |
| `input_image` | 用户输入图片 | role=user |

```json
// 文本
{ "type": "input_text", "text": "Hello" }
{ "type": "output_text", "text": "Response" }

// 图片
{ "type": "input_image", "image_url": "data:image/png;base64,..." }
```

### 2.4 工具定义格式

```json
// 函数类型工具
{
  "type": "function",
  "name": "shell_command",
  "description": "Run a shell command",
  "strict": false,
  "parameters": {
    "type": "object",
    "properties": {...},
    "required": [...]
  }
}

// 自定义类型工具 (如 apply_patch)
{
  "type": "custom",
  "name": "apply_patch",
  "description": "...",
  "format": {
    "type": "grammar",
    "syntax": "lark",
    "definition": "..."
  }
}
```

### 2.5 Streaming 事件类型

**事件流程:**
```
response.output_item.added → response.output_text.delta* / response.function_call_arguments.delta* → response.output_item.done → response.completed
```

#### 事件详解

| 事件名 | 数据结构 | 说明 |
|--------|----------|------|
| `response.output_item.added` | `{ "type": "...", "item": {...} }` | 输出项开始 |
| `response.output_text.delta` | `{ "type": "...", "delta": "..." }` | 文本增量 |
| `response.function_call_arguments.delta` | `{ "type": "...", "delta": "..." }` | 函数参数增量 |
| `response.output_item.done` | `{ "type": "...", "item": {...} }` | 输出项完成 |
| `response.completed` | `{ "type": "...", "response": {...} }` | 响应完成 |

#### 函数调用事件

```json
// 函数调用开始
{
  "type": "response.output_item.added",
  "item": {
    "type": "function_call",
    "call_id": "call_xxx",
    "name": "shell_command"
  }
}

// 参数增量
{
  "type": "response.function_call_arguments.delta",
  "delta": "{\"command\":"
}

// 函数调用完成
{
  "type": "response.output_item.done",
  "item": {
    "type": "function_call",
    "call_id": "call_xxx",
    "name": "shell_command",
    "arguments": "{\"command\":\"ls -la\"}"
  }
}
```

#### 响应完成事件

```json
{
  "type": "response.completed",
  "response": {
    "id": "resp_xxx",
    "status": "completed",
    "usage": {
      "input_tokens": 100,
      "output_tokens": 50
    }
  }
}
```

---

## 3. 格式映射对照表

### 3.1 请求字段映射

| Anthropic | Codex | 说明 |
|-----------|-------|------|
| `system` | `instructions` | 系统提示词 |
| `messages` | `input` | 对话历史 |
| `tools` | `tools` | 工具列表 (格式不同) |
| `tool_choice` | `tool_choice` | 工具选择策略 |
| `stream` | `stream` | 是否流式 |
| `max_tokens` | - | Codex 无此字段 |
| - | `reasoning` | Codex 特有推理配置 |
| - | `include` | Codex 特有包含项 |
| - | `prompt_cache_key` | Codex 特有缓存键 |

### 3.2 消息角色映射

| Anthropic | Codex |
|-----------|-------|
| `role: "user"` | `role: "user"`, content 用 `input_text` |
| `role: "assistant"` | `role: "assistant"`, content 用 `output_text` |
| `role: "system"` | 合并到 `instructions` |

### 3.3 内容类型映射

| Anthropic | Codex |
|-----------|-------|
| `{ "type": "text", "text": "..." }` | `{ "type": "input_text/output_text", "text": "..." }` |
| `{ "type": "image", "source": {...} }` | `{ "type": "input_image", "image_url": "..." }` |
| `{ "type": "tool_result", "tool_use_id": "...", "content": "..." }` | `{ "type": "function_call_output", "call_id": "...", "output": "..." }` |

### 3.4 工具格式映射

| Anthropic | Codex |
|-----------|-------|
| `{ "name": "...", "input_schema": {...} }` | `{ "type": "function", "name": "...", "parameters": {...} }` |

---

## 4. Streaming 事件映射

### 4.1 Codex → Anthropic 事件映射

| Codex 事件 | Anthropic 事件 |
|------------|----------------|
| (初始化) | `message_start` |
| `response.output_item.added` (text) | `content_block_start` (type: text) |
| `response.output_text.delta` | `content_block_delta` (type: text_delta) |
| `response.output_item.added` (function_call) | `content_block_start` (type: tool_use) |
| `response.function_call_arguments.delta` | `content_block_delta` (type: input_json_delta) |
| `response.output_item.done` | `content_block_stop` |
| `response.completed` | `message_delta` + `message_stop` |

### 4.2 Stop Reason 映射

| Codex | Anthropic |
|-------|-----------|
| 有 function_call | `tool_use` |
| 正常完成 | `end_turn` |

---

## 5. 参考来源

- [Anthropic Messages API](https://platform.claude.com/docs/en/api/messages)
- [Anthropic Streaming](https://platform.claude.com/docs/en/api/messages-streaming)
- [OpenAI Codex CLI GitHub](https://github.com/openai/codex)
- [OpenAI Responses API](https://platform.openai.com/docs/api-reference/responses)
- [DataCamp - OpenAI Responses API Guide](https://datacamp.com)

---

## 6. 本地模板参考

完整的 Codex 请求模板见: `codex-request.json`
