# Cerebrum

> OpenWolf's learning memory. Updated automatically as the AI learns from interactions.
> Do not edit manually unless correcting an error.
> Last updated: 2026-06-02

## User Preferences

<!-- How the user likes things done. Code style, tools, patterns, communication. -->

## Key Learnings

- **Project:** codexProxy
- **Description:** Codex Proxy 是一个本地网关：以 **Anthropic Messages** 作为统一入口，兼容 **Claude Code** 使用习惯，并将请求稳定路由到 **Codex / Gemini / Anthropic（透传）** 等上游。
- 日志目录 `~/.codexProxy/logs` 由 `main/src/logger.rs::AppLogger` 管理；单个 `proxy_*.log` 采用 500MB 触发、保留尾部 200MB 的字节级截断，另保留原有最多 3 个日志文件和 200 个请求块裁剪。
- 并发卡顿治理：`main/src/server.rs` 现在用 hyper-util auto connection builder 支持 HTTP/1/HTTP/2，并在建立流式下游 response 前释放全局 `max_concurrency` permit；Tauri 日志转发只 emit 到前端不再二次写文件；`AppLogger` 写文件使用 `try_lock`，避免并发流式日志阻塞 async 热路径。

## Do-Not-Repeat

<!-- Mistakes made and corrected. Each entry prevents the same mistake recurring. -->
<!-- Format: [YYYY-MM-DD] Description of what went wrong and what to do instead. -->

## Decision Log

<!-- Significant technical decisions with rationale. Why X was chosen over Y. -->
- 2026-06-03: codexProxy 日志大小治理对齐 codebuddy2api 的详情日志策略：不改日志目录/文件名/API，只在 AppLogger 写入前后及启动清理时做 500MB -> 200MB 尾部截断，避免单个日志无限增长。
