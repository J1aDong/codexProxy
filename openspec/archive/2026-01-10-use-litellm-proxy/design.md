# Design: use-litellm-proxy

## 架构对比

### 当前架构（推荐保留）

```
┌─────────────┐     ┌──────────────────────┐     ┌─────────────────┐
│ Claude Code │────▶│ codex-proxy-anthropic│────▶│ Codex API       │
│ (Anthropic) │◀────│ (Node.js, 720行)     │◀────│ (aicodemirror)  │
└─────────────┘     └──────────────────────┘     └─────────────────┘
                           │
                           ▼
                    单文件、零依赖
                    专门针对 Codex 优化
```

### LiteLLM 架构（不推荐）

```
┌─────────────┐     ┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐
│ Claude Code │────▶│ LiteLLM     │────▶│ Custom Provider  │────▶│ Codex API       │
│ (Anthropic) │◀────│ Proxy       │◀────│ (codex_provider) │◀────│ (aicodemirror)  │
└─────────────┘     └─────────────┘     └──────────────────┘     └─────────────────┘
                           │                    │
                           ▼                    ▼
                    Python 依赖            仍需编写转换逻辑
                    额外抽象层             约 500+ 行 Python
```

## 为什么 LiteLLM 不适合

### 1. API 格式不兼容

**LiteLLM 支持的格式**（都是标准格式）：
```python
# OpenAI 格式
{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello"}]
}

# Anthropic 格式
{
    "model": "claude-3",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 1024
}
```

**Codex Responses API 格式**（私有格式）：
```python
{
    "model": "gpt-5.2-codex",
    "instructions": "You are Codex, an AI coding assistant...",  # 必须的长模板
    "input": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "..."}]}
    ],
    "tools": [...],
    "reasoning": {"effort": "medium", "summary": "auto"},
    "include": ["reasoning.encrypted_content"]
}
```

### 2. SSE 事件格式不兼容

**标准 OpenAI SSE**：
```
data: {"choices":[{"delta":{"content":"Hello"}}]}
```

**Codex SSE**：
```
data: {"type":"response.output_text.delta","delta":"Hello"}
data: {"type":"response.function_call_arguments.delta","delta":"{\"command\":"}
data: {"type":"response.output_item.added","item":{"type":"function_call",...}}
```

### 3. 工具调用格式差异

**OpenAI/Anthropic 工具调用**：
```json
{
    "type": "function",
    "function": {
        "name": "shell_command",
        "arguments": "{\"command\": \"ls\"}"
    }
}
```

**Codex 工具调用**：
```json
{
    "type": "function_call",
    "call_id": "call_xxx",
    "name": "shell_command",
    "arguments": "{\"command\": \"ls\"}"
}
```

## 如果必须用 LiteLLM

需要实现的 Custom Provider 伪代码：

```python
# codex_provider.py
from litellm import CustomLLM
import httpx

class CodexProvider(CustomLLM):
    def completion(self, model, messages, **kwargs):
        # 1. 转换请求格式 (与当前 JS 逻辑相同)
        codex_request = self._transform_request(messages, kwargs)

        # 2. 发送请求
        response = httpx.post(
            "https://api.aicodemirror.com/api/codex/backend-api/codex/responses",
            json=codex_request,
            headers=self._get_headers(kwargs)
        )

        # 3. 转换响应格式 (与当前 JS 逻辑相同)
        return self._transform_response(response)

    def _transform_request(self, messages, kwargs):
        # 需要实现 ~200 行转换逻辑
        pass

    def _transform_response(self, response):
        # 需要实现 ~200 行 SSE 解析逻辑
        pass
```

**结论**：即使用 LiteLLM，核心转换逻辑仍需自己写，反而多了一层抽象。

## 推荐：保持当前方案

当前 `codex-proxy-anthropic.js` 的设计已经是最优解：

1. **单一职责** - 只做 Anthropic ↔ Codex 转换
2. **零依赖** - 只需 Node.js 标准库
3. **高性能** - 直接流式转发，无额外开销
4. **易调试** - 所有逻辑在一个文件中
