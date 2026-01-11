# Proposal: use-litellm-proxy

## 概述

评估使用 LiteLLM 替代当前自定义 `codex-proxy-anthropic.js` 作为 API 转换层的可行性。

## 背景

当前项目 `codex-proxy-anthropic.js` 实现了：
- Anthropic Messages API → Codex Responses API 的格式转换
- SSE 流式响应转换 (Codex → Anthropic)
- 工具调用格式转换
- 图片/文档等多模态内容处理

代码约 720 行，包含复杂的格式映射逻辑。

## LiteLLM 分析

### LiteLLM 是什么

LiteLLM 是一个 Python SDK + 代理服务器，提供：
- 统一的 OpenAI 格式接口调用 100+ LLM 提供商
- 支持 Chat Completions、Embeddings、Image Generation 等端点
- 8ms P95 延迟 @ 1k RPS 的性能

### LiteLLM 能做什么

| 功能 | 支持情况 |
|------|---------|
| OpenAI → Anthropic | ✅ 原生支持 |
| OpenAI → Azure/Bedrock/Vertex | ✅ 原生支持 |
| Anthropic → OpenAI | ✅ 原生支持 |
| 流式响应转换 | ✅ 支持 |
| 工具调用转换 | ✅ 支持 |
| 自定义端点 | ✅ 支持 (custom provider) |

### LiteLLM 不能直接做什么

| 需求 | 支持情况 |
|------|---------|
| Anthropic → **Codex Responses API** | ❌ 不支持 |
| 自定义 SSE 事件格式转换 | ❌ 需要扩展 |
| Codex 特有的 `input[]` 结构 | ❌ 需要自定义 |
| Codex 的 `instructions` + 模板注入 | ❌ 需要自定义 |

## 核心问题

**Codex Responses API 不是标准 API 格式**

LiteLLM 支持的是业界标准 API：
- OpenAI Chat Completions (`/v1/chat/completions`)
- Anthropic Messages (`/v1/messages`)
- Azure OpenAI、Google Vertex AI 等

但 Codex Responses API 是一个**非标准的私有 API**：
- 端点: `/api/codex/backend-api/codex/responses`
- 请求格式: `{ model, instructions, input[], tools[], reasoning{} }`
- 响应格式: 自定义 SSE 事件 (`response.output_text.delta`, `response.function_call_arguments.delta` 等)

## 可行性结论

### ❌ 直接使用 LiteLLM：不可行

LiteLLM 无法直接处理 Codex Responses API，因为：
1. Codex API 格式与任何标准 API 都不兼容
2. 需要自定义的请求/响应转换逻辑
3. 需要注入特定的 `instructions` 模板

### ⚠️ 扩展 LiteLLM：可行但复杂

可以通过编写 LiteLLM 自定义 Provider 来支持 Codex：
1. 创建 `codex_provider.py` 实现 `CustomLLM` 接口
2. 实现请求转换 (`completion()` 方法)
3. 实现响应转换 (SSE 解析)

**但这样做的问题**：
- 需要编写的代码量与当前 JS 实现相当
- 引入 Python 依赖（当前项目是纯 Node.js）
- 增加部署复杂度
- 调试更困难（多一层抽象）

### ✅ 保持当前方案：推荐

当前 `codex-proxy-anthropic.js` 的优势：
1. **单文件、零依赖** - 只需 Node.js 即可运行
2. **针对性优化** - 专门为 Codex API 设计
3. **已经可用** - 功能完整，测试通过
4. **易于维护** - 代码清晰，逻辑集中

## 替代建议

如果想简化代码，可以考虑：

### 方案 A：重构当前代码
- 将转换逻辑拆分为独立模块
- 添加单元测试
- 使用 TypeScript 增加类型安全

### 方案 B：使用 LiteLLM 处理其他场景
- 保留当前 Codex 代理
- 如果需要支持其他 LLM（如真正的 Claude、GPT-4），可以用 LiteLLM 作为补充

### 方案 C：等待 Codex API 标准化
- 如果 aicodemirror 未来提供 OpenAI 兼容端点，可以直接用 LiteLLM

## 决策建议

**保持现状**，原因：
1. LiteLLM 解决的是"多 LLM 统一调用"问题，不是"私有 API 适配"问题
2. 当前实现已经工作良好
3. 引入 LiteLLM 会增加复杂度而非减少

如果代码维护成为问题，建议走**方案 A（重构）**而非引入新依赖。
