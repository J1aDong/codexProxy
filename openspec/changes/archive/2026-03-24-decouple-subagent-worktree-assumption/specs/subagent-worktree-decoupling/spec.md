## ADDED Requirements

### Requirement: Ordinary subagent requests SHALL NOT default to Agent worktree isolation
系统在处理普通 `Agent`/subagent 请求时，MUST 将 worktree 视为显式隔离选项，而不是默认前置条件；如果用户只是要求开几个 subagent / explore agent，系统不得把 `Agent.isolation=worktree` 作为默认可行路径暴露给上游模型。

#### Scenario: User asks for parallel subagents in a normal repository context
- **WHEN** 用户请求“开几个 subagent”或“并行跑两个 explore agent”且并未显式要求 worktree
- **THEN** 系统 MUST 将其视为普通 Agent 使用场景，不得让上游模型默认看到或选择 `Agent.isolation=worktree`

### Requirement: Agent worktree failure SHALL be explained as an isolation-request failure rather than a generic subagent prerequisite
当上游返回 `Cannot create agent worktree...` 时，系统 MUST 将其解释为“这次请求错误地要求了 worktree 隔离”，而不是把 worktree 说成普通 subagent/Agent 的通用前置条件。

#### Scenario: Agent tool result reports worktree creation failure
- **WHEN** `Agent` tool result 返回 `Cannot create agent worktree` 一类错误
- **THEN** 系统 MUST 明确说明失败来自 worktree 隔离请求本身，并指出普通 subagent 请求并不默认依赖 worktree

### Requirement: Worktree tool filtering SHALL NOT leak into generic subagent availability claims
`EnterWorktree`/`ExitWorktree` 工具在特定模式下是否被过滤，只能影响工具列表本身，MUST NOT 被扩展解释为普通 subagent/Agent 的通用可用性约束。

#### Scenario: Plan mode filters worktree tools
- **WHEN** plan mode 请求导致 `EnterWorktree`/`ExitWorktree` 被过滤出转发工具列表
- **THEN** 系统 MUST 仅保留“工具列表变化”这一语义，不得把它扩展成“subagent 因缺少 worktree 无法启动”的解释

### Requirement: Git repository status SHALL be reported narrowly and accurately
系统在提及“当前目录是否位于 git 仓库”时，MUST 只在该条件确实决定某个操作能否执行时才引用它，且不得把“非 git 仓库”与“普通 subagent 必须 worktree”混为同一结论。

#### Scenario: Current directory is not a git repository and Agent worktree isolation was requested
- **WHEN** 当前主会话目录不是 git 仓库，且请求确实选择了 Agent 的 worktree 隔离
- **THEN** 系统 MUST 说明失败来自这次隔离请求无法满足，而不是把它泛化成普通 subagent 的默认限制
