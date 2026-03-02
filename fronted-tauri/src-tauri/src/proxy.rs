use codex_proxy_core::load_balancer::{
    EndpointPolicy as CoreEndpointPolicy, LoadBalancerConfig as CoreLoadBalancerConfig,
    LoadBalancerEndpoint as CoreLoadBalancerEndpoint,
    LoadBalancerProfile as CoreLoadBalancerProfile, LoadBalancerRuntime,
    SlotEndpointRef as CoreSlotEndpointRef, SlotMapping as CoreSlotMapping,
};
use codex_proxy_core::{
    AnthropicModelMapping, CodexModelMapping, GeminiReasoningEffortMapping, ProxyRuntimeHandle,
    ProxyServer, ReasoningEffort, ReasoningEffortMapping, RuntimeConfigUpdate, TransformContext,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::net::TcpListener;
use std::process::Command;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::broadcast;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReasoningEffortConfig {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodexModelMappingConfig {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AnthropicModelMappingConfig {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
}

impl Default for CodexModelMappingConfig {
    fn default() -> Self {
        Self {
            opus: "gpt-5.3-codex".to_string(),
            sonnet: "gpt-5.2-codex".to_string(),
            haiku: "gpt-5.1-codex-mini".to_string(),
        }
    }
}

impl Default for ReasoningEffortConfig {
    fn default() -> Self {
        Self {
            opus: "xhigh".to_string(),
            sonnet: "medium".to_string(),
            haiku: "low".to_string(),
        }
    }
}

impl ReasoningEffortConfig {
    pub fn to_mapping(&self) -> ReasoningEffortMapping {
        ReasoningEffortMapping::new()
            .with_opus(ReasoningEffort::from_str(&self.opus))
            .with_sonnet(ReasoningEffort::from_str(&self.sonnet))
            .with_haiku(ReasoningEffort::from_str(&self.haiku))
    }

    pub fn to_gemini_mapping(&self) -> GeminiReasoningEffortMapping {
        GeminiReasoningEffortMapping {
            opus: self.opus.clone(),
            sonnet: self.sonnet.clone(),
            haiku: self.haiku.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LbSlotEndpointRef {
    pub endpoint_id: String,
    pub custom_model_name: Option<String>,
    pub custom_reasoning_effort: Option<String>,
    pub converter_override: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelSlotMapping {
    pub opus: Vec<LbSlotEndpointRef>,
    pub sonnet: Vec<LbSlotEndpointRef>,
    pub haiku: Vec<LbSlotEndpointRef>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LbFailoverStrategy {
    pub error_threshold: u32,
    pub error_window_seconds: u32,
    pub cooldown_seconds: u32,
    pub degraded_concurrency: u32,
}

impl Default for LbFailoverStrategy {
    fn default() -> Self {
        Self {
            error_threshold: 5,
            error_window_seconds: 60,
            cooldown_seconds: 3600,
            degraded_concurrency: 4,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancerProfile {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub model_mapping: ModelSlotMapping,
    pub strategy: LbFailoverStrategy,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LbEndpointConfig {
    pub endpoint_id: String,
    pub enabled: bool,
    pub max_concurrency: u32,
    pub priority: u32,
    pub weight: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancerConfig {
    pub lb_profiles: Vec<LoadBalancerProfile>,
    pub selected_lb_profile_id: Option<String>,
    pub lb_endpoint_configs: std::collections::HashMap<String, LbEndpointConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EndpointOption {
    pub id: String,
    pub alias: String,
    pub url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,

    #[serde(default)]
    pub converter: Option<String>,

    #[serde(rename = "codexModel", default)]
    pub codex_model: Option<String>,

    #[serde(rename = "codexModelMapping", default)]
    pub codex_model_mapping: Option<CodexModelMappingConfig>,

    #[serde(rename = "codexEffortCapabilityMap", default)]
    pub codex_effort_capability_map: Option<std::collections::HashMap<String, Vec<String>>>,

    #[serde(rename = "geminiModelPreset", default)]
    pub gemini_model_preset: Option<Vec<String>>,

    #[serde(rename = "anthropicModelMapping", default)]
    pub anthropic_model_mapping: Option<AnthropicModelMappingConfig>,

    #[serde(rename = "reasoningEffort", default)]
    pub reasoning_effort: Option<ReasoningEffortConfig>,

    #[serde(rename = "geminiReasoningEffort", default)]
    pub gemini_reasoning_effort: Option<ReasoningEffortConfig>,
}

fn default_endpoint_options() -> Vec<EndpointOption> {
    vec![EndpointOption {
        id: "aicodemirror-default".to_string(),
        alias: "aicodemirror".to_string(),
        url: "https://api.aicodemirror.com/api/codex/backend-api/codex/responses".to_string(),
        api_key: String::new(),
        converter: None,
        codex_model: None,
        codex_model_mapping: None,
        codex_effort_capability_map: None,
        gemini_model_preset: None,
        anthropic_model_mapping: None,
        reasoning_effort: None,
        gemini_reasoning_effort: None,
    }]
}

fn default_selected_endpoint_id() -> String {
    "aicodemirror-default".to_string()
}

fn default_proxy_mode() -> String {
    "single".to_string()
}

fn default_load_balancer() -> LoadBalancerConfig {
    LoadBalancerConfig::default()
}

fn default_lb_model_cooldown_seconds() -> u32 {
    3600
}

fn default_lb_transient_backoff_seconds() -> u32 {
    6
}

fn default_anthropic_model_mapping() -> AnthropicModelMappingConfig {
    AnthropicModelMappingConfig::default()
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProxyConfig {
    pub port: u16,
    #[serde(rename = "targetUrl")]
    pub target_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    #[serde(rename = "endpointOptions", default = "default_endpoint_options")]
    pub endpoint_options: Vec<EndpointOption>,
    #[serde(
        rename = "selectedEndpointId",
        default = "default_selected_endpoint_id"
    )]
    pub selected_endpoint_id: String,
    #[serde(default = "default_converter")]
    pub converter: String,
    #[serde(rename = "codexModel", default = "default_codex_model")]
    pub codex_model: String,
    #[serde(rename = "codexModelMapping", default)]
    pub codex_model_mapping: CodexModelMappingConfig,
    #[serde(
        rename = "anthropicModelMapping",
        default = "default_anthropic_model_mapping"
    )]
    pub anthropic_model_mapping: AnthropicModelMappingConfig,

    #[serde(rename = "codexEffortCapabilityMap", default)]
    pub codex_effort_capability_map: Option<std::collections::HashMap<String, Vec<String>>>,

    #[serde(rename = "geminiModelPreset", default = "default_gemini_model_preset")]
    pub gemini_model_preset: Vec<String>,

    #[serde(rename = "maxConcurrency", default)]
    pub max_concurrency: u32,
    #[serde(rename = "ignoreProbeRequests", default)]
    pub ignore_probe_requests: bool,
    #[serde(
        rename = "allowCountTokensFallbackEstimate",
        default = "default_allow_count_tokens_fallback_estimate"
    )]
    pub allow_count_tokens_fallback_estimate: bool,
    #[serde(
        rename = "forceStreamForCodex",
        default = "default_force_stream_for_codex"
    )]
    pub force_stream_for_codex: bool,
    #[serde(
        rename = "enableSseFrameParser",
        default = "default_enable_sse_frame_parser"
    )]
    pub enable_sse_frame_parser: bool,
    #[serde(
        rename = "enableStreamHeartbeat",
        default = "default_enable_stream_heartbeat"
    )]
    pub enable_stream_heartbeat: bool,
    #[serde(
        rename = "streamHeartbeatIntervalMs",
        default = "default_stream_heartbeat_interval_ms"
    )]
    pub stream_heartbeat_interval_ms: u64,
    #[serde(
        rename = "enableStreamLogSampling",
        default = "default_enable_stream_log_sampling"
    )]
    pub enable_stream_log_sampling: bool,
    #[serde(
        rename = "streamLogSampleEveryN",
        default = "default_stream_log_sample_every_n"
    )]
    pub stream_log_sample_every_n: u32,
    #[serde(rename = "streamLogMaxChars", default = "default_stream_log_max_chars")]
    pub stream_log_max_chars: usize,
    #[serde(
        rename = "enableStreamMetrics",
        default = "default_enable_stream_metrics"
    )]
    pub enable_stream_metrics: bool,
    #[serde(
        rename = "enableStreamEventMetrics",
        default = "default_enable_stream_event_metrics"
    )]
    pub enable_stream_event_metrics: bool,
    #[serde(
        rename = "streamSilenceWarnMs",
        default = "default_stream_silence_warn_ms"
    )]
    pub stream_silence_warn_ms: u64,
    #[serde(
        rename = "streamSilenceErrorMs",
        default = "default_stream_silence_error_ms"
    )]
    pub stream_silence_error_ms: u64,
    #[serde(rename = "enableStallRetry", default = "default_enable_stall_retry")]
    pub enable_stall_retry: bool,
    #[serde(rename = "stallTimeoutMs", default = "default_stall_timeout_ms")]
    pub stall_timeout_ms: u64,
    #[serde(
        rename = "stallRetryMaxAttempts",
        default = "default_stall_retry_max_attempts"
    )]
    pub stall_retry_max_attempts: u32,
    #[serde(
        rename = "stallRetryOnlyHeartbeatPhase",
        default = "default_stall_retry_only_heartbeat_phase"
    )]
    pub stall_retry_only_heartbeat_phase: bool,
    #[serde(
        rename = "enableEmptyCompletionRetry",
        default = "default_enable_empty_completion_retry"
    )]
    pub enable_empty_completion_retry: bool,
    #[serde(
        rename = "emptyCompletionRetryMaxAttempts",
        default = "default_empty_completion_retry_max_attempts"
    )]
    pub empty_completion_retry_max_attempts: u32,
    #[serde(
        rename = "enableIncompleteStreamRetry",
        default = "default_enable_incomplete_stream_retry"
    )]
    pub enable_incomplete_stream_retry: bool,
    #[serde(
        rename = "incompleteStreamRetryMaxAttempts",
        default = "default_incomplete_stream_retry_max_attempts"
    )]
    pub incomplete_stream_retry_max_attempts: u32,
    #[serde(
        rename = "enableSiblingToolErrorRetry",
        default = "default_enable_sibling_tool_error_retry"
    )]
    pub enable_sibling_tool_error_retry: bool,
    #[serde(
        rename = "preferCodexV1Path",
        default = "default_prefer_codex_v1_path"
    )]
    pub prefer_codex_v1_path: bool,
    #[serde(
        rename = "enableCodexToolSchemaCompaction",
        default = "default_enable_codex_tool_schema_compaction"
    )]
    pub enable_codex_tool_schema_compaction: bool,
    #[serde(
        rename = "enableStatefulResponsesChain",
        default = "default_enable_stateful_responses_chain"
    )]
    pub enable_stateful_responses_chain: bool,
    #[serde(rename = "allowExternalAccess", default)]
    pub allow_external_access: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(rename = "proxyMode", default = "default_proxy_mode")]
    pub proxy_mode: String,
    #[serde(rename = "loadBalancer", default = "default_load_balancer")]
    pub load_balancer: LoadBalancerConfig,
    #[serde(
        rename = "lbModelCooldownSeconds",
        default = "default_lb_model_cooldown_seconds"
    )]
    pub lb_model_cooldown_seconds: u32,
    #[serde(
        rename = "lbTransientBackoffSeconds",
        default = "default_lb_transient_backoff_seconds"
    )]
    pub lb_transient_backoff_seconds: u32,
    #[serde(rename = "reasoningEffort", default)]
    pub reasoning_effort: ReasoningEffortConfig,
    #[serde(rename = "geminiReasoningEffort", default)]
    pub gemini_reasoning_effort: ReasoningEffortConfig,
    #[serde(
        rename = "customInjectionPrompt",
        alias = "skillInjectionPrompt",
        default = "default_custom_injection_prompt"
    )]
    pub custom_injection_prompt: String,
    #[serde(default = "default_lang")]
    pub lang: String,
}

fn default_lang() -> String {
    "zh".to_string()
}

const DEFAULT_CUSTOM_INJECTION_PROMPT: &str = "回复前先看下skills是否有一个skill符合的，可以执行。\n\nskills里的技能如果需要依赖，先安装，不要先用其他方案，如果还有问题告知用户解决方案让用户选择。\n\n如果你收到 <tool_use_error>Sibling tool call errored</tool_use_error>，请不要将其作为普通文本输出，而是分析导致该错误的原始Sibling的报错原因并继续工作。";

fn default_custom_injection_prompt() -> String {
    DEFAULT_CUSTOM_INJECTION_PROMPT.to_string()
}

fn resolve_custom_injection_prompt(raw: &str) -> String {
    if raw.trim().is_empty() {
        default_custom_injection_prompt()
    } else {
        raw.to_string()
    }
}

fn default_converter() -> String {
    "codex".to_string()
}

fn default_codex_model() -> String {
    "gpt-5.3-codex".to_string()
}

fn default_allow_count_tokens_fallback_estimate() -> bool {
    true
}

fn default_force_stream_for_codex() -> bool {
    true
}

fn default_enable_sse_frame_parser() -> bool {
    true
}

fn default_enable_stream_heartbeat() -> bool {
    true
}

fn default_stream_heartbeat_interval_ms() -> u64 {
    8_000
}

fn default_enable_stream_log_sampling() -> bool {
    true
}

fn default_stream_log_sample_every_n() -> u32 {
    20
}

fn default_stream_log_max_chars() -> usize {
    512
}

fn default_enable_stream_metrics() -> bool {
    true
}

fn default_enable_stream_event_metrics() -> bool {
    true
}

fn default_stream_silence_warn_ms() -> u64 {
    20_000
}

fn default_stream_silence_error_ms() -> u64 {
    90_000
}

fn default_enable_stall_retry() -> bool {
    false
}

fn default_stall_timeout_ms() -> u64 {
    300_000
}

fn default_stall_retry_max_attempts() -> u32 {
    0
}

fn default_stall_retry_only_heartbeat_phase() -> bool {
    false
}

fn default_enable_empty_completion_retry() -> bool {
    false
}

fn default_empty_completion_retry_max_attempts() -> u32 {
    0
}

fn default_enable_incomplete_stream_retry() -> bool {
    true
}

fn default_incomplete_stream_retry_max_attempts() -> u32 {
    2
}

fn default_enable_sibling_tool_error_retry() -> bool {
    true
}

fn default_prefer_codex_v1_path() -> bool {
    true
}

fn default_enable_codex_tool_schema_compaction() -> bool {
    true
}

fn default_enable_stateful_responses_chain() -> bool {
    true
}

fn default_gemini_model_preset() -> Vec<String> {
    vec![
        "gemini-2.5-flash-lite".to_string(),
        "gemini-3-pro-preview".to_string(),
        "gemini-3-pro-image-preview".to_string(),
        "gemini-3-flash-preview".to_string(),
        "gemini-2.5-flash".to_string(),
        "gemini-2.5-pro".to_string(),
    ]
}

fn build_lb_runtime(
    config: &ProxyConfig,
    log_tx: Option<broadcast::Sender<String>>,
) -> Option<LoadBalancerRuntime> {
    let selected_profile_id = config.load_balancer.selected_lb_profile_id.clone();
    let selected_profile_strategy = selected_profile_id
        .as_ref()
        .and_then(|profile_id| {
            config
                .load_balancer
                .lb_profiles
                .iter()
                .find(|p| &p.id == profile_id)
        })
        .map(|p| p.strategy.clone())
        .unwrap_or_default();

    let error_threshold = selected_profile_strategy.error_threshold.max(1);
    let error_window_seconds = selected_profile_strategy.error_window_seconds.max(1);
    let degraded_concurrency = selected_profile_strategy.degraded_concurrency.max(1);
    let cooldown_seconds = if config.lb_model_cooldown_seconds == 0 {
        default_lb_model_cooldown_seconds()
    } else {
        config.lb_model_cooldown_seconds
    };
    let transient_backoff_seconds = if config.lb_transient_backoff_seconds == 0 {
        default_lb_transient_backoff_seconds()
    } else {
        config.lb_transient_backoff_seconds
    };

    let profiles: Vec<CoreLoadBalancerProfile> = config
        .load_balancer
        .lb_profiles
        .iter()
        .map(|profile| CoreLoadBalancerProfile {
            id: profile.id.clone(),
            name: profile.name.clone(),
            model_mapping: CoreSlotMapping {
                opus: profile
                    .model_mapping
                    .opus
                    .iter()
                    .map(|item| CoreSlotEndpointRef {
                        endpoint_id: item.endpoint_id.clone(),
                        custom_model_name: item.custom_model_name.clone(),
                        custom_reasoning_effort: item.custom_reasoning_effort.clone(),
                        converter_override: item.converter_override.clone(),
                    })
                    .collect(),
                sonnet: profile
                    .model_mapping
                    .sonnet
                    .iter()
                    .map(|item| CoreSlotEndpointRef {
                        endpoint_id: item.endpoint_id.clone(),
                        custom_model_name: item.custom_model_name.clone(),
                        custom_reasoning_effort: item.custom_reasoning_effort.clone(),
                        converter_override: item.converter_override.clone(),
                    })
                    .collect(),
                haiku: profile
                    .model_mapping
                    .haiku
                    .iter()
                    .map(|item| CoreSlotEndpointRef {
                        endpoint_id: item.endpoint_id.clone(),
                        custom_model_name: item.custom_model_name.clone(),
                        custom_reasoning_effort: item.custom_reasoning_effort.clone(),
                        converter_override: item.converter_override.clone(),
                    })
                    .collect(),
            },
        })
        .collect();

    if profiles.is_empty() || selected_profile_id.is_none() {
        return None;
    }

    let endpoint_directory: HashMap<String, CoreLoadBalancerEndpoint> = config
        .endpoint_options
        .iter()
        .map(|item| {
            let converter = item
                .converter
                .clone()
                .unwrap_or_else(|| config.converter.clone());
            let api_key = if item.api_key.is_empty() {
                if config.api_key.is_empty() {
                    None
                } else {
                    Some(config.api_key.clone())
                }
            } else {
                Some(item.api_key.clone())
            };

            (
                item.id.clone(),
                CoreLoadBalancerEndpoint {
                    id: item.id.clone(),
                    target_url: item.url.clone(),
                    api_key,
                    converter,
                },
            )
        })
        .collect();

    let endpoint_policies: HashMap<String, CoreEndpointPolicy> = config
        .endpoint_options
        .iter()
        .map(|endpoint| {
            let endpoint_cfg = config.load_balancer.lb_endpoint_configs.get(&endpoint.id);
            let enabled = endpoint_cfg.map(|cfg| cfg.enabled).unwrap_or(true);
            let max_concurrency = endpoint_cfg.map(|cfg| cfg.max_concurrency).unwrap_or(16);

            (
                endpoint.id.clone(),
                CoreEndpointPolicy {
                    enabled,
                    max_concurrency: if max_concurrency == 0 {
                        1
                    } else {
                        max_concurrency
                    },
                    error_threshold,
                    error_window_seconds,
                    cooldown_seconds,
                    degraded_concurrency,
                    transient_backoff_seconds,
                },
            )
        })
        .collect();

    Some(LoadBalancerRuntime::new(
        CoreLoadBalancerConfig {
            selected_profile_id,
            profiles,
            endpoint_policies,
        },
        endpoint_directory,
        log_tx,
    ))
}

fn selected_endpoint<'a>(config: &'a ProxyConfig) -> Option<&'a EndpointOption> {
    config
        .endpoint_options
        .iter()
        .find(|item| item.id == config.selected_endpoint_id)
}

fn resolve_target_and_api_key(config: &ProxyConfig) -> (String, Option<String>) {
    let selected = selected_endpoint(config);
    let target_url = selected
        .map(|item| item.url.clone())
        .unwrap_or_else(|| config.target_url.clone());
    let resolved_api_key = selected
        .map(|item| item.api_key.clone())
        .unwrap_or_else(|| config.api_key.clone());
    let api_key = if resolved_api_key.is_empty() {
        None
    } else {
        Some(resolved_api_key)
    };
    (target_url, api_key)
}

fn build_runtime_update(
    config: &ProxyConfig,
    log_tx: Option<broadcast::Sender<String>>,
) -> RuntimeConfigUpdate {
    let (target_url, api_key) = resolve_target_and_api_key(config);
    let custom_injection_prompt = resolve_custom_injection_prompt(&config.custom_injection_prompt);
    let load_balancer_runtime = if config.proxy_mode.eq_ignore_ascii_case("load_balancer") {
        build_lb_runtime(config, log_tx)
    } else {
        None
    };

    RuntimeConfigUpdate {
        target_url,
        api_key,
        ctx: TransformContext {
            reasoning_mapping: config.reasoning_effort.to_mapping(),
            codex_model_mapping: CodexModelMapping {
                opus: config.codex_model_mapping.opus.clone(),
                sonnet: config.codex_model_mapping.sonnet.clone(),
                haiku: config.codex_model_mapping.haiku.clone(),
            },
            anthropic_model_mapping: AnthropicModelMapping {
                opus: config.anthropic_model_mapping.opus.clone(),
                sonnet: config.anthropic_model_mapping.sonnet.clone(),
                haiku: config.anthropic_model_mapping.haiku.clone(),
            },
            custom_injection_prompt: custom_injection_prompt,
            converter: config.converter.clone(),
            codex_model: config.codex_model.clone(),
            gemini_reasoning_effort: config.gemini_reasoning_effort.to_gemini_mapping(),
            enable_codex_tool_schema_compaction: config.enable_codex_tool_schema_compaction,
        },
        ignore_probe_requests: config.ignore_probe_requests,
        allow_count_tokens_fallback_estimate: config.allow_count_tokens_fallback_estimate,
        force_stream_for_codex: config.force_stream_for_codex,
        enable_sse_frame_parser: config.enable_sse_frame_parser,
        enable_stream_heartbeat: config.enable_stream_heartbeat,
        stream_heartbeat_interval_ms: config.stream_heartbeat_interval_ms,
        enable_stream_log_sampling: config.enable_stream_log_sampling,
        stream_log_sample_every_n: config.stream_log_sample_every_n,
        stream_log_max_chars: config.stream_log_max_chars,
        enable_stream_metrics: config.enable_stream_metrics,
        enable_stream_event_metrics: config.enable_stream_event_metrics,
        stream_silence_warn_ms: config.stream_silence_warn_ms,
        stream_silence_error_ms: config.stream_silence_error_ms,
        enable_stall_retry: config.enable_stall_retry,
        stall_timeout_ms: config.stall_timeout_ms,
        stall_retry_max_attempts: config.stall_retry_max_attempts,
        stall_retry_only_heartbeat_phase: config.stall_retry_only_heartbeat_phase,
        enable_empty_completion_retry: config.enable_empty_completion_retry,
        empty_completion_retry_max_attempts: config.empty_completion_retry_max_attempts,
        enable_incomplete_stream_retry: config.enable_incomplete_stream_retry,
        incomplete_stream_retry_max_attempts: config.incomplete_stream_retry_max_attempts,
        enable_sibling_tool_error_retry: config.enable_sibling_tool_error_retry,
        prefer_codex_v1_path: config.prefer_codex_v1_path,
        enable_codex_tool_schema_compaction: config.enable_codex_tool_schema_compaction,
        enable_stateful_responses_chain: config.enable_stateful_responses_chain,
        load_balancer_runtime,
    }
}

pub struct ProxyManager {
    running: bool,
    shutdown_tx: Option<broadcast::Sender<()>>,
    log_tx: Option<broadcast::Sender<String>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    runtime_handle: Option<ProxyRuntimeHandle>,
    current_port: Option<u16>,
}

impl Default for ProxyManager {
    fn default() -> Self {
        Self {
            running: false,
            shutdown_tx: None,
            log_tx: None,
            server_handle: None,
            runtime_handle: None,
            current_port: None,
        }
    }
}

impl ProxyManager {
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    pub fn set_shutdown_tx(&mut self, tx: broadcast::Sender<()>) {
        self.shutdown_tx = Some(tx);
    }

    pub fn set_runtime_handle(&mut self, handle: ProxyRuntimeHandle) {
        self.runtime_handle = Some(handle);
    }

    pub fn runtime_handle(&self) -> Option<ProxyRuntimeHandle> {
        self.runtime_handle.clone()
    }

    pub fn current_port(&self) -> Option<u16> {
        self.current_port
    }

    pub fn set_current_port(&mut self, port: u16) {
        self.current_port = Some(port);
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }
        self.running = false;
        self.log_tx = None;
        self.runtime_handle = None;
        self.current_port = None;
    }
}

fn get_config_path() -> Result<std::path::PathBuf, String> {
    dirs::config_dir()
        .map(|p| p.join("com.codex.proxy").join("proxy-config.json"))
        .ok_or_else(|| "Cannot find config directory".to_string())
}

#[tauri::command]
pub fn load_config() -> Result<Option<ProxyConfig>, String> {
    let path = get_config_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let config: ProxyConfig = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(Some(config))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub fn save_config(config: ProxyConfig) -> Result<(), String> {
    let path = get_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_lang(lang: String) -> Result<(), String> {
    let path = get_config_path()?;
    let mut config = if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        ProxyConfig::default()
    };
    config.lang = lang;
    save_config(config)
}

#[tauri::command]
pub fn check_port(port: u16) -> bool {
    check_port_for_bind(port, false)
}

fn check_port_for_bind(port: u16, allow_external_access: bool) -> bool {
    let bind_host = if allow_external_access {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    TcpListener::bind((bind_host, port)).is_err()
}

#[tauri::command]
pub async fn kill_port(port: u16) -> Result<(), String> {
    let command = if cfg!(target_os = "windows") {
        format!(
            "for /f \"tokens=5\" %a in ('netstat -aon ^| findstr :{port}') do taskkill /f /pid %a"
        )
    } else {
        format!("lsof -i :{port} -t | xargs kill -9 2>/dev/null || true")
    };

    if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/c", &command])
            .output()
            .map_err(|e| e.to_string())?;
    } else {
        Command::new("sh")
            .args(["-c", &command])
            .output()
            .map_err(|e| e.to_string())?;
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    Ok(())
}

#[tauri::command]
pub async fn start_proxy(app: AppHandle, config: ProxyConfig) -> Result<(), String> {
    // Save config
    save_config(config.clone())?;

    // Check port
    if !config.force && check_port_for_bind(config.port, config.allow_external_access) {
        app.emit("port-in-use", config.port)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    if config.force {
        app.emit(
            "proxy-log",
            format!("[System] Stopping process on port {}...", config.port),
        )
        .map_err(|e| e.to_string())?;
        kill_port(config.port).await?;
    }

    // Get state
    let state = app.state::<crate::AppState>();
    let mut manager = state.proxy_manager.lock().await;

    if manager.is_running() {
        return Err("Proxy is already running".to_string());
    }

    start_proxy_with_manager(&app, config, &mut manager).await
}

async fn start_proxy_with_manager(
    app: &AppHandle,
    config: ProxyConfig,
    manager: &mut ProxyManager,
) -> Result<(), String> {
    app.emit(
        "proxy-log",
        format!("[System] Starting proxy on port {}...", config.port),
    )
    .map_err(|e| e.to_string())?;
    app.emit(
        "proxy-log",
        format!(
            "[System] Bind mode: {}",
            if config.allow_external_access {
                "external (0.0.0.0)"
            } else {
                "local only (127.0.0.1)"
            }
        ),
    )
    .map_err(|e| e.to_string())?;
    let (resolved_target_url, api_key) = resolve_target_and_api_key(&config);

    app.emit(
        "proxy-log",
        format!("[System] Target: {}", resolved_target_url),
    )
    .map_err(|e| e.to_string())?;
    let custom_injection_prompt = resolve_custom_injection_prompt(&config.custom_injection_prompt);

    // 创建日志通道（容量 2048 减少高频场景下的 lag）
    let (log_tx, mut log_rx) = broadcast::channel::<String>(2048);
    manager.log_tx = Some(log_tx.clone());

    let server = ProxyServer::new(config.port, resolved_target_url.clone(), api_key)
        .with_reasoning_mapping(config.reasoning_effort.to_mapping())
        .with_custom_injection_prompt(custom_injection_prompt)
        .with_converter(config.converter.clone())
        .with_codex_model(config.codex_model.clone())
        .with_codex_model_mapping(CodexModelMapping {
            opus: config.codex_model_mapping.opus.clone(),
            sonnet: config.codex_model_mapping.sonnet.clone(),
            haiku: config.codex_model_mapping.haiku.clone(),
        })
        .with_anthropic_model_mapping(AnthropicModelMapping {
            opus: config.anthropic_model_mapping.opus.clone(),
            sonnet: config.anthropic_model_mapping.sonnet.clone(),
            haiku: config.anthropic_model_mapping.haiku.clone(),
        })
        .with_gemini_reasoning_effort(config.gemini_reasoning_effort.to_gemini_mapping())
        .with_ignore_probe_requests(config.ignore_probe_requests)
        .with_allow_count_tokens_fallback_estimate(config.allow_count_tokens_fallback_estimate)
        .with_force_stream_for_codex(config.force_stream_for_codex)
        .with_enable_sse_frame_parser(config.enable_sse_frame_parser)
        .with_enable_stream_heartbeat(config.enable_stream_heartbeat)
        .with_stream_heartbeat_interval_ms(config.stream_heartbeat_interval_ms)
        .with_enable_stream_log_sampling(config.enable_stream_log_sampling)
        .with_stream_log_sample_every_n(config.stream_log_sample_every_n)
        .with_stream_log_max_chars(config.stream_log_max_chars)
        .with_enable_stream_metrics(config.enable_stream_metrics)
        .with_enable_stream_event_metrics(config.enable_stream_event_metrics)
        .with_stream_silence_warn_ms(config.stream_silence_warn_ms)
        .with_stream_silence_error_ms(config.stream_silence_error_ms)
        .with_enable_stall_retry(config.enable_stall_retry)
        .with_stall_timeout_ms(config.stall_timeout_ms)
        .with_stall_retry_max_attempts(config.stall_retry_max_attempts)
        .with_stall_retry_only_heartbeat_phase(config.stall_retry_only_heartbeat_phase)
        .with_enable_empty_completion_retry(config.enable_empty_completion_retry)
        .with_empty_completion_retry_max_attempts(config.empty_completion_retry_max_attempts)
        .with_enable_incomplete_stream_retry(config.enable_incomplete_stream_retry)
        .with_incomplete_stream_retry_max_attempts(config.incomplete_stream_retry_max_attempts)
        .with_enable_sibling_tool_error_retry(config.enable_sibling_tool_error_retry)
        .with_prefer_codex_v1_path(config.prefer_codex_v1_path)
        .with_enable_codex_tool_schema_compaction(config.enable_codex_tool_schema_compaction)
        .with_enable_stateful_responses_chain(config.enable_stateful_responses_chain)
        .with_allow_external_access(config.allow_external_access)
        .with_max_concurrency(config.max_concurrency);

    let server = if config.proxy_mode.eq_ignore_ascii_case("load_balancer") {
        if let Some(runtime) = build_lb_runtime(&config, Some(log_tx.clone())) {
            app.emit("proxy-log", "[System] Load balancer mode enabled")
                .map_err(|e| e.to_string())?;
            server.with_load_balancer_runtime(runtime)
        } else {
            app.emit(
                "proxy-log",
                "[Warning] Load balancer config incomplete, fallback to single mode",
            )
            .map_err(|e| e.to_string())?;
            server
        }
    } else {
        server
    };

    // 启动日志转发（Lagged 时跳过丢失的消息继续接收，不退出）
    let app_clone = app.clone();
    tokio::spawn(async move {
        loop {
            match log_rx.recv().await {
                Ok(msg) => {
                    let _ = app_clone.emit("proxy-log", msg);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let _ = app_clone.emit(
                        "proxy-log",
                        format!("[Warning] Log receiver lagged, skipped {} messages", n),
                    );
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 启动代理服务器
    let app_clone = app.clone();
    match server.start(log_tx).await {
        Ok((shutdown_tx, server_handle, runtime_handle)) => {
            manager.set_shutdown_tx(shutdown_tx);
            manager.server_handle = Some(server_handle);
            manager.set_runtime_handle(runtime_handle);
            manager.set_current_port(config.port);
            manager.set_running(true);
            app.emit("proxy-status", "running")
                .map_err(|e| e.to_string())?;
        }
        Err(e) => {
            manager.set_running(false);
            let _ = app_clone.emit("proxy-log", format!("[Error] Server error: {}", e));
            let _ = app_clone.emit("proxy-status", "stopped");
            return Err(e.to_string());
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn apply_proxy_config(app: AppHandle, config: ProxyConfig) -> Result<(), String> {
    save_config(config.clone())?;

    let state = app.state::<crate::AppState>();
    let manager = state.proxy_manager.lock().await;

    if !manager.is_running() {
        app.emit("proxy-log", "[System] Proxy not running, config saved only")
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    if manager.current_port() != Some(config.port) {
        app.emit(
            "proxy-log",
            format!(
                "[Warning] Port change {} -> {} requires restart, hot apply skipped",
                manager.current_port().unwrap_or(config.port),
                config.port
            ),
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let Some(runtime_handle) = manager.runtime_handle() else {
        return Err("Proxy runtime handle missing".to_string());
    };
    let update = build_runtime_update(&config, manager.log_tx.clone());
    if config.proxy_mode.eq_ignore_ascii_case("load_balancer")
        && update.load_balancer_runtime.is_none()
    {
        app.emit(
            "proxy-log",
            "[Warning] Load balancer config incomplete, hot fallback to single mode",
        )
        .map_err(|e| e.to_string())?;
    }
    runtime_handle.apply_update(update);

    app.emit(
        "proxy-log",
        "[System] Runtime config hot-updated (no restart)",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn restart_proxy(app: AppHandle, config: ProxyConfig) -> Result<(), String> {
    save_config(config.clone())?;

    let state = app.state::<crate::AppState>();
    let mut manager = state.proxy_manager.lock().await;

    if manager.is_running() {
        app.emit(
            "proxy-log",
            "[System] Applying config changes and restarting proxy...",
        )
        .map_err(|e| e.to_string())?;
        manager.stop();
    }

    if !config.force && check_port_for_bind(config.port, config.allow_external_access) {
        app.emit("port-in-use", config.port)
            .map_err(|e| e.to_string())?;
        app.emit("proxy-status", "stopped")
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    if config.force {
        app.emit(
            "proxy-log",
            format!("[System] Stopping process on port {}...", config.port),
        )
        .map_err(|e| e.to_string())?;
        kill_port(config.port).await?;
    }

    start_proxy_with_manager(&app, config, &mut manager).await
}

#[tauri::command]
pub async fn stop_proxy(app: AppHandle) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let mut manager = state.proxy_manager.lock().await;

    manager.stop();

    app.emit("proxy-status", "stopped")
        .map_err(|e| e.to_string())?;
    app.emit("proxy-log", "[System] Proxy stopped")
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn export_config() -> Result<String, String> {
    let path = get_config_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| format!("读取配置文件失败: {}", e))?;
        let config: ProxyConfig =
            serde_json::from_str(&content).map_err(|e| format!("解析配置文件失败: {}", e))?;
        serde_json::to_string_pretty(&config).map_err(|e| format!("序列化配置失败: {}", e))
    } else {
        let default_config = ProxyConfig::default();
        serde_json::to_string_pretty(&default_config)
            .map_err(|e| format!("序列化默认配置失败: {}", e))
    }
}

#[tauri::command]
pub fn import_config(config_json: String) -> Result<(), String> {
    let config: ProxyConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("JSON 格式无效: {}", e))?;

    if config.endpoint_options.is_empty() {
        return Err("配置无效: endpoint_options 不能为空".to_string());
    }

    let path = get_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }

    let content =
        serde_json::to_string_pretty(&config).map_err(|e| format!("序列化配置失败: {}", e))?;
    fs::write(&path, content).map_err(|e| format!("写入配置文件失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn custom_injection_prompt_uses_default_when_field_missing() {
        let config: ProxyConfig = serde_json::from_value(json!({
            "port": 8889,
            "targetUrl": "http://127.0.0.1:3000",
            "apiKey": "test-key"
        }))
        .expect("config should deserialize");

        assert_eq!(
            config.custom_injection_prompt,
            DEFAULT_CUSTOM_INJECTION_PROMPT
        );
    }

    #[test]
    fn build_runtime_update_falls_back_for_blank_prompt_and_keeps_custom_prompt() {
        let mut blank = ProxyConfig::default();
        blank.target_url = "http://127.0.0.1:3000".to_string();
        blank.custom_injection_prompt = "   ".to_string();
        let blank_update = build_runtime_update(&blank, None);
        assert_eq!(
            blank_update.ctx.custom_injection_prompt,
            DEFAULT_CUSTOM_INJECTION_PROMPT
        );

        let mut custom = ProxyConfig::default();
        custom.target_url = "http://127.0.0.1:3000".to_string();
        custom.custom_injection_prompt = "user custom prompt".to_string();
        let custom_update = build_runtime_update(&custom, None);
        assert_eq!(
            custom_update.ctx.custom_injection_prompt,
            "user custom prompt"
        );
    }
}
