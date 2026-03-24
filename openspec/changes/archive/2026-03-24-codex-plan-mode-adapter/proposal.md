## Why

当前 Codex 转换器虽然能把 Claude Code 的 plan mode 请求转到 Codex Responses API，但行为仍主要停留在“把 plan 相关提示词和 schema 原样透传”的层面，无法稳定达到 codex-cli plan mode 那种“先返回计划，再等待用户确认是否执行”的交互效果。

现在需要先补一个最小可用适配层，让 Claude Code 经过 codexProxy 访问 Codex API 时，plan mode 至少能稳定表现为：先输出计划，再由用户决定是否继续执行，从而缩小与 codex-cli plan mode 的行为差距。

## What Changes

- 在 Codex request augmentation 决策里增加 plan 模式识别，而不是一律只分 agent / passthrough
- 为 Claude Code 的 plan 信号建立最小适配规则，包括 `metadata.plan_mode`、`tool_choice=ExitPlanMode` 及相关 plan 文本信号
- 在 Codex SSE → Anthropic SSE 响应转换里去掉 `<proposed_plan>...</proposed_plan>` 可见包装标签，但保留其中的计划正文
- 保持本次改动为方案 A 的最小适配：优先改善“先给 plan 再等确认”的体验，不实现完整 codex-cli 结构化 plan/approval 状态机
- 为 request / response 两侧补回归测试，覆盖 plan 检测与 proposed_plan 包装剥离

## Capabilities

### New Capabilities
- `codex-plan-mode-adapter`: 为 Claude Code → codexProxy → Codex API 的 plan mode 增加最小行为适配，确保计划先返回给用户并避免 `<proposed_plan>` 标签泄露到最终文本

### Modified Capabilities
- 无

## Impact

- 影响后端文件：`main/src/transform/codex/request.rs`、`main/src/transform/codex/response.rs`
- 影响测试文件：`main/src/transform/codex/request.rs` 内 request tests、`main/src/transform/codex/response/tests/text_hygiene.rs`
- 参考资料：`/Users/mr.j/myRoom/code/ai/codex-cli-source` 中 codex-cli plan mode 协议实现，以及代理日志 `/Users/mr.j/.codexProxy/logs/proxy_20260316_161702_副本.log`
- 兼容性：仅增强 plan mode 相关行为；非 plan 请求继续保持现有 agent / passthrough 逻辑
