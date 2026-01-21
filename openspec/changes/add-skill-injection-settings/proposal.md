# Change: Add Skill Injection Settings

## Why
Currently, Skill instructions are injected with a default behavior (simply extracting and injecting text). However, users may want to configure how skills behave, specifically regarding dependency management (e.g., auto-install vs manual). Hardcoding these prompts in the Skill Markdown files is inflexible. The user requested a UI setting to configure the default injection strategy, specifically to instructing the agent to attempt auto-installation of dependencies.

## What Changes
- **Frontend**: Add a "Settings" button in the top-right corner of the app header.
- **Frontend**: Implement a Settings dialog containing a "Skill Injection" configuration section.
- **Frontend**: Add a text area or preset selector for "Default Skill Injection Prompt".
- **Backend**: Update the `start_proxy` command payload to include the skill injection config.
- **Backend**: Update `ProxyServer` struct to store this configuration.
- **Backend**: Update `TransformRequest` to use the configured injection prompt when processing skills.

## Impact
- **Affected specs**: `fronted-tauri` (UI), `codex-proxy` (backend logic)
- **Affected code**: `fronted-tauri/src/App.vue`, `main/src/server.rs`, `main/src/transform.rs`
- **Dependencies**: None.

## Risks
- **Prompt Injection Risk**: Users might input prompts that degrade model performance. We should provide a safe, proven default.
- **Complexity**: Passing config deep into `TransformRequest` might require refactoring how config is propagated.
