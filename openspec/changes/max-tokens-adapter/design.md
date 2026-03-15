## Context

当 Claude Code 使用 Codex Proxy 调用 OpenAI 兼容 API 时，Anthropic 协议中的 `max_tokens` 参数直接透传到 OpenAI 请求体中。然而：

1. **不同模型有不同的输出 token 限制**：
   - GPT-4o: 最大 4096/16384（取决于版本）
   - GPT-4 Turbo: 最大 4096
   - 某些 OpenAI 兼容 API 可能有更严格的限制（如 8192）

2. **Anthropic Claude 模型的 max_tokens 可以很大**：
   - Claude 3.5 Sonnet: 最大 8192
   - Claude 3 Opus: 可达 64000

3. **当前代码直接透传**：
   ```rust
   if let Some(max_tokens) = anthropic_body.max_tokens {
       obj.insert("max_tokens".to_string(), json!(max_tokens));
   }
   ```
   这会导致当 Anthropic 请求设置较大值时，OpenAI API 返回 400 错误。

4. **用户需求**：在 UI 中为每个 slot（Opus/Sonnet/Haiku）单独配置 `max_tokens`，留空则透传 Claude Code 传入的值。

## Goals / Non-Goals

**Goals:**
- 在 OpenAI Chat 转换器中实现 `max_tokens` 限制逻辑
- 支持按 slot（Opus/Sonnet/Haiku）配置最大输出 token 限制
- 在 UI 的 "OpenAI Chat 模型映射" 区域添加三个 `max_tokens` 输入框
- 配置跟随单个 endpoint 保存，类似模型映射的记住方式

**Non-Goals:**
- 不修改 Anthropic 透传转换器（无需限制）
- 不实现动态查询模型限制（超出当前范围）
- 不修改 count_tokens 端点的行为
- 不修改 Gemini 转换器（当前无此需求）

## Decisions

### 1. 限制策略：配置优先，留空透传

**决定**：当 slot 配置了 `max_tokens` 值时，使用该值作为上限；留空（None）则透传 Claude Code 传入的值。

**理由**：
- 用户明确配置时才生效，避免意外行为
- 留空时保持现有行为，向后兼容
- 符合用户期望：配置了就限制，没配置就透传

**逻辑**：
```rust
// 伪代码
let effective_max_tokens = if let Some(configured_limit) = slot_configured_limit {
    // 配置了限制：取 min(请求值, 配置值)
    request_max_tokens.min(configured_limit)
} else {
    // 未配置：透传请求值
    request_max_tokens
};
```

### 2. 配置结构：按 slot 映射

**决定**：添加 `OpenAIMaxTokensMapping` 结构，与 `OpenAIModelMapping` 对应。

```rust
pub struct OpenAIMaxTokensMapping {
    pub opus: Option<u32>,   // None 表示透传
    pub sonnet: Option<u32>, // None 表示透传
    pub haiku: Option<u32>,  // None 表示透传
}
```

前端配置类型：
```typescript
export interface OpenAIMaxTokensMapping {
    opus: number | null    // null 表示透传
    sonnet: number | null
    haiku: number | null
}
```

**理由**：
- 与现有模型映射结构一致，用户易于理解
- 按 slot 配置符合负载均衡的设计
- `Option<u32>` 可以明确表示"未配置/透传"状态

### 3. 配置位置：EndpointOption

**决定**：将 `openai_max_tokens_mapping` 添加到 `EndpointOption`，跟随单个 endpoint 保存。

**理由**：
- 不同 endpoint 可能连接不同的上游 API，需要不同的限制
- 与 `openai_model_mapping` 保持一致
- 支持负载均衡场景下不同 endpoint 的不同配置

### 4. UI 设计

在 "OpenAI Chat 模型映射" 区域下方添加 "Max Tokens 限制" 区域：

```
OpenAI Chat 模型映射
┌─────────────┬─────────────┬─────────────┐
│ Opus        │ Sonnet      │ Haiku       │
│ deepseek-...│ deepseek-...│ deepseek-...│
└─────────────┴─────────────┴─────────────┘

Max Tokens 限制 (留空透传)
┌─────────────┬─────────────┬─────────────┐
│ Opus        │ Sonnet      │ Haiku       │
│ [      ]    │ [      ]    │ [      ]    │
└─────────────┴─────────────┴─────────────┘
```

- 输入框类型：number，min=1
- 留空（空字符串）保存为 `null`，表示透传
- 有值时保存为对应的数字

### 5. 实现位置

**后端**：`OpenAIChatBackend::transform_request`
- 根据当前请求的 slot（从模型名推断）获取配置的限制值
- 应用限制逻辑

**前端**：配置表单组件
- 添加三个输入框
- 处理空值转换为 `null`

## Risks / Trade-offs

- **用户可能不理解"留空透传"的含义**
  - 缓解：添加提示文本 "留空则透传 Claude Code 传入的值"

- **配置值可能超过某些 API 的实际限制**
  - 缓解：这是用户主动配置的值，用户应了解其上游 API 的限制
  - 如果仍超出，API 会返回错误，用户可以调整配置

- **与模型映射的耦合**
  - 缓解：虽然按 slot 配置，但实际限制应用于转换后的模型
  - 这是设计上的有意选择，简化用户心智模型
