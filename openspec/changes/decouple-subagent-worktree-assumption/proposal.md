## Why

日志 `/Users/mr.j/.codexProxy/logs/proxy_20260320_224508.log` 已经显示同一请求里后台 explore agent 实际被成功启动，并输出了“已启动后台 explorer：查上海天气…”与“已启动后台 explorer：查北京天气…”，但最终自然语言却错误声称“subagent 没拉起来，因为当前会话不在 git 仓库里，Agent 这里需要 worktree 才能启动”。这说明当前代理链路仍存在把普通 Agent/subagent 使用与 worktree 要求错误耦合的现象，而参考项目 `/Users/mr.j/myRoom/code/ai/proxy/claude-code-hub` 也没有显示出这种把 `Agent` 与 `worktree` 绑定的运行时语义。

现在需要专门收敛这条误导路径，确保普通 subagent / explore agent 的成功启动、git 仓库要求、以及 `EnterWorktree`/`ExitWorktree` 工具的存在是彼此独立的概念，不能再在自然语言中被错误混同。

## What Changes

- 修复普通 `Agent`/subagent 使用被错误表述为“必须 worktree 才能启动”的问题
- 对齐日志回放与运行时语义：如果后台 explore agent 已成功启动，下游自然语言不得再声称启动失败或归因到 worktree
- 明确区分三件事：`Agent` 工具本身、`EnterWorktree`/`ExitWorktree` 工具、以及“当前目录是否处于 git 仓库”这些条件的边界
- 收敛与 plan mode 黑名单相关的 worktree 工具处理，避免把“工具在 plan mode 下被过滤”错误扩展成“subagent 本身依赖 worktree”
- 为日志回放、subagent 启动提示、worktree 工具过滤和普通 Agent 请求补回归测试
- 不修改 team mailbox 协议、`SendMessage` 语义或 `anthropic/openai/gemini` 的协议转换要求

## Capabilities

### New Capabilities
- `subagent-worktree-decoupling`: 定义普通 subagent/Agent 使用、worktree 工具可见性、git 仓库状态与自然语言解释之间的解耦要求

### Modified Capabilities

## Impact

- 重点影响代码：`main/src/transform/codex/request.rs`, `main/src/transform/processor.rs`，以及相关回归测试
- 重点影响行为：Agent/subagent 请求增强、plan mode 下的 worktree 工具过滤、日志与自然语言对外解释的一致性
- 不引入新依赖，不新增新的工具类型或新的路由入口
- 不改变 team mailbox 协议，不改变 `EnterWorktree`/`ExitWorktree` 工具本身的接口定义
