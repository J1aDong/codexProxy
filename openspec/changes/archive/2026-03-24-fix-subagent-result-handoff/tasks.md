## 1. Reproduce and isolate the handoff gap

- [x] 1.1 Add a focused regression fixture or test that reproduces the async subagent weather-query flow where one background agent later resolves to placeholder/progress text instead of a usable result.
- [x] 1.2 Trace the proxy path that handles ordinary background `Agent` launch metadata, later completion notifications, and any injected teammate/idle messages to identify where usable result content is lost or downgraded.

## 2. Normalize background subagent completion results

- [x] 2.1 Extend proxy-side message normalization so background-agent launch metadata and completion payloads are represented as distinct lifecycle states.
- [x] 2.2 Add explicit handling for completion payloads that contain usable result content, preserving subagent identity and result text in a structured handoff shape.
- [x] 2.3 Add explicit handling for placeholder/progress-only completions so they surface as incomplete or missing-result states instead of successful result delivery.

## 3. Guard downstream summarization and verification

- [x] 3.1 Ensure downstream logic cannot safely treat launch metadata or incomplete completion payloads as evidence that all subagents returned usable results.
- [x] 3.2 Add regression tests for both ordinary async agents and mailbox/team-style agents covering launch normalization, completion handoff, and incomplete-result behavior.
- [x] 3.3 Run the relevant transform/server test suites and confirm the reproduced subagent handoff regression is covered before closing the change.
