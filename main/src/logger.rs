use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// 日志目录
const LOG_DIR: &str = "logs";

/// 全局调试日志开关
static DEBUG_LOG_ENABLED: AtomicBool = AtomicBool::new(cfg!(debug_assertions));

/// 全局应用日志记录器
static APP_LOGGER: OnceLock<Arc<AppLogger>> = OnceLock::new();

/// 设置调试日志开关
pub fn set_debug_log(enabled: bool) {
    DEBUG_LOG_ENABLED.store(enabled, Ordering::SeqCst);
}

/// 获取调试日志开关状态
pub fn is_debug_log_enabled() -> bool {
    DEBUG_LOG_ENABLED.load(Ordering::SeqCst)
}

/// 截断字符串用于日志显示
pub fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}... (len={})", &s[..max_len], s.len())
    }
}

/// 应用级日志记录器 - 每次启动应用一个日志文件
pub struct AppLogger {
    log_path: PathBuf,
}

impl AppLogger {
    /// 初始化全局日志记录器（应用启动时调用一次）
    pub fn init(log_dir: Option<&str>) -> Arc<AppLogger> {
        APP_LOGGER.get_or_init(|| {
            let dir = log_dir.unwrap_or(LOG_DIR);
            let _ = fs::create_dir_all(dir);

            let start_time = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
            let log_path = PathBuf::from(dir).join(format!("proxy_{}.log", start_time));

            let logger = Arc::new(AppLogger {
                log_path,
            });

            // 写入启动信息
            logger.log("════════════════════════════════════════════════════════════════════════");
            logger.log("🚀 Codex Proxy Started");
            logger.log(&format!("📁 Log file: {:?}", logger.log_path));
            logger.log("════════════════════════════════════════════════════════════════════════");

            logger
        }).clone()
    }

    /// 获取全局日志记录器
    pub fn get() -> Option<Arc<AppLogger>> {
        APP_LOGGER.get().cloned()
    }

    /// 写入日志（仅在 debug 模式下）
    pub fn log(&self, message: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] {}\n", timestamp, message);

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// 写入原始内容（不带时间戳，用于记录 JSON）
    pub fn log_raw(&self, content: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(content.as_bytes());
            let _ = file.write_all(b"\n");
        }
    }

    /// 记录新请求开始（带 session ID 分隔）
    pub fn log_request_start(&self, session_id: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_content = format!(
            "\n\n[{}] ╔══════════════════════════════════════════════════════════════════════╗\n\
             [{}] ║ 🆕 NEW REQUEST - Session: {}\n\
             [{}] ╚══════════════════════════════════════════════════════════════════════╝\n",
            timestamp, timestamp, session_id, timestamp
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 记录 curl 格式的请求
    pub fn log_curl_request(&self, method: &str, url: &str, headers: &[(&str, &str)], body: &Value) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let mut curl_cmd = format!("curl -X {} '{}'", method, url);

        for (key, value) in headers {
            curl_cmd.push_str(&format!(" \\\n  -H '{}: {}'", key, value));
        }

        // 格式化 JSON body
        let pretty_body = serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string());

        let log_content = format!(
            "\n[{}] ════════════════════════════════════════════════════════════════\n\
             [{}] 📤 OUTGOING REQUEST (Codex API)\n\
             [{}] ════════════════════════════════════════════════════════════════\n\
             {}\n\
             \n\
             Request Body:\n\
             {}\n\
             ════════════════════════════════════════════════════════════════════════\n",
            timestamp, timestamp, timestamp, curl_cmd, pretty_body
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 记录原始 Anthropic 请求
    pub fn log_anthropic_request(&self, body_bytes: &[u8]) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");

        // 尝试格式化 JSON，失败则使用原始字符串
        let body_str = String::from_utf8_lossy(body_bytes);
        let pretty_body = if let Ok(json) = serde_json::from_slice::<Value>(body_bytes) {
            serde_json::to_string_pretty(&json).unwrap_or_else(|_| body_str.to_string())
        } else {
            body_str.to_string()
        };

        let log_content = format!(
            "\n[{}] ════════════════════════════════════════════════════════════════\n\
             [{}] 📥 INCOMING ANTHROPIC REQUEST\n\
             [{}] ════════════════════════════════════════════════════════════════\n\
             {}\n\
             ════════════════════════════════════════════════════════════════════════\n",
            timestamp, timestamp, timestamp, pretty_body
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 记录上游响应
    pub fn log_upstream_response(&self, status: u16, line: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] 📩 [Upstream {}] {}\n", timestamp, status, line);

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// 记录转换后的 Anthropic 响应
    pub fn log_anthropic_response(&self, line: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] 📤 [To Client] {}\n", timestamp, line.trim());

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// 记录请求完成
    pub fn log_request_end(&self) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_content = format!(
            "[{}] ════════════════════════════════════════════════════════════════\n\
             [{}] ✅ Request completed\n\
             [{}] ════════════════════════════════════════════════════════════════\n",
            timestamp, timestamp, timestamp
        );

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = file.write_all(log_content.as_bytes());
        }
    }

    /// 获取日志文件路径
    pub fn log_path(&self) -> &PathBuf {
        &self.log_path
    }
}
