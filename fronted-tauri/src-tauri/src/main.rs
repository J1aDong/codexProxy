#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod proxy;

use proxy::ProxyManager;
use tokio::sync::Mutex;

pub struct AppState {
    pub proxy_manager: Mutex<ProxyManager>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            proxy_manager: Mutex::new(ProxyManager::default()),
        }
    }
}

fn main() {
    // 始终开启调试日志（用于排查问题）
    codex_proxy_core::set_debug_log(true);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            proxy::start_proxy,
            proxy::stop_proxy,
            proxy::load_config,
            proxy::save_config,
            proxy::save_lang,
            proxy::check_port,
            proxy::kill_port,
            proxy::export_config,
            proxy::import_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
