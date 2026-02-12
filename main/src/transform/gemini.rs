use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::AnthropicRequest;

use super::{ResponseTransformer, TransformBackend, TransformContext, MessageProcessor};

pub struct GeminiBackend;

impl GeminiBackend {
    fn normalize_model(model: &str) -> String {
        model.trim().to_string()
    }

    fn convert_content_block(block: &Value) -> Option<Value> {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        
        match block_type {
            "text" | "input_text" | "output_text" => {
                let text = block.get("text").and_then(|t| t.as_str())?;
                Some(json!({ "text": text }))
            },
            "thinking" | "thought" | "reasoning" => {
                let text = block.get("thinking").or_else(|| block.get("text")).and_then(|t| t.as_str())?;
                let sig = block.get("signature").and_then(|s| s.as_str());
                if let Some(s) = sig {
                    Some(json!({ "text": text, "thought": true, "thought_signature": s }))
                } else {
                    Some(json!({ "text": text, "thought": true }))
                }
            },
            "image" | "image_url" | "input_image" => {
                let source = block.get("source").or_else(|| block.get("image_url"));
                if let Some(src) = source {
                     // Handle data URI in image_url string (common in Codex output)
                     if let Some(url_str) = src.as_str() {
                         if url_str.starts_with("data:") {
                             let parts: Vec<&str> = url_str.splitn(2, ",").collect();
                             if parts.len() == 2 {
                                 let header = parts[0]; 
                                 let mime_type = header.trim_start_matches("data:").split(";").next().unwrap_or("image/jpeg");
                                 return Some(json!({
                                     "inline_data": {
                                         "mime_type": mime_type,
                                         "data": parts[1]
                                     }
                                 }));
                             }
                         }
                     }
                     // Handle object source
                     if let Some(data) = src.get("data").and_then(|d| d.as_str()) {
                         let mime_type = src.get("media_type").and_then(|m| m.as_str()).unwrap_or("image/jpeg");
                         return Some(json!({
                             "inline_data": {
                                 "mime_type": mime_type,
                                 "data": data
                             }
                         }));
                     }
                }
                None
            },
            // Note: Codex outputs function_call as separate item, not content block usually
            _ => None
        }
    }

    fn build_contents(messages: &[Value]) -> Vec<Value> {
        let mut gemini_messages: Vec<Value> = Vec::new();
        let mut tool_id_name_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        
        for item in messages {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("message");
            
            if item_type == "message" {
                let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                let content = item.get("content");
                let mut parts = Vec::new();
                
                if let Some(content_array) = content.and_then(|c| c.as_array()) {
                    for block in content_array {
                        if let Some(part) = Self::convert_content_block(block) {
                            parts.push(part);
                        }
                    }
                } else if let Some(text) = content.and_then(|c| c.as_str()) {
                     parts.push(json!({ "text": text }));
                }
                
                if parts.is_empty() { continue; }
                
                let gemini_role = if role == "assistant" { "model" } else { "user" };
                
                // Merge logic
                if let Some(last_msg) = gemini_messages.last_mut() {
                    if last_msg["role"] == gemini_role {
                        if let Some(last_parts) = last_msg["parts"].as_array_mut() {
                            last_parts.extend(parts);
                        }
                        continue;
                    }
                }
                
                gemini_messages.push(json!({
                    "role": gemini_role,
                    "parts": parts
                }));

            } else if item_type == "function_call" {
                 let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                 let args_str = item.get("arguments").and_then(|s| s.as_str()).unwrap_or("{}");
                 let args = serde_json::from_str::<Value>(args_str).unwrap_or(json!({}));
                 let signature = item.get("signature").and_then(|s| s.as_str());
                 
                 if let Some(id) = item.get("call_id").and_then(|i| i.as_str()) {
                     tool_id_name_map.insert(id.to_string(), name.to_string());
                 }

                 let fc_obj = json!({
                     "name": name,
                     "args": args
                 });
                 
                 let mut part = json!({
                     "functionCall": fc_obj
                 });

                 if let Some(s) = signature {
                     if let Some(obj) = part.as_object_mut() {
                         obj.insert("thought_signature".to_string(), json!(s));
                     }
                 }
                 
                 let gemini_role = "model";
                 if let Some(last_msg) = gemini_messages.last_mut() {
                    if last_msg["role"] == gemini_role {
                        if let Some(last_parts) = last_msg["parts"].as_array_mut() {
                            last_parts.push(part);
                        }
                        continue;
                    }
                 }
                 gemini_messages.push(json!({
                     "role": gemini_role,
                     "parts": [part]
                 }));

            } else if item_type == "function_call_output" {
                 let id = item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
                 let name = tool_id_name_map.get(id).map(|s| s.as_str()).unwrap_or("unknown_tool");
                 
                 let output_text = item.get("output").and_then(|s| s.as_str()).unwrap_or("");
                 let content = json!({ "result": output_text });

                 let part = json!({
                    "functionResponse": {
                        "name": name,
                        "response": content
                    }
                });
                
                // Gemini REST requires role 'user' for function response in some documentation, 
                // but 'function' is correct for the Python SDK. Let's try 'user' or 'function'.
                // Standard REST API often uses 'function' role.
                let gemini_role = "function"; 
                
                if let Some(last_msg) = gemini_messages.last_mut() {
                    if last_msg["role"] == gemini_role {
                        if let Some(last_parts) = last_msg["parts"].as_array_mut() {
                            last_parts.push(part);
                        }
                        continue;
                    }
                 }
                 
                 gemini_messages.push(json!({
                     "role": gemini_role,
                     "parts": [part]
                 }));
            }
        }
        
        gemini_messages
    }

    pub(crate) fn build_contents_for_count(messages: &[Value]) -> Vec<Value> {
        Self::build_contents(messages)
    }

    fn convert_tools(tools: Option<&Vec<Value>>) -> Option<Vec<Value>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }

        let function_declarations: Vec<Value> = tools.iter().filter_map(|tool| {
            // Anthropic format: { name, description, input_schema }
            // Gemini format: { name, description, parameters }
            let name = tool.get("name").and_then(|n| n.as_str())?;
            let description = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let input_schema = tool.get("input_schema").cloned().unwrap_or(json!({ "type": "object", "properties": {} }));
            
            Some(json!({
                "name": name,
                "description": description,
                "parameters": input_schema
            }))
        }).collect();

        if function_declarations.is_empty() {
            None
        } else {
            Some(vec![json!({
                "function_declarations": function_declarations
            })])
        }
    }
}

impl TransformBackend for GeminiBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        log_tx: Option<&broadcast::Sender<String>>,
        _ctx: &TransformContext,
        model_override: Option<String>,
    ) -> (Value, String) {
        let session_id = Uuid::new_v4().to_string();

        let requested = model_override
            .or_else(|| anthropic_body.model.clone())
            // Fallback (though model_override should usually be present)
            .unwrap_or_else(|| "gemini-3-pro-preview".to_string());
        let gemini_model = Self::normalize_model(&requested);

        // Transform messages using Codex logic as base but outputting Gemini format
        // We reuse TransformRequest::transform_messages to resolve images and extract basic info
        // BUT here we need to map to Gemini structure directly.
        // Actually, let's use the `build_contents` we just wrote, but we need `messages` with resolved images first.
        // Since `TransformRequest::transform_messages` returns Codex-specific JSON, it might be better to 
        // iterate `anthropic_body.messages` directly and resolve images using helper if needed, 
        // OR reuse `transform_messages` output if it's generic enough.
        // usage: let (messages, _) = TransformRequest::transform_messages(&anthropic_body.messages, None);
        // The output of `transform_messages` is `Vec<Value>` in Codex format (content blocks).
        // Our `convert_content_block` expects Codex-like blocks (type=text, image, tool_use).
        // So yes, we can reuse `TransformRequest::transform_messages` to handle the heavy lifting of image resolution!
        // Note: we pass None for log_tx to avoid double logging or just pass it if we want.
        
        let (messages, _) = MessageProcessor::transform_messages(&anthropic_body.messages, log_tx);
        let contents = Self::build_contents(&messages);

        let system_instruction = if let Some(system) = &anthropic_body.system {
             Some(json!({
                "parts": [{ "text": system.to_string() }]
            }))
        } else {
            None
        };

        let tools = Self::convert_tools(anthropic_body.tools.as_ref());

        // Configuration mapping
        let mut generation_config = json!({});
        if let Some(cfg) = generation_config.as_object_mut() {
            if let Some(max_tokens) = anthropic_body.max_tokens {
                cfg.insert("maxOutputTokens".to_string(), json!(max_tokens));
            }
            if let Some(temp) = anthropic_body.temperature {
                cfg.insert("temperature".to_string(), json!(temp));
            }
            if let Some(top_p) = anthropic_body.top_p {
                cfg.insert("topP".to_string(), json!(top_p));
            }
            if let Some(top_k) = anthropic_body.top_k {
                cfg.insert("topK".to_string(), json!(top_k));
            }
            if let Some(stop) = &anthropic_body.stop_sequences {
                cfg.insert("stopSequences".to_string(), json!(stop));
            }
        }

        let body = json!({
            "model": gemini_model,
            "system_instruction": system_instruction,
            "contents": contents,
            "tools": tools,
            "generationConfig": generation_config,
            "safetySettings": [
                {
                    "category": "HARM_CATEGORY_HARASSMENT",
                    "threshold": "BLOCK_NONE"
                },
                {
                    "category": "HARM_CATEGORY_HATE_SPEECH",
                    "threshold": "BLOCK_NONE"
                },
                {
                    "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                    "threshold": "BLOCK_NONE"
                },
                {
                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "threshold": "BLOCK_NONE"
                }
            ]
        });

        (body, session_id)
    }

    fn build_upstream_request(
        &self,
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        _session_id: &str,
        _anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        let model = body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gemini-3-pro-preview");

        let endpoint = if target_url.contains(":streamGenerateContent") {
            target_url.to_string()
        } else if target_url.contains("{model}") {
            target_url.replace("{model}", model)
        } else {
            let base = target_url.trim_end_matches('/');
            format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", base, model)
        };

        let mut upstream_body = body.clone();
        if let Some(obj) = upstream_body.as_object_mut() {
            obj.remove("model");
        }

        client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", api_key)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Accept", "text/event-stream")
            .body(upstream_body.to_string())
    }

    fn create_response_transformer(&self, model: &str) -> Box<dyn ResponseTransformer> {
        Box::new(GeminiResponseTransformer::new(model))
    }
}

pub struct GeminiResponseTransformer {
    message_id: String,
    model: String,
    content_index: usize,
    open_text_index: Option<usize>,
    open_text_block_kind: Option<TextBlockKind>,
    open_tool_index: Option<usize>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    saw_tool_call: bool,
    sent_message_start: bool,
    sent_message_stop: bool,
    thought_signature: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextBlockKind {
    Text,
    Thinking,
}

impl GeminiResponseTransformer {
    pub fn new(model: &str) -> Self {
        Self {
            message_id: format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            model: model.to_string(),
            content_index: 0,
            open_text_index: None,
            open_text_block_kind: None,
            open_tool_index: None,
            tool_call_id: None,
            tool_name: None,
            saw_tool_call: false,
            sent_message_start: false,
            sent_message_stop: false,
            thought_signature: None,
        }
    }

    fn ensure_message_start(&mut self, out: &mut Vec<String>) {
        if self.sent_message_start {
            return;
        }
        self.sent_message_start = true;
        out.push(format!(
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

    fn open_text_block_if_needed(&mut self, out: &mut Vec<String>) {
        if self.open_text_index.is_some() {
            if self.open_text_block_kind == Some(TextBlockKind::Text) {
                return;
            }
            self.close_text_block(out);
        }

        let idx = self.content_index;
        self.content_index += 1;
        self.open_text_index = Some(idx);
        self.open_text_block_kind = Some(TextBlockKind::Text);
        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "text", "text": "" }
            })
        ));
    }

    fn close_text_block(&mut self, out: &mut Vec<String>) {
        if let Some(idx) = self.open_text_index.take() {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
        self.open_text_block_kind = None;
    }

    fn open_tool_block_if_needed(&mut self, out: &mut Vec<String>) {
        if self.open_tool_index.is_some() {
            return;
        }
        self.saw_tool_call = true;
        self.close_text_block(out);

        let call_id = self
            .tool_call_id
            .clone()
            .unwrap_or_else(|| format!("tool_{}", chrono::Utc::now().timestamp_millis()));
        let tool_name = self
            .tool_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let idx = self.content_index;
        self.content_index += 1;
        self.open_tool_index = Some(idx);

        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": {
                    "type": "tool_use",
                    "id": call_id,
                    "name": tool_name,
                    "input": {}
                }
            })
        ));
    }

    fn close_tool_block(&mut self, out: &mut Vec<String>) {
        if let Some(idx) = self.open_tool_index.take() {
            out.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({ "type": "content_block_stop", "index": idx })
            ));
        }
    }

    fn open_thinking_block_if_needed(&mut self, out: &mut Vec<String>, signature: Option<&str>) {
        if self.open_text_index.is_some() {
            if self.open_text_block_kind == Some(TextBlockKind::Thinking) {
                return;
            }
            self.close_text_block(out);
        }

        let idx = self.content_index;
        self.content_index += 1;
        self.open_text_index = Some(idx);
        self.open_text_block_kind = Some(TextBlockKind::Thinking);

        out.push(format!(
            "event: content_block_start\ndata: {}\n\n",
            json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": {
                    "type": "thinking",
                    "thinking": "",
                    "signature": signature
                }
            })
        ));
    }

    fn emit_message_stop(&mut self, out: &mut Vec<String>, stop_reason: &str) {
        if self.sent_message_stop {
            return;
        }
        self.sent_message_stop = true;

        self.close_text_block(out);
        self.close_tool_block(out);

        out.push(format!(
            "event: message_delta\ndata: {}\n\n",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason },
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            })
        ));
        out.push(format!(
            "event: message_stop\ndata: {}\n\n",
            json!({ "type": "message_stop", "stop_reason": stop_reason })
        ));
    }

    fn extract_thinking_from_candidates(data: &Value) -> Vec<String> {
        data.get("candidates")
            .and_then(|v| v.as_array())
            .map(|candidates| {
                candidates
                    .iter()
                    .flat_map(|candidate| {
                        candidate
                            .get("content")
                            .and_then(|v| v.get("parts"))
                            .and_then(|v| v.as_array())
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(|part| {
                                        if part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
                                            part.get("text")
                                                .and_then(|t| t.as_str())
                                                .map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn extract_thought_signature(data: &Value) -> Option<String> {
        let candidates = data.get("candidates")?.as_array()?;
        for candidate in candidates {
            // Check top-level of candidate first
            if let Some(sig) = candidate.get("thoughtSignature").and_then(|v| v.as_str()) {
                return Some(sig.to_string());
            }
            // Check in parts (sometimes it's inside a part)
            if let Some(parts) = candidate.get("content").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
                for part in parts {
                    if let Some(sig) = part.get("thoughtSignature").and_then(|v| v.as_str()) {
                        return Some(sig.to_string());
                    }
                }
            }
        }
        None
    }

    fn extract_tool_call(data: &Value) -> Option<(String, Value)> {
        let candidates = data.get("candidates")?.as_array()?;
        for candidate in candidates {
            let parts = candidate
                .get("content")
                .and_then(|v| v.get("parts"))
                .and_then(|v| v.as_array())?;

            for part in parts {
                if let Some(function_call) = part.get("functionCall") {
                    let name = function_call
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let args = function_call.get("args").cloned().unwrap_or_else(|| json!({}));
                    return Some((name, args));
                }
            }
        }
        None
    }

    fn has_finish_reason(data: &Value) -> bool {
        data.get("candidates")
            .and_then(|v| v.as_array())
            .map(|candidates| {
                candidates.iter().any(|candidate| {
                    candidate
                        .get("finishReason")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }
}

impl ResponseTransformer for GeminiResponseTransformer {
    fn transform_line(&mut self, line: &str) -> Vec<String> {
        let mut output = Vec::new();

        if !line.starts_with("data: ") {
            return output;
        }

        self.ensure_message_start(&mut output);

        let payload = line[6..].trim();
        if payload == "[DONE]" {
            let stop_reason = if self.saw_tool_call { "tool_use" } else { "end_turn" };
            self.emit_message_stop(&mut output, stop_reason);
            return output;
        }

        let Ok(data) = serde_json::from_str::<Value>(payload) else {
            return output;
        };

        // 1. Extract thought signature if present
        if let Some(sig) = Self::extract_thought_signature(&data) {
            self.thought_signature = Some(sig);
        }

        // 2. Process Thinking/Thought
        let sig = self.thought_signature.clone();
        for thinking in Self::extract_thinking_from_candidates(&data) {
            if thinking.is_empty() { continue; }
            self.open_thinking_block_if_needed(&mut output, sig.as_deref());
            output.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": self.open_text_index,
                    "delta": { "type": "thinking_delta", "thinking": thinking }
                })
            ));
        }

        // 3. Process normal text
        data.get("candidates")
            .and_then(|v| v.as_array())
            .map(|candidates| {
                for candidate in candidates {
                    if let Some(parts) = candidate.get("content").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
                        for part in parts {
                            // Only process text that is NOT thought
                            if part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
                                continue;
                            }
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                if text.is_empty() { continue; }
                                self.open_text_block_if_needed(&mut output);
                                output.push(format!(
                                    "event: content_block_delta\ndata: {}\n\n",
                                    json!({
                                        "type": "content_block_delta",
                                        "index": self.open_text_index,
                                        "delta": { "type": "text_delta", "text": text }
                                    })
                                ));
                            }
                        }
                    }
                }
            });

        // 4. Process tool calls

        if let Some((tool_name, args)) = Self::extract_tool_call(&data) {
            self.tool_name = Some(tool_name);
            self.tool_call_id = Some(format!("tool_{}", chrono::Utc::now().timestamp_millis()));
            self.open_tool_block_if_needed(&mut output);

            let partial_json = if args.is_string() {
                args.as_str().unwrap_or("").to_string()
            } else {
                serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
            };
            output.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": self.open_tool_index,
                    "delta": { "type": "input_json_delta", "partial_json": partial_json }
                })
            ));
            self.close_tool_block(&mut output);
            self.tool_call_id = None;
            self.tool_name = None;
        }

        if Self::has_finish_reason(&data) {
            let stop_reason = if self.saw_tool_call { "tool_use" } else { "end_turn" };
            self.emit_message_stop(&mut output, stop_reason);
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_sse_event(raw: &str) -> (String, Value) {
        let mut event = None;
        let mut data = None;

        for line in raw.lines() {
            if let Some(value) = line.strip_prefix("event: ") {
                event = Some(value.to_string());
            }
            if let Some(value) = line.strip_prefix("data: ") {
                data = serde_json::from_str::<Value>(value).ok();
            }
        }

        (
            event.expect("missing SSE event name"),
            data.expect("missing SSE data payload"),
        )
    }

    #[test]
    fn switches_to_dedicated_text_block_after_thinking() {
        let mut transformer = GeminiResponseTransformer::new("gemini-test");
        let line = r#"data: {"candidates":[{"content":{"parts":[{"thought":true,"text":"internal reasoning"},{"text":"final answer"}]}}]}"#;

        let events = transformer.transform_line(line);
        let parsed_events: Vec<(String, Value)> = events.iter().map(|event| parse_sse_event(event)).collect();

        let thinking_start = parsed_events
            .iter()
            .position(|(name, payload)| {
                name == "content_block_start"
                    && payload
                        .get("content_block")
                        .and_then(|block| block.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("thinking")
            })
            .expect("missing thinking block start event");

        let thinking_index = parsed_events[thinking_start]
            .1
            .get("index")
            .and_then(|value| value.as_u64())
            .expect("thinking block index should exist");

        let thinking_stop = parsed_events
            .iter()
            .position(|(name, payload)| {
                name == "content_block_stop"
                    && payload
                        .get("index")
                        .and_then(|value| value.as_u64())
                        == Some(thinking_index)
            })
            .expect("missing thinking block stop event");

        let text_start = parsed_events
            .iter()
            .position(|(name, payload)| {
                name == "content_block_start"
                    && payload
                        .get("content_block")
                        .and_then(|block| block.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("text")
            })
            .expect("missing text block start event");

        let text_index = parsed_events[text_start]
            .1
            .get("index")
            .and_then(|value| value.as_u64())
            .expect("text block index should exist");

        let text_delta = parsed_events
            .iter()
            .position(|(name, payload)| {
                name == "content_block_delta"
                    && payload
                        .get("index")
                        .and_then(|value| value.as_u64())
                        == Some(text_index)
                    && payload
                        .get("delta")
                        .and_then(|delta| delta.get("type"))
                        .and_then(|value| value.as_str())
                        == Some("text_delta")
                    && payload
                        .get("delta")
                        .and_then(|delta| delta.get("text"))
                        .and_then(|value| value.as_str())
                        == Some("final answer")
            })
            .expect("missing text delta on text block");

        assert_ne!(thinking_index, text_index, "thinking/text should use different block indices");
        assert!(thinking_stop < text_start, "thinking block should stop before text starts");
        assert!(text_start < text_delta, "text delta should follow text block start");
    }

}
