## Teammate / Subagent 速查

一句话判断：

- 单个子任务，优先用 `Agent`
- 明确需要多人持续协作，才用 `TeamCreate`
- `plan mode` 下默认不要建 team
- 系统出现 `team_name`、mailbox、teammate 等提示，不等于必须按 team 处理

### `Agent` 的两种模式

`Agent` 只是同一个工具名，不代表只有一种语义。

硬规则：

- 没有显式 `team_name` 时，按普通 `subagent` 处理
- `team_name` 为空、缺失或等于 `default` 时，也按普通 `subagent` 处理
- 只有先创建了 team，且 `team_name` 是明确的非默认 team 名时，才按 `team teammate` 处理
- 不能因为用了 `Agent` 工具，就自动推断成 team 协作

### `Agent`

默认含义：

- 优先理解为普通 `subagent`
- 适合一次性委派
- 不需要共享任务列表
- 不需要多人依赖管理
- 默认不要传 `team_name`
- 只有在明确加入 team 时才传 `team_name`

典型场景：

- 查实现位置
- 做只读探索
- 做实现计划
- 跑一次 code review
- 补一个局部实现

### `Agent` 参数规范

普通 `subagent`：

- 允许：`description`、`prompt`、`subagent_type`、`model`、`run_in_background`、`isolation`、`mode`
- `name` 可选；有 `name` 不代表它是 teammate
- 禁止默认补 `team_name`
- 禁止补 `team_name: "default"`
- 禁止因为系统提示出现 mailbox 或 teammate 就改走 team

`team teammate`：

- 必须先 `TeamCreate`
- `Agent` 调用里必须显式传入非默认 `team_name`
- 建议同时传 `name`，使其可被 `SendMessage` 和任务分配引用
- 只有这种情况才允许后续按 team mailbox、`TaskUpdate`、`SendMessage` 语义处理

### 并行硬规则

- 如果用户明确说“用 2/3/... 个 `subagent`”、`并行`、`同时`、`分别去查`，必须在同一 assistant 回合里发出对应数量的 `Agent` tool use
- 不允许先启动 1 个，再等结果回来后补启动剩余几个，除非后续任务定义确实依赖前一个结果
- 对互相独立的任务，默认视为可并行
- 像“按不同城市分别查天气”“按不同目录分别搜索”“按不同模块分别探索”这种任务，默认应一次性并行发起
- `plan mode` 下如果允许使用 Explore `subagent`，且用户明确要求多个独立 `subagent`，也应并行发起；这里的 `Explore` 指 `subagent_type=Explore`，不是 team
- 用户明确给了数量时，优先遵守该数量；不要无故减少为 1 个

### `TeamCreate`

只在这些情况使用：

- 用户明确说 `team`、`teammate`、`组队`、`swarm`、多人协作
- 任务需要多个 agent 持续协作
- 需要共享任务列表、owner、依赖关系
- 需要后续使用 `TaskCreate`、`TaskUpdate`、`SendMessage` 进行团队编排

不要因为下面这些信号就自动建 team：

- tool schema 里出现 `team_name`
- 系统提示里出现 mailbox、teammate
- `plan mode` 激活
- 用户只是说 “开几个 subagent”
- 普通 `subagent` 调用默认不应补 `team_name: "default"`

### `SendMessage`

默认理解：

- 这是 team / teammate 的主通信工具
- 也可以用于继续一个已经启动的 background `subagent`
- 但“能用”不等于“应该默认按 team 处理”

适合场景：

- 已经有 team，要给某个 teammate 发指令
- 已经有 team，要处理 `shutdown_request` / `shutdown_response`
- 已经有 team，要处理 `plan_approval_request` / `plan_approval_response`
- 已经明确启动了一个 background `subagent`，需要继续它

不适合场景：

- 用户没有明确要求 team，却把普通任务强行解释成 teammate 协作
- 在 `plan mode` 里为了问问题或继续分析就直接拉 team

### `TaskUpdate`

前提：已有 team 和 task。

用途：

- 更新 `status`
- 指定 `owner`
- 设置依赖关系
- 修改任务标题或描述

`TaskUpdate` 是任务板变更，不是聊天工具。

### `plan mode` 下

硬规则：

- 默认先做分析、只读探索、澄清问题、写计划
- 默认不要创建 team，不要把普通问题升级成多人协作
- 如果用户只是追问、补充信息、修正方向，应直接回答或更新计划
- `plan mode` 提到 “只用 Explore subagent type” 时，理解为 `subagent_type=Explore`，不是 `TeamCreate`
- 准备好后使用 `ExitPlanMode`

只在这些情况才允许 `plan mode` 里走 team：

- 用户明确要求 team / teammate / swarm / 组队
- 任务本身明确要求多人持续协作，而不是单次探索

优先顺序：

1. 直接阅读、搜索、分析
2. 必要时使用单个非编辑型 `subagent`
3. 写计划
4. `ExitPlanMode`
5. 只有用户明确要求时才 `TeamCreate`

### Team 与 Subagent 的区别

`team teammate`：

- 属于 team
- 有共享任务列表
- `Agent` 调用中应显式包含非默认 `team_name`
- 可以用 `SendMessage` 做 mailbox 通信
- 可以配合 `TaskCreate`、`TaskUpdate`

普通 background `subagent`：

- 不属于 team
- 没有共享任务列表
- `Agent` 调用中不应出现 `team_name`
- 可以被 `SendMessage` 继续，但这不代表它是 teammate
- 默认不传 `team_name`
- 不要把它误写成 team 协议或 team 编排

### 推荐流程

普通任务：

- `Agent`

`plan mode` 下的普通任务：

1. 直接分析
2. 必要时一个只读 `subagent`
3. 写计划
4. `ExitPlanMode`

明确多人协作任务：

1. `TeamCreate`
2. `TaskCreate`
3. `Agent(..., team_name=...)`
4. `TaskUpdate`
5. `SendMessage`
6. 完成后 `TeamDelete`

### 补充工具

- `TaskList`：查看任务列表
- `TaskGet`：查看任务详情
- `TaskOutput`：读取后台任务输出
- `multi_tool_use.parallel`：并行发起多个独立操作

### 最短口诀

- 单次委派：`Agent`
- 明确协作：`TeamCreate`
- 队内通信：`SendMessage`
- 任务板变更：`TaskUpdate`
- `plan mode`：先分析，默认不组队
