use codex_proxy_core::{ProxyServer, ReasoningEffort, ReasoningEffortMapping};
use serde::{Deserialize, Serialize};
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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EndpointOption {
    pub id: String,
    pub alias: String,
    pub url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
}

fn default_endpoint_options() -> Vec<EndpointOption> {
    vec![EndpointOption {
        id: "aicodemirror-default".to_string(),
        alias: "aicodemirror".to_string(),
        url: "https://api.aicodemirror.com/api/codex/backend-api/codex/responses".to_string(),
        api_key: String::new(),
    }]
}

fn default_selected_endpoint_id() -> String {
    "aicodemirror-default".to_string()
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
    #[serde(rename = "selectedEndpointId", default = "default_selected_endpoint_id")]
    pub selected_endpoint_id: String,
    #[serde(rename = "codexModel", default = "default_codex_model")]
    pub codex_model: String,
    #[serde(default)]
    pub force: bool,
    #[serde(rename = "reasoningEffort", default)]
    pub reasoning_effort: ReasoningEffortConfig,
    #[serde(rename = "skillInjectionPrompt", default)]
    pub skill_injection_prompt: String,
    #[serde(default = "default_lang")]
    pub lang: String,
}

fn default_lang() -> String {
    "zh".to_string()
}

fn default_codex_model() -> String {
    "gpt-5.3-codex".to_string()
}

pub struct ProxyManager {
    running: bool,
    shutdown_tx: Option<broadcast::Sender<()>>,
    log_tx: Option<broadcast::Sender<String>>,
}

impl Default for ProxyManager {
    fn default() -> Self {
        Self {
            running: false,
            shutdown_tx: None,
            log_tx: None,
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

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.running = false;
        self.log_tx = None;
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
    TcpListener::bind(("127.0.0.1", port)).is_err()
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
    if !config.force && check_port(config.port) {
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

    app.emit(
        "proxy-log",
        format!("[System] Starting proxy on port {}...", config.port),
    )
    .map_err(|e| e.to_string())?;
    let selected_endpoint = config
        .endpoint_options
        .iter()
        .find(|item| item.id == config.selected_endpoint_id);

    let resolved_target_url = selected_endpoint
        .map(|item| item.url.clone())
        .unwrap_or_else(|| config.target_url.clone());

    app.emit("proxy-log", format!("[System] Target: {}", resolved_target_url))
        .map_err(|e| e.to_string())?;

    // 创建日志通道（容量 2048 减少高频场景下的 lag）
    let (log_tx, mut log_rx) = broadcast::channel::<String>(2048);
    manager.log_tx = Some(log_tx.clone());

    let resolved_api_key = selected_endpoint
        .map(|item| item.api_key.clone())
        .unwrap_or_else(|| config.api_key.clone());

    let api_key = if resolved_api_key.is_empty() {
        None
    } else {
        Some(resolved_api_key)
    };

    let server = ProxyServer::new(config.port, resolved_target_url.clone(), api_key)
    .with_reasoning_mapping(config.reasoning_effort.to_mapping())
    .with_codex_model(config.codex_model.clone());

    // 启动日志转发（Lagged 时跳过丢失的消息继续接收，不退出）
    let app_clone = app.clone();
    tokio::spawn(async move {
        loop {
            match log_rx.recv().await {
                Ok(msg) => {
                    let _ = app_clone.emit("proxy-log", msg);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let _ = app_clone.emit("proxy-log",
                        format!("[Warning] Log receiver lagged, skipped {} messages", n));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 启动代理服务器
    let app_clone = app.clone();
    match server.start(log_tx).await {
        Ok(shutdown_tx) => {
            manager.set_shutdown_tx(shutdown_tx);
            manager.set_running(true);
            app.emit("proxy-status", "running")
                .map_err(|e| e.to_string())?;
        }
        Err(e) => {
            let _ = app_clone.emit("proxy-log", format!("[Error] Server error: {}", e));
            let _ = app_clone.emit("proxy-status", "stopped");
            return Err(e.to_string());
        }
    }

    Ok(())
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
