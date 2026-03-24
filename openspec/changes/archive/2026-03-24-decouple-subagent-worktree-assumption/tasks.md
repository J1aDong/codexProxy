## 1. Reproduce and Isolate

- [x] 1.1 用日志样本 `/Users/mr.j/.codexProxy/logs/proxy_20260320_224508.log` 复核“模型主动请求 `Agent.isolation=worktree` 并失败”的完整链路
- [x] 1.2 定位当前仓库中把普通 `Agent`/subagent、git 仓库状态和 worktree 绑定到一起的具体判断或提示来源

## 2. Decouple Runtime Semantics

- [x] 2.1 修正普通 `Agent`/subagent 请求的解释逻辑，确保 worktree 只作为显式隔离选项而不是默认前置条件
- [x] 2.2 收敛 plan-mode 下 `EnterWorktree`/`ExitWorktree` 工具过滤的影响范围，避免外溢成普通 subagent 可用性判断
- [x] 2.3 当上游返回 `Cannot create agent worktree` 时，统一改写为“错误请求了隔离”而不是“subagent 默认依赖 worktree”

## 3. Regression Coverage

- [x] 3.1 为日志回放/等价场景补测试，锁定“普通 subagent 不默认请求 worktree 隔离”
- [x] 3.2 为普通 subagent 请求补测试，确认不会再生成“必须 worktree”类误导文案
- [x] 3.3 为 plan-mode worktree 工具过滤补测试，确认只影响工具列表不影响普通 Agent 语义
- [x] 3.4 运行相关回归测试并确认与 `claude-code-hub` 的目标语义一致
