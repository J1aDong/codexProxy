## ADDED Requirements

### Requirement: Anthropic canonical model SHALL be the only transformer baseline
The system SHALL treat Anthropic Messages request/response semantics as the only canonical model inside the transformer layer. Every provider backend MUST accept Anthropic-style input as its upstream-independent source contract, and every provider response path MUST normalize streaming and non-stream output back into Anthropic-compatible semantics before returning to clients.

#### Scenario: Anthropic request enters a non-Anthropic backend
- **WHEN** the server dispatches a `/v1/messages` request to a non-Anthropic provider backend
- **THEN** the backend MUST receive the original Anthropic request contract as input and perform provider-specific mapping from that canonical model instead of from a provider-specific intermediate schema

#### Scenario: Provider streaming events are returned to the client
- **WHEN** a provider backend emits streaming chunks from an upstream service
- **THEN** the response path MUST normalize those chunks into Anthropic-compatible SSE event semantics before writing them to the client response

### Requirement: Transformer backends SHALL expose stable override boundaries
The system SHALL organize each provider transformer around stable override boundaries for request mapping, upstream request building, and response transformation. Provider-specific behavior MUST be implemented within those override boundaries, while shared utilities MUST remain reusable and upstream-agnostic.

#### Scenario: Anthropic passthrough is used as the identity baseline
- **WHEN** the selected provider backend is the Anthropic passthrough path
- **THEN** request serialization and SSE event handling MUST preserve canonical Anthropic semantics with only minimal transport-level override such as model override or header normalization

#### Scenario: Codex or OpenAI custom behavior is introduced
- **WHEN** a provider backend needs custom request or response handling for its upstream protocol
- **THEN** that behavior MUST be implemented in request-mapping or response-transformation override points rather than by duplicating the whole forwarding pipeline

### Requirement: Shared transformer helpers SHALL remain transport-agnostic and stateless by default
The system SHALL keep reusable transformer helpers as stateless, transport-agnostic utilities unless a stateful streaming lifecycle is strictly required. Helper responsibilities such as content block cleanup, system text flattening, SSE frame parsing, tool payload normalization, and usage normalization MUST NOT become hidden provider-specific orchestration layers.

#### Scenario: A utility is reused by multiple providers
- **WHEN** a helper is used across two or more provider backends
- **THEN** the helper MUST accept canonical or explicitly documented inputs and MUST NOT require provider-specific mutable state to operate

#### Scenario: Stateful stream processing is required
- **WHEN** a transformation step depends on request-local streaming state
- **THEN** that state MUST remain in the response transformer or request-scoped orchestration component instead of being stored in a generic shared helper
