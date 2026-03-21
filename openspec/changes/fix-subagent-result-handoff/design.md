## Context

The proxy already recognizes background-agent launch outputs and rewrites them into structured metadata objects such as `{"kind":"background_agent", ...}` in `main/src/transform/processor.rs`. It also rewrites invalid `TaskOutput` polling errors into guidance that mailbox-style or async background agents should be observed through `teammate-message`, `idle_notification`, or direct output-file inspection instead.

The reproduced weather-query flow shows a gap after launch: the main agent emitted a synthesized summary claiming all three subagents had completed, yet a later correction stated that the Beijing subagent had not actually returned a concrete forecast and had only replied with placeholder text like “准备去查”. This indicates the proxy currently preserves launch-time metadata better than completion-time payload integrity.

## Goals / Non-Goals

**Goals:**
- Preserve a clear lifecycle for ordinary background subagents: launched, still running, completed-with-result, completed-without-result, or failed.
- Ensure completion-time payloads carry enough structured content for the main agent to reuse without guessing or silently backfilling from unrelated context.
- Prevent the main agent from confidently summarizing background subagent work when the proxy only has placeholder/progress text or incomplete completion payloads.
- Add regression coverage for the reproduced async subagent handoff path.

**Non-Goals:**
- Redesign Agent Teams / team mailbox protocol semantics beyond what is required for result handoff.
- Change the user-facing semantics of ordinary subagent launches unrelated to completion/result delivery.
- Introduce new external storage or a persistent job queue.

## Decisions

### 1. Model background subagent launch and completion as separate normalized events
The proxy should continue rewriting launch outputs into structured `background_agent` metadata, but it must also normalize later completion notifications into a distinct structured result shape rather than relying on raw free-form text alone.

**Why:** launch metadata answers “how do I track this agent?”, while completion payloads answer “what useful work came back?”. Mixing those concerns makes it easy for the main agent to treat “completed” as equivalent to “usable result received”.

**Alternatives considered:**
- Keep launch-only normalization and trust later plain-text completions: rejected because the reproduced failure shows plain text can look complete while still lacking usable output.
- Force polling via `TaskOutput`: rejected because the current proxy guidance explicitly says ordinary mailbox/background agent IDs should not be polled that way.

### 2. Treat placeholder/progress-only completions as incomplete handoff, not success
If a background agent completion lacks substantive result content and only contains placeholder/progress text, the proxy should surface that state explicitly instead of letting downstream summarization treat it as a successful result.

**Why:** the root problem is not only that data is missing, but that the missing data is presented in a shape that looks final enough for the main agent to over-trust.

**Alternatives considered:**
- Allow the main agent to self-correct later: rejected because it produces contradictory answers and hides the real transport/state issue.
- Silently drop incomplete completions: rejected because the caller still needs visibility that the subagent finished without a usable result.

### 3. Keep the fix concentrated in proxy-side message normalization and regression tests
The first implementation should focus on the proxy’s transform/normalization layer and any completion-event bridging path that injects async agent results into the main conversation.

**Why:** the current evidence points to a handoff/normalization problem, and the existing background-agent logic already lives in the transform layer.

**Alternatives considered:**
- Rebuild the entire async agent orchestration flow: rejected as too large for the reproduced bug.
- Patch individual prompts or summarization heuristics only: rejected because the issue is structural, not prompt-specific.

## Risks / Trade-offs

- **[Risk]** Completion payload formats may differ between ordinary async agents and team/mailbox agents. → **Mitigation:** normalize both through explicit structured result shapes and cover each path with focused tests.
- **[Risk]** Over-classifying plain text as “placeholder” could hide legitimate terse completions. → **Mitigation:** base detection on explicit lifecycle metadata plus conservative placeholder/progress heuristics, and preserve raw text for inspection.
- **[Risk]** The real fault may span both transform logic and upstream stream stitching. → **Mitigation:** add regression tests around both message normalization and the higher-level handoff path so failures localize the true boundary.
