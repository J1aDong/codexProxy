use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Xhigh,
    High,
    #[default]
    Medium,
    Low,
}

impl ReasoningEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasoningEffort::Xhigh => "xhigh",
            ReasoningEffort::High => "high",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::Low => "low",
        }
    }
}

impl ReasoningEffort {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "xhigh" => ReasoningEffort::Xhigh,
            "high" => ReasoningEffort::High,
            "medium" => ReasoningEffort::Medium,
            "low" => ReasoningEffort::Low,
            _ => ReasoningEffort::Medium,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReasoningEffortMapping {
    #[serde(default = "default_opus")]
    pub opus: ReasoningEffort,
    #[serde(default = "default_sonnet")]
    pub sonnet: ReasoningEffort,
    #[serde(default = "default_haiku")]
    pub haiku: ReasoningEffort,
}

fn default_opus() -> ReasoningEffort { ReasoningEffort::Xhigh }
fn default_sonnet() -> ReasoningEffort { ReasoningEffort::Medium }
fn default_haiku() -> ReasoningEffort { ReasoningEffort::Low }

impl Default for ReasoningEffortMapping {
    fn default() -> Self {
        Self {
            opus: default_opus(),
            sonnet: default_sonnet(),
            haiku: default_haiku(),
        }
    }
}

impl ReasoningEffortMapping {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_opus(mut self, effort: ReasoningEffort) -> Self {
        self.opus = effort;
        self
    }

    pub fn with_sonnet(mut self, effort: ReasoningEffort) -> Self {
        self.sonnet = effort;
        self
    }

    pub fn with_haiku(mut self, effort: ReasoningEffort) -> Self {
        self.haiku = effort;
        self
    }
}

pub fn get_reasoning_effort(model: &str, mapping: &ReasoningEffortMapping) -> ReasoningEffort {
    let model_lower = model.to_lowercase();

    if model_lower.contains("opus") {
        return mapping.opus;
    }
    if model_lower.contains("sonnet") {
        return mapping.sonnet;
    }
    if model_lower.contains("haiku") {
        return mapping.haiku;
    }

    ReasoningEffort::Medium
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiReasoningEffortMapping {
    #[serde(default = "default_gemini_pro")]
    pub opus: String,
    #[serde(default = "default_gemini_flash")]
    pub sonnet: String,
    #[serde(default = "default_gemini_flash")]
    pub haiku: String,
}

fn default_gemini_pro() -> String { "gemini-3-pro-preview".to_string() }
fn default_gemini_flash() -> String { "gemini-3-flash-preview".to_string() }

impl Default for GeminiReasoningEffortMapping {
    fn default() -> Self {
        Self {
            opus: default_gemini_pro(),
            sonnet: default_gemini_flash(),
            haiku: default_gemini_flash(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodexModelMapping {
    #[serde(default = "default_codex_opus")]
    pub opus: String,
    #[serde(default = "default_codex_sonnet")]
    pub sonnet: String,
    #[serde(default = "default_codex_haiku")]
    pub haiku: String,
}

fn default_codex_opus() -> String { "gpt-5.3-codex".to_string() }
fn default_codex_sonnet() -> String { "gpt-5.2-codex".to_string() }
fn default_codex_haiku() -> String { "gpt-5.1-codex-mini".to_string() }

impl Default for CodexModelMapping {
    fn default() -> Self {
        Self {
            opus: default_codex_opus(),
            sonnet: default_codex_sonnet(),
            haiku: default_codex_haiku(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AnthropicModelMapping {
    #[serde(default)]
    pub opus: String,
    #[serde(default)]
    pub sonnet: String,
    #[serde(default)]
    pub haiku: String,
}
