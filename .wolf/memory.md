# Memory

> Chronological action log. Hooks and AI append to this file automatically.
> Old sessions are consolidated by the daemon weekly.

| 08:55 | implemented size-based codexProxy log truncation | main/src/logger.rs | logger tests passed | ~4k |

## Session: 2026-06-04 14:25

| Time | Action | File(s) | Outcome | ~Tokens |
|------|--------|---------|---------|--------|
| 14:35 | diagnosed concurrent request latency bottlenecks | main/src/server.rs, main/src/logger.rs, main/src/load_balancer/mod.rs | found stream-held global semaphore and synchronous duplicate log writes; LB permits constrain upstream acquisition/first-response phase | ~18k |
| 14:48 | optimized concurrent streaming and logging hot paths | main/src/server.rs, main/src/logger.rs, fronted-tauri/src-tauri/src/proxy.rs | tests passed; stream no longer holds global maxConcurrency permit and log forwarding no longer double-writes | ~8k |
| 14:50 | ran frontend build regression | fronted-tauri | vue-tsc and vite build passed | ~1k |

## Session: 2026-06-17 10:48

| Time | Action | File(s) | Outcome | ~Tokens |
|------|--------|---------|---------|--------|
| 10:49 | Edited .github/workflows/release.yml | 3→3 lines | ~27 |
| 10:49 | Session end: 1 writes across 1 files (release.yml) | 1 reads | ~1947 tok |
| 10:50 | Session end: 1 writes across 1 files (release.yml) | 2 reads | ~2180 tok |
| 10:52 | Session end: 1 writes across 1 files (release.yml) | 2 reads | ~2180 tok |
| 10:58 | Session end: 1 writes across 1 files (release.yml) | 2 reads | ~2180 tok |
| 11:06 | Session end: 1 writes across 1 files (release.yml) | 2 reads | ~2180 tok |
| 11:11 | Session end: 1 writes across 1 files (release.yml) | 2 reads | ~2183 tok |
| 11:15 | Created .github/workflows/release.yml | — | ~2121 |
| 11:32 | Session end: 2 writes across 1 files (release.yml) | 2 reads | ~4304 tok |

## Session: 2026-06-17 21:50

| Time | Action | File(s) | Outcome | ~Tokens |
|------|--------|---------|---------|--------|
| 21:53 | Created .claude/plans/auto-write-config-plan.md | — | ~652 |
| 22:06 | Created .claude/plans/auto-write-config-plan.md | — | ~873 |
| 22:10 | Edited fronted-tauri/src-tauri/Cargo.toml | 4→5 lines | ~37 |
| 22:10 | Edited fronted-tauri/src-tauri/src/proxy.rs | modified get_claude_settings_path() | ~1171 |
| 22:10 | Edited fronted-tauri/src-tauri/src/main.rs | 3→5 lines | ~42 |
| 22:10 | Edited fronted-tauri/src/bridge/configBridge.ts | expanded (+6 lines) | ~109 |
| 22:11 | Created fronted-tauri/src/components/features/GuideSection.vue | — | ~1009 |
| 22:11 | Edited fronted-tauri/src/i18n/en.ts | 4→8 lines | ~61 |
| 22:11 | Edited fronted-tauri/src/i18n/zh.ts | 4→8 lines | ~52 |

## Session: 2026-06-17

| Time | Action | File(s) | Outcome | ~Tokens |
|------|--------|---------|---------|--------|
| (session) | 改造配置指南为一键写入文件 | fronted-tauri/src-tauri/src/proxy.rs, main.rs, configBridge.ts, GuideSection.vue, i18n | 新增 apply_claude_config / apply_codex_config 命令，支持 merge 写入 ~/.claude/settings.json 和 ~/.codex/config.toml；cargo check + vue-tsc 通过 | ~12k |
| 22:14 | Session end: 9 writes across 8 files (auto-write-config-plan.md, Cargo.toml, proxy.rs, main.rs, configBridge.ts) | 11 reads | ~54565 tok |
| 22:17 | Edited fronted-tauri/src/components/features/GuideSection.vue | 3→4 lines | ~72 |
| 22:17 | Edited fronted-tauri/src/components/features/GuideSection.vue | modified if() | ~75 |
| 22:17 | Session end: 11 writes across 8 files (auto-write-config-plan.md, Cargo.toml, proxy.rs, main.rs, configBridge.ts) | 11 reads | ~55107 tok |
