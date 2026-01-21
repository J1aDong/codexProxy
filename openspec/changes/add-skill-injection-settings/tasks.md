## 1. Backend: Data Structure & Persistence
- [x] 1.1 Update `ProxyConfig` struct in `fronted-tauri/src-tauri/src/proxy.rs` to include `skill_injection_prompt` (Option<String> or String).
- [x] 1.2 Verify `load_config` and `save_config` automatically handle the new field via Serde.
- [x] 1.3 Update `ProxyServer` struct in `main/src/server.rs` to store `skill_injection_prompt`.
- [x] 1.4 Update `start_proxy` in `proxy.rs` to initialize `ProxyServer` with the prompt from `ProxyConfig`.

## 2. Backend: Injection Logic
- [x] 2.1 Update `handle_request` in `main/src/server.rs` to pass `skill_injection_prompt` to `TransformRequest::transform`.
- [x] 2.2 Update `TransformRequest::transform` signature in `main/src/transform.rs` to accept the prompt.
- [x] 2.3 Implement injection logic in `TransformRequest::transform`:
    - Check if `extracted_skills` is not empty.
    - If yes, and `skill_injection_prompt` is not empty, create a new User message containing the prompt.
    - Append this message **after** the skill injection messages.

## 3. Frontend: Settings UI
- [x] 3.1 Update `DEFAULT_CONFIG` in `App.vue` to include `skillInjectionPrompt`.
- [x] 3.2 Add "Settings" icon button (gear icon) to `header-actions`.
- [x] 3.3 Create a Settings Dialog (`el-dialog`) or repurpose an existing one.
- [x] 3.4 Add form item for "Skill Injection Prompt" (textarea).
- [x] 3.5 Implement "Reset to Default" logic for this field (handling zh/en defaults).
- [x] 3.6 Add input validation (max length 500 chars).
- [x] 3.7 Ensure `toggleProxy` sends the updated config to backend.

## 4. Verification & Testing
- [x] 4.1 Unit Test: Update `transform.rs` tests to verify prompt injection occurs only when skills are present and follows the correct order.
- [x] 4.2 Manual Verification: Configure a custom prompt, start proxy, trigger a skill, and verify logs show the injected prompt.
- [x] 4.3 Persistence Verification: Restart app and ensure custom prompt is loaded.
