#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolInterceptionAction {
    PassThrough,
    Intercept,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolInterceptionDecision {
    pub action: ToolInterceptionAction,
    pub normalized_tool_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestScopedToolRouter {
    intercepted_tools: Vec<String>,
}

impl RequestScopedToolRouter {
    pub fn new<T>(intercepted_tools: T) -> Self
    where
        T: IntoIterator,
        T::Item: AsRef<str>,
    {
        Self {
            intercepted_tools: intercepted_tools
                .into_iter()
                .map(|tool| tool.as_ref().to_string())
                .collect(),
        }
    }

    pub fn decide_by_name(&self, tool_name: &str) -> ToolInterceptionDecision {
        let normalized_tool_name = normalize_tool_name(tool_name);
        let should_intercept = normalized_tool_name
            .as_deref()
            .map(|name| {
                self.intercepted_tools
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(name))
            })
            .unwrap_or(false);

        ToolInterceptionDecision {
            action: if should_intercept {
                ToolInterceptionAction::Intercept
            } else {
                ToolInterceptionAction::PassThrough
            },
            normalized_tool_name: if should_intercept {
                normalized_tool_name
            } else {
                None
            },
        }
    }
}

pub fn normalize_tool_name(tool_name: &str) -> Option<String> {
    let trimmed = tool_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let squashed = trimmed
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-' && !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();

    match squashed.as_str() {
        "websearch" => Some("web_search".to_string()),
        _ => None,
    }
}
