## 执行顺序

- **Phase 1（先做）**：完成第 1～5 节，对应 P0 / P1。
- **Phase 2（后做）**：完成第 6 节，对应 P2。
- 实施时先收敛高优先级协议风险，再推进更广语义保真与端到端增强。

## 1. 请求兼容性修正（P0 / P1）

- [x] 1.1 审查并调整 `main/src/transform/openai.rs` 的请求构造逻辑，明确区分调用方 `stream=true/false` 时的上游请求语义
- [x] 1.2 在 OpenAI 请求体中补齐 `tool_choice` 映射，并为不支持的上游定义明确的降级或报错策略
- [x] 1.3 在 OpenAI 请求体中补齐 `parallel_tool_calls` 控制，并确保现有串行 fallback 逻辑能够实际生效
- [x] 1.4 为 `response_format`、`seed`、`top_k`、`metadata` 等 P1 参数定义处理策略（映射、显式忽略或显式报错）
- [x] 1.5 调整请求侧 thinking 可见性策略，确保请求级意图能传递到响应转换阶段

## 2. 流式生命周期与 delta 兼容性修正（P0 / P1）

- [x] 2.1 调整 OpenAI chunk → Anthropic SSE 转换逻辑，避免在无有效 `choices[0]` 内容时过早发出 `message_start`
- [x] 2.2 补充对 deprecated `function_call` 流式增量的兼容处理，并保持与 `tool_calls` 的行为一致性
- [x] 2.3 补充对 `refusal` 相关 delta / finish 语义的显式下游映射
- [x] 2.4 让 `allow_visible_thinking` 在 OpenAI backend response transformer 中真正生效
- [x] 2.5 澄清通用 OpenAI Chat 路径下的 thinking 完整性边界，并明确 `stop_sequence` 的下游行为
  - Phase 1：已支持 `thinking_delta` 与 `stop_sequence`
  - Phase 1：不默认支持 `signature_delta`
  - 后续仅在存在稳定 provider-specific 字段约定时，再单独实现 signature 桥接

## 3. 工具参数收口与聚合路径修正（P0 / P1）

- [x] 3.1 审查 `main/src/server.rs` 的非流聚合路径，移除损坏 tool 参数静默收口为 `{}` 的行为
- [x] 3.2 为工具参数损坏场景设计显式错误或显式降级输出，保证问题可观测
- [x] 3.3 校验多工具并发、参数分片交错、usage-only chunk 与 `[DONE]` 收尾在聚合路径中的一致性

## 4. 输入语义一致性收敛（P1）

- [x] 4.1 明确 `thinking` 输入块在本次 P1 范围内的处理策略，并使请求/响应两侧行为一致
- [x] 4.2 明确当前版本对无法完整保真的输入块采用何种显式降级或跳过策略，并把行为固定为可测试结果

## 5. 回归测试与验收（P0 / P1）

- [x] 5.1 为请求侧新增测试：`tool_choice`、`parallel_tool_calls`、stream 语义、参数降级策略
- [x] 5.2 为流式转换新增测试：无 `choices` chunk、旧式 `function_call`、`refusal`、thinking 可见性、usage-only chunk
- [x] 5.3 为聚合路径新增测试：损坏 tool 参数、多工具交错、非流聚合错误路径
- [x] 5.4 运行现有相关测试并修复回归，确认基础文本流、单工具、多工具、非流聚合仍然正确

## 6. 第二阶段增强（P2，延后执行）

- [ ] 6.1 为 `Document` 与 unknown block 设计结构化降级或透传策略，至少保留类型标记与关键元数据
- [ ] 6.2 提升 system block fidelity，减少当前简单扁平化带来的语义损失
- [ ] 6.3 扩展非 user 图像输入与更广多模态输入的处理策略，消除静默忽略路径
- [ ] 6.4 为 transform 层与 `main/src/server/stream_decision.rs` 之间的协同行为补充端到端一致性回归测试
- [ ] 6.5 在完成 P0/P1 稳定后，重新评估 P2 是否继续留在当前 change 内执行，或拆分为独立后续 change
