## ADDED Requirements

### Requirement: Codex request construction SHALL prefer explicit request context over hidden bundled instructions
系统在将 Anthropic 风格请求转换为 Codex Responses 请求时，MUST 以调用方显式提供的 system / instruction 上下文为主，不得在默认主路径中无条件追加隐藏的静态官方提示词；任何额外注入行为 MUST 是可判定、可测试且可审计的。

#### Scenario: Request already carries explicit system context
- **WHEN** 请求已经包含非空 `system` 内容，或通过显式上下文提供了等价指令信息
- **THEN** 系统 MUST 直接基于这些显式上下文构造 Codex 请求，不得再默认补上一份隐藏的 bundled `instructions`

#### Scenario: Compatibility branch injects extra instructions
- **WHEN** 系统确实进入了某个兼容性注入分支
- **THEN** 该分支 MUST 具备明确触发条件与可验证行为，并且测试能够区分“透传”与“注入”两种路径

### Requirement: Codex request mapping SHALL separate instruction assembly from transport adaptation
系统在构建 Codex Responses request body 时，MUST 先完成 `input`、`tools`、`tool_choice`、`reasoning`、`instructions` 等 body 语义组装，再由独立的上游请求构建步骤处理 headers、session headers、Accept 与 target URL，不得把 transport concern 混入 instruction 组装逻辑。

#### Scenario: Build request body before upstream headers
- **WHEN** 系统收到需要转发到 Codex Responses API 的请求
- **THEN** 系统 MUST 先生成与上游传输细节无关的 request body，再由独立 builder 添加 `Authorization`、`x-api-key`、`session_id` 等 header

### Requirement: Codex transformer SHALL preserve explicit request augmentation paths that are still supported
系统在重构 Codex transformer 时，MUST 保留仍然受支持的显式增强路径，例如 plan mode、tool state、skills 或 custom injection prompt，但这些增强 MUST 通过明确规则进入请求，而不是与默认静态 `instructions` 注入耦合在一起。

#### Scenario: Plan mode adds explicit prompt context
- **WHEN** 请求显式进入 plan mode 或携带对应的增强信号
- **THEN** 系统 MUST 仅把与 plan mode 相关的额外上下文按可测试规则加入请求，而不得因此自动恢复隐藏的默认官方提示词注入

### Requirement: Codex response transformation SHALL keep the existing Anthropic lifecycle guarantees
系统在重构 Codex response transformer 时，MUST 继续输出合法的 Anthropic 生命周期事件，并保留当前已验证的 reasoning、tool use、usage 与终态收口语义，除非 spec 明确要求移除某条行为。

#### Scenario: Response stream contains reasoning and usage tail data
- **WHEN** Codex 上游流中包含 reasoning 片段、tool 调用以及最终 usage 信息
- **THEN** 系统 MUST 继续将其转换为合法的 Anthropic 内容块与结束事件，不得因为 request 侧重构而破坏响应生命周期
