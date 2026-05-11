## ADDED Requirements

### Requirement: 保留工具选择与并行调用意图
系统在将 Anthropic 风格请求转换为 OpenAI Chat Completions 请求时，MUST 保留会影响工具调用行为的关键意图，包括工具选择策略与并行工具调用控制；当上游不支持某项语义时，系统 MUST 采用显式且可观测的降级策略，而不是静默丢弃。

#### Scenario: 透传或映射工具选择策略
- **WHEN** 请求包含会影响是否调用工具、必须调用工具、或指定工具的选择语义
- **THEN** 转换后的 OpenAI 请求 MUST 显式携带与该语义等价的工具选择参数，或进入已定义的可观测降级路径

#### Scenario: 控制并行工具调用
- **WHEN** 请求需要禁止或收敛并行工具调用行为
- **THEN** 转换后的 OpenAI 请求 MUST 显式携带并行工具调用控制参数，使后续重试/串行回退逻辑能够生效

### Requirement: 保持调用方 stream 语义
系统在构造 OpenAI 上游请求时 MUST 尊重调用方的 stream 意图，不得在会改变协议边界语义的情况下无条件强制上游进入流式模式。

#### Scenario: 调用方请求非流式响应
- **WHEN** 调用方请求 `stream=false`
- **THEN** 系统 MUST 采用与该语义一致的上游请求/本地聚合策略，且不得无条件覆盖为固定的流式请求语义

#### Scenario: 仅在需要时请求 usage 扩展 chunk
- **WHEN** 系统需要依赖上游流式 usage chunk 完成统计或兼容逻辑
- **THEN** 系统 MUST 仅在与调用方语义兼容的前提下启用对应流式选项

### Requirement: 显式处理无法完整表达的输入语义
对于 Anthropic 请求中无法在当前 P0/P1 范围内完整表达为 OpenAI Chat Completions 请求的输入语义，系统 MUST 采用显式、一致、可测试的处理策略；不得在无标记的情况下悄然扭曲高优先级语义。

#### Scenario: thinking 可见性策略生效
- **WHEN** 请求声明不允许可见 thinking 输出
- **THEN** 请求构造与响应转换 MUST 共同遵守该策略，不得因为后端实现差异而泄露 thinking 内容

#### Scenario: 无法完整表达的输入块
- **WHEN** 请求包含当前版本不能完整保真的输入块类型
- **THEN** 系统 MUST 按已定义策略进行明确降级或跳过，并保持行为可预测、可测试

### Requirement: 在 P2 阶段提升输入语义保真度
系统在 P2 阶段 SHOULD 逐步提升 Anthropic 输入语义到 OpenAI 上游请求的保真度，优先覆盖 document/unknown block、system block fidelity，以及当前被静默忽略的更广多模态输入。

#### Scenario: Document 或 unknown block 输入
- **WHEN** 请求包含 `Document` 或当前无法识别的输入块
- **THEN** 系统 SHOULD 保留可测试的类型标记和关键元数据，而不是仅退化为不可区分的普通文本

#### Scenario: 更高保真的 system block 处理
- **WHEN** system 内容包含超出简单文本拼接的结构化语义
- **THEN** 系统 SHOULD 按已定义策略保留更多 system 级语义，而不是一律粗粒度扁平化

#### Scenario: 非 user 的图像或更广多模态输入
- **WHEN** 请求包含当前路径尚未处理的图像或其他多模态输入
- **THEN** 系统 SHOULD 采用明确支持或明确降级的策略，避免静默忽略
