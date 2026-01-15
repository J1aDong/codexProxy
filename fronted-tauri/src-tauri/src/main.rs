#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod proxy;

use proxy::ProxyManager;
use std::sync::Mutex;

#[derive(Default)]
pub struct AppState {
    pub proxy_manager: Mutex<ProxyManager>,
}

fn main() {
    // 开发模式下强制开启调试日志
    #[cfg(debug_assertions)]
    codex_proxy_core::set_debug_log(true);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            proxy::start_proxy,
            proxy::stop_proxy,
            proxy::load_config,
            proxy::save_config,
            proxy::check_port,
            proxy::kill_port,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
