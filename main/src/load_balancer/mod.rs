use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMode {
    Single,
    LoadBalancer,
}

impl ProxyMode {
    pub fn from_config(value: &str) -> Self {
        if value.eq_ignore_ascii_case("load_balancer") {
            Self::LoadBalancer
        } else {
            Self::Single
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelSlot {
    Opus,
    Sonnet,
    Haiku,
}

impl ModelSlot {
    pub fn from_model_name(model: &str) -> Self {
        let lower = model.to_ascii_lowercase();
        if lower.contains("opus") {
            Self::Opus
        } else if lower.contains("haiku") {
            Self::Haiku
        } else {
            Self::Sonnet
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SlotEndpointRef {
    pub endpoint_id: String,
    pub custom_model_name: Option<String>,
    pub custom_reasoning_effort: Option<String>,
    pub converter_override: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SlotMapping {
    pub opus: Vec<SlotEndpointRef>,
    pub sonnet: Vec<SlotEndpointRef>,
    pub haiku: Vec<SlotEndpointRef>,
}

impl SlotMapping {
    pub fn get(&self, slot: ModelSlot) -> &[SlotEndpointRef] {
        match slot {
            ModelSlot::Opus => &self.opus,
            ModelSlot::Sonnet => &self.sonnet,
            ModelSlot::Haiku => &self.haiku,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadBalancerProfile {
    pub id: String,
    pub name: String,
    pub model_mapping: SlotMapping,
}

#[derive(Debug, Clone)]
pub struct EndpointPolicy {
    pub enabled: bool,
    pub max_concurrency: u32,
    pub error_threshold: u32,
    pub error_window_seconds: u32,
    pub cooldown_seconds: u32,
    pub degraded_concurrency: u32,
    pub transient_backoff_seconds: u32,
}

impl Default for EndpointPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrency: 16,
            error_threshold: 5,
            error_window_seconds: 60,
            cooldown_seconds: 3600,
            degraded_concurrency: 4,
            transient_backoff_seconds: 6,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadBalancerConfig {
    pub selected_profile_id: Option<String>,
    pub profiles: Vec<LoadBalancerProfile>,
    pub endpoint_policies: HashMap<String, EndpointPolicy>,
}

#[derive(Debug, Clone)]
pub struct LoadBalancerEndpoint {
    pub id: String,
    pub target_url: String,
    pub api_key: Option<String>,
    pub converter: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedEndpoint {
    pub endpoint_id: String,
    pub target_url: String,
    pub api_key: Option<String>,
    pub converter: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub slot: ModelSlot,
    pub route_key: String,
    pub model_hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointHealth {
    Healthy,
    Constrained,
    Cooldown,
}

#[derive(Debug, Clone)]
struct EndpointState {
    in_flight: u32,
    transient_backoff_until: Option<Instant>,
}

impl Default for EndpointState {
    fn default() -> Self {
        Self {
            in_flight: 0,
            transient_backoff_until: None,
        }
    }
}

#[derive(Debug, Clone)]
struct RouteState {
    errors: VecDeque<Instant>,
    cooldown_until: Option<Instant>,
    health: EndpointHealth,
}

impl Default for RouteState {
    fn default() -> Self {
        Self {
            errors: VecDeque::new(),
            cooldown_until: None,
            health: EndpointHealth::Healthy,
        }
    }
}

#[derive(Debug, Default)]
struct RuntimeState {
    by_endpoint: HashMap<String, EndpointState>,
    by_route: HashMap<String, RouteState>,
}

#[derive(Debug, Clone)]
pub struct LoadBalancerRuntime {
    config: LoadBalancerConfig,
    profile_index_by_id: HashMap<String, usize>,
    endpoint_directory: HashMap<String, LoadBalancerEndpoint>,
    state: Arc<Mutex<RuntimeState>>,
    log_tx: Option<broadcast::Sender<String>>,
}

#[derive(Debug)]
pub struct EndpointPermit {
    endpoint_id: String,
    state: Arc<Mutex<RuntimeState>>,
}

impl Drop for EndpointPermit {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.lock() {
            if let Some(endpoint_state) = guard.by_endpoint.get_mut(&self.endpoint_id) {
                endpoint_state.in_flight = endpoint_state.in_flight.saturating_sub(1);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AcquireRejectReason {
    RouteCooldown,
    EndpointBackoff,
    EndpointBusy,
}

impl LoadBalancerRuntime {
    pub fn new(
        config: LoadBalancerConfig,
        endpoint_directory: HashMap<String, LoadBalancerEndpoint>,
        log_tx: Option<broadcast::Sender<String>>,
    ) -> Self {
        let profile_index_by_id = config
            .profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| (profile.id.clone(), index))
            .collect();

        Self {
            config,
            profile_index_by_id,
            endpoint_directory,
            state: Arc::new(Mutex::new(RuntimeState::default())),
            log_tx,
        }
    }

    fn send_log(&self, msg: String) {
        if let Some(ref tx) = self.log_tx {
            let _ = tx.send(msg);
        }
    }

    fn send_route_status(
        &self,
        slot: ModelSlot,
        endpoint_id: &str,
        converter: &str,
        model_hint: &str,
        state: &str,
        reason: &str,
        cooldown_secs: Option<u32>,
    ) {
        let key = Self::build_route_key(slot, endpoint_id, converter, model_hint);
        let mut msg = format!(
            "[LBStatus] key={} slot={} endpoint={} converter={} model={} state={} reason={}",
            key,
            slot.as_str(),
            Self::sanitize_token(endpoint_id),
            Self::sanitize_token(converter),
            Self::sanitize_token(model_hint),
            state,
            Self::sanitize_token(reason),
        );
        if let Some(secs) = cooldown_secs {
            msg.push_str(&format!(" cooldown_secs={}", secs));
        }
        self.send_log(msg);
    }

    pub fn resolve_and_acquire(&self, model_name: &str) -> Option<(ResolvedEndpoint, EndpointPermit)> {
        let slot = ModelSlot::from_model_name(model_name);
        let profile = self.current_profile()?;

        for candidate in profile.model_mapping.get(slot) {
            let endpoint = match self.endpoint_directory.get(&candidate.endpoint_id) {
                Some(endpoint) => endpoint,
                None => {
                    self.send_log(format!(
                        "[LB] resolve endpoint_id={} not found in directory",
                        candidate.endpoint_id
                    ));
                    continue;
                }
            };

            let policy = self
                .config
                .endpoint_policies
                .get(&candidate.endpoint_id)
                .cloned()
                .unwrap_or_default();

            if !policy.enabled {
                self.send_log(format!(
                    "[LB] resolve endpoint_id={} skipped (disabled)",
                    candidate.endpoint_id
                ));
                continue;
            }

            let converter = candidate
                .converter_override
                .clone()
                .unwrap_or_else(|| endpoint.converter.clone());
            let model_hint = Self::normalize_model_hint(candidate.custom_model_name.as_deref());
            let route_key = Self::build_route_key(slot, &candidate.endpoint_id, &converter, &model_hint);

            match self.try_acquire_endpoint_for_route(
                &candidate.endpoint_id,
                &route_key,
                &policy,
                slot,
                &converter,
                &model_hint,
            ) {
                Ok(()) => {}
                Err(AcquireRejectReason::RouteCooldown) => {
                    self.send_log(format!(
                        "[LB] resolve endpoint_id={} slot={} route_key={} skipped (health=Cooldown)",
                        candidate.endpoint_id,
                        slot.as_str(),
                        route_key,
                    ));
                    continue;
                }
                Err(AcquireRejectReason::EndpointBackoff) => {
                    self.send_log(format!(
                        "[LB] resolve endpoint_id={} slot={} route_key={} skipped (endpoint backoff)",
                        candidate.endpoint_id,
                        slot.as_str(),
                        route_key,
                    ));
                    continue;
                }
                Err(AcquireRejectReason::EndpointBusy) => {
                    self.send_log(format!(
                        "[LB] resolve endpoint_id={} slot={} route_key={} skipped (in_flight limit reached)",
                        candidate.endpoint_id,
                        slot.as_str(),
                        route_key,
                    ));
                    continue;
                }
            }

            self.send_log(format!(
                "[LB] resolve model={} slot={} -> endpoint_id={} url={} converter={} route_key={}",
                model_name,
                slot.as_str(),
                candidate.endpoint_id,
                endpoint.target_url,
                converter,
                route_key,
            ));

            let permit = EndpointPermit {
                endpoint_id: candidate.endpoint_id.clone(),
                state: Arc::clone(&self.state),
            };

            return Some((
                ResolvedEndpoint {
                    endpoint_id: candidate.endpoint_id.clone(),
                    target_url: endpoint.target_url.clone(),
                    api_key: endpoint.api_key.clone(),
                    converter,
                    model: candidate.custom_model_name.clone(),
                    reasoning_effort: candidate.custom_reasoning_effort.clone(),
                    slot,
                    route_key,
                    model_hint,
                },
                permit,
            ));
        }

        self.send_log(format!(
            "[LB] resolve failed model={} slot={} no available endpoint",
            model_name,
            slot.as_str()
        ));
        None
    }

    pub fn record_result(&self, resolved: &ResolvedEndpoint, status: Option<u16>, network_error: bool) {
        let policy = self
            .config
            .endpoint_policies
            .get(&resolved.endpoint_id)
            .cloned()
            .unwrap_or_default();

        let mut became_available = false;
        let mut became_unavailable = false;

        if let Ok(mut guard) = self.state.lock() {
            let route_state = guard
                .by_route
                .entry(resolved.route_key.clone())
                .or_insert_with(RouteState::default);

            let previous_health = route_state.health;
            let now = Instant::now();
            if Self::refresh_route_state(route_state, &policy, now) && previous_health == EndpointHealth::Cooldown {
                became_available = true;
            }

            if !Self::is_counted_error(status, network_error) {
                drop(guard);

                if became_available {
                    self.send_log(format!(
                        "[LB] route={} state=Cooldown->Healthy (cooldown expired)",
                        resolved.route_key,
                    ));
                    self.send_route_status(
                        resolved.slot,
                        &resolved.endpoint_id,
                        &resolved.converter,
                        &resolved.model_hint,
                        "available",
                        "cooldown_expired",
                        None,
                    );
                }
                return;
            }

            route_state.errors.push_back(now);
            Self::prune_errors(route_state, &policy, now);

            if route_state.errors.len() as u32 >= policy.error_threshold {
                route_state.health = match route_state.health {
                    EndpointHealth::Healthy => EndpointHealth::Constrained,
                    EndpointHealth::Constrained => {
                        route_state.cooldown_until = Some(now + Duration::from_secs(policy.cooldown_seconds as u64));
                        EndpointHealth::Cooldown
                    }
                    EndpointHealth::Cooldown => EndpointHealth::Cooldown,
                };
            }

            let current_health = route_state.health;
            drop(guard);

            if previous_health != current_health {
                match (previous_health, current_health) {
                    (EndpointHealth::Healthy, EndpointHealth::Constrained) => {
                        self.send_log(format!(
                            "[LB] route={} state=Healthy->Constrained errors>={}",
                            resolved.route_key,
                            policy.error_threshold,
                        ));
                    }
                    (EndpointHealth::Constrained, EndpointHealth::Cooldown)
                    | (EndpointHealth::Healthy, EndpointHealth::Cooldown) => {
                        became_unavailable = true;
                        self.send_log(format!(
                            "[LB] route={} state={:?}->Cooldown cooldown_secs={}",
                            resolved.route_key,
                            previous_health,
                            policy.cooldown_seconds,
                        ));
                    }
                    (EndpointHealth::Cooldown, EndpointHealth::Constrained)
                    | (EndpointHealth::Cooldown, EndpointHealth::Healthy) => {
                        became_available = true;
                        self.send_log(format!(
                            "[LB] route={} state=Cooldown->{:?}",
                            resolved.route_key,
                            current_health,
                        ));
                    }
                    _ => {
                        self.send_log(format!(
                            "[LB] route={} state={:?}->{:?}",
                            resolved.route_key,
                            previous_health,
                            current_health,
                        ));
                    }
                }
            }
        }

        if became_unavailable {
            self.send_route_status(
                resolved.slot,
                &resolved.endpoint_id,
                &resolved.converter,
                &resolved.model_hint,
                "unavailable",
                "error_threshold",
                Some(policy.cooldown_seconds),
            );
        } else if became_available {
            self.send_route_status(
                resolved.slot,
                &resolved.endpoint_id,
                &resolved.converter,
                &resolved.model_hint,
                "available",
                "recovered",
                None,
            );
        }
    }

    pub fn handle_upstream_outcome(
        &self,
        resolved: &ResolvedEndpoint,
        status: Option<u16>,
        network_error: bool,
        error_text: Option<&str>,
    ) {
        let policy = self
            .config
            .endpoint_policies
            .get(&resolved.endpoint_id)
            .cloned()
            .unwrap_or_default();

        if network_error {
            self.record_result(resolved, status, true);
            return;
        }

        let Some(code) = status else {
            return;
        };

        if (200..=299).contains(&code) {
            self.record_result(resolved, Some(code), false);
            return;
        }

        let detail = error_text.unwrap_or("");
        if let Some(reason) = Self::classify_unavailable_reason(code, detail) {
            self.mark_unavailable(resolved, reason);
            return;
        }

        if Self::is_transient_overload(code, detail) {
            let backoff_secs = policy.transient_backoff_seconds.max(1) as u64;
            self.set_endpoint_backoff(
                &resolved.endpoint_id,
                backoff_secs,
                "overload",
            );
            self.send_log(format!(
                "[LB] route={} transient_overload status={} endpoint_backoff={}s",
                resolved.route_key,
                code,
                backoff_secs,
            ));
            return;
        }

        self.record_result(resolved, Some(code), false);
    }

    pub fn mark_unavailable(&self, resolved: &ResolvedEndpoint, reason: &str) {
        let policy = self
            .config
            .endpoint_policies
            .get(&resolved.endpoint_id)
            .cloned()
            .unwrap_or_default();

        if let Ok(mut guard) = self.state.lock() {
            let route_state = guard
                .by_route
                .entry(resolved.route_key.clone())
                .or_insert_with(RouteState::default);
            route_state.health = EndpointHealth::Cooldown;
            route_state.cooldown_until = Some(Instant::now() + Duration::from_secs(policy.cooldown_seconds as u64));
            route_state.errors.clear();
        }

        self.send_log(format!(
            "[LB] route={} force_cooldown reason={} cooldown_secs={}",
            resolved.route_key,
            reason,
            policy.cooldown_seconds,
        ));

        self.send_route_status(
            resolved.slot,
            &resolved.endpoint_id,
            &resolved.converter,
            &resolved.model_hint,
            "unavailable",
            reason,
            Some(policy.cooldown_seconds),
        );
    }

    fn current_profile(&self) -> Option<&LoadBalancerProfile> {
        let selected_id = self.config.selected_profile_id.as_ref()?;
        let index = self.profile_index_by_id.get(selected_id)?;
        self.config.profiles.get(*index)
    }

    fn try_acquire_endpoint_for_route(
        &self,
        endpoint_id: &str,
        route_key: &str,
        policy: &EndpointPolicy,
        slot: ModelSlot,
        converter: &str,
        model_hint: &str,
    ) -> Result<(), AcquireRejectReason> {
        let mut cooldown_expired = false;
        let mut result = Ok(());

        if let Ok(mut guard) = self.state.lock() {
            let now = Instant::now();
            let route_health = {
                let route_state = guard
                    .by_route
                    .entry(route_key.to_string())
                    .or_insert_with(RouteState::default);
                cooldown_expired = Self::refresh_route_state(route_state, policy, now)
                    && route_state.health != EndpointHealth::Cooldown;
                route_state.health
            };

            if route_health == EndpointHealth::Cooldown {
                result = Err(AcquireRejectReason::RouteCooldown);
            } else {
                let allowed = if route_health == EndpointHealth::Constrained {
                    policy.max_concurrency.min(policy.degraded_concurrency)
                } else {
                    policy.max_concurrency
                };
                let endpoint_state = guard
                    .by_endpoint
                    .entry(endpoint_id.to_string())
                    .or_insert_with(EndpointState::default);

                if let Some(until) = endpoint_state.transient_backoff_until {
                    if until > now {
                        result = Err(AcquireRejectReason::EndpointBackoff);
                        // keep existing flow; caller receives reject reason and tries next candidate
                    } else {
                        endpoint_state.transient_backoff_until = None;
                    }
                }

                if result.is_ok() && endpoint_state.in_flight >= allowed {
                    result = Err(AcquireRejectReason::EndpointBusy);
                } else if result.is_ok() {
                    endpoint_state.in_flight = endpoint_state.in_flight.saturating_add(1);
                }
            }
        } else {
            result = Err(AcquireRejectReason::EndpointBusy);
        }

        if cooldown_expired {
            self.send_route_status(slot, endpoint_id, converter, model_hint, "available", "cooldown_expired", None);
        }

        result
    }

    fn refresh_route_state(route_state: &mut RouteState, policy: &EndpointPolicy, now: Instant) -> bool {
        Self::prune_errors(route_state, policy, now);

        let mut cooldown_expired = false;
        if let Some(until) = route_state.cooldown_until {
            if until > now {
                route_state.health = EndpointHealth::Cooldown;
                return false;
            }
            route_state.cooldown_until = None;
            cooldown_expired = true;
        }

        route_state.health = if route_state.errors.len() as u32 >= policy.error_threshold {
            EndpointHealth::Constrained
        } else {
            EndpointHealth::Healthy
        };

        cooldown_expired
    }

    fn prune_errors(route_state: &mut RouteState, policy: &EndpointPolicy, now: Instant) {
        let window = Duration::from_secs(policy.error_window_seconds as u64);
        while let Some(oldest) = route_state.errors.front() {
            if now.duration_since(*oldest) > window {
                route_state.errors.pop_front();
            } else {
                break;
            }
        }
    }

    fn set_endpoint_backoff(&self, endpoint_id: &str, seconds: u64, reason: &str) {
        if seconds == 0 {
            return;
        }

        if let Ok(mut guard) = self.state.lock() {
            let endpoint_state = guard
                .by_endpoint
                .entry(endpoint_id.to_string())
                .or_insert_with(EndpointState::default);
            endpoint_state.transient_backoff_until = Some(Instant::now() + Duration::from_secs(seconds));
        }

        self.send_log(format!(
            "[LB] endpoint={} transient_backoff_secs={} reason={}",
            endpoint_id,
            seconds,
            reason,
        ));
    }

    fn classify_unavailable_reason(status: u16, error_text: &str) -> Option<&'static str> {
        if status == 401 || status == 403 {
            return Some("auth");
        }

        let lower = error_text.to_ascii_lowercase();
        let has_quota_signal = lower.contains("insufficient_quota")
            || lower.contains("quota exceeded")
            || lower.contains("out of credits")
            || lower.contains("insufficient balance")
            || lower.contains("billing")
            || lower.contains("额度")
            || lower.contains("余额")
            || lower.contains("欠费")
            || lower.contains("quota")
            || lower.contains("insufficient");

        if status == 429 && has_quota_signal {
            return Some("quota");
        }

        let has_model_signal = lower.contains("model_not_found")
            || lower.contains("unknown model")
            || lower.contains("unknown provider for model")
            || lower.contains("invalid model")
            || lower.contains("model does not exist")
            || lower.contains("unsupported model")
            || lower.contains("模型不存在")
            || lower.contains("模型不可用");

        let model_unavailable_status = status == 400
            || status == 404
            || status == 422
            || (500..=599).contains(&status);
        if model_unavailable_status && has_model_signal {
            return Some("model_unavailable");
        }

        None
    }

    fn is_transient_overload(status: u16, error_text: &str) -> bool {
        let lower = error_text.to_ascii_lowercase();
        if status == 429 {
            return true;
        }
        if status == 503 || status == 529 {
            return lower.contains("too many")
                || lower.contains("rate limit")
                || lower.contains("overload")
                || lower.contains("concurr")
                || lower.contains("busy")
                || lower.contains("高并发")
                || lower.contains("拥塞");
        }
        false
    }

    fn is_counted_error(status: Option<u16>, network_error: bool) -> bool {
        if network_error {
            return true;
        }

        let Some(code) = status else {
            return false;
        };

        if (500..=599).contains(&code) {
            return true;
        }

        code == 401 || code == 403
    }

    fn normalize_model_hint(model: Option<&str>) -> String {
        let trimmed = model.unwrap_or("").trim();
        if trimmed.is_empty() {
            "_default".to_string()
        } else {
            Self::sanitize_token(trimmed)
        }
    }

    fn build_route_key(slot: ModelSlot, endpoint_id: &str, converter: &str, model_hint: &str) -> String {
        format!(
            "{}|{}|{}|{}",
            slot.as_str(),
            Self::sanitize_token(endpoint_id),
            Self::sanitize_token(converter),
            Self::sanitize_token(model_hint),
        )
    }

    fn sanitize_token(value: &str) -> String {
        value
            .trim()
            .chars()
            .map(|ch| if ch.is_whitespace() || ch == '|' { '_' } else { ch })
            .collect()
    }
}
