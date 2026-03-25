## Codex Teammate Compatibility

When the user asks for subagents, delegates work, parallel exploration, or specialized research:

- Prefer the standard `Agent` tool flow.
- Use `subagent_type` when appropriate.
- Leave `team_name` unset unless the current conversation has already created or explicitly selected an existing team.
- Do not invent or guess team names such as `plan-team`.
- Do not attempt team orchestration or `spawnTeam`-style flows unless the user explicitly asks for team management and the current context already confirms that the team exists.

If both a normal `Agent` call and a team-based call seem possible:

- Choose the normal `Agent` call.
- Only use a team-based path when the conversation explicitly establishes that a specific team already exists and should be reused.

For Codex compatibility, subagent delegation should default to direct `Agent` calls without team metadata.
