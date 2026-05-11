## Why

当前 Codex Proxy 的桌面配置界面主要面向 Claude Code。直接把 Codex 支持塞进现有配置会混淆 Claude 与 Codex 的目标地址，也会让用户在 `cc switch` 之类的外部切换后还需要重启会话。

新增独立的 Codex 档位后，Claude Code 与 Codex 可以共用同一个正在运行的本地代理，但各自保持独立的目标地址配置和访问入口。

## What Changes

- 在桌面配置界面新增 Claude / Codex 档位切换。
- Claude 档位维持当前行为和当前界面。
- Codex 档位复用单模型代理界面布局，但只展示端口、代理模式、目标地址和 Codex API 密钥；转换器固定为 Codex 透传，不提供选择。
- Codex 档位的目标地址配置与 Claude 档位完全独立，互不生效。
- Codex 通过 `http://localhost:<port>/codex/v1` 作为 Codex/OpenAI 原生 Base URL，避免和现有 Claude 兼容入口混淆。
- 启动代理 / 停止代理仍然是全局动作：同一个运行中的代理同时服务 Claude 与 Codex 入口。

## Capabilities

### New Capabilities

- `codex-proxy-mode`: 在现有 Claude 模式旁新增隔离的 Codex 档位，覆盖桌面 UI、独立配置和 `/codex` 代理入口行为。

### Modified Capabilities

- 无。

## Impact

- `fronted-tauri/` 的 Vue UI 状态、配置表单和配置指南展示。
- `fronted-tauri/src-tauri/` 的配置读写、热更新、导入导出命令。
- `main/src/server.rs` 的 `/codex` 路由注册与请求分发。
- 运行时配置模型需要区分 Claude 目标地址与 Codex 目标地址。
- 如果现有导入导出和热更新逻辑默认只有一套目标地址配置，需要同步扩展。
