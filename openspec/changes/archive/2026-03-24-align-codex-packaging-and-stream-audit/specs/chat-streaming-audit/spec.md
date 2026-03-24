## ADDED Requirements

### Requirement: OpenAI chat streaming event parity
The OpenAI Chat backend SHALL emit Anthropic-compatible streaming events with correct event ordering, message lifecycle boundaries, and stop signaling.

#### Scenario: Text response emits Anthropic event lifecycle
- **WHEN** an OpenAI chat completion upstream emits text deltas
- **THEN** the transformer SHALL emit `message_start`, one or more `content_block_*` text events, a `message_delta` with mapped stop reason when available, and a terminating `message_stop`

### Requirement: Tool call streaming fidelity
The OpenAI Chat backend SHALL preserve streamed tool call semantics, including tool start boundaries, incremental argument delivery, and completion ordering across multiple tool calls.

#### Scenario: Tool arguments streamed incrementally
- **WHEN** an upstream OpenAI chat completion emits `tool_calls[].function.arguments` in multiple deltas
- **THEN** the transformer SHALL emit Anthropic-compatible incremental tool input events without losing order or merging separate tool calls incorrectly

#### Scenario: Multiple tool calls remain distinct
- **WHEN** an upstream OpenAI chat completion emits more than one tool call in the same response
- **THEN** the transformer SHALL maintain distinct content block indexes and stop boundaries for each tool call

### Requirement: Thinking and reasoning mapping audit
The OpenAI Chat backend SHALL define and test how reasoning or thinking-style upstream fields are represented in downstream Anthropic-compatible streams.

#### Scenario: Reasoning content opens thinking block
- **WHEN** the upstream response includes reasoning or thinking content that the backend chooses to expose
- **THEN** the transformer SHALL emit a consistent Anthropic-compatible thinking block lifecycle and corresponding regression tests SHALL cover the behavior

### Requirement: Finish reason and termination mapping audit
The streaming adapters SHALL document and test how upstream finish reasons map to downstream stop reasons or termination markers, including tool-use completion, normal stop, length limits, refusal/content-filter cases.

#### Scenario: Tool completion maps to tool stop reason
- **WHEN** an upstream OpenAI chat completion finishes with `tool_calls`
- **THEN** the downstream Anthropic-compatible stream SHALL map that completion to the corresponding tool-use stop semantics before `message_stop`

#### Scenario: Refusal-like termination is observable
- **WHEN** an upstream completion ends in a refusal or content-filter condition
- **THEN** the adapter SHALL produce a deterministic downstream termination signal and logs/tests SHALL make the mapping auditable

### Requirement: Anthropic passthrough SSE integrity
The Anthropic backend SHALL preserve SSE event/data pairing and comment-style keepalive lines without reordering or collapsing events.

#### Scenario: Passthrough preserves event/data association
- **WHEN** the upstream Anthropic stream emits named events followed by data lines
- **THEN** the passthrough transformer SHALL forward the same event/data pairing and preserve SSE framing boundaries

### Requirement: OpenAI chat transformation boundary
The OpenAI Chat transformation layer in `codexProxy` SHALL remain focused on protocol conversion and SHALL NOT introduce vendor-specific prompt packaging mutations for downstream adapters such as CodeBuddy.

#### Scenario: Downstream-specific packaging remains out of scope
- **WHEN** a downstream adapter requires CodeBuddy CLI-style prompt, system, or tool packaging
- **THEN** `codexProxy` SHALL leave that packaging responsibility to the downstream adapter layer rather than rewriting the OpenAI Chat transform to emit CodeBuddy-specific envelopes

### Requirement: Streaming regression coverage
The codebase SHALL include regression tests for the audited streaming behaviors so that future changes to packaging or protocol conversion cannot silently break event semantics.

#### Scenario: Regression tests cover audited mappings
- **WHEN** streaming conversion code changes in the future
- **THEN** the test suite SHALL include assertions for text event ordering, tool call argument streaming, thinking mapping, finish reason mapping, and passthrough SSE integrity
