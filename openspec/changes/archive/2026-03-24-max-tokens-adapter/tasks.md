## 1. Backend Configuration

- [x] 1.1 Add `OpenAIMaxTokensMapping` struct to `main/src/models/common.rs`
- [x] 1.2 Add `openai_max_tokens_mapping` field to `TransformContext` in `main/src/transform/mod.rs`
- [x] 1.3 Update `EndpointOption` and related config types to include max tokens mapping

## 2. Backend Core Implementation

- [x] 2.1 Add helper method to resolve slot from model name in `OpenAIChatBackend`
- [x] 2.2 Modify `OpenAIChatBackend::transform_request` to apply max tokens limiting based on slot configuration
- [x] 2.3 Add logging when `max_tokens` is reduced due to configured limit

## 3. Frontend Types

- [x] 3.1 Add `OpenAIMaxTokensMapping` interface to `fronted-tauri/src/types/configTypes.ts`
- [x] 3.2 Add `openaiMaxTokensMapping` field to `EndpointOption` interface
- [x] 3.3 Update default config values

## 4. Frontend UI

- [x] 4.1 Add three number input fields for Opus/Sonnet/Haiku max tokens in OpenAI Chat config section
- [x] 4.2 Handle empty input → null conversion
- [x] 4.3 Add label/tooltip: "留空则透传 Claude Code 传入的值"
- [x] 4.4 Style inputs to match existing model mapping inputs

## 5. Integration

- [x] 5.1 Update Tauri command handlers to pass max tokens mapping
- [x] 5.2 Ensure config save/load includes max tokens mapping
- [x] 5.3 Test end-to-end: configure → save → reload → apply limiting

## 6. Testing

- [x] 6.1 Add unit tests for `max_tokens` limiting logic in `transform/openai.rs`
- [x] 6.2 Test scenario: configured limit lower than request → reduced to limit
- [x] 6.3 Test scenario: configured limit higher than request → preserved
- [x] 6.4 Test scenario: no configured limit (null) → preserved (pass-through)
- [x] 6.5 Test scenario: request without max_tokens → not included in output

## 7. Documentation

- [x] 7.1 Update AGENTS.md with new configuration option
- [x] 7.2 Add inline code comments explaining the limiting logic
