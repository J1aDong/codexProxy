use crate::models::{
    AnthropicRequest, ContentBlock, ImageSource, ImageUrlValue, Message, MessageContent,
};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnifiedMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl UnifiedMessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnifiedContent {
    Text { text: String },
    ImageUrl {
        url: String,
        media_type: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedThinking {
    pub content: String,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedToolCall {
    pub id: String,
    pub function: UnifiedFunctionCall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedMessage {
    pub role: UnifiedMessageRole,
    pub content: Vec<UnifiedContent>,
    pub tool_calls: Vec<UnifiedToolCall>,
    pub tool_call_id: Option<String>,
    pub thinking: Option<UnifiedThinking>,
}

impl UnifiedMessage {
    pub fn content_text(&self) -> Option<String> {
        let parts: Vec<&str> = self
            .content
            .iter()
            .filter_map(|item| match item {
                UnifiedContent::Text { text } => Some(text.as_str()),
                UnifiedContent::ImageUrl { .. } => None,
            })
            .collect();

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n"))
        }
    }

    pub fn content_items(&self) -> &[UnifiedContent] {
        &self.content
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnifiedTool {
    pub function: UnifiedToolDefinition,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnifiedToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnifiedToolChoice {
    Auto,
    None,
    Required,
    Function { name: String },
}

impl UnifiedToolChoice {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Required => "required",
            Self::Function { .. } => "function",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedReasoning {
    pub enabled: bool,
    pub effort: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnifiedChatRequest {
    pub messages: Vec<UnifiedMessage>,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub tools: Option<Vec<UnifiedTool>>,
    pub tool_choice: Option<UnifiedToolChoice>,
    pub reasoning: Option<UnifiedReasoning>,
}

impl UnifiedChatRequest {
    pub fn from_anthropic(request: &AnthropicRequest) -> Self {
        let mut messages = Vec::new();

        if let Some(system) = request.system.as_ref().map(|value| value.to_string()) {
            if !system.trim().is_empty() {
                messages.push(UnifiedMessage {
                    role: UnifiedMessageRole::System,
                    content: vec![UnifiedContent::Text { text: system }],
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    thinking: None,
                });
            }
        }

        for message in &request.messages {
            messages.extend(convert_message(message));
        }

        Self {
            messages,
            model: request.model.clone().unwrap_or_default(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stream: request.stream,
            tools: convert_tools(request.tools.as_ref()),
            tool_choice: convert_tool_choice(request.tool_choice.as_ref()),
            reasoning: convert_reasoning(request),
        }
    }

    pub fn has_system_text(&self) -> bool {
        self.messages.iter().any(|message| {
            message.role == UnifiedMessageRole::System
                && message
                    .content_text()
                    .map(|text| !text.trim().is_empty())
                    .unwrap_or(false)
        })
    }

    pub fn append_system_texts<I, S>(&mut self, texts: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for text in texts {
            let text = text.as_ref();
            if text.trim().is_empty() {
                continue;
            }

            self.messages.push(UnifiedMessage {
                role: UnifiedMessageRole::System,
                content: vec![UnifiedContent::Text {
                    text: text.to_string(),
                }],
                tool_calls: Vec::new(),
                tool_call_id: None,
                thinking: None,
            });
        }
    }
}

fn is_proxy_lifecycle_progress_text(text: &str) -> bool {
    let normalized = text.replace('\r', "");
    let lines: Vec<&str> = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    !lines.is_empty()
        && lines.iter().all(|line| {
            matches!(
                *line,
                "请求已发送，正在等待上游开始输出…" | "模型正在处理中…"
            )
        })
}

fn convert_message(message: &Message) -> Vec<UnifiedMessage> {
    match message.role.as_str() {
        "user" => convert_user_message(message),
        "assistant" => convert_assistant_message(message),
        _ => Vec::new(),
    }
}

fn convert_user_message(message: &Message) -> Vec<UnifiedMessage> {
    let mut result = Vec::new();
    let mut pending_content = Vec::new();

    match &message.content {
        Some(MessageContent::Text(text)) => {
            if !text.is_empty() {
                result.push(UnifiedMessage {
                    role: UnifiedMessageRole::User,
                    content: vec![UnifiedContent::Text { text: text.clone() }],
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    thinking: None,
                });
            }
        }
        Some(MessageContent::Blocks(blocks)) => {
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        if !text.is_empty() {
                            pending_content.push(UnifiedContent::Text { text: text.clone() });
                        }
                    }
                    ContentBlock::Image {
                        source,
                        image_url,
                        ..
                    } => {
                        if let Some(image) = resolve_image_content(source.as_ref(), image_url.as_ref(), None)
                        {
                            pending_content.push(image);
                        }
                    }
                    ContentBlock::ImageUrl { image_url } => {
                        if let Some(image) = resolve_image_content(None, Some(image_url), None) {
                            pending_content.push(image);
                        }
                    }
                    ContentBlock::InputImage {
                        image_url,
                        url,
                        ..
                    } => {
                        if let Some(image) =
                            resolve_image_content(None, image_url.as_ref(), url.as_deref())
                        {
                            pending_content.push(image);
                        }
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        if !pending_content.is_empty() {
                            result.push(UnifiedMessage {
                                role: UnifiedMessageRole::User,
                                content: std::mem::take(&mut pending_content),
                                tool_calls: Vec::new(),
                                tool_call_id: None,
                                thinking: None,
                            });
                        }

                        result.push(UnifiedMessage {
                            role: UnifiedMessageRole::Tool,
                            content: vec![UnifiedContent::Text {
                                text: stringify_value(content.as_ref().unwrap_or(&Value::Null)),
                            }],
                            tool_calls: Vec::new(),
                            tool_call_id: tool_use_id.clone(),
                            thinking: None,
                        });
                    }
                    _ => {}
                }
            }

            if !pending_content.is_empty() {
                result.push(UnifiedMessage {
                    role: UnifiedMessageRole::User,
                    content: pending_content,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    thinking: None,
                });
            }
        }
        None => {}
    }

    result
}

fn convert_assistant_message(message: &Message) -> Vec<UnifiedMessage> {
    let mut content = Vec::new();
    let mut tool_calls = Vec::new();
    let mut thinking = None;

    match &message.content {
        Some(MessageContent::Text(text)) => {
            if !text.is_empty() && !is_proxy_lifecycle_progress_text(text) {
                content.push(UnifiedContent::Text { text: text.clone() });
            }
        }
        Some(MessageContent::Blocks(blocks)) => {
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        if !text.is_empty() && !is_proxy_lifecycle_progress_text(text) {
                            content.push(UnifiedContent::Text { text: text.clone() });
                        }
                    }
                    ContentBlock::Thinking {
                        thinking: content_text,
                        signature,
                    } => {
                        if !content_text.is_empty()
                            && !is_proxy_lifecycle_progress_text(content_text)
                        {
                            thinking = Some(UnifiedThinking {
                                content: content_text.clone(),
                                signature: signature.clone(),
                            });
                        }
                    }
                    ContentBlock::ToolUse { id, name, input, .. } => {
                        let call_id = id.clone().unwrap_or_else(|| "tool_call".to_string());
                        tool_calls.push(UnifiedToolCall {
                            id: call_id,
                            function: UnifiedFunctionCall {
                                name: name.clone(),
                                arguments: serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string()),
                            },
                        });
                    }
                    _ => {}
                }
            }
        }
        None => {}
    }

    if content.is_empty() && tool_calls.is_empty() && thinking.is_none() {
        Vec::new()
    } else {
        vec![UnifiedMessage {
            role: UnifiedMessageRole::Assistant,
            content,
            tool_calls,
            tool_call_id: None,
            thinking,
        }]
    }
}

fn resolve_image_content(
    source: Option<&ImageSource>,
    image_url: Option<&ImageUrlValue>,
    raw_url: Option<&str>,
) -> Option<UnifiedContent> {
    let from_image_url = image_url.and_then(|value| match value {
        ImageUrlValue::Str(url) => Some((url.clone(), None)),
        ImageUrlValue::ObjUrl { url } => Some((url.clone(), None)),
        ImageUrlValue::ObjUri { uri } => Some((uri.clone(), None)),
    });

    let from_source = source.and_then(|value| {
        if let Some(url) = value.url.as_ref().or(value.uri.as_ref()) {
            return Some((
                url.clone(),
                value
                    .media_type
                    .clone()
                    .or_else(|| value.mime_type.clone()),
            ));
        }

        if let Some(path) = value.path.as_ref() {
            let normalized = if path.starts_with("file://") {
                path.clone()
            } else {
                format!("file://{}", path)
            };
            return Some((
                normalized,
                value
                    .media_type
                    .clone()
                    .or_else(|| value.mime_type.clone()),
            ));
        }

        value.data.as_ref().map(|data| {
            let media_type = value
                .media_type
                .clone()
                .or_else(|| value.mime_type.clone())
                .unwrap_or_else(|| "image/png".to_string());
            (format!("data:{};base64,{}", media_type, data), Some(media_type))
        })
    });

    let resolved = from_image_url
        .or(from_source)
        .or_else(|| raw_url.map(|url| (url.to_string(), None)))?;

    Some(UnifiedContent::ImageUrl {
        url: resolved.0,
        media_type: resolved.1,
    })
}

fn convert_tools(tools: Option<&Vec<Value>>) -> Option<Vec<UnifiedTool>> {
    let converted: Vec<UnifiedTool> = tools
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            let name = tool.get("name").and_then(|value| value.as_str())?;
            Some(UnifiedTool {
                function: UnifiedToolDefinition {
                    name: name.to_string(),
                    description: tool
                        .get("description")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    parameters: tool
                        .get("input_schema")
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Default::default())),
                },
            })
        })
        .collect();

    if converted.is_empty() {
        None
    } else {
        Some(converted)
    }
}

fn convert_tool_choice(tool_choice: Option<&Value>) -> Option<UnifiedToolChoice> {
    let choice = tool_choice?;
    let kind = choice.get("type").and_then(|value| value.as_str())?;

    match kind {
        "auto" => Some(UnifiedToolChoice::Auto),
        "none" => Some(UnifiedToolChoice::None),
        "any" | "required" => Some(UnifiedToolChoice::Required),
        "tool" => choice
            .get("name")
            .and_then(|value| value.as_str())
            .map(|name| UnifiedToolChoice::Function {
                name: name.to_string(),
            }),
        _ => None,
    }
}

fn convert_reasoning(request: &AnthropicRequest) -> Option<UnifiedReasoning> {
    let thinking = request.thinking.as_ref()?;
    let enabled = !thinking.is_disabled();
    let max_tokens = thinking
        .extra
        .get("budget_tokens")
        .and_then(|value| value.as_u64())
        .map(|value| value as u32);

    Some(UnifiedReasoning {
        enabled,
        effort: None,
        max_tokens,
    })
}

fn stringify_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_message(role: UnifiedMessageRole, text: &str) -> UnifiedMessage {
        UnifiedMessage {
            role,
            content: vec![UnifiedContent::Text {
                text: text.to_string(),
            }],
            tool_calls: Vec::new(),
            tool_call_id: None,
            thinking: None,
        }
    }

    #[test]
    fn append_system_texts_preserves_existing_history_order() {
        let mut request = UnifiedChatRequest {
            messages: vec![
                text_message(UnifiedMessageRole::System, "base system"),
                text_message(UnifiedMessageRole::User, "hello"),
                text_message(UnifiedMessageRole::Assistant, "hi"),
                text_message(UnifiedMessageRole::Tool, "{\"ok\":true}"),
            ],
            model: "claude-sonnet-4-5".to_string(),
            max_tokens: Some(256),
            temperature: None,
            stream: true,
            tools: None,
            tool_choice: None,
            reasoning: None,
        };

        request.append_system_texts(["extension a", "extension b"]);

        assert_eq!(
            request
                .messages
                .iter()
                .map(|message| message.role.as_str())
                .collect::<Vec<_>>(),
            vec!["system", "user", "assistant", "tool", "system", "system"]
        );
        assert_eq!(
            request.messages[1].content_text().as_deref(),
            Some("hello")
        );
        assert_eq!(
            request.messages[2].content_text().as_deref(),
            Some("hi")
        );
    }

    #[test]
    fn append_system_texts_skips_blank_entries_and_marks_system_presence() {
        let mut request = UnifiedChatRequest {
            messages: vec![text_message(UnifiedMessageRole::User, "hello")],
            model: String::new(),
            max_tokens: None,
            temperature: None,
            stream: false,
            tools: None,
            tool_choice: None,
            reasoning: None,
        };

        assert!(!request.has_system_text());

        request.append_system_texts(["  ", "\n", "extension"]);

        assert!(request.has_system_text());
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[1].role, UnifiedMessageRole::System);
        assert_eq!(
            request.messages[1].content_text().as_deref(),
            Some("extension")
        );
    }
}
