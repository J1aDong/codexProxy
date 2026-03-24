## ADDED Requirements

### Requirement: Proxy-side tool interception SHALL be policy-driven
The system SHALL support policy-driven interception of selected tool calls at the proxy layer. Tool interception decisions MUST be based on a request-scoped router or policy component instead of being hardcoded into a single provider transformer implementation.

#### Scenario: A tool call matches an interception policy
- **WHEN** a provider response produces a tool invocation whose normalized tool name matches an enabled proxy interception policy
- **THEN** the proxy MUST route that invocation to the proxy-side tool execution path instead of immediately forwarding the raw tool call to the client

#### Scenario: A tool call does not match an interception policy
- **WHEN** a provider response produces a tool invocation that is not covered by an enabled interception policy
- **THEN** the proxy MUST preserve the normal tool passthrough behavior and forward the tool request using the standard Anthropic-compatible contract

### Requirement: Intercepted tools SHALL round-trip through canonical tool result semantics
When the proxy intercepts a tool invocation, it SHALL execute the selected tool using proxy-managed integrations and SHALL convert the result back into canonical tool result semantics before re-entering provider orchestration.

#### Scenario: Proxy-side web search completes successfully
- **WHEN** an intercepted `websearch` or `web_search` invocation completes through a proxy-managed integration
- **THEN** the proxy MUST create a canonical tool result payload that can be injected into the next model turn without requiring the client to execute the tool

#### Scenario: Proxy-side tool execution fails
- **WHEN** an intercepted tool invocation fails because of policy, transport, timeout, or upstream integration error
- **THEN** the proxy MUST return a deterministic failure outcome that is representable as either a canonical tool result error payload or an explicit fallback to normal passthrough behavior

### Requirement: Tool interception SHALL preserve provider independence
The system SHALL normalize intercepted tool invocations and results in a provider-independent shape so that the same proxy-side tool policy can be reused across Codex, OpenAI, Gemini, and Anthropic-compatible backends.

#### Scenario: Multiple providers emit equivalent web search requests
- **WHEN** different provider backends emit tool invocations that map to the same normalized web search capability
- **THEN** the proxy MUST evaluate the same interception policy and execution contract regardless of which provider produced the invocation
