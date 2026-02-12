use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

/// Anthropic 请求体
#[derive(Debug, Deserialize)]
pub struct AnthropicRequest {
    pub model: Option<String>,
    pub messages: Vec<Message>,
    pub system: Option<SystemContent>,
    pub tools: Option<Vec<Value>>,
    #[serde(default = "default_stream")]
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub stop_sequences: Option<Vec<String>>,
}

fn default_stream() -> bool {
    false
}

/// system 字段可以是字符串或数组
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum SystemContent {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

impl SystemContent {
    pub fn to_string(&self) -> String {
        match self {
            SystemContent::Text(s) => s.clone(),
            SystemContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    SystemBlock::Text { text } => Some(text.clone()),
                    SystemBlock::PlainString(s) => Some(s.clone()),
                    SystemBlock::Other(v) => {
                        // 尝试提取 text 字段，或者将整个值转为字符串
                        v.get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| Some(serde_json::to_string(v).unwrap_or_default()))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum SystemBlock {
    PlainString(String),
    Text { text: String },
    Other(Value),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Message {
    pub role: String,
    #[serde(default, deserialize_with = "deserialize_message_content")]
    pub content: Option<MessageContent>,
}

/// 消息内容 - 支持多种格式
#[derive(Debug, Serialize, Clone)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// 自定义反序列化：支持字符串、单个对象、数组（纯字符串/纯对象/混合）
fn deserialize_message_content<'de, D>(deserializer: D) -> Result<Option<MessageContent>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<Value> = Option::deserialize(deserializer)?;

    let Some(value) = value else {
        return Ok(None);
    };

    match value {
        // 单个字符串
        Value::String(s) => Ok(Some(MessageContent::Text(s))),

        // 单个对象 - 转为单元素数组
        Value::Object(obj) => {
            let block = parse_content_block(Value::Object(obj));
            Ok(Some(MessageContent::Blocks(vec![block])))
        }

        // 数组 - 可能是字符串数组、对象数组或混合数组
        Value::Array(arr) => {
            if arr.is_empty() {
                return Ok(Some(MessageContent::Blocks(vec![])));
            }

            // 将所有元素转换为 ContentBlock
            let blocks: Vec<ContentBlock> = arr
                .into_iter()
                .map(|item| match item {
                    // 字符串元素转为 Text block
                    Value::String(s) => ContentBlock::Text { text: s },
                    // 对象元素解析为 ContentBlock
                    other => parse_content_block(other),
                })
                .collect();

            Ok(Some(MessageContent::Blocks(blocks)))
        }

        // null
        Value::Null => Ok(None),

        // 其他类型转为字符串
        other => Ok(Some(MessageContent::Text(other.to_string()))),
    }
}

/// 解析单个 ContentBlock
fn parse_content_block(value: Value) -> ContentBlock {
    parse_content_block_from_value(value)
}

#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text { text: String },
    Thinking { thinking: String, signature: Option<String> },
    Image {
        source: Option<ImageSource>,
        source_raw: Option<Value>,
        image_url: Option<ImageUrlValue>,
    },
    ImageUrl { image_url: ImageUrlValue },
    InputImage {
        image_url: Option<ImageUrlValue>,
        url: Option<String>,
        detail: Option<String>,
    },
    ToolUse {
        id: Option<String>,
        name: String,
        input: Value,
        signature: Option<String>,
    },
    ToolResult {
        tool_use_id: Option<String>,
        id: Option<String>,
        content: Option<Value>,
    },
    Document { source: Option<Value>, name: Option<String> },
    /// 用于存储无法解析的原始值
    OtherValue(Value),
}

impl Serialize for ContentBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            ContentBlock::Text { text } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("type", "text")?;
                map.serialize_entry("text", text)?;
                map.end()
            }
            ContentBlock::Thinking { thinking, signature } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("type", "thinking")?;
                map.serialize_entry("thinking", thinking)?;
                if let Some(sig) = signature {
                    map.serialize_entry("signature", sig)?;
                }
                map.end()
            }
            ContentBlock::ToolUse { id, name, input, signature } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("type", "tool_use")?;
                if let Some(id_val) = id {
                    map.serialize_entry("id", id_val)?;
                }
                map.serialize_entry("name", name)?;
                map.serialize_entry("input", input)?;
                if let Some(sig) = signature {
                    map.serialize_entry("signature", sig)?;
                }
                map.end()
            }
            ContentBlock::OtherValue(v) => v.serialize(serializer),
            _ => {
                // 其他类型简单序列化为 JSON
                let value = json!({"type": "unknown"});
                value.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for ContentBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(parse_content_block_from_value(value))
    }
}

/// 从 Value 解析 ContentBlock
fn parse_content_block_from_value(value: Value) -> ContentBlock {
    let obj = match &value {
        Value::Object(obj) => obj,
        _ => return ContentBlock::OtherValue(value),
    };

    let block_type = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match block_type {
        "text" => {
            let text = obj.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            ContentBlock::Text { text }
        }
        "thinking" | "thought" => {
            let thinking = obj.get("thinking")
                .or_else(|| obj.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let signature = obj.get("signature").and_then(|s| s.as_str()).map(|s| s.to_string());
            ContentBlock::Thinking { thinking, signature }
        }
        "image" => {
            let source_raw = obj.get("source").cloned();
            let source = source_raw.as_ref().and_then(|s| serde_json::from_value(s.clone()).ok());
            let image_url = obj.get("image_url").and_then(|u| serde_json::from_value(u.clone()).ok());
            ContentBlock::Image { source, source_raw, image_url }
        }
        "image_url" => {
            let image_url = obj.get("image_url")
                .and_then(|u| serde_json::from_value(u.clone()).ok())
                .unwrap_or(ImageUrlValue::Str(String::new()));
            ContentBlock::ImageUrl { image_url }
        }
        "input_image" => {
            let image_url = obj.get("image_url").and_then(|u| serde_json::from_value(u.clone()).ok());
            let url = obj.get("url").and_then(|u| u.as_str()).map(|s| s.to_string());
            let detail = obj.get("detail").and_then(|d| d.as_str()).map(|s| s.to_string());
            ContentBlock::InputImage { image_url, url, detail }
        }
        "tool_use" => {
            let id = obj.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
            let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            // input 为 null 或缺失时归一化为 {}
            let input = obj.get("input")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(json!({}));
            let signature = obj.get("signature")
                .or_else(|| obj.get("thought_signature"))
                .or_else(|| obj.get("thoughtSignature"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
            ContentBlock::ToolUse { id, name, input, signature }
        }
        "tool_result" => {
            let tool_use_id = obj.get("tool_use_id").and_then(|i| i.as_str()).map(|s| s.to_string());
            let id = obj.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
            let content = obj.get("content").cloned();
            ContentBlock::ToolResult { tool_use_id, id, content }
        }
        "document" => {
            let source = obj.get("source").cloned();
            let name = obj.get("name").and_then(|n| n.as_str()).map(|s| s.to_string());
            ContentBlock::Document { source, name }
        }
        "" => {
            // 没有 type 字段，检查是否有 text 字段
            if obj.get("image_url").is_some() {
                let image_url = obj.get("image_url")
                    .and_then(|u| serde_json::from_value(u.clone()).ok())
                    .unwrap_or(ImageUrlValue::Str(String::new()));
                return ContentBlock::ImageUrl { image_url };
            }
            if obj.get("source").is_some() {
                let source_raw = obj.get("source").cloned();
                let source = source_raw.as_ref().and_then(|s| serde_json::from_value(s.clone()).ok());
                return ContentBlock::Image { source, source_raw, image_url: None };
            }
            if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                return ContentBlock::Text { text: text.to_string() };
            }
            ContentBlock::OtherValue(value)
        }
        _ => {
            if obj.get("image_url").is_some() {
                let image_url = obj.get("image_url")
                    .and_then(|u| serde_json::from_value(u.clone()).ok())
                    .unwrap_or(ImageUrlValue::Str(String::new()));
                return ContentBlock::ImageUrl { image_url };
            }
            if obj.get("source").is_some() {
                let source_raw = obj.get("source").cloned();
                let source = source_raw.as_ref().and_then(|s| serde_json::from_value(s.clone()).ok());
                return ContentBlock::Image { source, source_raw, image_url: None };
            }
            ContentBlock::OtherValue(value)
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: Option<String>,
    #[serde(alias = "mediaType")]
    pub media_type: Option<String>,
    #[serde(alias = "mime_type", alias = "mimeType")]
    pub mime_type: Option<String>,
    pub data: Option<String>,
    pub url: Option<String>,
    pub uri: Option<String>,
    #[serde(alias = "file_path", alias = "filePath", alias = "local_path", alias = "localPath", alias = "file")]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum ImageUrlValue {
    Str(String),
    ObjUrl { url: String },
    ObjUri { uri: String },
}
