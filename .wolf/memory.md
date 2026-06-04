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
