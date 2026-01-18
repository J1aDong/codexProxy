## ADDED Requirements
### Requirement: Desktop configuration UI
系统 MUST 提供一个可打包的桌面应用，用于配置代理服务的关键参数。

#### Scenario: Configure target and port
- **WHEN** 用户在界面中修改端口与目标地址
- **THEN** 新配置应被保存并应用到代理启动参数

#### Scenario: Optional API key
- **WHEN** 用户留空 API key
- **THEN** 应以“透传模式”运行，不在本地存储或注入 API key

#### Scenario: Start proxy from desktop app
- **WHEN** 用户启动桌面应用
- **THEN** 应自动启动本地代理并在界面中展示运行状态
