## ADDED Requirements

### Requirement: Ordinary subagent requests SHALL NOT be explained as requiring worktree isolation by default
系统在处理普通 `Agent`/subagent 请求时，MUST 将 worktree 视为显式隔离选项，而不是默认前置条件；如果用户只是要求开几个 subagent / explore agent，系统不得自动解释为“必须 worktree 才能启动”。

#### Scenario: User asks for parallel subagents in a normal repository context
- **WHEN** 用户请求“开几个 subagent”或“并行跑两个 explore agent”且并未显式要求 worktree
- **THEN** 系统 MUST 将其视为普通 Agent 使用场景，不得自然语言声称“Agent 需要 worktree 才能启动”

### Requirement: Successful background agent launch SHALL take precedence over failure narration
当日志、tool result 或上游事件已经表明后台 explore agent 成功启动时，系统 MUST 以成功事实为准，不得在后续自然语言中改口声称 subagent 启动失败。

#### Scenario: Explore agents already launched in background
- **WHEN** 日志或事件中已经出现后台 explore agent 成功启动的证据
- **THEN** 系统 MUST 继续围绕“已启动”给出说明，而不是再补充“两个 subagent 没拉起来”这类相反结论

### Requirement: Worktree tool filtering SHALL NOT leak into generic subagent availability claims
`EnterWorktree`/`ExitWorktree` 工具在特定模式下是否被过滤，只能影响工具列表本身，MUST NOT 被扩展解释为普通 subagent/Agent 的通用可用性约束。

#### Scenario: Plan mode filters worktree tools
- **WHEN** plan mode 请求导致 `EnterWorktree`/`ExitWorktree` 被过滤出转发工具列表
- **THEN** 系统 MUST 仅保留“工具列表变化”这一语义，不得把它扩展成“subagent 因缺少 worktree 无法启动”的解释

### Requirement: Git repository status SHALL be reported narrowly and accurately
系统在提及“当前目录是否位于 git 仓库”时，MUST 只在该条件确实决定某个操作能否执行时才引用它，且不得把“非 git 仓库”与“必须 worktree”混为同一结论。

#### Scenario: Current directory is not a git repository but background agent already launched
- **WHEN** 当前主会话目录不是 git 仓库，但实际后台 explore agent 已成功启动
- **THEN** 系统 MUST 不得再用“当前会话不在 git 仓库里，所以 subagent 拉不起来”作为解释
