use codex_proxy_core::ProxyServer;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProxyConfig {
    pub port: u16,
    #[serde(rename = "targetUrl")]
    pub target_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    #[serde(default)]
    pub force: bool,
}

pub struct ProxyManager {
    server: Option<Arc<Mutex<ProxyServer>>>,
    log_tx: Option<broadcast::Sender<String>>,
}

impl Default for ProxyManager {
    fn default() -> Self {
        Self {
            server: None,
            log_tx: None,
        }
    }
}

impl ProxyManager {
    pub fn is_running(&self) -> bool {
        self.server.is_some()
    }

    pub fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            let server_clone = server.clone();
            tokio::spawn(async move {
                let mut s = server_clone.lock().await;
                s.stop();
            });
        }
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
    let mut manager = state.proxy_manager.lock().map_err(|e| e.to_string())?;

    if manager.is_running() {
        return Err("Proxy is already running".to_string());
    }

    app.emit(
        "proxy-log",
        format!("[System] Starting proxy on port {}...", config.port),
    )
    .map_err(|e| e.to_string())?;
    app.emit("proxy-log", format!("[System] Target: {}", config.target_url))
        .map_err(|e| e.to_string())?;

    // 创建日志通道
    let (log_tx, mut log_rx) = broadcast::channel::<String>(256);
    manager.log_tx = Some(log_tx.clone());

    // 创建代理服务器
    let api_key = if config.api_key.is_empty() {
        None
    } else {
        Some(config.api_key.clone())
    };

    let server = Arc::new(Mutex::new(ProxyServer::new(
        config.port,
        config.target_url.clone(),
        api_key,
    )));

    manager.server = Some(server.clone());

    // 启动日志转发
    let app_clone = app.clone();
    tokio::spawn(async move {
        while let Ok(msg) = log_rx.recv().await {
            let _ = app_clone.emit("proxy-log", msg);
        }
    });

    // 启动代理服务器
    let app_clone = app.clone();
    tokio::spawn(async move {
        let mut server_guard = server.lock().await;
        if let Err(e) = server_guard.start(log_tx).await {
            let _ = app_clone.emit("proxy-log", format!("[Error] Server error: {}", e));
            let _ = app_clone.emit("proxy-status", "stopped");
        }
    });

    app.emit("proxy-status", "running")
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn stop_proxy(app: AppHandle) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let mut manager = state.proxy_manager.lock().map_err(|e| e.to_string())?;

    manager.stop();

    app.emit("proxy-status", "stopped")
        .map_err(|e| e.to_string())?;
    app.emit("proxy-log", "[System] Proxy stopped")
        .map_err(|e| e.to_string())?;

    Ok(())
}
