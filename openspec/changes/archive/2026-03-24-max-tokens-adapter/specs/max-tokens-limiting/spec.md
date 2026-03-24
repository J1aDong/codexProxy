## ADDED Requirements

### Requirement: Max tokens limiting for OpenAI Chat API

The OpenAI Chat adapter SHALL support per-slot configuration of `max_tokens` limiting. When a slot has a configured limit, the adapter SHALL apply that limit to the request.

#### Scenario: Configured limit is lower than request max_tokens
- **WHEN** an Anthropic request has `max_tokens` value greater than the configured limit for the target slot
- **THEN** the OpenAI request SHALL have `max_tokens` reduced to the configured limit

#### Scenario: Configured limit is higher than request max_tokens
- **WHEN** an Anthropic request has `max_tokens` value less than or equal to the configured limit for the target slot
- **THEN** the OpenAI request SHALL preserve the original `max_tokens` value

#### Scenario: No limit configured (pass-through)
- **WHEN** no `max_tokens` limit is configured for the target slot (null/None)
- **THEN** the OpenAI request SHALL preserve the original `max_tokens` value from the Anthropic request

#### Scenario: Max tokens not specified in request
- **WHEN** an Anthropic request does not specify `max_tokens`
- **THEN** the OpenAI request SHALL NOT include `max_tokens` parameter, regardless of configuration

### Requirement: Per-slot max tokens configuration

The system SHALL support configuration of `max_tokens` limits per slot (Opus/Sonnet/Haiku).

#### Scenario: Opus slot limit configured
- **WHEN** the Opus slot has a configured `max_tokens` value
- **AND** a request is mapped to the Opus slot
- **THEN** the system SHALL apply the Opus slot limit

#### Scenario: Sonnet slot limit configured
- **WHEN** the Sonnet slot has a configured `max_tokens` value
- **AND** a request is mapped to the Sonnet slot
- **THEN** the system SHALL apply the Sonnet slot limit

#### Scenario: Haiku slot limit configured
- **WHEN** the Haiku slot has a configured `max_tokens` value
- **AND** a request is mapped to the Haiku slot
- **THEN** the system SHALL apply the Haiku slot limit

#### Scenario: Slot limit is null (pass-through)
- **WHEN** a slot has no configured `max_tokens` value (null/None)
- **THEN** the system SHALL NOT apply any limiting for that slot

### Requirement: Configuration persistence per endpoint

The `max_tokens` configuration SHALL be stored per endpoint, similar to model mapping.

#### Scenario: Configuration saved with endpoint
- **WHEN** a user configures `max_tokens` limits for an endpoint
- **THEN** the configuration SHALL be saved as part of the endpoint options
- **AND** the configuration SHALL persist across application restarts

### Requirement: UI for max tokens configuration

The UI SHALL provide input fields for configuring `max_tokens` limits per slot.

#### Scenario: Display max tokens inputs
- **WHEN** the user views the OpenAI Chat configuration
- **THEN** the UI SHALL display three input fields for Opus/Sonnet/Haiku max tokens
- **AND** each field SHALL accept numeric values or be left empty

#### Scenario: Empty input means pass-through
- **WHEN** a max tokens input field is left empty
- **THEN** the UI SHALL save the value as null (pass-through)
- **AND** the system SHALL NOT apply limiting for that slot

### Requirement: Logging of max tokens limiting

The system SHALL log when `max_tokens` is limited due to configuration.

#### Scenario: Max tokens limited with logging
- **WHEN** `max_tokens` is reduced due to exceeding the configured limit
- **THEN** the system SHALL log a message indicating the original value, the configured limit, and the effective value
