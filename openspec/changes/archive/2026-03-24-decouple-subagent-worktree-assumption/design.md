## Context

从日志 `/Users/mr.j/.codexProxy/logs/proxy_20260320_224508.log` 可以观察到更准确的两步链路：
1. 模型在普通“用两个 subagent 搜索天气”的请求里，主动把 `Agent` 调用写成了 `"isolation":"worktree"`；
2. 随后的 `tool_result` 明确失败：`Cannot create agent worktree: not in a git repository...`。

因此当前问题的真正根因不是“后台已成功启动但自然语言误报失败”，而是 `Agent` schema 中暴露的 worktree 隔离选项误导模型把普通 subagent 请求走成了 worktree 隔离路径；后续自然语言又把这次隔离失败泛化成了“普通 subagent 需要 worktree”。

参考项目 `/Users/mr.j/myRoom/code/ai/proxy/claude-code-hub` 中并没有找到把 `Agent` 与 `worktree` 强绑定的实现证据。当前仓库真正与 worktree 相关的明确逻辑主要出现在 `main/src/transform/codex/request.rs` 的 plan-mode 黑名单与测试中：它只说明某些 worktree 工具在特定 plan-mode 请求下会被过滤，而不是“普通 subagent 需要 worktree”。

因此本次设计重点不在于新增工具或改协议，而在于：
- 让普通 `Agent` 请求默认不再暴露误导性的 `worktree` 隔离选项；
- 当上游真实返回 `Cannot create agent worktree` 时，把解释收敛为“这次隔离请求失败”，而不是把 worktree 说成普通 subagent 的前置条件。

## Goals / Non-Goals

**Goals:**
- 确保普通 `Agent`/subagent 使用不再被错误引导为 `worktree` 隔离
- 明确区分普通 Agent 调用、worktree 工具可见性、以及 git 仓库前置条件的边界
- 当上游返回 `Cannot create agent worktree` 时，下游解释必须准确归因到“隔离请求失败”
- 为这类误判建立日志回放与回归测试，防止再次出现“普通 subagent 被误导成 worktree 请求”的问题

**Non-Goals:**
- 不重写 team mailbox 协议或 `SendMessage` 语义
- 不改 `EnterWorktree`/`ExitWorktree` 工具定义本身
- 不新增新的 subagent 类型或新的 tool schema
- 不顺手改造与本问题无关的 `anthropic/openai/gemini` 转换链

## Decisions

### D1: 普通 Agent 请求默认移除误导性的 `isolation=worktree` schema 入口
**Decision**: 对普通 `Agent`/subagent 请求，默认不向上游模型暴露 `Agent.parameters.properties.isolation` 中的 `worktree` 选项；只有用户显式提到 worktree 时，才保留该入口。

**Why**: 日志已经证明模型正是因为看到了这个 schema 入口，才把普通 subagent 请求主动写成了 worktree 隔离调用。

**Alternatives considered**:
- 保留 schema 不动，只靠提示语澄清：仍可能继续被模型误用。
- 一律移除所有 worktree 相关工具：会损失用户显式请求 worktree 时的能力。

### D2: `Cannot create agent worktree` 必须被解释为隔离请求失败，而不是普通 Agent 失败
**Decision**: 当 `Agent` tool result 返回 `Cannot create agent worktree...` 时，下游说明必须明确这是一次被请求出来的 worktree 隔离失败，并指出普通 subagent 请求并不默认依赖 worktree。

**Why**: 这是用户截图里最直接的误导点：真实失败是“worktree 创建失败”，不是“subagent 本身起不来”。

**Alternatives considered**:
- 继续透传原始错误：技术上真实，但仍容易被模型在后续自然语言里泛化成错误结论。

### D3: plan-mode 下的 worktree 工具黑名单只能影响工具列表，不能外溢成普通 Agent 失败解释
**Decision**: `EnterWorktree`/`ExitWorktree` 在 plan-mode 下的过滤行为，只能影响转发给上游的工具列表；不得再被扩展解释为“普通 Agent/subagent 因 worktree 不可用而启动失败”。

**Why**: 当前代码里对 worktree 最明确的处理就是这一层边界，必须把范围锁死。

**Alternatives considered**:
- 把 plan-mode 黑名单继续视作更广义的运行能力约束：会制造更多误判。

## Risks / Trade-offs

- [过度收紧 worktree 相关提示] → 仅移除普通 Agent 请求下的误导入口，不禁止用户显式请求 worktree 的合法路径。
- [日志回放与真实在线行为仍有偏差] → 用日志样本等价测试和结构化 `tool_result` 断言来锁定最关键场景。
- [修复解释层时误伤 plan-mode 工具过滤] → 为 plan-mode 与普通 Agent 请求分别补测试，防止范围外回归。

## Migration Plan

1. 先基于该日志样本确认普通 subagent 请求被错误写成 `isolation=worktree` 的链路。
2. 调整普通 Agent 请求的 tool schema 暴露逻辑，默认移除误导性的 worktree 隔离入口。
3. 对 `Cannot create agent worktree` 的 tool result 增加更准确的解释改写。
4. 回放普通请求与 plan-mode 请求，确认普通 subagent 不再被误导为 worktree，且原有 worktree 工具边界不回退。

## Open Questions

1. 是否还需要在普通 Agent 请求中追加一条额外文字提示，进一步声明“普通 subagent 不需要 worktree”？默认先靠 schema 收敛 + 失败改写解决。
2. 是否需要把这条规则推广到其他可能携带 `isolation=worktree` 的 agent-like 工具？默认先聚焦 `Agent` 本身。
