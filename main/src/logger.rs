use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// 日志目录
const LOG_DIR: &str = "logs";
/// 日志保留天数
const LOG_RETENTION_DAYS: u64 = 3;
/// 单个日志文件最大请求交互数
const LOG_MAX_REQUESTS: usize = 200;
/// 运行时每隔多少个请求触发一次裁剪
const LOG_TRIM_INTERVAL_REQUESTS: usize = 20;
/// 请求块分隔标记
const REQUEST_MARKER: &str = "🆕 NEW REQUEST - Session:";

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
    write_lock: Mutex<()>,
    request_counter: AtomicUsize,
}

impl AppLogger {
    fn default_log_dir() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".codexProxy").join("logs");
        }

        if let Some(user_profile) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(user_profile).join(".codexProxy").join("logs");
        }

        PathBuf::from(LOG_DIR)
    }

    fn cleanup_log_dir(log_dir: &Path) {
        let retention = Duration::from_secs(LOG_RETENTION_DAYS * 24 * 60 * 60);
        let entries = match fs::read_dir(log_dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("log"))
                .unwrap_or(false)
            {
                continue;
            }

            let metadata = match entry.metadata() {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };

            let is_expired = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .map(|elapsed| elapsed > retention)
                .unwrap_or(false);

            if is_expired {
                let _ = fs::remove_file(&path);
                continue;
            }

            let _ = Self::trim_file_requests(&path, LOG_MAX_REQUESTS);
        }
    }

    fn trim_file_requests(path: &Path, max_requests: usize) -> std::io::Result<()> {
        let content = fs::read_to_string(path)?;
        let trimmed = Self::trim_log_content_by_requests(&content, max_requests);
        if trimmed != content {
            fs::write(path, trimmed)?;
        }
        Ok(())
    }

    fn trim_log_content_by_requests(content: &str, max_requests: usize) -> String {
        if max_requests == 0 {
            return String::new();
        }

        if !content.contains(REQUEST_MARKER) {
            return content.to_string();
        }

        let mut blocks: Vec<String> = Vec::new();
        let mut current_block = String::new();

        for segment in content.split_inclusive('\n') {
            if segment.contains(REQUEST_MARKER) && !current_block.is_empty() {
                blocks.push(current_block);
                current_block = String::new();
            }
            current_block.push_str(segment);
        }

        if !current_block.is_empty() {
            blocks.push(current_block);
        }

        if blocks.len() <= max_requests {
            return content.to_string();
        }

        let keep_from = blocks.len() - max_requests;
        blocks[keep_from..].concat()
    }

    fn with_write_lock<F>(&self, f: F)
    where
        F: FnOnce(),
    {
        if let Ok(_guard) = self.write_lock.lock() {
            f();
        }
    }

    fn append_content(&self, content: &str) {
        self.with_write_lock(|| {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                let _ = file.write_all(content.as_bytes());
            }
        });
    }

    fn append_request_with_conditional_trim(&self, content: &str) {
        self.with_write_lock(|| {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                let _ = file.write_all(content.as_bytes());
            }

            let current = self.request_counter.fetch_add(1, Ordering::Relaxed) + 1;
            if current % LOG_TRIM_INTERVAL_REQUESTS == 0 {
                let _ = Self::trim_file_requests(&self.log_path, LOG_MAX_REQUESTS);
            }
        });
    }

    /// 初始化全局日志记录器（应用启动时调用一次）
    pub fn init(log_dir: Option<&str>) -> Arc<AppLogger> {
        APP_LOGGER
            .get_or_init(|| {
                let dir = log_dir
                    .map(PathBuf::from)
                    .unwrap_or_else(Self::default_log_dir);
                let _ = fs::create_dir_all(&dir);
                Self::cleanup_log_dir(&dir);

                let start_time = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                let log_path = dir.join(format!("proxy_{}.log", start_time));

                let logger = Arc::new(AppLogger {
                    log_path,
                    write_lock: Mutex::new(()),
                    request_counter: AtomicUsize::new(0),
                });

                // 写入启动信息
                logger.log(
                    "════════════════════════════════════════════════════════════════════════",
                );
                logger.log("🚀 Codex Proxy Started");
                logger.log(&format!("📁 Log file: {:?}", logger.log_path));
                logger.log(
                    "════════════════════════════════════════════════════════════════════════",
                );

                logger
            })
            .clone()
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
        self.append_content(&line);
    }

    /// 写入原始内容（不带时间戳，用于记录 JSON）
    pub fn log_raw(&self, content: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let mut line = String::with_capacity(content.len() + 1);
        line.push_str(content);
        line.push('\n');
        self.append_content(&line);
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

        self.append_request_with_conditional_trim(&log_content);
    }

    /// 记录 curl 格式的请求
    pub fn log_curl_request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
        backend_label: &str,
    ) {
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
             [{}] 📤 OUTGOING REQUEST ({})\n\
             [{}] ════════════════════════════════════════════════════════════════\n\
             {}\n\
             \n\
             Request Body:\n\
             {}\n\
             ════════════════════════════════════════════════════════════════════════\n",
            timestamp, timestamp, backend_label, timestamp, curl_cmd, pretty_body
        );

        self.append_content(&log_content);
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

        self.append_content(&log_content);
    }

    /// 记录上游响应
    pub fn log_upstream_response(&self, status: u16, line: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] 📩 [Upstream {}] {}\n", timestamp, status, line);

        self.append_content(&line);
    }

    /// 记录转换后的 Anthropic 响应
    pub fn log_anthropic_response(&self, line: &str) {
        if !is_debug_log_enabled() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!("[{}] 📤 [To Client] {}\n", timestamp, line.trim());

        self.append_content(&line);
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

        self.append_content(&log_content);
    }

    /// 获取日志文件路径
    pub fn log_path(&self) -> &PathBuf {
        &self.log_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_last_request_blocks() {
        let input = concat!(
            "header\n",
            "[a] 🆕 NEW REQUEST - Session: s1\n",
            "s1-out\n",
            "[b] 🆕 NEW REQUEST - Session: s2\n",
            "s2-out\n",
            "[c] 🆕 NEW REQUEST - Session: s3\n",
            "s3-out\n"
        );

        let out = AppLogger::trim_log_content_by_requests(input, 2);
        assert!(!out.contains("Session: s1"));
        assert!(out.contains("Session: s2"));
        assert!(out.contains("Session: s3"));
        assert!(out.contains("s2-out"));
        assert!(out.contains("s3-out"));
    }

    #[test]
    fn trim_keeps_content_when_requests_under_limit() {
        let input = concat!("boot\n", "[a] 🆕 NEW REQUEST - Session: s1\n", "s1-out\n");

        let out = AppLogger::trim_log_content_by_requests(input, 200);
        assert_eq!(out, input);
    }
}
