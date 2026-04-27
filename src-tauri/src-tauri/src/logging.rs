use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub fn get_log_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    exe_dir.join(crate::constants::LOG_FILE)
}

pub fn chrono_lite_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let secs = now.as_secs();
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

pub fn write_log(msg: &str) {
    let log_path = get_log_path();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let timestamp = chrono_lite_timestamp();
        let _ = writeln!(file, "[{}] {}", timestamp, msg);
    }
}
