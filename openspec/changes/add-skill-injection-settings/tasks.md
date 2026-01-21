## 1. Frontend Implementation
- [ ] 1.1 Add "Settings" icon button to `header-actions` in `App.vue`.
- [ ] 1.2 Create a `SettingsDialog` component (or add to `App.vue` if simple) with a form.
- [ ] 1.3 Add "Skill Injection Strategy" section to the settings form.
- [ ] 1.4 Add a toggle/select for "Auto-Install Dependencies" and a custom prompt text area (advanced mode).
- [ ] 1.5 Default prompt: "If dependencies are missing, please install them first. Do not use workarounds. If issues persist, ask the user."
- [ ] 1.6 Persist these settings to local storage or backend config.

## 2. Backend Configuration Update
- [ ] 2.1 Update `ProxyConfig` struct (if exists, or the args for `start_proxy`) to accept `skill_injection_prompt`.
- [ ] 2.2 Update `ProxyServer` to store `skill_injection_prompt`.

## 3. Backend Logic Implementation
- [ ] 3.1 Pass `skill_injection_prompt` from `ProxyServer` to `handle_request`.
- [ ] 3.2 Pass `skill_injection_prompt` from `handle_request` to `TransformRequest::transform`.
- [ ] 3.3 In `TransformRequest::transform`, when injecting skills (around line 740), prepend/append the configured prompt to the system message or the skill message.
- [ ] 3.4 Ensure the prompt is only injected if skills are present.

## 4. Verification
- [ ] 4.1 Verify UI saves/loads settings.
- [ ] 4.2 Verify backend receives the new config on start/restart.
- [ ] 4.3 Verify the prompt appears in the final request to Codex when skills are used.
