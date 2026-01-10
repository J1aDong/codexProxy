## ADDED Requirements

### Requirement: Context Injection Conversion
The proxy SHALL convert Claude Code's system context to Codex's AGENTS.md format.

#### Scenario: System field to AGENTS.md conversion
- **WHEN** a Claude Code request contains a `system` field
- **THEN** the proxy SHALL convert it to AGENTS.md format with proper header
- **AND** wrap the content in `<INSTRUCTIONS>...</INSTRUCTIONS>` tags
- **AND** inject it as the first user message in the `input` array

#### Scenario: Environment context injection
- **WHEN** converting a Claude Code request to Codex format
- **THEN** the proxy SHALL inject an `<environment_context>` message
- **AND** include cwd, approval_policy, sandbox_mode, network_access, and shell information
- **AND** place it as the second user message in the `input` array

### Requirement: Skill Tool Conversion
The proxy SHALL convert Claude Code `Skill` tool calls to Codex's context injection format.

#### Scenario: Skill tool call interception
- **WHEN** a request contains a `Skill` tool call
- **THEN** the proxy SHALL intercept the tool call
- **AND** read the specified SKILL.md file from the local filesystem
- **AND** validate the file path to prevent path traversal attacks

#### Scenario: Skill content injection
- **WHEN** a Skill file is successfully read
- **THEN** the proxy SHALL wrap the content in `<skill>` tags
- **AND** include the skill name and file path in the wrapper
- **AND** inject it as a user message after the user's original message

#### Scenario: Skill tool response
- **WHEN** a Skill tool call is processed
- **THEN** the proxy SHALL return a success response to Claude Code
- **AND** the response SHALL indicate the skill was loaded successfully

### Requirement: Input Message Ordering
The proxy SHALL maintain correct ordering of input messages.

#### Scenario: Standard message ordering
- **WHEN** constructing the Codex request input array
- **THEN** the messages SHALL be ordered as follows:
  1. AGENTS.md content (converted from system field)
  2. environment_context
  3. User's original message
  4. Skill content (if triggered)
  5. Conversation history (if any)

### Requirement: Instructions Field Preservation
The proxy SHALL preserve the Codex instructions field unchanged.

#### Scenario: Instructions field handling
- **WHEN** converting to Codex format
- **THEN** the `instructions` field SHALL use the standard Codex template
- **AND** custom instructions SHALL NOT be placed in the `instructions` field
- **AND** all custom content SHALL be injected via `input` messages only

### Requirement: Path Security
The proxy SHALL validate all file paths to prevent security vulnerabilities.

#### Scenario: Path traversal prevention
- **WHEN** a Skill tool call specifies a file path
- **THEN** the proxy SHALL validate the path does not contain `..` sequences
- **AND** the proxy SHALL validate the path is within allowed directories
- **AND** reject requests with invalid paths with an appropriate error message
