## Why

日志 `/Users/mr.j/.codexProxy/logs/proxy_20260320_224508.log` 显示，模型在处理“用两个 subagent 搜索天气”这种普通请求时，主动给 `Agent` 调用填入了 `"isolation":"worktree"`，随后 `tool_result` 明确失败：`Cannot create agent worktree: not in a git repository...`。这说明当前问题的根因不是“后台已成功启动却口头误报失败”，而是普通 subagent 请求被 `Agent` schema 中暴露的 worktree 隔离选项误导，导致模型把 worktree 当成默认路径，而参考项目 `/Users/mr.j/myRoom/code/ai/proxy/claude-code-hub` 也没有体现出这种把普通 Agent 与 worktree 强绑定的运行时语义。

现在需要专门收敛这条误导路径，确保普通 subagent / explore agent 请求不会再默认走 `worktree` 隔离；同时即便上游真的返回了 `Cannot create agent worktree`，下游自然语言也必须准确说明“这是一次被错误请求出来的隔离失败”，而不是把 worktree 说成普通 subagent 的必要前置条件。

## What Changes

- 修复普通 `Agent`/subagent 请求被错误引导为 `worktree` 隔离的问题
- 调整 `Agent` 工具在普通请求下暴露给上游模型的 schema，避免把 `isolation=worktree` 当成默认可行路径
- 当上游真实返回 `Cannot create agent worktree` 时，统一改写为更准确的失败解释：失败来自错误请求了 worktree 隔离，而不是“subagent 必须 worktree”
- 明确区分三件事：普通 `Agent` 工具、`EnterWorktree`/`ExitWorktree` 工具、以及“当前目录是否处于 git 仓库”这些条件的边界
- 收敛与 plan mode 黑名单相关的 worktree 工具处理，避免把“工具在 plan mode 下被过滤”错误扩展成“subagent 本身依赖 worktree”
- 为日志回放、普通 subagent 请求、Agent worktree 失败改写和 plan-mode worktree 工具过滤补回归测试
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
