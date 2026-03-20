## 1. Reproduce and Isolate

- [ ] 1.1 用日志样本 `/Users/mr.j/.codexProxy/logs/proxy_20260320_224508.log` 复核“后台已启动但自然语言声称失败”的完整链路
- [ ] 1.2 定位当前仓库中把普通 `Agent`/subagent、git 仓库状态和 worktree 绑定到一起的具体判断或提示来源

## 2. Decouple Runtime Semantics

- [ ] 2.1 修正普通 `Agent`/subagent 请求的解释逻辑，确保 worktree 只作为显式隔离选项而不是默认前置条件
- [ ] 2.2 收敛 plan-mode 下 `EnterWorktree`/`ExitWorktree` 工具过滤的影响范围，避免外溢成普通 subagent 可用性判断
- [ ] 2.3 当日志或上游事件已经证明后台 explore agent 成功启动时，统一自然语言说明为成功语义

## 3. Regression Coverage

- [ ] 3.1 为日志回放场景补测试，锁定“已启动”优先于“失败归因”
- [ ] 3.2 为普通 subagent 请求补测试，确认不会再生成“必须 worktree”类误导文案
- [ ] 3.3 为 plan-mode worktree 工具过滤补测试，确认只影响工具列表不影响普通 Agent 语义
- [ ] 3.4 运行相关回归测试并确认与 `claude-code-hub` 的目标语义一致
