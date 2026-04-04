use std::collections::HashMap;

use serde_json::Value;
use tokio::sync::broadcast;

use super::response::TransformResponse;
use crate::logger::AppLogger;
use crate::models::AnthropicRequest;
use crate::transform::{
    providers::CodexAdapter, request_envelope_hints_from_anthropic, RequestEnvelopeHints,
    processor::{ExtractedSkillPayload, MessageProcessor},
    unified::{
        sanitize_agent_worktree_history, UnifiedContent, UnifiedMessage, UnifiedMessageRole,
    },
    ResponseTransformer, TransformBackend, TransformContext, UnifiedChatRequest,
};

/// Codex 后端 —— 将 Anthropic 请求转为 Codex Responses API 格式
pub struct CodexBackend;

pub(crate) fn build_codex_unified_request(
    anthropic_body: &AnthropicRequest,
) -> (UnifiedChatRequest, RequestEnvelopeHints) {
    let mut unified = UnifiedChatRequest::from_anthropic(anthropic_body);
    let hints = request_envelope_hints_from_anthropic(anthropic_body);
    let (_, extracted_skills) = MessageProcessor::transform_messages(&anthropic_body.messages, None);
    let worktree_stats = sanitize_agent_worktree_history(&mut unified);

    let appended_skill_outputs = append_extracted_skill_outputs(&mut unified, &extracted_skills);
    let stripped_skill_scaffolding =
        strip_skill_scaffolding_user_messages(&mut unified, !extracted_skills.is_empty());

    if hints.request_kind == crate::transform::ClaudeCodeRequestKind::ConversationTurn
        && unified.has_system_text()
    {
        unified.append_system_texts(crate::prompts::codex_system_prompt_extensions());
    }

    if let Some(logger) = AppLogger::get() {
        logger.log_raw(&format!(
            "[CodexSkillBridge] extracted_skills={} appended_skill_outputs={} stripped_skill_scaffolding={} stripped_agent_worktree_calls={} dropped_agent_worktree_tool_errors={}",
            extracted_skills.len(),
            appended_skill_outputs,
            stripped_skill_scaffolding,
            worktree_stats.stripped_call_isolation,
            worktree_stats.dropped_tool_errors
        ));
    }

    (unified, hints)
}

fn append_extracted_skill_outputs(
    unified: &mut UnifiedChatRequest,
    extracted_skills: &[ExtractedSkillPayload],
) -> usize {
    if extracted_skills.is_empty() {
        return 0;
    }

    let mut skills_by_call_id: HashMap<String, Vec<&ExtractedSkillPayload>> = HashMap::new();
    for skill in extracted_skills {
        let Some(call_id) = skill
            .tool_use_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        skills_by_call_id
            .entry(call_id.to_string())
            .or_default()
            .push(skill);
    }

    if skills_by_call_id.is_empty() {
        return 0;
    }

    let mut appended = 0usize;
    let mut enriched = Vec::with_capacity(unified.messages.len() + extracted_skills.len());
    for message in &unified.messages {
        enriched.push(message.clone());

        if message.role != UnifiedMessageRole::Tool {
            continue;
        }

        let Some(call_id) = message
            .tool_call_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let Some(skill_payloads) = skills_by_call_id.remove(call_id) else {
            continue;
        };

        for skill in skill_payloads {
            enriched.push(UnifiedMessage {
                role: UnifiedMessageRole::Tool,
                content: vec![UnifiedContent::Text {
                    text: skill.payload.clone(),
                }],
                tool_calls: Vec::new(),
                tool_call_id: Some(call_id.to_string()),
                thinking: None,
            });
            appended += 1;
        }
    }

    unified.messages = enriched;
    appended
}

fn strip_skill_scaffolding_user_messages(
    unified: &mut UnifiedChatRequest,
    has_extracted_skills: bool,
) -> usize {
    if !has_extracted_skills {
        return 0;
    }

    let before = unified.messages.len();
    unified.messages.retain(|message| {
        if message.role != UnifiedMessageRole::User {
            return true;
        }

        let Some(text) = message.content_text() else {
            return true;
        };
        let trimmed = text.trim();
        !trimmed.starts_with("Base directory for this skill:")
            && !trimmed.starts_with("Base Path:")
            && !trimmed.contains("<command-name>")
    });
    before.saturating_sub(unified.messages.len())
}

struct CodexUpstreamRequestBuilder;

impl CodexUpstreamRequestBuilder {
    fn apply_standard_headers(
        builder: reqwest::RequestBuilder,
        api_key: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        builder
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("x-api-key", api_key)
            .header("User-Agent", "Anthropic-Node/0.3.4")
            .header("x-anthropic-version", anthropic_version)
            .header("originator", "codex_cli_rs")
            .header("Accept", "text/event-stream")
    }

    fn apply_session_headers(
        builder: reqwest::RequestBuilder,
        session_id: &str,
    ) -> reqwest::RequestBuilder {
        builder
            .header("conversation_id", session_id)
            .header("session_id", session_id)
    }

    fn build_request(
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        session_id: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        let builder = client.post(target_url);
        let builder = Self::apply_standard_headers(builder, api_key, anthropic_version);
        let builder = Self::apply_session_headers(builder, session_id);
        builder.body(body.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::build_codex_unified_request;
    use super::CodexUpstreamRequestBuilder;
    use crate::models::AnthropicRequest;
    use serde_json::json;

    #[test]
    fn codex_upstream_request_builder_sets_transport_headers_and_session_ids() {
        let client = reqwest::Client::new();
        let body = json!({"model": "gpt-5.3-codex", "input": [], "stream": true});

        let request = CodexUpstreamRequestBuilder::build_request(
            &client,
            "https://example.com/v1/responses",
            "test-key",
            &body,
            "session-123",
            "2023-06-01",
        )
        .build()
        .expect("request should build");

        assert_eq!(request.url().as_str(), "https://example.com/v1/responses");
        assert_eq!(
            request
                .headers()
                .get("Authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-key")
        );
        assert_eq!(
            request
                .headers()
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("test-key")
        );
        assert_eq!(
            request
                .headers()
                .get("Accept")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        assert_eq!(
            request
                .headers()
                .get("conversation_id")
                .and_then(|value| value.to_str().ok()),
            Some("session-123")
        );
        assert_eq!(
            request
                .headers()
                .get("session_id")
                .and_then(|value| value.to_str().ok()),
            Some("session-123")
        );
    }

    #[test]
    fn build_codex_unified_request_reinjects_extracted_skill_payloads() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "stream": true,
            "messages": [
                {
                    "role": "user",
                    "content": "帮我并行开 3 个 subagent 查天气"
                },
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_skill_1",
                        "name": "Skill",
                        "input": { "skill": "superpowers:dispatching-parallel-agents" }
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_skill_1",
                        "content": "<command-name>superpowers:dispatching-parallel-agents</command-name>\nBase Path: /tmp\n并行分派时必须一次性启动多个 subagent。"
                    }]
                }
            ]
        }))
        .expect("request");

        let (unified, _) = build_codex_unified_request(&request);

        let tool_messages: Vec<_> = unified
            .messages
            .iter()
            .filter(|message| {
                message.tool_call_id.as_deref() == Some("call_skill_1")
                    && message.role == crate::transform::unified::UnifiedMessageRole::Tool
            })
            .collect();

        assert_eq!(tool_messages.len(), 2);
        assert_eq!(
            tool_messages[0].content_text().as_deref(),
            Some("<command-name>superpowers:dispatching-parallel-agents</command-name>\nBase Path: /tmp\n并行分派时必须一次性启动多个 subagent。")
        );
        assert!(
            tool_messages[1]
                .content_items()
                .iter()
                .any(|item| matches!(
                    item,
                    crate::transform::unified::UnifiedContent::Text { text }
                    if text.contains("<skill>")
                        && text.contains("<name>superpowers:dispatching-parallel-agents</name>")
                        && text.contains("并行分派时必须一次性启动多个 subagent")
                ))
        );

        let leaked_skill_scaffold = unified.messages.iter().any(|message| {
            message.role == crate::transform::unified::UnifiedMessageRole::User
                && message
                    .content_text()
                    .map(|text| text.starts_with("Base directory for this skill:"))
                    .unwrap_or(false)
        });
        assert!(
            !leaked_skill_scaffold,
            "raw skill scaffold text should be stripped from user history"
        );
    }

    #[test]
    fn build_codex_unified_request_strips_historical_agent_worktree_isolation() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "claude-sonnet-4-6",
            "stream": true,
            "messages": [
                {
                    "role": "user",
                    "content": "用 3 个 subagent 查天气"
                },
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_agent_1",
                        "name": "Agent",
                        "input": {
                            "description": "查北京未来天气",
                            "isolation": "worktree",
                            "subagent_type": "general-purpose"
                        }
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_agent_1",
                        "content": "Cannot create agent worktree: not in a git repository"
                    }]
                }
            ]
        }))
        .expect("request");

        let (unified, _) = build_codex_unified_request(&request);

        let agent_calls: Vec<_> = unified
            .messages
            .iter()
            .flat_map(|message| message.tool_calls.iter())
            .filter(|call| call.function.name == "Agent")
            .collect();

        assert_eq!(agent_calls.len(), 1);
        assert!(
            !agent_calls[0].function.arguments.contains("\"isolation\":\"worktree\""),
            "historical agent tool calls should not preserve worktree isolation"
        );
    }
}

impl TransformBackend for CodexBackend {
    fn transform_request(
        &self,
        anthropic_body: &AnthropicRequest,
        _log_tx: Option<&broadcast::Sender<String>>,
        ctx: &TransformContext,
        effective_stream: bool,
        model_override: Option<String>,
    ) -> (Value, String) {
        let (unified, hints) = build_codex_unified_request(anthropic_body);
        let prepared = CodexAdapter.prepare_messages_request_with_hints(
            &unified,
            ctx,
            "",
            "",
            "2023-06-01",
            model_override.as_deref().unwrap_or(&ctx.codex_model),
            effective_stream,
            &hints,
        );
        (prepared.body, prepared.session_id)
    }

    fn build_upstream_request(
        &self,
        client: &reqwest::Client,
        target_url: &str,
        api_key: &str,
        body: &Value,
        session_id: &str,
        anthropic_version: &str,
    ) -> reqwest::RequestBuilder {
        CodexUpstreamRequestBuilder::build_request(
            client,
            target_url,
            api_key,
            body,
            session_id,
            anthropic_version,
        )
    }

    fn create_response_transformer(
        &self,
        model: &str,
        allow_visible_thinking: bool,
    ) -> Box<dyn ResponseTransformer> {
        Box::new(TransformResponse::new_with_visible_thinking(
            model,
            allow_visible_thinking,
        ))
    }
}
