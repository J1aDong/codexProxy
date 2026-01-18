use codex_proxy_core::{set_debug_log, ReasoningEffortMapping, get_reasoning_effort};

#[tokio::main]
async fn main() {
    set_debug_log(true);
    println!("Debug logging enabled");

    let default_mapping = ReasoningEffortMapping::default();
    
    println!("Testing reasoning effort mapping:");
    println!("Sonnet -> {:?}", get_reasoning_effort("claude-3-5-sonnet-20241022", &default_mapping));
    println!("Opus -> {:?}", get_reasoning_effort("claude-3-opus-20240229", &default_mapping));
    println!("Haiku -> {:?}", get_reasoning_effort("claude-3-haiku-20240307", &default_mapping));
    println!("Unknown -> {:?}", get_reasoning_effort("unknown-model", &default_mapping));
    
    println!("Test completed successfully!");
}