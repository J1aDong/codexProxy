## 1. Streaming audit baseline

- [x] 1.1 Audit `main/src/transform/openai.rs` against `deep-research-report.md` and list the concrete gaps in event ordering, thinking mapping, tool-call streaming, finish-reason mapping, usage handling, and error termination.
- [x] 1.2 Audit `main/src/transform/anthropic.rs` passthrough framing to verify `event/data` pairing, comment keepalive forwarding, and termination behavior.

## 2. Protocol conversion fixes

- [x] 2.1 Implement the minimal `openai chat` streaming fixes required by the audit without introducing CodeBuddy-specific prompt packaging logic.
- [x] 2.2 Implement any required Anthropic passthrough SSE integrity fixes discovered by the audit.
- [x] 2.3 Add or adjust logs/comments where needed so finish-reason and termination mappings are auditable during debugging.

## 3. Regression coverage

- [x] 3.1 Expand OpenAI chat streaming tests to cover text event ordering, multiple tool calls, incremental arguments, thinking blocks, refusal/content-filter termination, and usage/ending behavior where applicable.
- [x] 3.2 Add or update Anthropic passthrough tests to cover event/data association and keepalive forwarding.
- [x] 3.3 Add a regression assertion that `codexProxy`'s `openai chat` transform does not take on downstream-specific CodeBuddy packaging responsibilities.
