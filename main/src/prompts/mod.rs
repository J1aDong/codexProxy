pub const TEAMMATE_PROMPT: &str = include_str!("teammate.md");

pub fn codex_system_prompt_extensions() -> Vec<&'static str> {
    vec![TEAMMATE_PROMPT]
}
