# Findings & Decisions

## Requirements
- Audit the repository's OpenAI Chat Completions conversion layer.
- Compare Anthropic protocol vs OpenAI protocol.
- Focus on streaming/SSE behavior and event mapping.
- Search the web for relevant open-source compatibility/adapter projects.
- Produce a concise gap report with priorities.
- Do not make code changes in this pass.

## Research Findings
- User approved a team-based analysis workflow.
- Planning files did not already exist in the repository root.
- One unrelated planning file exists at `fronted-tauri/src-tauri/task_plan.md`.
- Initial planning templates were reviewed to shape task-specific tracking files.
- Core implementation appears centered in `main/src/transform/openai.rs`.
- `main/src/transform/openai.rs` contains both request conversion (`/v1/chat/completions`) and SSE response transformation logic.
- `main/src/transform/anthropic.rs` appears to handle Anthropic passthrough response transformation rather than OpenAI compatibility synthesis.
- `main/src/server/stream_decision.rs` also references Anthropic streaming event handling and may contain additional edge-case logic worth checking.
- `main/src/transform/openai.rs` request-side mapping builds OpenAI chat messages from transformed Codex-style items, including assistant `tool_calls` and tool-role outputs.
- Content block conversion currently handles text and `input_image` → `image_url`, but skips `thinking` input blocks and appears not to handle other multimodal block types in this layer.
- Request-side function calls are merged into the last assistant message when possible, with `content` forced to an empty string if missing.
- System prompt handling flattens Anthropic system content into a single OpenAI `system` message and appends any custom injection prompt.
- Request building currently hardcodes `stream: true` and `stream_options.include_usage: true` in the OpenAI body.
- Anthropic request model defines `stream` with default `false`, so this backend currently overrides caller intent at the protocol boundary.
- Request-side optional mappings currently observed in `openai.rs`: `max_tokens`, `temperature`, `top_p`, `stop` (from `stop_sequences`), and `tools`.
- Fields present on Anthropic-side models but not observed being mapped in `openai.rs` request body include at least `tool_choice`, `thinking`, and `top_k`; official OpenAI docs also show relevant parameters `parallel_tool_calls`, `response_format` (`text`, `json_schema`, `json_object`), and deprecated `seed`, none of which are mapped here.
- Server-level helper `disable_parallel_tool_calls_in_upstream_body()` only rewrites the body if `parallel_tool_calls` already exists and is `true`; since `openai.rs` does not appear to set that field, this fallback likely does not help the OpenAI converter path.
- `MessageProcessor` only processes `user` and `assistant` roles and skips `system` plus other roles before `openai.rs` builds OpenAI messages.
- `MessageProcessor` preserves `thinking` blocks in its intermediate Codex-style representation, but `openai.rs` later drops those blocks on request conversion, creating an internal/external semantics gap.
- `MessageProcessor` adds an image hint and only emits image inputs for `user` messages, which means non-user image-like content is silently ignored in this transformation path.
- `MessageProcessor` converts `ToolUse` / `ToolResult` blocks into top-level `function_call` / `function_call_output` items before `openai.rs` builds final OpenAI messages.
- `MessageProcessor` downgrades `Document` blocks to plain text (`[document omitted]`), and serializes unknown block types into JSON text, so document/unknown input semantics are also lost on this path.
- SSE response transformation in `main/src/transform/openai.rs` is OpenAI chat-completions chunk → Anthropic SSE, not the reverse direction.
- The SSE transformer emits `message_start`, opens/closes text or thinking blocks, accumulates tool call state by `index`, emits `input_json_delta` for argument fragments, stores usage from OpenAI chunks, and delays `message_stop` until `data: [DONE]`.
- Finish-reason mapping currently includes: `tool_calls` → `tool_use`, `length` → `max_tokens`, `content_filter`/`refusal` → `refusal`, `stop` → `end_turn`, fallback unknown → `end_turn`.
- The OpenAI streaming docs document streamed `delta` fields including `role`, `content`, deprecated `function_call`, `refusal`, and `tool_calls`. Current transformer only handles `reasoning_content` (nonstandard/provider-specific), `content`, and `tool_calls`, so official `refusal` and deprecated `function_call` deltas are currently ignored.
- The OpenAI backend's `create_response_transformer()` ignores its `allow_visible_thinking` input, so request-level thinking visibility policy is not enforced in this backend.
- The transformer emits Anthropic `message_delta` with `stop_reason` but not `stop_sequence`, although Anthropic streaming examples show both fields at the message level.
- The transformer opens Anthropic thinking blocks from `reasoning_content`, but Anthropic official streaming semantics also include `signature_delta`; there is no signature handling in this backend.
- `stream_decision.rs` applies a second-stage output policy after transformation: it suppresses duplicate `message_start`, suppresses premature `message_stop`, and drops business output after `message_stop`.
- Stream retry/guard behavior is sensitive to whether the stream has emitted tool events, business events, response completion/failure markers, and whether a serial fallback is available.
- Official Anthropic streaming docs were fetched successfully. Key documented event families include `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`, plus `ping` and `error`.
- Anthropic docs explicitly describe incremental tool JSON via `input_json_delta`, and thinking-related deltas including `thinking_delta` and `signature_delta`.
- Direct fetch of the OpenAI Chat Completions docs returned HTTP 403 via one fetch path, so OpenAI semantics were cross-checked via official search/open and official SDK source.
- Official OpenAI Chat Completions docs confirm `stream_options.include_usage`: if set, an extra chunk is streamed before `data: [DONE]`, that extra chunk has empty `choices`, and all earlier chunks include `usage: null`.
- Official OpenAI Chat Completions docs confirm request-side parameters relevant to compatibility include `tool_choice`, `parallel_tool_calls`, `response_format` (`text`, `json_schema`, `json_object`), and deprecated `seed`.
- Official OpenAI streaming docs define streamed objects as `chat.completion.chunk` and document `choices[].delta` fields including `role`, `content`, deprecated `function_call`, `refusal`, and `tool_calls`.
- Official OpenAI streaming docs require streamed tool-call deltas to carry `tool_calls[].index`, and document finish reasons including `stop`, `length`, `tool_calls`, `content_filter`, and deprecated `function_call`.
- Official OpenAI Python SDK streaming helper code relies on `choice.delta.tool_calls[*].index` and reconstructs higher-level events from incremental argument fragments, so index stability and argument-fragment fidelity are client-visible compatibility requirements.
- LiteLLM official repo issues/PRs show recurring Anthropic↔OpenAI compatibility failures around streamed tool calls, including spurious `role: assistant` on tool-call chunks, dropped `tool_calls` in multi-output streaming, blank/partial tool arguments, and Anthropic tool protocol mismatches.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Use repository audit + external research in parallel | Minimizes blind spots and speeds up cross-checking |
| Use official protocol docs and official project repos as primary sources | Required for technical accuracy on protocol semantics |
| Treat streaming/tool-call semantics as the primary risk area | Small mismatches here often break SDKs and frontends |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Invalid `pages` parameter passed to `Read` while loading templates | Corrected by using standard text-file reads |
| Direct fetch of OpenAI docs returned 403 | Fall back to official search results and official repos/specs |
| Full-file Edit replacement became brittle as findings evolved | Switched to overwriting the file with `Write` when necessary |

## Resources
- Repository root: `/Users/mr.j/myRoom/code/ai/MyProjects/codexProxy`
- Team: `protocol-audit-team`
- Planning templates: `/Users/mr.j/.claude/plugins/cache/planning-with-files/planning-with-files/2.11.0/templates/`
- Likely internal audit targets: `main/src/transform/openai.rs`, `main/src/transform/anthropic.rs`, `main/src/server/stream_decision.rs`
- Anthropic streaming docs fetch artifact: `/Users/mr.j/.claude/projects/-Users-mr-j-myRoom-code-ai-MyProjects-codexProxy/675e1729-69b2-442d-ba21-30658cf9973d/tool-results/call_jRiRGMuPeJ9oZoyAS8A0nVKY.txt`
- OpenAI official sources consulted: API reference/search-open pages plus official `openai-python` streaming helper/source files
- Representative adapter evidence gathered from LiteLLM official GitHub issues/PRs

## Visual/Browser Findings
- Official Anthropic streaming docs confirm named SSE event types and documented tool/thinking delta subtypes.
- Official OpenAI docs confirm the final include-usage chunk has empty `choices`, while streamed tool-call deltas carry `index` and may arrive incrementally.
- LiteLLM issue history suggests real-world clients break on subtle streaming mismatches, especially around role emission, tool-call fragmentation, and mixed text/tool output.

---
*Update this file after every 2 view/browser/search operations*
*This prevents visual information from being lost*
