## ADDED Requirements
### Requirement: Skill Injection Configuration
The system SHALL allow users to configure a custom instruction prompt that is injected whenever Skills are used.

#### Scenario: User configures auto-install strategy
- **WHEN** the user opens Settings and enters "Always auto-install dependencies" in the Skill Injection Prompt field
- **AND** starts the proxy
- **AND** a request contains a Skill tool use
- **THEN** the system injects the Skill content AND the custom prompt "Always auto-install dependencies" into the request to Codex.

#### Scenario: Default configuration
- **WHEN** the user has not configured a custom prompt
- **THEN** the system uses the default prompt: "skills里的技能如果需要依赖，先安装，不要先用其他方案，如果还有问题告知用户解决方案让用户选择" (or English equivalent depending on lang).
