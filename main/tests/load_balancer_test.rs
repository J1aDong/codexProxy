use codex_proxy_core::load_balancer::*;
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;

fn create_test_runtime() -> LoadBalancerRuntime {
    let endpoint_directory: HashMap<String, LoadBalancerEndpoint> = [
        (
            "ep-1".to_string(),
            LoadBalancerEndpoint {
                id: "ep-1".to_string(),
                target_url: "https://api1.example.com".to_string(),
                api_key: Some("key1".to_string()),
                converter: "codex".to_string(),
            },
        ),
        (
            "ep-2".to_string(),
            LoadBalancerEndpoint {
                id: "ep-2".to_string(),
                target_url: "https://api2.example.com".to_string(),
                api_key: Some("key2".to_string()),
                converter: "gemini".to_string(),
            },
        ),
    ]
    .into_iter()
    .collect();

    let profiles = vec![LoadBalancerProfile {
        id: "profile-1".to_string(),
        name: "Test Profile".to_string(),
        model_mapping: SlotMapping {
            opus: vec![SlotEndpointRef {
                endpoint_id: "ep-1".to_string(),
                custom_model_name: None,
                custom_reasoning_effort: None,
                converter_override: None,
            }],
            sonnet: vec![
                SlotEndpointRef {
                    endpoint_id: "ep-1".to_string(),
                    custom_model_name: None,
                    custom_reasoning_effort: None,
                    converter_override: None,
                },
                SlotEndpointRef {
                    endpoint_id: "ep-2".to_string(),
                    custom_model_name: None,
                    custom_reasoning_effort: None,
                    converter_override: None,
                },
            ],
            haiku: vec![SlotEndpointRef {
                endpoint_id: "ep-2".to_string(),
                custom_model_name: None,
                custom_reasoning_effort: None,
                converter_override: None,
            }],
        },
    }];

    let endpoint_policies: HashMap<String, EndpointPolicy> = [(
        "ep-1".to_string(),
        EndpointPolicy {
            enabled: true,
            max_concurrency: 2,
            error_threshold: 3,
            error_window_seconds: 60,
            cooldown_seconds: 5,
            degraded_concurrency: 1,
            transient_backoff_seconds: 1,
        },
    )]
    .into_iter()
    .collect();

    LoadBalancerRuntime::new(
        LoadBalancerConfig {
            selected_profile_id: Some("profile-1".to_string()),
            profiles,
            endpoint_policies,
        },
        endpoint_directory,
        None,
    )
}

fn resolve_opus_route(runtime: &LoadBalancerRuntime) -> ResolvedEndpoint {
    let result = runtime.resolve_and_acquire("claude-opus");
    assert!(result.is_some());
    let (resolved, permit) = result.unwrap();
    drop(permit);
    resolved
}

#[test]
fn test_resolve_by_model() {
    let runtime = create_test_runtime();

    let result = runtime.resolve_and_acquire("claude-opus-4-5");
    assert!(result.is_some());
    let (resolved, _permit) = result.unwrap();
    assert_eq!(resolved.endpoint_id, "ep-1");

    let result = runtime.resolve_and_acquire("claude-haiku");
    assert!(result.is_some());
    let (resolved, _permit) = result.unwrap();
    assert_eq!(resolved.endpoint_id, "ep-2");
}

#[test]
fn test_concurrent_limit() {
    let runtime = create_test_runtime();

    let r1 = runtime.resolve_and_acquire("claude-opus");
    assert!(r1.is_some());

    let r2 = runtime.resolve_and_acquire("claude-opus");
    assert!(r2.is_some());

    let r3 = runtime.resolve_and_acquire("claude-opus");
    assert!(r3.is_none(), "Should reject when concurrency limit reached");

    drop(r1);

    let r4 = runtime.resolve_and_acquire("claude-opus");
    assert!(r4.is_some(), "Should accept after permit dropped");
}

#[test]
fn test_error_threshold_to_constrained() {
    let runtime = create_test_runtime();
    let resolved = resolve_opus_route(&runtime);

    for _ in 0..3 {
        runtime.record_result(&resolved, Some(500), false);
    }

    let result = runtime.resolve_and_acquire("claude-opus");
    assert!(result.is_some());
    let (_resolved, permit) = result.unwrap();

    drop(permit);
}

#[test]
fn test_network_error_counted() {
    let runtime = create_test_runtime();
    let resolved = resolve_opus_route(&runtime);

    runtime.record_result(&resolved, None, true);
    runtime.record_result(&resolved, None, true);

    let result = runtime.resolve_and_acquire("claude-opus");
    assert!(result.is_some());
}

#[test]
fn test_4xx_not_counted() {
    let runtime = create_test_runtime();
    let resolved = resolve_opus_route(&runtime);

    for _ in 0..10 {
        runtime.record_result(&resolved, Some(400), false);
    }

    let result = runtime.resolve_and_acquire("claude-opus");
    assert!(result.is_some());
}

#[test]
fn test_transient_overload_uses_short_backoff_without_marking_unavailable() {
    let runtime = create_test_runtime();

    let result = runtime.resolve_and_acquire("claude-opus");
    assert!(result.is_some());
    let (resolved, permit) = result.unwrap();
    drop(permit);

    runtime.handle_upstream_outcome(
        &resolved,
        Some(429),
        false,
        Some("too many concurrent requests"),
    );

    let immediate = runtime.resolve_and_acquire("claude-opus");
    assert!(immediate.is_none(), "endpoint should be in transient backoff");

    sleep(Duration::from_millis(1100));

    let after = runtime.resolve_and_acquire("claude-opus");
    assert!(after.is_some(), "endpoint should recover after transient backoff");
}
