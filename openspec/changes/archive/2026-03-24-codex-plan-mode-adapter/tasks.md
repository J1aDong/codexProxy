## 1. Test foundation and red cases

- [x] 1.1 Repair test-only compile blockers needed for this change, including missing request test imports and stale `TransformContext` initializers
- [x] 1.2 Add and run a failing request-side regression test proving Claude Code plan signals select the dedicated plan augmentation path
- [x] 1.3 Add and run failing response-side regression tests proving `<proposed_plan>` wrapper tags stay hidden while plan body remains visible, including a split-chunk case

## 2. Request-side plan augmentation

- [x] 2.1 Add `Plan` to `RequestAugmentationMode` and surface a stable mode label for diagnostics
- [x] 2.2 Implement strong-signal plan detection in `decide_request_augmentation` using `metadata.plan_mode`, `tool_choice.name == "ExitPlanMode"`, and `plan_approval_response`
- [x] 2.3 Update Codex request construction so plan-mode turns prefer “propose a plan before execution” instead of falling back to generic agent augmentation
- [x] 2.4 Add or update request-side assertions ensuring non-plan traffic still uses existing agent / passthrough behavior

## 3. Response-side proposed-plan hygiene

- [x] 3.1 Add `<proposed_plan>` wrapper stripping to the visible-text hygiene path without suppressing wrapped plan content
- [x] 3.2 Make the wrapper stripping work across `text_carryover` boundaries so chunk-split tags are still hidden correctly
- [x] 3.3 Add or update response-side assertions ensuring ordinary visible text is unchanged when no proposed-plan wrapper is present

## 4. Verification

- [x] 4.1 Run targeted codex request/response tests covering plan detection and proposed-plan stripping until all green
- [x] 4.2 Run the relevant codex transform regression subset to confirm the new plan adapter does not break existing non-plan behavior
- [x] 4.3 Review the final diff against this change scope and keep it limited to方案 A的最小 plan adapter
