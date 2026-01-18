use codex_proxy_core::{ProxyServer, ReasoningEffortMapping, ReasoningEffort, set_debug_log};
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::init();
    set_debug_log(true);

    let reasoning_mapping = ReasoningEffortMapping::new()
        .with_opus(ReasoningEffort::Xhigh)
        .with_sonnet(ReasoningEffort::Medium)
        .with_haiku(ReasoningEffort::Low);

    println!("🚀 Starting Codex Proxy Server with dynamic reasoning effort mapping...");
    println!("📋 Default reasoning effort mappings:");
    println!("  Opus models -> {:?}", reasoning_mapping.opus);
    println!("  Sonnet models -> {:?}", reasoning_mapping.sonnet);
    println!("  Haiku models -> {:?}", reasoning_mapping.haiku);

    let server = ProxyServer::new(
        8889,
        "https://api.aicodemirror.com/api/codex/backend-api/codex/responses".to_string(),
        std::env::var("ANTHROPIC_API_KEY").ok()
    ).with_reasoning_mapping(reasoning_mapping);
    
    println!("🌐 Server listening on http://127.0.0.1:8889");
    println!("📡 Ready to proxy requests to Anthropic API");
    
    let (log_tx, _log_rx) = broadcast::channel(100);
    let _shutdown_tx = server.start(log_tx).await?;
    
    println!("✅ Server started successfully!");
    
    tokio::signal::ctrl_c().await?;
    println!("🛑 Shutting down server...");
    
    Ok(())
}