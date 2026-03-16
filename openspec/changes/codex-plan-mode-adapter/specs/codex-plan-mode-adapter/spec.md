## ADDED Requirements

### Requirement: Plan mode requests use dedicated Codex plan augmentation
The Codex request adapter SHALL detect Claude Code plan mode turns and build the outbound Codex request using a dedicated plan augmentation path that prefers proposing a plan before execution.

#### Scenario: metadata plan_mode triggers plan augmentation
- **WHEN** an Anthropic request contains `metadata.plan_mode = true`
- **THEN** the request augmentation SHALL use the plan path instead of generic passthrough
- **AND** the adapter SHALL preserve a plan detection reason for diagnostics and regression tests

#### Scenario: ExitPlanMode tool choice triggers plan augmentation
- **WHEN** an Anthropic request selects `tool_choice.name = "ExitPlanMode"`
- **THEN** the request augmentation SHALL use the plan path
- **AND** the outbound Codex request SHALL prefer returning a plan before execution

#### Scenario: plan approval signal keeps the request on the plan path
- **WHEN** the request text or tool schema contains `plan_approval_response`
- **THEN** the request augmentation SHALL use the plan path

### Requirement: Proposed plan wrapper tags stay hidden from visible text
The Codex response adapter SHALL remove `<proposed_plan>` wrapper tags from visible Anthropic text while preserving the wrapped plan body.

#### Scenario: single text chunk contains a proposed plan wrapper
- **WHEN** upstream text contains `<proposed_plan>计划正文</proposed_plan>` in a visible text fragment
- **THEN** the client-visible Anthropic text SHALL include `计划正文`
- **AND** the client-visible Anthropic text SHALL NOT include `<proposed_plan>` or `</proposed_plan>`

#### Scenario: wrapper tags are split across multiple text chunks
- **WHEN** upstream visible text emits the opening or closing proposed plan wrapper across multiple text fragments
- **THEN** the adapter SHALL still hide the wrapper tags
- **AND** the wrapped plan body SHALL remain visible in the original order

### Requirement: Non-plan Codex traffic remains on existing behavior
The adapter SHALL preserve existing behavior for requests and responses that do not carry plan mode signals.

#### Scenario: non-plan request keeps existing augmentation behavior
- **WHEN** an Anthropic request does not contain plan mode signals
- **THEN** request augmentation SHALL continue to use the existing agent or passthrough logic

#### Scenario: ordinary visible text is not modified by plan hygiene
- **WHEN** upstream visible text does not contain `<proposed_plan>` wrapper tags
- **THEN** the Anthropic text stream SHALL be emitted unchanged except for pre-existing hygiene rules
