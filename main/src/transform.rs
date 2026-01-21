use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::broadcast;
use uuid::Uuid;

const CODEX_INSTRUCTIONS: &str = include_str!("instructions.txt");
const IMAGE_SYSTEM_HINT: &str = "\n<system_hint>IMAGE PROVIDED. You can see the image above directly. Analyze it as requested. DO NOT ask for file paths.</system_hint>\n";

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
            "low" => ReasoningEffort::Low,
            "medium" => ReasoningEffort::Medium,
            "high" => ReasoningEffort::High,
            "xhigh" => ReasoningEffort::Xhigh,
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
        mapping.opus
    } else if model_lower.contains("sonnet") {
        mapping.sonnet
    } else if model_lower.contains("haiku") {
        mapping.haiku
    } else {
        ReasoningEffort::Medium
    }
}

/// 日志目录
const LOG_DIR: &str = "logs";

/// 全局调试日志开关
static DEBUG_LOG_ENABLED: AtomicBool = AtomicBool::new(cfg!(debug_assertions));

/// 全局应用日志记录器
static APP_LOGGER: OnceLock<Arc<AppLogger>> = OnceLock::new();

/// 设置调试日志开关
pub fn set_debug_log(enabled: bool) {
    DEBUG_LOG_ENABLED.store(enabled, Ordering::SeqCst);
}

/// 获取调试日志开关状态
pub fn is_debug_log_enabled() -> bool {
    DEBUG_LOG_ENABLED.load(Ordering::SeqCst)
}

/// 截断字符串用于日志显示
fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}... (len={})", &s[..max_len], s.len())
    }
}

/// 应用级日志记录器 - 每次启动应用一个日志文件
pub struct AppLogger {
    log_path: PathBuf,
}

impl AppLogger {
    /// 初始化全局日志记录器（应用启动时调用一次）
    pub fn init(log_dir: Option<&str>) -> Arc<AppLogger> {
        APP_LOGGER.get_or_init(|| {
            let dir = log_dir.unwrap_or(LOG_DIR);
            let _ = fs::create_dir_all(dir);

            let start_time = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
            let log_path = PathBuf::from(dir).join(format!("proxy_{}.log", start_time));

            let logger = Arc::new(AppLogger {
                log_path,
            });

            // 写入启动信息
            logger.log("════════════════════════════════════════════════════════════════════════");
            logger.log("🚀 Codex Proxy Started");
            logger.log(&format!("📁 Log file: {:?}", logger.log_path));
            logger.log("════════════════════════════════════════════════════════════════════════");

            logger
        }).clone()
    }

    /// 获取全局日志记录器
    pub fn get() -> Option<Arc<AppLogger>> {
        APP_LOGGER.get().cloned()
    }

    /// 写入日志（仅在 debug 模式下）
    pub fn log(&self, message: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] {}\n", timestamp, message);

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// 写入原始内容（不带时间戳，用于记录 JSON）
    pub fn log_raw(&self, content: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(content.as_bytes());
            let _ = file.write_all(b"\n");
        }
    }

    /// 记录新请求开始（带 session ID 分隔）
    pub fn log_request_start(&self, session_id: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_content = format!(
            "\n\n[{}] ╔══════════════════════════════════════════════════════════════════════╗\n\
             [{}] ║ 🆕 NEW REQUEST - Session: {}\n\
             [{}] ╚══════════════════════════════════════════════════════════════════════╝\n",
            timestamp, timestamp, session_id, timestamp
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 记录 curl 格式的请求
    pub fn log_curl_request(&self, method: &str, url: &str, headers: &[(&str, &str)], body: &Value) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let mut curl_cmd = format!("curl -X {} '{}'", method, url);

        for (key, value) in headers {
            curl_cmd.push_str(&format!(" \\\n  -H '{}: {}'", key, value));
        }

        // 格式化 JSON body
        let pretty_body = serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string());

        let log_content = format!(
            "\n[{}] ════════════════════════════════════════════════════════════════\n\
             [{}] 📤 OUTGOING REQUEST (Codex API)\n\
             [{}] ════════════════════════════════════════════════════════════════\n\
             {}\n\
             \n\
             Request Body:\n\
             {}\n\
             ════════════════════════════════════════════════════════════════════════\n",
            timestamp, timestamp, timestamp, curl_cmd, pretty_body
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 记录原始 Anthropic 请求
    pub fn log_anthropic_request(&self, body_bytes: &[u8]) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");

        // 尝试格式化 JSON，失败则使用原始字符串
        let body_str = String::from_utf8_lossy(body_bytes);
        let pretty_body = if let Ok(json) = serde_json::from_slice::<Value>(body_bytes) {
            serde_json::to_string_pretty(&json).unwrap_or_else(|_| body_str.to_string())
        } else {
            body_str.to_string()
        };

        let log_content = format!(
            "\n[{}] ════════════════════════════════════════════════════════════════\n\
             [{}] 📥 INCOMING ANTHROPIC REQUEST\n\
             [{}] ════════════════════════════════════════════════════════════════\n\
             {}\n\
             ════════════════════════════════════════════════════════════════════════\n",
            timestamp, timestamp, timestamp, pretty_body
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 记录上游响应
    pub fn log_upstream_response(&self, status: u16, line: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] 📩 [Upstream {}] {}\n", timestamp, status, line);

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// 记录转换后的 Anthropic 响应
    pub fn log_anthropic_response(&self, line: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] 📤 [To Client] {}\n", timestamp, line.trim());

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// 记录请求完成
    pub fn log_request_end(&self) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_content = format!(
            "[{}] ════════════════════════════════════════════════════════════════\n\
             [{}] ✅ Request completed\n\
             [{}] ════════════════════════════════════════════════════════════════\n",
            timestamp, timestamp, timestamp
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 获取日志文件路径
    pub fn log_path(&self) -> &PathBuf {
        &self.log_path
    }
}

/// Anthropic 请求体
#[derive(Debug, Deserialize)]
pub struct AnthropicRequest {
    pub model: Option<String>,
    pub messages: Vec<Message>,
    pub system: Option<SystemContent>,
    pub tools: Option<Vec<Value>>,
    #[serde(default = "default_stream")]
    pub stream: bool,
}

fn default_stream() -> bool {
    true
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
            ContentBlock::ToolUse { id, name, input }
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

pub struct TransformRequest;

impl TransformRequest {
    pub fn transform(
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        reasoning_mapping: &ReasoningEffortMapping,
    ) -> (Value, String) {
        let session_id = Uuid::new_v4().to_string();
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());

        // 获取全局日志记录器
        let logger = AppLogger::get();

        // 辅助函数：同时写入 broadcast 和文件
        let log = |msg: &str| {
            if is_debug_log_enabled() {
                if let Some(tx) = log_tx {
                    let _ = tx.send(msg.to_string());
                }
                if let Some(ref l) = logger {
                    l.log(msg);
                }
            }
        };

        log(&format!("📋 [Transform] Session: {}", &session_id[..8]));

        let original_model = anthropic_body.model.as_deref().unwrap_or("unknown");
        let reasoning_effort = get_reasoning_effort(original_model, reasoning_mapping);
        let codex_model = anthropic_body
            .model
            .as_ref()
            .map(|m| {
                if m.to_lowercase().contains("claude")
                    || m.to_lowercase().contains("sonnet")
                    || m.to_lowercase().contains("opus")
                    || m.to_lowercase().contains("haiku")
                {
                    "gpt-5.2-codex".to_string()
                } else {
                    m.clone()
                }
            })
            .unwrap_or_else(|| "gpt-5.2-codex".to_string());

        log(&format!("📋 [Transform] Model: {} → {} (reasoning: {})", original_model, codex_model, reasoning_effort.as_str()));

        let (chat_messages, extracted_skills) = Self::transform_messages(&anthropic_body.messages, log_tx);

        // 构建 input 数组
        let mut final_input: Vec<Value> = vec![Self::build_template_input()];

        // 注入 system prompt
        if let Some(system) = &anthropic_body.system {
            let system_text = system.to_string();
            log(&format!("📋 [Transform] System prompt: {} chars", system_text.len()));

            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": format!("# AGENTS.md instructions for {}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>", cwd, system_text)
                }]
            }));

            final_input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": format!(r#"<environment_context>
  <cwd>{}</cwd>
  <approval_policy>on-request</approval_policy>
  <sandbox_mode>workspace-write</sandbox_mode>
  <network_access>restricted</network_access>
  <shell>{}</shell>
</environment_context>"#, cwd, std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string()))
                }]
            }));
        }

        // 注入提取的 Skills
        if !extracted_skills.is_empty() {
            log(&format!("🎯 [Transform] Injecting {} skill(s)", extracted_skills.len()));
            for skill in extracted_skills {
                final_input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": skill
                    }]
                }));
            }
        }

        // 追加对话历史
        final_input.extend(chat_messages);

        // 转换工具
        let transformed_tools = Self::transform_tools(anthropic_body.tools.as_ref(), log_tx);

        log(&format!(
            "📋 [Transform] Final: {} input items, {} tools",
            final_input.len(),
            transformed_tools.len()
        ));

        let body = json!({
            "model": codex_model,
            "instructions": CODEX_INSTRUCTIONS,
            "input": final_input,
            "tools": transformed_tools,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": { "effort": reasoning_effort.as_str(), "summary": "auto" },
            "store": false,
            "stream": anthropic_body.stream,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": session_id
        });

        (body, session_id.clone())
    }

    fn build_template_input() -> Value {
        // 从 codex-request.json 读取完整的模板，与 JavaScript 版本保持一致
        let template_path = std::path::Path::new("codex-request.json");
        if let Ok(template_content) = std::fs::read_to_string(template_path) {
            if let Ok(template) = serde_json::from_str::<Value>(&template_content) {
                if let Some(input) = template.get("input").and_then(|i| i.as_array()) {
                    if let Some(first_input) = input.first() {
                        return first_input.clone();
                    }
                }
            }
        }
        
        // 如果无法读取模板，使用备用值
        json!({
            "type": "message",
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": "# AGENTS.md instructions for /Users/mr.j\n\n<INSTRUCTIONS>\n---\nname: engineer-professional\ndescription: 专业的软件工程师\n---\n</INSTRUCTIONS>"
            }]
        })
    }

    fn transform_messages(
        messages: &[Message],
        log_tx: Option<&broadcast::Sender<String>>,
    ) -> (Vec<Value>, Vec<String>) {
        let mut input = Vec::new();
        let mut extracted_skills = Vec::new();
        let mut skill_tool_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 获取全局日志记录器
        let logger = AppLogger::get();

        // 辅助函数：同时写入 broadcast 和文件
        let log = |msg: &str| {
            if is_debug_log_enabled() {
                if let Some(tx) = log_tx {
                    let _ = tx.send(msg.to_string());
                }
                if let Some(ref l) = logger {
                    l.log(msg);
                }
            }
        };

        log(&format!("📝 [Messages] Processing {} messages", messages.len()));

        // 第一遍：收集 skill tool ids
        for msg in messages {
            if let Some(MessageContent::Blocks(blocks)) = &msg.content {
                for block in blocks {
                    if let ContentBlock::ToolUse { id, name, .. } = block {
                        if name.to_lowercase() == "skill" {
                            if let Some(tool_id) = id {
                                skill_tool_ids.insert(tool_id.clone());
                            }
                        }
                    }
                }
            }
        }

        // 第二遍：转换消息
        for (msg_idx, msg) in messages.iter().enumerate() {
            if msg.role == "system" {
                continue;
            }

            if msg.role != "user" && msg.role != "assistant" {
                continue;
            }

            let text_type = if msg.role == "user" {
                "input_text"
            } else {
                "output_text"
            };

            let Some(content) = &msg.content else {
                log(&format!("📝 [Message #{}] role={}, content=null (skipped)", msg_idx, msg.role));
                continue;
            };

            match content {
                MessageContent::Text(text) => {
                    log(&format!(
                        "📝 [Message #{}] role={}, type=Text, len={}",
                        msg_idx,
                        msg.role,
                        text.len()
                    ));
                    input.push(json!({
                        "type": "message",
                        "role": msg.role,
                        "content": [{
                            "type": text_type,
                            "text": text
                        }]
                    }));
                }
                MessageContent::Blocks(blocks) => {
                    log(&format!(
                        "📝 [Message #{}] role={}, type=Blocks({})",
                        msg_idx,
                        msg.role,
                        blocks.len()
                    ));

                    let mut current_msg_content = Vec::new();
                    let mut image_hint_added = false;
                    let mut ensure_image_hint = |current_msg_content: &mut Vec<Value>| {
                        if image_hint_added {
                            return;
                        }
                        let already_has_hint = current_msg_content.iter().any(|item| {
                            item.get("type").and_then(|t| t.as_str()) == Some("input_text")
                                && item
                                    .get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|t| t.contains("IMAGE PROVIDED"))
                                    .unwrap_or(false)
                        });
                        if !already_has_hint {
                            current_msg_content.push(json!({
                                "type": "input_text",
                                "text": IMAGE_SYSTEM_HINT
                            }));
                        }
                        image_hint_added = true;
                    };

                    for (block_idx, block) in blocks.iter().enumerate() {
                        match block {
                            ContentBlock::Text { text } => {
                                current_msg_content.push(json!({
                                    "type": text_type,
                                    "text": text
                                }));
                            }
                            ContentBlock::Image { source, source_raw, image_url } => {
                                let mut resolved_url = if let Some(image_url) = image_url {
                                    match image_url {
                                        ImageUrlValue::Str(s) => s.clone(),
                                        ImageUrlValue::ObjUrl { url } => url.clone(),
                                        ImageUrlValue::ObjUri { uri } => uri.clone(),
                                    }
                                } else if let Some(src) = source {
                                    Self::resolve_image_url(src, &log, msg_idx, block_idx)
                                } else {
                                    String::new()
                                };

                                if !resolved_url.is_empty() {
                                    let media_type = source.as_ref()
                                        .and_then(|s| s.media_type.as_deref().or(s.mime_type.as_deref()));
                                    resolved_url = Self::normalize_image_url(
                                        resolved_url,
                                        media_type,
                                        &log,
                                        msg_idx,
                                        block_idx,
                                    );
                                }

                                if resolved_url.is_empty() {
                                    if let Some(raw) = source_raw {
                                        resolved_url = Self::resolve_image_url_raw(raw, &log, msg_idx, block_idx);
                                    }
                                }

                                if !resolved_url.is_empty() && msg.role == "user" {
                                    ensure_image_hint(&mut current_msg_content);
                                    log(&format!(
                                        "🖼️ [Message #{} Block #{}] Image processed (len={})",
                                        msg_idx,
                                        block_idx,
                                        resolved_url.len()
                                    ));
                                    current_msg_content.push(json!({
                                        "type": "input_image",
                                        "image_url": resolved_url,
                                        "detail": "auto"
                                    }));
                                }
                            }
                            ContentBlock::ImageUrl { image_url } => {
                                let url = match image_url {
                                    ImageUrlValue::Str(s) => s.clone(),
                                    ImageUrlValue::ObjUrl { url } => url.clone(),
                                    ImageUrlValue::ObjUri { uri } => uri.clone(),
                                };
                                let url = Self::normalize_image_url(url, None, &log, msg_idx, block_idx);
                                if !url.is_empty() && msg.role == "user" {
                                    ensure_image_hint(&mut current_msg_content);
                                    log(&format!(
                                        "🖼️ [Message #{} Block #{}] ImageUrl processed (len={})",
                                        msg_idx,
                                        block_idx,
                                        url.len()
                                    ));
                                    current_msg_content.push(json!({
                                        "type": "input_image",
                                        "image_url": url,
                                        "detail": "auto"
                                    }));
                                }
                            }
                            ContentBlock::InputImage { image_url, url, .. } => {
                                let resolved_url = match image_url {
                                    Some(ImageUrlValue::Str(s)) => s.clone(),
                                    Some(ImageUrlValue::ObjUrl { url }) => url.clone(),
                                    Some(ImageUrlValue::ObjUri { uri }) => uri.clone(),
                                    None => url.clone().unwrap_or_default(),
                                };
                                let resolved_url = Self::normalize_image_url(resolved_url, None, &log, msg_idx, block_idx);
                                if !resolved_url.is_empty() && msg.role == "user" {
                                    ensure_image_hint(&mut current_msg_content);
                                    log(&format!(
                                        "🖼️ [Message #{} Block #{}] InputImage processed (len={})",
                                        msg_idx,
                                        block_idx,
                                        resolved_url.len()
                                    ));
                                    current_msg_content.push(json!({
                                        "type": "input_image",
                                        "image_url": resolved_url,
                                        "detail": "auto"
                                    }));
                                }
                            }
                            ContentBlock::ToolUse { id, name, input: tool_input } => {
                                if !current_msg_content.is_empty() {
                                    input.push(json!({
                                        "type": "message",
                                        "role": msg.role,
                                        "content": current_msg_content
                                    }));
                                    current_msg_content = Vec::new();
                                }

                                if name.to_lowercase() == "skill" {
                                    log(&format!("🔧 [ToolUse] Skipping Skill tool_use: {:?}", id));
                                    continue;
                                }

                                input.push(json!({
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": serde_json::to_string(tool_input).unwrap_or_default()
                                }));
                            }
                            ContentBlock::ToolResult { tool_use_id, content: tool_content, .. } => {
                                let is_skill = if let Some(tid) = tool_use_id {
                                    skill_tool_ids.contains(tid)
                                } else {
                                    false
                                };

                                if is_skill || Self::is_potential_skill_result(tool_content) {
                                    if let Some((s_name, s_content)) = Self::extract_skill_info(tool_content) {
                                        let skill_formatted = Self::convert_to_codex_skill_format(&s_name, &s_content);
                                        extracted_skills.push(skill_formatted);
                                        log(&format!("🎯 Skill extracted: {}", s_name));
                                        continue;
                                    }
                                }

                                if !current_msg_content.is_empty() {
                                    input.push(json!({
                                        "type": "message",
                                        "role": msg.role,
                                        "content": current_msg_content
                                    }));
                                    current_msg_content = Vec::new();
                                }

                                let result_text = if let Some(cv) = tool_content {
                                    match cv {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Array(arr) => {
                                            arr.iter().filter_map(|item| {
                                                if let serde_json::Value::Object(obj) = item {
                                                    if let Some(serde_json::Value::String(text)) = obj.get("text") {
                                                        return Some(text.clone());
                                                    }
                                                }
                                                None
                                            }).collect::<Vec<_>>().join("\n")
                                        },
                                        _ => cv.to_string(),
                                    }
                                } else {
                                    String::new()
                                };

                                input.push(json!({
                                    "type": "function_call_output",
                                    "call_id": tool_use_id,
                                    "output": result_text
                                }));
                            }
                            ContentBlock::Document { .. } => {
                                current_msg_content.push(json!({
                                    "type": text_type,
                                    "text": "[document omitted]"
                                }));
                            }
                            ContentBlock::OtherValue(v) => {
                                let text = serde_json::to_string(v).unwrap_or_else(|_| "[unknown content]".to_string());
                                current_msg_content.push(json!({
                                    "type": text_type,
                                    "text": text
                                }));
                            }
                        }
                    }

                    if !current_msg_content.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": msg.role,
                            "content": current_msg_content
                        }));
                    }
                }
            }
        }

        (input, extracted_skills)
    }

    fn normalize_image_url<F>(
        url: String,
        _media_type: Option<&str>,
        _log: &F,
        _msg_idx: usize,
        _block_idx: usize,
    ) -> String
    where
        F: Fn(&str),
    {
        url
    }

    fn resolve_image_url<F>(
        source: &ImageSource,
        log: &F,
        msg_idx: usize,
        block_idx: usize,
    ) -> String
    where
        F: Fn(&str),
    {
        if let Some(url) = &source.url {
            let media_type = source.media_type.as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");
            let normalized = Self::normalize_image_url(url.clone(), Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source.url: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(uri) = &source.uri {
            let media_type = source.media_type.as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");
            let normalized = Self::normalize_image_url(uri.clone(), Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source.uri: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(path) = &source.path {
            let media_type = source.media_type.as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");
            let file_url = if path.starts_with("file://") {
                path.clone()
            } else {
                format!("file://{}", path)
            };
            let normalized = Self::normalize_image_url(file_url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source.path: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(data) = &source.data {
            let media_type = source.media_type.as_deref()
                .or(source.mime_type.as_deref())
                .unwrap_or("image/png");

            log(&format!(
                "🖼️ [Message #{} Block #{}] Image base64: media={}, size={} bytes, prefix={}",
                msg_idx,
                block_idx,
                media_type,
                data.len(),
                truncate_for_log(data, 20)
            ));

            if data.starts_with("data:") {
                return data.clone();
            }
            return format!("data:{};base64,{}", media_type, data);
        }

        log(&format!(
            "🖼️ [Message #{} Block #{}] Image source is empty (no url/uri/data)",
            msg_idx,
            block_idx
        ));
        String::new()
    }

    fn resolve_image_url_raw<F>(
        source: &Value,
        log: &F,
        msg_idx: usize,
        block_idx: usize,
    ) -> String
    where
        F: Fn(&str),
    {
        let Some(obj) = source.as_object() else {
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw is not object",
                msg_idx,
                block_idx
            ));
            return String::new();
        };

        let keys = obj.keys().cloned().collect::<Vec<_>>().join(",");
        log(&format!(
            "🖼️ [Message #{} Block #{}] Image source raw keys: {}",
            msg_idx,
            block_idx,
            keys
        ));

        let media_type = obj.get("media_type")
            .or_else(|| obj.get("mediaType"))
            .or_else(|| obj.get("mime_type"))
            .or_else(|| obj.get("mimeType"))
            .and_then(|v| v.as_str())
            .unwrap_or("image/png");

        let extract_str = |value: &Value| -> Option<String> {
            if let Some(s) = value.as_str() {
                return Some(s.to_string());
            }
            if let Some(obj) = value.as_object() {
                if let Some(s) = obj.get("url").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
                if let Some(s) = obj.get("uri").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
                if let Some(s) = obj.get("data").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
                if let Some(s) = obj.get("base64").and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
            }
            None
        };

        if let Some(url) = obj.get("url").and_then(|v| extract_str(v)) {
            let normalized = Self::normalize_image_url(url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.url: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(uri) = obj.get("uri").and_then(|v| extract_str(v)) {
            let normalized = Self::normalize_image_url(uri, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.uri: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        if let Some(image_url) = obj.get("image_url").and_then(|v| extract_str(v)) {
            let normalized = Self::normalize_image_url(image_url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.image_url: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        let path_value = obj.get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| obj.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .or_else(|| obj.get("filePath").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .or_else(|| obj.get("local_path").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .or_else(|| obj.get("localPath").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .or_else(|| obj.get("file").and_then(|v| v.as_str()).map(|s| s.to_string()));

        if let Some(path) = path_value {
            let file_url = if path.starts_with("file://") {
                path
            } else {
                format!("file://{}", path)
            };
            let normalized = Self::normalize_image_url(file_url, Some(media_type), log, msg_idx, block_idx);
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image source raw.path: {}",
                msg_idx,
                block_idx,
                truncate_for_log(&normalized, 50)
            ));
            return normalized;
        }

        let data = obj.get("data")
            .and_then(|v| extract_str(v))
            .or_else(|| obj.get("base64").and_then(|v| v.as_str()).map(|s| s.to_string()));

        if let Some(data) = data {
            log(&format!(
                "🖼️ [Message #{} Block #{}] Image raw base64: media={}, size={} bytes, prefix={}",
                msg_idx,
                block_idx,
                media_type,
                data.len(),
                truncate_for_log(&data, 20)
            ));
            if data.starts_with("data:") {
                return data;
            }
            return format!("data:{};base64,{}", media_type, data);
        }

        log(&format!(
            "🖼️ [Message #{} Block #{}] Image source raw is empty",
            msg_idx,
            block_idx
        ));
        String::new()
    }

    fn transform_tools(
        tools: Option<&Vec<Value>>,
        log_tx: Option<&broadcast::Sender<String>>,
    ) -> Vec<Value> {
        // 获取全局日志记录器
        let logger = AppLogger::get();

        // 辅助函数：同时写入 broadcast 和文件
        let log = |msg: &str| {
            if is_debug_log_enabled() {
                if let Some(tx) = log_tx {
                    let _ = tx.send(msg.to_string());
                }
                if let Some(ref l) = logger {
                    l.log(msg);
                }
            }
        };

        let Some(tools) = tools else {
            log("🔧 [Tools] No tools provided, using defaults");
            return Self::default_tools();
        };

        if tools.is_empty() {
            log("🔧 [Tools] Empty tools array, using defaults");
            return Self::default_tools();
        }

        log(&format!("🔧 [Tools] Processing {} tools", tools.len()));

        tools
            .iter()
            .filter(|tool| {
                let name = tool
                    .get("name")
                    .or_else(|| tool.get("function").and_then(|f| f.get("name")))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                if name.to_lowercase() == "skill" {
                    log(&format!("🔧 [Tools] Filtered out: {}", name));
                    return false;
                }
                true
            })
            .map(|tool| {
// Claude Code 格式: { name, description, input_schema }
                if tool.get("name").is_some() && tool.get("type").is_none() {
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                    log(&format!("🔧 [Tools] {} (Claude Code format)", name));

                    let mut parameters = tool.get("input_schema").cloned().unwrap_or_else(|| {
                        json!({
                            "type": "object",
                            "properties": {}
                        })
                    });

                    if let Some(obj) = parameters.as_object_mut() {
                        obj.entry("properties").or_insert_with(|| json!({}));
                    }

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": tool.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                        "strict": false,
                        "parameters": parameters
                    });
                }

                // Anthropic 格式: { type: "tool", name, ... }
                if tool.get("type").and_then(|t| t.as_str()) == Some("tool") {
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                    log(&format!("🔧 [Tools] {} (Anthropic format)", name));

                    let mut parameters = tool.get("input_schema").cloned().unwrap_or_else(|| {
                        json!({
                            "type": "object",
                            "properties": {}
                        })
                    });

                    if let Some(obj) = parameters.as_object_mut() {
                        obj.entry("properties").or_insert_with(|| json!({}));
                    }

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": tool.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                        "strict": false,
                        "parameters": parameters
                    });
                }

                // OpenAI 格式: { type: "function", function: {...} }
                if tool.get("type").and_then(|t| t.as_str()) == Some("function") {
                    let func = tool.get("function").unwrap_or(tool);
                    let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                    log(&format!("🔧 [Tools] {} (OpenAI format)", name));

                    let mut parameters = func.get("parameters").cloned().unwrap_or_else(|| {
                        json!({
                            "type": "object",
                            "properties": {}
                        })
                    });

                    if let Some(obj) = parameters.as_object_mut() {
                        obj.entry("properties").or_insert_with(|| json!({}));
                    }

                    return json!({
                        "type": "function",
                        "name": name,
                        "description": func.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                        "strict": false,
                        "parameters": parameters
                    });
                }

                // 未知格式
                let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                log(&format!("🔧 [Tools] {} (unknown format)", name));

                let mut parameters = tool.get("input_schema").cloned().unwrap_or_else(|| {
                    json!({
                        "type": "object",
                        "properties": {}
                    })
                });

                if let Some(obj) = parameters.as_object_mut() {
                    obj.entry("properties").or_insert_with(|| json!({}));
                }

                json!({
                    "type": "function",
                    "name": name,
                    "description": tool.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                    "strict": false,
                    "parameters": parameters
                })
            })
            .collect()
    }

    fn default_tools() -> Vec<Value> {
        let template_path = std::path::Path::new("codex-request.json");
        if let Ok(template_content) = std::fs::read_to_string(template_path) {
            if let Ok(template) = serde_json::from_str::<Value>(&template_content) {
                if let Some(tools) = template.get("tools").and_then(|t| t.as_array()) {
                    return tools.clone();
                }
            }
        }
        
        vec![json!({
            "type": "function",
            "name": "shell_command",
            "description": "Runs a shell command and returns its output.",
            "strict": false,
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell script to execute"
                    }
                },
                "required": ["command"]
            }
        })]
    }

    fn is_potential_skill_result(content: &Option<Value>) -> bool {
        let Some(content_val) = content else { return false; };
        let text = match content_val {
            Value::String(s) => s.as_str(),
            Value::Array(arr) => {
                for item in arr {
                    if let Value::Object(obj) = item {
                        if let Some(Value::String(t)) = obj.get("text") {
                            if t.contains("<command-name>") || t.contains("Base Path:") {
                                return true;
                            }
                        }
                    }
                }
                ""
            }
            _ => "",
        };
        text.contains("<command-name>") || text.contains("Base Path:")
    }

    fn extract_skill_info(content: &Option<Value>) -> Option<(String, String)> {
        let content_val = content.as_ref()?;
        let full_text = match content_val {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|item| {
                    if let Value::Object(obj) = item {
                        if let Some(Value::String(text)) = obj.get("text") {
                            return Some(text.clone());
                        }
                    }
                    if let Value::String(s) = item {
                        return Some(s.clone());
                    }
                    None
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => content_val.to_string(),
        };

        if !full_text.contains("<command-name>") && !full_text.contains("Base Path:") {
            return None;
        }

        let skill_name = if let Some(start) = full_text.find("<command-name>") {
            let sub = &full_text[start + 14..];
            let end = sub.find("</command-name>")?;
            sub[..end].trim().trim_start_matches('/').to_string()
        } else {
            return None;
        };

        let skill_content = if let Some(path_idx) = full_text.find("Base Path:") {
            let next_line = full_text[path_idx..].find('\n')?;
            full_text[path_idx + next_line..].trim().to_string()
        } else {
            full_text
                .replace(&format!("<command-name>{}</command-name>", skill_name), "")
                .replace(&format!("<command-name>/{}</command-name>", skill_name), "")
                .trim()
                .to_string()
        };

        if skill_name.is_empty() || skill_content.is_empty() {
            return None;
        }

        Some((skill_name, skill_content))
    }

    fn convert_to_codex_skill_format(name: &str, content: &str) -> String {
        format!("<skill>\n<name>{}</name>\n<path>unknown</path>\n{}\n</skill>", name, content)
    }
}

/// 响应转换器 - Codex SSE -> Anthropic SSE
pub struct TransformResponse {
    message_id: String,
    model: String,
    content_index: usize,
    open_text_index: Option<usize>,
    open_tool_index: Option<usize>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    saw_tool_call: bool,
    sent_message_start: bool,
}

impl TransformResponse {
    pub fn new(model: &str) -> Self {
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            content_index: 0,
            open_text_index: None,
            open_tool_index: None,
            tool_call_id: None,
            tool_name: None,
            saw_tool_call: false,
            sent_message_start: false,
        }
    }

    pub fn transform_line(&mut self, line: &str) -> Vec<String> {
        let mut output = Vec::new();

        if !line.starts_with("data: ") {
            return output;
        }

        // 发送 message_start
        if !self.sent_message_start {
            self.sent_message_start = true;
            output.push(format!(
                "event: message_start\ndata: {}\n\n",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": self.message_id,
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": self.model,
                        "stop_reason": null,
                        "usage": { "input_tokens": 0, "output_tokens": 0 }
                    }
                })
            ));
        }

        let Ok(data) = serde_json::from_str::<Value>(&line[6..]) else {
            return output;
        };

        let event_type = data.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            // 文本输出
            "response.output_text.delta" => {
                if self.open_text_index.is_none() {
                    let idx = self.content_index;
                    self.content_index += 1;
                    self.open_text_index = Some(idx);
                    output.push(format!(
                        "event: content_block_start\ndata: {}\n\n",
                        json!({
                            "type": "content_block_start",
                            "index": idx,
                            "content_block": { "type": "text", "text": "" }
                        })
                    ));
                }

                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                output.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    json!({
                        "type": "content_block_delta",
                        "index": self.open_text_index,
                        "delta": { "type": "text_delta", "text": delta }
                    })
                ));
            }

            // 工具调用开始
            "response.output_item.added" => {
                if let Some(item) = data.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                        self.saw_tool_call = true;

                        // 关闭文本块
                        if let Some(idx) = self.open_text_index.take() {
                            output.push(format!(
                                "event: content_block_stop\ndata: {}\n\n",
                                json!({ "type": "content_block_stop", "index": idx })
                            ));
                        }

                        let call_id = item
                            .get("call_id")
                            .and_then(|c| c.as_str())
                            .unwrap_or("tool_0")
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        self.tool_call_id = Some(call_id.clone());
                        self.tool_name = Some(name.clone());

                        let idx = self.content_index;
                        self.content_index += 1;
                        self.open_tool_index = Some(idx);

                        output.push(format!(
                            "event: content_block_start\ndata: {}\n\n",
                            json!({
                                "type": "content_block_start",
                                "index": idx,
                                "content_block": {
                                    "type": "tool_use",
                                    "id": call_id,
                                    "name": name,
                                    "input": {}
                                }
                            })
                        ));
                    }
                }
            }

            // 工具调用参数
            "response.function_call_arguments.delta" | "response.function_call_arguments_delta" => {
                if self.open_tool_index.is_none() {
                    self.saw_tool_call = true;

                    // 关闭文本块
                    if let Some(idx) = self.open_text_index.take() {
                        output.push(format!(
                            "event: content_block_stop\ndata: {}\n\n",
                            json!({ "type": "content_block_stop", "index": idx })
                        ));
                    }

                    let call_id = self
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| format!("tool_{}", chrono::Utc::now().timestamp_millis()));
                    let name = self.tool_name.clone().unwrap_or_else(|| "unknown".to_string());

                    let idx = self.content_index;
                    self.content_index += 1;
                    self.open_tool_index = Some(idx);

                    output.push(format!(
                        "event: content_block_start\ndata: {}\n\n",
                        json!({
                            "type": "content_block_start",
                            "index": idx,
                            "content_block": {
                                "type": "tool_use",
                                "id": call_id,
                                "name": name,
                                "input": {}
                            }
                        })
                    ));
                }

                let delta = data
                    .get("delta")
                    .or_else(|| data.get("arguments"))
                    .map(|d| {
                        if d.is_string() {
                            d.as_str().unwrap_or("").to_string()
                        } else {
                            serde_json::to_string(d).unwrap_or_default()
                        }
                    })
                    .unwrap_or_default();

                output.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    json!({
                        "type": "content_block_delta",
                        "index": self.open_tool_index,
                        "delta": { "type": "input_json_delta", "partial_json": delta }
                    })
                ));
            }

            // 工具调用完成
            "response.output_item.done" => {
                if let Some(idx) = self.open_tool_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }
                self.tool_call_id = None;
                self.tool_name = None;
            }

            // 响应完成
            "response.completed" => {
                // 关闭所有打开的块
                if let Some(idx) = self.open_text_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }
                if let Some(idx) = self.open_tool_index.take() {
                    output.push(format!(
                        "event: content_block_stop\ndata: {}\n\n",
                        json!({ "type": "content_block_stop", "index": idx })
                    ));
                }

                let stop_reason = if self.saw_tool_call {
                    "tool_use"
                } else {
                    "end_turn"
                };

                // 发送 message_delta
                let usage = data
                    .get("response")
                    .and_then(|r| r.get("usage"))
                    .cloned()
                    .unwrap_or(json!({}));

                output.push(format!(
                    "event: message_delta\ndata: {}\n\n",
                    json!({
                        "type": "message_delta",
                        "delta": { "stop_reason": stop_reason },
                        "usage": {
                            "input_tokens": usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0),
                            "output_tokens": usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0)
                        }
                    })
                ));

                // 发送 message_stop
                output.push(format!(
                    "event: message_stop\ndata: {}\n\n",
                    json!({ "type": "message_stop", "stop_reason": stop_reason })
                ));
            }

            _ => {}
        }

        output
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_reasoning_effort_opus_default() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("claude-3-opus-20240229", &mapping);
        assert_eq!(effort, ReasoningEffort::Xhigh);
    }

    #[test]
    fn test_reasoning_effort_sonnet_default() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("claude-sonnet-4-20250514", &mapping);
        assert_eq!(effort, ReasoningEffort::Medium);
    }

    #[test]
    fn test_reasoning_effort_haiku_default() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("claude-3-5-haiku-20241022", &mapping);
        assert_eq!(effort, ReasoningEffort::Low);
    }

    #[test]
    fn test_custom_mapping_applied() {
        let mut mapping = ReasoningEffortMapping::default();
        mapping.sonnet = ReasoningEffort::High;
        
        let effort = get_reasoning_effort("claude-sonnet-4-20250514", &mapping);
        assert_eq!(effort, ReasoningEffort::High);
    }

    #[test]
    fn test_reasoning_effort_as_str() {
        assert_eq!(ReasoningEffort::Xhigh.as_str(), "xhigh");
        assert_eq!(ReasoningEffort::High.as_str(), "high");
        assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
    }

    #[test]
    fn test_reasoning_effort_from_str() {
        assert_eq!(ReasoningEffort::from_str("xhigh"), ReasoningEffort::Xhigh);
        assert_eq!(ReasoningEffort::from_str("HIGH"), ReasoningEffort::High);
        assert_eq!(ReasoningEffort::from_str("Medium"), ReasoningEffort::Medium);
        assert_eq!(ReasoningEffort::from_str("low"), ReasoningEffort::Low);
        assert_eq!(ReasoningEffort::from_str("invalid"), ReasoningEffort::Medium); // default
    }

    #[test]
    fn test_unknown_model_defaults_to_medium() {
        let mapping = ReasoningEffortMapping::default();
        let effort = get_reasoning_effort("gpt-4-turbo", &mapping);
        assert_eq!(effort, ReasoningEffort::Medium);
    }

    #[test]
    fn test_case_insensitive_model_matching() {
        let mapping = ReasoningEffortMapping::default();
        assert_eq!(get_reasoning_effort("CLAUDE-3-OPUS", &mapping), ReasoningEffort::Xhigh);
        assert_eq!(get_reasoning_effort("Claude-Sonnet-4", &mapping), ReasoningEffort::Medium);
        assert_eq!(get_reasoning_effort("claude-haiku", &mapping), ReasoningEffort::Low);
    }
}
