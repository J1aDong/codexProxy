## Context

从日志 `/Users/mr.j/.codexProxy/logs/proxy_20260320_224508.log` 可以观察到两个关键事实：
1. 同一请求中，后台 explore agent 已实际启动，并在上游事件里留下了“已启动后台 explorer：查上海天气…”与“已启动后台 explorer：查北京天气…”的 thinking/progress 痕迹；
2. 最终给用户的自然语言却说“两个 subagent 没拉起来：当前会话不在 git 仓库里，Agent 这里需要 worktree 才能启动”。

这两个事实彼此矛盾，说明当前问题不是简单的“Agent 真启动失败”，而是某处解释层把 `Agent` schema 中出现的 `isolation: "worktree"`、`EnterWorktree`/`ExitWorktree` 工具、以及当前工作目录不是 git 仓库这些线索错误拼接成了一个笼统结论。

参考项目 `/Users/mr.j/myRoom/code/ai/proxy/claude-code-hub` 中并没有找到把 `Agent` 与 `worktree` 强绑定的实现证据。当前仓库真正与 worktree 相关的明确逻辑主要出现在 `main/src/transform/codex/request.rs` 的 plan-mode 黑名单与测试中：它只说明某些 worktree 工具在特定 plan-mode 请求下会被过滤，而不是“普通 subagent 需要 worktree”。

因此本次设计重点不在于新增工具或改协议，而在于把“真实执行状态”和“自然语言解释”重新对齐，并消除普通 Agent/subagent 与 worktree 的错误耦合叙事。

## Goals / Non-Goals

**Goals:**
- 确保普通 `Agent`/subagent 使用不再被错误表述为依赖 worktree
- 明确区分普通 Agent 调用、worktree 工具可见性、以及 git 仓库前置条件的边界
- 当日志或上游事件已经证明后台 explore agent 成功启动时，下游解释必须与之保持一致
- 为这类误判建立日志回放与回归测试，防止再次出现“实际已启动但口头上说失败”的矛盾输出

**Non-Goals:**
- 不重写 team mailbox 协议或 `SendMessage` 语义
- 不改 `EnterWorktree`/`ExitWorktree` 工具定义本身
- 不新增新的 subagent 类型或新的 tool schema
- 不顺手改造与本问题无关的 `anthropic/openai/gemini` 转换链

## Decisions

### D1: 把“普通 Agent 可用性”和“worktree 工具可见性”视为两个独立判断面
**Decision**: 普通 `Agent`/subagent 是否可用，必须与 `EnterWorktree`/`ExitWorktree` 工具是否被过滤、是否显式请求隔离 worktree 分开判断；自然语言不得再把后者当作前者的必要条件。

**Why**: 目前真正能从代码中看到的 worktree 特殊逻辑只与 plan-mode 工具过滤相关，不能据此推出“subagent 必须 worktree”。

**Alternatives considered**:
- 继续复用当前笼统表述：实现简单，但会持续误导用户。
- 一律隐藏 worktree 工具：会损失用户显式请求 worktree 时的能力。

### D2: 日志/事件已证明成功启动时，自然语言必须优先服从成功事实
**Decision**: 当上游事件、tool result 或日志回放已显示 explore agent/background agent 成功启动时，下游说明必须优先承认“已启动”，而不是再生成失败归因。

**Why**: 这是本次用户指出的直接问题：事实层和解释层矛盾。

**Alternatives considered**:
- 继续依赖 prompt 侧自由发挥：最省事，但无法消除同类误判。

### D3: plan-mode 下的 worktree 工具黑名单只能影响工具列表，不能外溢成普通 Agent 失败解释
**Decision**: `EnterWorktree`/`ExitWorktree` 在 plan-mode 下的过滤行为，只能影响转发给上游的工具列表；不得再被扩展解释为“普通 Agent/subagent 因 worktree 不可用而启动失败”。

**Why**: 当前代码里对 worktree 最明确的处理就是这一层边界，必须把范围锁死。

**Alternatives considered**:
- 把 plan-mode 黑名单继续视作更广义的运行能力约束：会制造更多误判。

## Risks / Trade-offs

- [过度收紧 worktree 相关提示] → 仅移除错误耦合，不禁止用户显式请求 worktree 的合法路径。
- [日志回放与真实在线行为仍有偏差] → 用日志样本回放测试和结构化事件断言来锁定最关键场景。
- [修复解释层时误伤 plan-mode 工具过滤] → 为 plan-mode 与普通 Agent 请求分别补测试，防止范围外回归。

## Migration Plan

1. 先基于该日志样本复现“后台已启动但口头说失败”的场景。
2. 找出当前把 Agent、git 仓库、worktree 三者串成错误结论的判断或提示来源。
3. 将普通 Agent 可用性与 worktree 工具过滤逻辑拆开，并补测试。
4. 回放日志样本与普通请求/plan-mode 请求，确认误导文案消失且原有 worktree 工具边界不回退。

## Open Questions

1. 误导文案究竟来自 request augmentation、tool result 改写，还是上游模型在当前 prompt 下的自由生成？默认先以日志回放和现有重写层排查。
2. 是否需要新增结构化提示来明确“Agent 可在当前仓库上下文下直接运行，worktree 仅是显式隔离选项”？默认建议仅在确有误判来源时添加最小提示。
