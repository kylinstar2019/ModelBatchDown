use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub url: String,
    pub quant: Option<String>,
    pub mode: Option<String>,
}

pub fn get_tasks_file_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    exe_dir.join("download_tasks.json")
}

#[tauri::command]
pub fn load_tasks() -> Result<Vec<DownloadTask>, String> {
    let path = get_tasks_file_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取任务文件失败: {}", e))?;
    let tasks: Vec<DownloadTask> = serde_json::from_str(&content)
        .unwrap_or_default();
    Ok(tasks)
}

#[tauri::command]
pub fn save_tasks(tasks: Vec<DownloadTask>) -> Result<(), String> {
    let path = get_tasks_file_path();
    let content = serde_json::to_string_pretty(&tasks)
        .map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("写入任务文件失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn clear_tasks() -> Result<(), String> {
    let path = get_tasks_file_path();
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("删除任务文件失败: {}", e))?;
    }
    Ok(())
}
