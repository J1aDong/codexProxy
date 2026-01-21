## Context
The user wants to control how the AI agent handles Skill dependencies (e.g., missing tools). Instead of modifying every Skill MD file, they want a global setting to inject a "meta-instruction" whenever a skill is used.

## Goals
- Allow users to configure a global "Skill Injection Prompt".
- Provide a default prompt that encourages auto-installation of dependencies.
- Persist this setting across sessions.

## Decisions
- **UI Location**: A generic "Settings" button in the header is better than cluttering the main dashboard. This allows for future expansion.
- **Config Propagation**: The frontend passes config to the backend via `start_proxy`. We will extend this payload. This avoids needing a separate dynamic config update mechanism during runtime (restart required to apply changes is acceptable for now, or dynamic if easy).
- **Injection Strategy**: The prompt will be added as a separate `system` or `user` message alongside the injected skills, or appended to the skill description. 
  - *Decision*: Append as a separate context block or instruction in the `AGENTS.md` injection, or a standalone message. Since `AGENTS.md` is constructed in `transform.rs`, we can append it there.
  - Actually, `transform.rs` injects skills as `user` messages. We can add this instruction as a `user` message *after* the skills are injected, to reinforce the instruction.

## Data Flow
1. User saves settings in Vue (persisted via `tauri-plugin-store` or simple file IO if already used, currently `load_config`/`save_config` commands seem to exist).
2. User clicks "Start Proxy".
3. Frontend calls `start_proxy` with `config` object.
4. Rust backend initializes `ProxyServer` with this config.
5. On request, `ProxyServer` passes config to `TransformRequest`.
6. `TransformRequest` uses config to format output.

## Risks
- If the prompt is too long, it consumes context window.
- If the prompt contradicts specific skill instructions, model might be confused. (User responsibility).
