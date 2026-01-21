## ADDED Requirements
### Requirement: Skill Injection Configuration
The system SHALL allow users to configure a custom instruction prompt that is injected whenever Skills are used.

#### Scenario: User configures auto-install strategy
- **WHEN** the user opens Settings and enters "Always auto-install dependencies" in the Skill Injection Prompt field
- **AND** starts the proxy
- **AND** a request contains a Skill tool use
- **THEN** the system injects the Skill content AND the custom prompt "Always auto-install dependencies" into the request to Codex
- **AND** the custom prompt is injected as a USER message AFTER the skills.

#### Scenario: Default configuration (Chinese)
- **WHEN** the user has not configured a custom prompt (or resets to default)
- **AND** the app language is Chinese (zh)
- **THEN** the system uses the default prompt: "skills里的技能如果需要依赖，先安装，不要先用其他方案，如果还有问题告知用户解决方案让用户选择".

#### Scenario: Default configuration (English)
- **WHEN** the user has not configured a custom prompt
- **AND** the app language is English (en)
- **THEN** the system uses the default prompt: "If skills require dependencies, install them first. Do not use workarounds. If issues persist, provide solutions for the user to choose."

#### Scenario: Empty configuration
- **WHEN** the user clears the Skill Injection Prompt (empty string)
- **THEN** the system does NOT inject any additional prompt message for skills.

#### Scenario: Length Validation
- **WHEN** the user attempts to enter a prompt longer than 500 characters
- **THEN** the system SHALL block the input or truncate it (or provide a validation error).
