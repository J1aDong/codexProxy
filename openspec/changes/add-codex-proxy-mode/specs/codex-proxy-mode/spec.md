## ADDED Requirements

### Requirement: 桌面端提供 Claude 与 Codex 档位切换
系统 SHALL 在桌面配置界面提供 Claude 与 Codex 两个客户端档位，并根据当前档位展示对应配置表单。

#### Scenario: 切换到 Claude 档位
- **WHEN** 用户选择 Claude 档位
- **THEN** 系统 SHALL 展示当前 Claude Code 配置界面
- **AND** 系统 SHALL 保持现有 Claude 配置字段和行为不变

#### Scenario: 切换到 Codex 档位
- **WHEN** 用户选择 Codex 档位
- **THEN** 系统 SHALL 展示 Codex 精简配置界面
- **AND** 系统 SHALL 隐藏 Claude 专属的模型映射与配置指南字段
- **AND** 系统 SHALL 隐藏转换器选择，并固定使用 Codex 透传

### Requirement: Codex 档位使用独立目标配置
系统 SHALL 为 Codex 档位保存独立于 Claude 档位的目标地址、API 密钥、端点列表和当前选中端点。

#### Scenario: 修改 Claude 目标地址不影响 Codex
- **WHEN** 用户在 Claude 档位修改目标地址并保存配置
- **THEN** Codex 档位的目标地址 SHALL 保持修改前的值

#### Scenario: 修改 Codex 目标地址不影响 Claude
- **WHEN** 用户在 Codex 档位修改目标地址并保存配置
- **THEN** Claude 档位的目标地址 SHALL 保持修改前的值

#### Scenario: 重新打开应用恢复两套配置
- **WHEN** 用户保存 Claude 与 Codex 两套不同目标地址后重新打开应用
- **THEN** 系统 SHALL 在 Claude 档位恢复 Claude 目标地址
- **AND** 系统 SHALL 在 Codex 档位恢复 Codex 目标地址

### Requirement: 代理生命周期同时作用于 Claude 与 Codex
系统 SHALL 将启动代理、停止代理、端口占用检测和运行状态作为全局代理进程状态，而不是当前 UI 档位的局部状态。

#### Scenario: 启动代理后两个入口同时可用
- **WHEN** 用户点击启动代理
- **THEN** 系统 SHALL 启动一个本地代理进程
- **AND** 该代理 SHALL 同时处理 Claude 入口和 Codex 入口的请求

#### Scenario: 停止代理后两个入口同时停止
- **WHEN** 用户点击停止代理
- **THEN** 系统 SHALL 停止本地代理进程
- **AND** Claude 入口与 Codex 入口 SHALL 都不再继续代理上游请求

### Requirement: Codex 请求使用 /codex 前缀入口
系统 SHALL 通过 `http://localhost:<port>/codex` 前缀区分 Codex 客户端请求，并为该前缀下的请求选择 Codex 档位配置。

#### Scenario: Codex messages 请求选择 Codex 配置
- **WHEN** 客户端向 `/codex/v1/messages` 或 `/codex/messages` 发送 POST 请求
- **THEN** 系统 SHALL 使用 Codex 档位的目标地址、API 密钥和固定 Codex 透传转换器处理请求
- **AND** 系统 SHALL NOT 使用 Claude 档位的目标地址配置

#### Scenario: Claude messages 请求继续选择 Claude 配置
- **WHEN** 客户端向 `/v1/messages` 或 `/messages` 发送 POST 请求
- **THEN** 系统 SHALL 使用 Claude 档位的目标地址、API 密钥和转换器配置处理请求
- **AND** 系统 SHALL NOT 使用 Codex 档位的目标地址配置

#### Scenario: Codex count_tokens 请求选择 Codex 配置
- **WHEN** 客户端向 `/codex/v1/messages/count_tokens` 或 `/codex/messages/count_tokens` 发送 POST 请求
- **THEN** 系统 SHALL 使用 Codex 档位配置处理 count_tokens 请求

#### Scenario: Codex 原生 models 请求透传
- **WHEN** 客户端向 `/codex/v1/models` 发送 GET 请求
- **THEN** 系统 SHALL 将请求透传到 Codex 档位目标地址对应的 `/v1/models`
- **AND** 系统 SHALL NOT 返回本地 404

#### Scenario: Codex 原生任意 v1 路径透传
- **WHEN** 客户端向 `/codex/v1/responses`、`/codex/v1/images/generations` 或 `/codex/v1/files` 等非 messages 兼容路径发送请求
- **THEN** 系统 SHALL 保留 method、query、headers 和 body 并透传到 Codex 档位目标地址对应的 `/v1/**` 路径
- **AND** 系统 SHALL 使用 Codex 档位 API 密钥作为上游认证

### Requirement: 配置热更新包含两套档位配置
系统 SHALL 在运行中应用配置时同时提交 Claude 与 Codex 两套路由配置，端口不变时不得要求重启代理。

#### Scenario: 运行中修改 Codex 目标地址
- **WHEN** 代理正在运行且用户修改 Codex 档位目标地址并保存
- **THEN** 系统 SHALL 热更新 Codex 路由配置
- **AND** Claude 路由配置 SHALL 保持原值

#### Scenario: 运行中修改 Claude 目标地址
- **WHEN** 代理正在运行且用户修改 Claude 档位目标地址并保存
- **THEN** 系统 SHALL 热更新 Claude 路由配置
- **AND** Codex 路由配置 SHALL 保持原值

#### Scenario: 运行中修改端口
- **WHEN** 代理正在运行且用户修改代理端口
- **THEN** 系统 SHALL 提示该变更需要重启
- **AND** 系统 SHALL NOT 只通过热更新改变监听端口

### Requirement: 旧配置文件保持兼容
系统 MUST 能加载不包含 Codex 档位字段的旧配置文件，并将旧顶层目标配置继续视为 Claude 档位配置。

#### Scenario: 加载旧配置文件
- **WHEN** 系统加载旧版 `proxy-config.json`
- **THEN** 系统 SHALL 保留旧顶层字段对应的 Claude 配置
- **AND** 系统 SHALL 为 Codex 档位补齐默认目标配置

#### Scenario: 保存迁移后的配置
- **WHEN** 用户加载旧配置后保存
- **THEN** 系统 SHALL 写入 Codex 档位配置字段
- **AND** 系统 SHALL 保留 Claude 档位的既有配置值
