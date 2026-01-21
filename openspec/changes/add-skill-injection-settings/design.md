## Context
The user wants to control how the AI agent handles Skill dependencies (e.g., missing tools). Instead of modifying every Skill MD file, they want a global setting to inject a "meta-instruction" whenever a skill is used.

## Goals
- Allow users to configure a global "Skill Injection Prompt".
- Provide a default prompt that encourages auto-installation of dependencies.
- Persist this setting via existing `proxy-config.json` mechanism.

## Decisions
- **Architecture**: Leverage existing `start_proxy` command which already accepts a `ProxyConfig` object and handles persistence (`save_config`).
  - *Implication*: Changing the setting requires restarting the proxy to take effect (since config is passed at start time). This is acceptable for an MVP.
- **UI Location**: A generic "Settings" button in the header is better than cluttering the main dashboard. This allows for future expansion.
- **Data Flow**:
  1. Frontend (Vue) maintains the `skillInjectionPrompt` in its reactive state.
  2. `save_config` (backend) stores it in JSON.
  3. `start_proxy` receives the full config, initializes `ProxyServer`.
  4. `ProxyServer` passes the prompt string to `TransformRequest`.
  5. `TransformRequest` injects the prompt if and only if skills are present in the request.
- **Injection Strategy**:
  - The prompt will be injected as a **User** message.
  - Position: **After** the injected skills. This ensures the instruction ("if dependencies are missing...") overrides or contextualizes the skills provided before it.
  - Format: A standalone user message block.
- **Defaults & Localization**:
  - The default prompt will adapt to the UI language (Chinese/English).
  - If the user clears the input, no prompt is injected.
- **Constraints**:
  - Max length: 500 characters to prevent context bloat.

## Risks
- **Prompt Injection**: Users might input prompts that degrade model performance. We should provide a safe, proven default.
- **Context Window**: Long prompts reduce available tokens. Length limit mitigates this.
