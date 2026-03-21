## Why

When Claude Code launches ordinary background subagents, the proxy can preserve the launch metadata but still lose or under-deliver the subagent's final usable result back to the main agent. In the reproduced weather query flow, the main agent stated that all three subagents had completed, but later had to correct itself because the Beijing subagent had only returned a placeholder like “准备去查” instead of a concrete forecast.

## What Changes

- Add a dedicated capability for background subagent result handoff so launch metadata and completion payloads are treated as separate, explicit lifecycle states.
- Define proxy behavior for ordinary async subagents so completion notifications MUST carry enough structured result data for the main agent to summarize or cite, or else surface an explicit incomplete/failed state instead of looking completed.
- Define safeguards so the proxy does not let the main agent claim that all subagents finished successfully when one or more background agents only produced progress text, placeholder text, or non-terminal metadata.
- Add regression coverage around the reproduced flow: background agent launch, later completion, and delivery of usable result content back into the main conversation.

## Capabilities

### New Capabilities
- `subagent-result-handoff`: Preserve and deliver usable completion results from background subagents back to the main agent with explicit lifecycle semantics.

### Modified Capabilities
- None.

## Impact

- Affected code will likely include background-agent tool result rewriting and message normalization in `main/src/transform/processor.rs` and any request/response translation path that injects teammate-message or idle/completion notifications back into the main conversation.
- Affected behavior includes async `Agent` tool launches (`run_in_background: true`), mailbox/team-style agent notifications, and any proxy-side normalization that currently turns agent launch output into structured metadata without guaranteeing later result delivery.
- No new external dependencies are expected.
