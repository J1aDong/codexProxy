# Change: Add desktop configuration UI

## Why
需要一个可打包的桌面前端配置页面，用于管理 codex-proxy-anthropic.js 的关键配置，并让非命令行用户可用。

## What Changes
- 新增 Electron + Vue3 桌面应用，用于配置代理参数
- 新增本地配置存储（端口、目标地址、API key）
- 启动时由桌面壳启动代理服务

## Impact
- Affected specs: desktop-config-ui
- Affected code: 新增 fronted/ 前端与桌面壳工程；可能调整代理读取配置方式
