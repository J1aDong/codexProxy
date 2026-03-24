# subagent-result-handoff Specification

## Purpose
TBD - created by archiving change fix-subagent-result-handoff. Update Purpose after archive.
## Requirements
### Requirement: Background subagent completion handoff preserves usable result content
The proxy MUST preserve background subagent completion payloads in a structured form that lets the main agent distinguish launch metadata from final usable result content.

#### Scenario: Async background subagent completes with a usable result
- **WHEN** an ordinary `Agent` launch runs in the background and later completes with substantive result content
- **THEN** the proxy SHALL inject a completion payload that includes the subagent identity and the usable result content in a form the main agent can directly inspect or summarize

#### Scenario: Team/mailbox agent completes with a usable result
- **WHEN** a mailbox-style or teammate-style background agent sends a completion notification with usable result content
- **THEN** the proxy SHALL preserve the completion as a structured handoff instead of reducing it to ambiguous plain text

### Requirement: Incomplete background subagent completions are not treated as successful result handoff
The proxy MUST NOT represent a background subagent as having delivered a usable result when the observed completion content only contains launch metadata, placeholder text, or progress-only text.

#### Scenario: Placeholder text is surfaced as incomplete handoff
- **WHEN** a background subagent completion only contains placeholder or progress text such as “准备去查” and no substantive result content
- **THEN** the proxy SHALL mark or surface the completion as incomplete or missing-result rather than a successful usable handoff

#### Scenario: Main agent cannot safely summarize all subagents as complete
- **WHEN** one or more background subagents lack a usable completion payload
- **THEN** the proxy SHALL preserve enough state for downstream logic to avoid claiming that all launched subagents completed successfully with usable results

### Requirement: Regression coverage exists for launch-to-completion handoff
The repository MUST include automated tests that cover both background subagent launch normalization and later completion/result handoff behavior.

#### Scenario: Launch metadata remains distinct from completion payload
- **WHEN** tests exercise a background agent launch followed by a later completion
- **THEN** the assertions SHALL verify that launch metadata alone is not mistaken for a completed usable result

#### Scenario: Reproduced missing-result flow is covered
- **WHEN** tests simulate the reproduced flow where one subagent later proves to have returned only placeholder text
- **THEN** the proxy SHALL fail the test unless that subagent is surfaced as incomplete/missing-result instead of silently counted as a normal successful completion

