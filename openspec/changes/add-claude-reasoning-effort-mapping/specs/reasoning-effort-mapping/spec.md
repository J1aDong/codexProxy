# Reasoning Effort Mapping Specification

## ADDED Requirements

### Requirement: Reasoning Effort Mapping Configuration
系统 SHALL 支持为不同的 Claude 模型系列配置不同的 reasoning effort 级别。

#### Scenario: 默认映射配置
- **WHEN** 用户未配置自定义映射
- **THEN** 系统使用以下默认映射：
  - `opus` 系列模型 → `xhigh`
  - `sonnet` 系列模型 → `medium`
  - `haiku` 系列模型 → `low`

#### Scenario: 自定义映射配置
- **WHEN** 用户在 UI 中修改了某个模型的 reasoning effort 映射
- **THEN** 系统使用用户配置的映射值
- **AND** 配置被持久化到本地存储

#### Scenario: 模型名称匹配
- **WHEN** 收到包含 Claude 模型名称的请求（如 `claude-3-opus-20240229`）
- **THEN** 系统根据模型名称中包含的关键字（`opus`/`sonnet`/`haiku`）匹配对应的映射配置
- **AND** 将匹配到的 reasoning effort 值应用到 Codex API 请求中

#### Scenario: 未知模型处理
- **WHEN** 收到的模型名称不包含已知的 Claude 模型关键字
- **THEN** 系统使用 `medium` 作为默认 reasoning effort 值

### Requirement: Reasoning Effort UI Configuration
系统 SHALL 在桌面应用 UI 中提供 reasoning effort 映射配置界面。

#### Scenario: 配置界面显示
- **WHEN** 用户打开桌面应用
- **THEN** 显示 reasoning effort 配置区域
- **AND** 每个模型系列（opus/sonnet/haiku）有一个下拉选择器
- **AND** 下拉选项包括：`xhigh`, `high`, `medium`, `low`

#### Scenario: 多语言支持
- **WHEN** 用户切换语言
- **THEN** reasoning effort 配置区域的标签文本相应切换
- **AND** 支持中文和英文两种语言

### Requirement: Configuration Persistence
系统 SHALL 持久化 reasoning effort 映射配置。

#### Scenario: 配置保存
- **WHEN** 用户修改 reasoning effort 映射配置
- **AND** 用户启动代理服务
- **THEN** 配置被保存到本地配置文件

#### Scenario: 配置恢复
- **WHEN** 用户重新打开应用
- **THEN** 系统加载之前保存的 reasoning effort 映射配置
- **AND** UI 显示已保存的配置值

#### Scenario: 恢复默认配置
- **WHEN** 用户点击"恢复默认"按钮
- **THEN** reasoning effort 映射配置重置为默认值
- **AND** UI 显示默认配置值

### Requirement: Reasoning Effort Valid Values
系统 SHALL 限制 reasoning effort 的有效值范围。

#### Scenario: 有效值验证
- **WHEN** 设置 reasoning effort 值
- **THEN** 值必须是以下之一：`xhigh`, `high`, `medium`, `low`

#### Scenario: 无效值处理
- **WHEN** 配置文件中包含无效的 reasoning effort 值
- **THEN** 系统使用该模型的默认值替代
