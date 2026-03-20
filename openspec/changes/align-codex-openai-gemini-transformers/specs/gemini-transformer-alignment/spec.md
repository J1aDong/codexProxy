## ADDED Requirements

### Requirement: Gemini transformer SHALL align with the same layered boundary as other non-Anthropic transformers
系统在把 Anthropic 风格请求转换为 Gemini 请求时，MUST 将 `contents/systemInstruction` 的语义映射、Gemini 上游请求构建和响应归一拆分为清晰的职责层，以便和 Codex/OpenAI 使用一致的验证方式。

#### Scenario: Request mapping is tested without transport details
- **WHEN** 系统对一个包含 `system`、消息内容和采样参数的 Anthropic 请求做 Gemini 转换
- **THEN** 系统 MUST 先得到与 transport 无关的 Gemini request body，再由独立 builder 处理 endpoint 与 headers

### Requirement: Gemini request mapping SHALL normalize systemInstruction and contents deterministically
系统 MUST 以稳定规则把 system、user、assistant、图像内容和工具相关内容映射为 Gemini `systemInstruction` 与 `contents`，并且这些规则在重构后 MUST 可独立测试。

#### Scenario: Merge system sources into systemInstruction
- **WHEN** 请求同时含有顶层 `system` 与消息中的 system 语义内容
- **THEN** 系统 MUST 以确定规则将其归并到 Gemini `systemInstruction` 中，而不是把 system 语义散落到 transport 或响应层

#### Scenario: Convert image and text content into Gemini parts
- **WHEN** 消息同时包含文本与图像内容
- **THEN** 系统 MUST 将其映射为合法的 Gemini `parts` 结构，并保持各内容类型边界稳定

### Requirement: Gemini upstream request builder SHALL handle official Gemini and Gemini CLI transport differences explicitly
系统 MUST 通过独立的上游构建层处理 Gemini 官方接口与 Gemini CLI 风格接口在 endpoint、认证头和客户端标识上的差异，但不得在此阶段重新解释请求体业务语义。

#### Scenario: Target is Gemini CLI provider
- **WHEN** 上游供应商类型为 Gemini CLI
- **THEN** 系统 MUST 使用 Gemini CLI 所需的 endpoint/header 语义，并保持 request body 与同语义的官方 Gemini 请求一致

### Requirement: Gemini response transformation SHALL normalize upstream output into a stable Anthropic-compatible result
系统在处理 Gemini 上游返回时，MUST 把文本、finish reason、usage 以及兼容包装结构稳定归一为下游可消费的结果，不得因为供应商返回格式轻微差异而导致生命周期不稳定。

#### Scenario: Response is wrapped or includes usage metadata variants
- **WHEN** 上游返回存在包装层，或 usage 字段出现在不同但等价的位置
- **THEN** 系统 MUST 仍然产出稳定的归一结果，并正确映射 finish reason 与 usage 信息
