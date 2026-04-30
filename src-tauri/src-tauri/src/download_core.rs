use futures::StreamExt;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};

use crate::login::AppState;
use crate::model_info::{extract_model_id, extract_model_name, get_hf_files_with_size, get_ms_files_with_size, FileInfo};
use crate::network_speed::format_speed;

const MAX_CONNECTIONS_PER_FILE: usize = 64;
const MAX_CONCURRENT_DOWNLOADS: usize = 2;

static BANDWIDTH_CACHE: AtomicU64 = AtomicU64::new(0);

pub struct GlobalDownloadTracker {
    total_downloaded: AtomicU64,
    total_uploaded: AtomicU64,
    start_time: Mutex<Instant>,
    last_downloaded: AtomicU64,
    instant_speed: AtomicU64,
}

impl GlobalDownloadTracker {
    pub fn new() -> Self {
        Self {
            total_downloaded: AtomicU64::new(0),
            total_uploaded: AtomicU64::new(0),
            start_time: Mutex::new(Instant::now()),
            last_downloaded: AtomicU64::new(0),
            instant_speed: AtomicU64::new(0),
        }
    }

    pub fn add_downloaded(&self, bytes: u64) {
        self.total_downloaded.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn get_total_downloaded(&self) -> u64 {
        self.total_downloaded.load(Ordering::Relaxed)
    }

    pub fn calculate_speed(&self) -> (u64, u64) {
        let now = Instant::now();
        let current_downloaded = self.total_downloaded.load(Ordering::Relaxed);

        let start_time = {
            *self.start_time.lock().unwrap()
        };

        let elapsed_total = now.duration_since(start_time).as_secs_f64();

        let last_downloaded = self.last_downloaded.load(Ordering::Relaxed);
        let elapsed = now.duration_since(start_time).as_secs_f64();

        if elapsed >= 1.0 && current_downloaded > 0 {
            let download_diff = current_downloaded.saturating_sub(last_downloaded);
            let download_speed = if download_diff > 0 && elapsed > 0.0 {
                (download_diff as f64 / elapsed) as u64
            } else {
                (current_downloaded as f64 / elapsed_total.max(1.0)) as u64
            };

            self.last_downloaded.store(current_downloaded, Ordering::Relaxed);
            self.instant_speed.store(download_speed, Ordering::Relaxed);

            (download_speed, 0)
        } else if current_downloaded > 0 && elapsed_total > 0.0 {
            let avg_speed = (current_downloaded as f64 / elapsed_total) as u64;
            self.instant_speed.store(avg_speed, Ordering::Relaxed);
            (avg_speed, 0)
        } else {
            (0, 0)
        }
    }
}

pub async fn estimate_bandwidth(test_url: &str) -> u64 {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(10)
        .tcp_nodelay(true)
        .build() {
        Ok(c) => c,
        Err(_) => return 8_000_000,
    };

    let start = Instant::now();
    let mut total_bytes = 0u64;
    let test_size = 8_000_000;

    let test_urls = if test_url.contains("modelscope.cn") {
        vec![
            test_url.to_string(),
            format!("{}/resolve/master/.gitkeep", test_url),
            format!("{}/resolve/master/config.json", test_url),
        ]
    } else {
        vec![test_url.to_string()]
    };

    for url in test_urls {
        if total_bytes >= test_size {
            break;
        }

        let encoded_url = url.replace(" ", "%20").replace("[", "%5D").replace("]", "%5D");

        let Ok(response) = client.get(&encoded_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Range", format!("bytes=0-{}", test_size - 1))
            .send()
            .await else {
            continue;
        };

        if !response.status().is_success() && response.status().as_u16() != 206 {
            continue;
        }

        let mut stream = response.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            if let Ok(chunk) = chunk_result {
                total_bytes += chunk.len() as u64;
                if total_bytes >= test_size {
                    break;
                }
            }
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    if elapsed > 0.0 && total_bytes > 0 {
        (total_bytes as f64 / elapsed) as u64
    } else {
        8_000_000
    }
}

pub fn start_bandwidth_test() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        let test_url = "https://modelscope.cn/modelscope/modelscope_hub/snapshots/download?file=examples/test.bin".to_string();
        let bandwidth = estimate_bandwidth(&test_url).await;
        BANDWIDTH_CACHE.store(bandwidth, Ordering::SeqCst);
    });
}

pub async fn get_hf_file_size(url: &str) -> Option<u64> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;

    let encoded_url = url
        .replace(" ", "%20")
        .replace("[", "%5D")
        .replace("]", "%5D");

    let response = client.get(&encoded_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Range", "bytes=0-0")
        .send()
        .await
        .ok()?;

    if let Some(range_header) = response.headers().get("content-range") {
        let range_str = range_header.to_str().ok()?;
        if let Some(total_size) = range_str.split('/').last() {
            return total_size.parse::<u64>().ok();
        }
    }

    if let Some(content_length) = response.headers().get("content-length") {
        let size = content_length.to_str().ok()?.parse::<u64>().ok()?;
        if size > 100 {
            return Some(size);
        }
    }

    None
}

pub async fn download_model_rust(
    url: &str,
    save_path: &str,
    gguf_quant: Option<String>,
    mode: &str,
    app_handle: AppHandle,
) -> Result<String, String> {
    let model_id = extract_model_id(url).ok_or("无法解析模型 ID")?;

    let allow_patterns = if let Some(ref quant) = gguf_quant {
        let pattern = quant.clone();
        let _ = app_handle.emit("download-log", format!("[DEBUG] GGUF quant: {}, pattern: {}", quant, pattern));
        Some(vec![pattern])
    } else if mode == "main" {
        Some(vec![
            "config.json".to_string(),
            "generation_config.json".to_string(),
            "tokenizer*.json".to_string(),
            "tokenizer.model".to_string(),
            "special_tokens_map.json".to_string(),
            "*.safetensors".to_string(),
            "*.safetensors.index.json".to_string(),
            "preprocessor_config.json".to_string(),
            "processor_config.json".to_string(),
        ])
    } else {
        None
    };

    if url.contains("huggingface.co") || url.contains("hf-mirror.com") {
        huggingface_download(&model_id, save_path, allow_patterns, app_handle).await
    } else if url.contains("modelscope.cn") {
        modelscope_download(&model_id, save_path, allow_patterns, app_handle).await
    } else {
        Err("不支持的平台".to_string())
    }
}

pub async fn huggingface_download(
    model_id: &str,
    save_path: &str,
    allow_patterns: Option<Vec<String>>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let _ = app_handle.emit("download-log", format!("[HF] 开始下载模型：{}", model_id));

    let save_dir = std::path::PathBuf::from(save_path);
    if let Err(e) = std::fs::create_dir_all(&save_dir) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建目录失败：{}", e));
    }

    let incomplete_marker = save_dir.join(".incomplete");
    if let Err(e) = std::fs::File::create(&incomplete_marker) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建未完成标记失败：{}", e));
    }

    let _ = app_handle.emit("download-log", format!("[HF] 开始获取文件列表..."));

    let files = get_hf_files_with_size(model_id)?;
    let _ = app_handle.emit("download-log", format!("[HF] 获取文件列表完成，共 {} 个文件", files.len()));

    let files_to_download = filter_files_with_size(&files, &allow_patterns);
    let _ = app_handle.emit("download-log", format!("[HF] 过滤后 {} 个文件需要下载", files_to_download.len()));

    if files_to_download.is_empty() {
        let _ = app_handle.emit("download-log", format!("[HF] 原始文件列表: {:?}", files.iter().take(10).map(|f| &f.path).collect::<Vec<_>>()));
        let _ = std::fs::remove_file(&incomplete_marker);
        return Err("没有找到需要下载的文件".to_string());
    }

    let endpoint = std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".to_string());
    let base_url = format!("{}/{}/resolve/main", endpoint, model_id);
    let _ = app_handle.emit("download-log", format!("[HF] 下载基础URL: {}", base_url));

    let mut files_with_size = files_to_download.clone();
    for f in &mut files_with_size {
        let min_expected_size = if f.path.ends_with(".gguf") { 1024 * 1024 } else { 1024 };
        if f.size < min_expected_size {
            let file_url = format!("{}/{}", base_url, f.path);
            let _ = app_handle.emit("download-log", format!("[HF] 验证文件大小: {}", f.path));
            if let Some(size) = get_hf_file_size(&file_url).await {
                let old_size = f.size;
                f.size = size;
                if old_size != size {
                    let _ = app_handle.emit("download-log", format!("[HF] 文件大小修正: {} -> {}", old_size, size));
                }
            }
        }
    }

    let total_size_all: u64 = files_with_size.iter().map(|f| f.size).sum();
    let _ = app_handle.emit("download-log", format!("[HF] 准备下载 {} 个文件，总大小：{}", files_with_size.len(), format_bytes(total_size_all)));

    download_files_from_urls(&base_url, &files_with_size, save_path, app_handle, incomplete_marker).await
}

pub async fn modelscope_download(
    model_id: &str,
    save_path: &str,
    allow_patterns: Option<Vec<String>>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let _ = app_handle.emit("download-log", format!("[MS] 开始下载模型：{}", model_id));

    let save_dir = std::path::PathBuf::from(save_path);
    if let Err(e) = std::fs::create_dir_all(&save_dir) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建目录失败：{}", e));
    }

    let incomplete_marker = save_dir.join(".incomplete");
    if let Err(e) = std::fs::File::create(&incomplete_marker) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建未完成标记失败：{}", e));
    }

    let files = get_ms_files_with_size(model_id)?;
    let files_to_download = filter_files_with_size(&files, &allow_patterns);

    if files_to_download.is_empty() {
        let _ = std::fs::remove_file(&incomplete_marker);
        return Err("没有找到需要下载的文件".to_string());
    }

    let total_size_all: u64 = files_to_download.iter().map(|f| f.size).sum();
    let total_files = files_to_download.len();

    let _ = app_handle.emit("download-log", format!("[MS] 准备下载 {} 个文件，总大小：{}", total_files, format_bytes(total_size_all)));

    let bandwidth = BANDWIDTH_CACHE.load(Ordering::SeqCst);
    let target_speed = (bandwidth as f64 * 1.6) as u64;
    let connections_per_file = if bandwidth > 0 {
        (((bandwidth as f64 * 0.8) / 200_000.0) as usize).max(5).min(MAX_CONNECTIONS_PER_FILE)
    } else {
        5
    };

    let _ = app_handle.emit("download-log", format!("[测速] 带宽: {}，目标速度: {}/s，每文件 {} 连接 (上限{})",
        format_speed(bandwidth), format_speed(target_speed), connections_per_file, MAX_CONNECTIONS_PER_FILE));

    let tracker = Arc::new(GlobalDownloadTracker::new());
    let tracker_clone = tracker.clone();
    let app_handle_progress = app_handle.clone();
    let total_size_for_progress = total_size_all;

    let monitor_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let (speed, _) = tracker_clone.calculate_speed();
            let total_downloaded = tracker_clone.get_total_downloaded();
            let speed_str = format_speed(speed);

            let percent = if total_size_for_progress > 0 {
                (total_downloaded as f64 / total_size_for_progress as f64 * 100.0).min(100.0)
            } else {
                0.0
            };

            let remaining = total_size_for_progress.saturating_sub(total_downloaded);
            let eta_secs = if speed > 0 {
                (remaining as f64 / speed as f64) as u64
            } else {
                0
            };
            let eta_str = format_eta(eta_secs as f64);

            let _ = app_handle_progress.emit("download-progress", serde_json::json!({
                "file_index": 0,
                "total_files": total_files,
                "file_name": "多文件下载",
                "downloaded": total_downloaded,
                "total": total_size_for_progress,
                "speed": speed_str,
                "eta": eta_str,
                "percent": percent
            }));

            let _ = app_handle_progress.emit("download-log", format!(
                "[进度] {:.2} MB / {:.2} MB ({:.2}%) - {} - ETA: {}",
                total_downloaded as f64 / (1024.0 * 1024.0),
                total_size_for_progress as f64 / (1024.0 * 1024.0),
                percent,
                speed_str,
                eta_str
            ));
        }
    });

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_DOWNLOADS));
    let _ = app_handle.emit("download-log", format!("[下载] 同时下载文件数上限: {}", MAX_CONCURRENT_DOWNLOADS));

    let mut tasks = Vec::new();
    for (i, file_info) in files_to_download.into_iter().enumerate() {
        let url = format!("https://modelscope.cn/{}/resolve/master/{}", model_id, file_info.path);
        let target_path = std::path::PathBuf::from(save_path).join(&file_info.path);
        let app_handle_clone = app_handle.clone();
        let tracker_clone = tracker.clone();
        let file_size = file_info.size;
        let conn_count = connections_per_file;
        let sem_clone = semaphore.clone();

        let task = tokio::spawn(async move {
            let _permit = sem_clone.acquire().await.unwrap();
            if let Some(parent) = target_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }

            if file_size > 100 * 1024 * 1024 && conn_count > 1 {
                match download_file_with_segments(&url, &target_path, &app_handle_clone, i, total_files, tracker_clone, file_size, conn_count).await {
                    Ok(_) => {
                        let _ = app_handle_clone.emit("download-log", format!("[完成] {}", file_info.path));
                        1
                    }
                    Err(e) => {
                        let _ = app_handle_clone.emit("download-log", format!("[下载失败] {} - {}", file_info.path, e));
                        0
                    }
                }
            } else {
                match download_single_file_with_tracker(&url, &target_path, &app_handle_clone, i, total_files, tracker_clone, file_size).await {
                    Ok(_) => {
                        let _ = app_handle_clone.emit("download-log", format!("[完成] {}", file_info.path));
                        1
                    }
                    Err(e) => {
                        let _ = app_handle_clone.emit("download-log", format!("[下载失败] {} - {}", file_info.path, e));
                        0
                    }
                }
            }
        });

        tasks.push(task);
    }

    let results = futures::future::join_all(tasks).await;
    let success_count = results.iter().filter(|r| r.as_ref().ok().copied().unwrap_or(0) == 1).count();
    let fail_count = total_files - success_count;

    monitor_task.abort();

    let (speed, _) = tracker.calculate_speed();
    let total_downloaded = tracker.get_total_downloaded();
    let _ = app_handle.emit("download-log", format!("[完成] 下载速度: {}", format_speed(speed)));
    let _ = app_handle.emit("download-log", format!("[完成] 总下载量: {:.2} MB", total_downloaded as f64 / (1024.0 * 1024.0)));

    if fail_count == 0 {
        let _ = std::fs::remove_file(&incomplete_marker);
        let _ = app_handle.emit("download-log", format!("[完成] 模型下载完成，移除未完成标记"));
    }

    if fail_count > 0 {
        Ok(format!("下载完成：成功 {} 个，失败 {} 个", success_count, fail_count))
    } else {
        Ok(format!("下载完成：{} 个文件", success_count))
    }
}

#[tauri::command]
pub fn stop_download(state: State<'_, AppState>) -> String {
    if let Ok(mut child_guard) = state.download_child.lock() {
        if let Some(ref mut child) = *child_guard {
            let _ = child.kill();
            let _ = child.wait();
            *child_guard = None;
            return "下载已停止".to_string();
        }
    }
    "没有正在运行的下载".to_string()
}

#[tauri::command]
pub fn stop_download_with_cleanup(save_root: String, state: State<'_, AppState>) -> String {
    if let Ok(mut child_guard) = state.download_child.lock() {
        if let Some(ref mut child) = *child_guard {
            let _ = child.kill();
            let _ = child.wait();
            *child_guard = None;
        }
    }

    let root_path = std::path::PathBuf::from(&save_root);
    if !root_path.exists() {
        return "下载已停止，目录不存在".to_string();
    }

    let mut deleted_count = 0;
    let mut deleted_dirs: Vec<String> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&root_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let incomplete_marker = path.join(".incomplete");
                if incomplete_marker.exists() {
                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        let _ = std::io::stdout().write_all(format!("[清理] 删除失败 {}: {}\n", path.display(), e).as_bytes());
                    } else {
                        deleted_count += 1;
                        deleted_dirs.push(path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default());
                    }
                }
            }
        }
    }

    if deleted_count > 0 {
        format!("下载已停止，已清理 {} 个未完成的模型文件夹: {}", deleted_count, deleted_dirs.join(", "))
    } else {
        "下载已停止，未发现未完成的模型文件夹".to_string()
    }
}

#[tauri::command]
pub async fn start_download(
    app: tauri::AppHandle,
    urls: String,
    save_root: String,
    _gguf_quant: String,
    auto_shutdown: bool,
) -> Result<String, String> {
    let _ = app.emit("download-log", format!("[启动] 开始处理下载任务..."));

    let url_lines: Vec<&str> = urls.lines().collect();
    let total_tasks = url_lines.len();
    let _ = app.emit("download-log", format!("[启动] 共 {} 个任务", total_tasks));

    let mut success_count = 0;
    let mut fail_count = 0;

    for (i, line) in url_lines.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let _ = app.emit("download-log", format!("[任务] {}/{} 处理中...", i + 1, total_tasks));

        let (url, quant, mode) = if let Some(pos) = line.find("::QUANT:") {
            let url = line[..pos].to_string();
            let quant = line[pos + 8..].to_string();
            (url, Some(quant), None)
        } else if let Some(pos) = line.find("::MODE:") {
            let url = line[..pos].to_string();
            let mode = line[pos + 7..].to_string();
            (url, None, Some(mode))
        } else {
            (line.to_string(), None, Some("all".to_string()))
        };

        let model_name = extract_model_name(&url);
        let task_save_path = std::path::PathBuf::from(&save_root).join(&model_name);

        let result = download_model_rust(
            &url,
            task_save_path.to_str().unwrap_or(&save_root),
            quant.clone(),
            mode.as_deref().unwrap_or("all"),
            app.clone(),
        ).await;

        match result {
            Ok(msg) => {
                let _ = app.emit("download-log", format!("[完成] {} - {}", model_name, msg));
                success_count += 1;
            }
            Err(e) => {
                let _ = app.emit("download-log", format!("[失败] {} - {}", model_name, e));
                fail_count += 1;
            }
        }
    }

    let _ = app.emit("download-log", format!("[全部完成] 成功 {} 个，失败 {} 个", success_count, fail_count));
    let _ = app.emit("download-finished", "");

    if auto_shutdown {
        let _ = app.emit("download-log", "[关机] 下载完成，准备关机...");
    }

    Ok(format!("下载完成：成功 {} 个，失败 {} 个", success_count, fail_count))
}

pub async fn get_file_size(url: &str) -> Result<u64, String> {
    let client = reqwest::Client::new();
    let response = client.head(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| format!("请求失败：{}", e))?;

    if let Some(content_length) = response.headers().get("content-length") {
        if let Ok(size_str) = content_length.to_str() {
            if let Ok(size) = size_str.parse::<u64>() {
                return Ok(size);
            }
        }
    }
    Ok(0)
}

pub fn filter_files_with_size(files: &[FileInfo], patterns: &Option<Vec<String>>) -> Vec<FileInfo> {
    match patterns {
        Some(patterns) => {
            files.iter()
                .filter(|f| {
                    patterns.iter().any(|p| {
                        let p = p.as_str();
                        if p.ends_with(".gguf") && !p.contains('*') {
                            f.path.as_str() == p
                        } else if p.ends_with(".gguf") {
                            let before_gguf = &p[..p.len() - 5];
                            if let Some(star_pos) = before_gguf.rfind('*') {
                                let after_star = &before_gguf[star_pos + 1..];
                                f.path.contains(after_star) && f.path.ends_with(".gguf")
                            } else {
                                f.path.contains(before_gguf) && f.path.ends_with(".gguf")
                            }
                        } else if p.starts_with('*') && p.ends_with('*') {
                            f.path.contains(&p[1..p.len()-1])
                        } else if p.starts_with('*') {
                            f.path.ends_with(&p[1..])
                        } else if p.ends_with('*') {
                            f.path.starts_with(&p[..p.len()-1])
                        } else {
                            f.path.as_str() == p
                        }
                    })
                })
                .cloned()
                .collect()
        }
        None => files.to_vec()
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn filter_files(files: &[String], patterns: &Option<Vec<String>>) -> Vec<String> {
    match patterns {
        Some(patterns) => {
            files.iter()
                .filter(|f| patterns.iter().any(|p| {
                    if p.starts_with('*') && p.ends_with('*') {
                        f.contains(&p[1..p.len()-1])
                    } else if p.starts_with('*') {
                        f.ends_with(&p[1..])
                    } else if p.ends_with('*') {
                        f.starts_with(&p[..p.len()-1])
                    } else {
                        f.as_str() == p.as_str()
                    }
                }))
                .cloned()
                .collect()
        }
        None => files.to_vec()
    }
}

pub async fn test_download_speed(url: &str) -> Result<f64, String> {
    let client = reqwest::Client::new();
    let response = client.get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Range", "bytes=0-1048575")
        .send()
        .await
        .map_err(|e| format!("请求失败：{}", e))?;

    let start = std::time::Instant::now();
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取失败：{}", e))?;
        downloaded += chunk.len() as u64;
        if downloaded >= 1024 * 1024 {
            break;
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let speed = if elapsed > 0.0 { downloaded as f64 / elapsed } else { 0.0 };
    Ok(speed)
}

pub async fn download_single_file(
    url: &str,
    save_path: &std::path::PathBuf,
    app_handle: &AppHandle,
    file_index: usize,
    total_files: usize,
    total_size_all: u64,
    downloaded_size_all: Arc<AtomicU64>,
    file_size: u64,
) -> Result<(), String> {
    let start_pos = if save_path.exists() {
        let metadata = std::fs::metadata(save_path).map_err(|e| format!("读取文件失败：{}", e))?;
        let len = metadata.len();
        if len >= file_size {
            let _ = app_handle.emit("download-log", format!("[跳过] {} 已存在", save_path.display()));
            downloaded_size_all.fetch_add(file_size, Ordering::SeqCst);
            return Ok(());
        }
        len
    } else {
        0
    };

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(save_path)
        .await
        .map_err(|e| format!("创建文件失败：{}", e))?;

    file.set_len(file_size).await.map_err(|e| format!("设置文件大小失败：{}", e))?;

    let client = reqwest::Client::new();

    let _ = app_handle.emit("download-log", format!("[URL] {}", url));

    let encoded_url = url
        .replace(" ", "%20")
        .replace("[", "%5D")
        .replace("]", "%5D")
        .replace("#", "%23")
        .replace("%", "%25");

    let _ = app_handle.emit("download-log", format!("[编码 URL] {}", encoded_url));

    let mut request_builder = client.get(&encoded_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "*/*")
        .header("Connection", "keep-alive");

    if start_pos > 0 {
        request_builder = request_builder.header("Range", format!("bytes={}-", start_pos));
    }

    let response = request_builder
        .send()
        .await
        .map_err(|e| format!("请求失败：{}", e))?;

    let status = response.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(format!("HTTP 错误：{}", status));
    }

    let mut stream = response.bytes_stream();
    let mut total_bytes = 0usize;
    let mut last_report_time = std::time::Instant::now();
    let mut last_reported_bytes = 0u64;
    let mut speed_history: Vec<u64> = Vec::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取数据失败：{}", e))?;
        let len = chunk.len();
        file.seek(std::io::SeekFrom::Start(start_pos as u64 + total_bytes as u64))
            .await
            .map_err(|e| format!("定位失败：{}", e))?;
        file.write_all(&chunk).await.map_err(|e| format!("写入文件失败：{}", e))?;
        total_bytes += len;

        let now = std::time::Instant::now();
        let elapsed = now.duration_since(last_report_time).as_secs_f64();

        if elapsed >= 1.0 {
            let current_increment = total_bytes as u64;
            let overall_downloaded = downloaded_size_all.load(Ordering::Relaxed) + current_increment;

            let bytes_since_last = (start_pos as u64 + total_bytes as u64).saturating_sub(last_reported_bytes);
            let instant_speed = if elapsed > 0.0 { (bytes_since_last as f64 / elapsed) as u64 } else { 0 };

            speed_history.push(instant_speed);
            if speed_history.len() > 5 {
                speed_history.remove(0);
            }
            let avg_speed = speed_history.iter().sum::<u64>() / speed_history.len() as u64;
            let speed_str = format_speed(avg_speed);

            if total_size_all > 0 {
                let percent = overall_downloaded as f64 / total_size_all as f64 * 100.0;
                let remaining = total_size_all.saturating_sub(overall_downloaded);
                let eta_secs = if avg_speed > 0 {
                    (remaining as f64 / avg_speed as f64) as u64
                } else {
                    0
                };
                let eta_str = format_eta(eta_secs as f64);

                let _ = app_handle.emit("download-progress", serde_json::json!({
                    "file_index": file_index,
                    "total_files": total_files,
                    "file_name": save_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    "downloaded": overall_downloaded,
                    "total": total_size_all,
                    "speed": speed_str,
                    "eta": eta_str,
                    "percent": percent
                }));

                let progress_mb = (start_pos as u64 + total_bytes as u64) as f64 / (1024.0 * 1024.0);
                let _ = app_handle.emit("download-log", format!(
                    "[进度] {}/{} - {:.2} MB ({:.2}%) - {} - ETA: {}",
                    file_index + 1, total_files, progress_mb, percent, speed_str, eta_str
                ));
            } else {
                let _ = app_handle.emit("download-progress", serde_json::json!({
                    "file_index": file_index,
                    "total_files": total_files,
                    "file_name": save_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    "downloaded": overall_downloaded,
                    "total": 0,
                    "speed": speed_str,
                    "eta": "--",
                    "percent": 0.0
                }));

                let progress_mb = (start_pos as u64 + total_bytes as u64) as f64 / (1024.0 * 1024.0);
                let _ = app_handle.emit("download-log", format!(
                    "[进度] {}/{} - {:.2} MB - {}",
                    file_index + 1, total_files, progress_mb, speed_str
                ));
            }

            last_report_time = now;
            last_reported_bytes = start_pos as u64 + total_bytes as u64;
        }
    }

    downloaded_size_all.fetch_add(total_bytes as u64, Ordering::SeqCst);

    Ok(())
}

pub async fn download_single_file_with_tracker(
    url: &str,
    save_path: &std::path::PathBuf,
    app_handle: &AppHandle,
    file_index: usize,
    total_files: usize,
    tracker: Arc<GlobalDownloadTracker>,
    file_size: u64,
) -> Result<(), String> {
    let start_pos = if save_path.exists() && file_size > 0 {
        let metadata = std::fs::metadata(save_path).map_err(|e| format!("读取文件失败：{}", e))?;
        let len = metadata.len();
        if len >= file_size {
            let _ = app_handle.emit("download-log", format!("[跳过] {} 已存在", save_path.display()));
            tracker.add_downloaded(file_size);
            return Ok(());
        }
        len
    } else {
        0
    };

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(save_path)
        .await
        .map_err(|e| format!("创建文件失败：{}", e))?;

    let mut buffered_file = BufWriter::new(file);

    if file_size > 0 {
        buffered_file.get_mut().set_len(file_size).await.map_err(|e| format!("设置文件大小失败：{}", e))?;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(20)
        .tcp_nodelay(true)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let _ = app_handle.emit("download-log", format!("[URL] {}", url));

    let encoded_url = url
        .replace(" ", "%20")
        .replace("[", "%5D")
        .replace("]", "%5D")
        .replace("#", "%23")
        .replace("%", "%25");

    let _ = app_handle.emit("download-log", format!("[编码 URL] {}", encoded_url));

    let mut request_builder = client.get(&encoded_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "*/*")
        .header("Connection", "keep-alive");

    if start_pos > 0 {
        request_builder = request_builder.header("Range", format!("bytes={}-", start_pos));
    }

    let response = request_builder
        .send()
        .await
        .map_err(|e| format!("请求失败：{}", e))?;

    let status = response.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(format!("HTTP 错误：{}", status));
    }

    let mut stream = response.bytes_stream();
    let mut total_bytes = 0usize;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取数据失败：{}", e))?;
        let len = chunk.len();
        buffered_file.seek(std::io::SeekFrom::Start(start_pos as u64 + total_bytes as u64))
            .await
            .map_err(|e| format!("定位失败：{}", e))?;
        buffered_file.write_all(&chunk).await.map_err(|e| format!("写入文件失败：{}", e))?;
        total_bytes += len;

        tracker.add_downloaded(len as u64);
    }

    buffered_file.flush().await.map_err(|e| format!("刷新文件失败：{}", e))?;

    let _ = app_handle.emit("download-log", format!(
        "[文件完成] {}/{} - {}",
        file_index + 1,
        total_files,
        save_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    ));

    Ok(())
}

async fn download_segment_single(
    encoded_url: &str,
    target_path: &std::path::PathBuf,
    start_byte: u64,
    end_byte: u64,
    tracker: &Arc<GlobalDownloadTracker>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .tcp_nodelay(true)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let response = client.get(encoded_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Range", format!("bytes={}-{}", start_byte, end_byte - 1))
        .send()
        .await
        .map_err(|e| format!("分段请求失败：{}", e))?;

    if !response.status().is_success() && response.status().as_u16() != 206 {
        return Err(format!("HTTP 错误：{}", response.status()));
    }

    let mut stream = response.bytes_stream();
    let mut segment_pos = start_byte;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取数据失败：{}", e))?;
        let chunk_len = chunk.len() as u64;

        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(target_path)
            .await
            .map_err(|e| format!("打开文件失败：{}", e))?;

        file.seek(std::io::SeekFrom::Start(segment_pos)).await
            .map_err(|e| format!("定位失败：{}", e))?;
        file.write_all(&chunk).await
            .map_err(|e| format!("写入数据失败：{}", e))?;

        segment_pos += chunk_len;
        tracker.add_downloaded(chunk_len);
    }

    Ok(())
}

pub async fn download_file_with_segments(
    url: &str,
    save_path: &std::path::PathBuf,
    app_handle: &AppHandle,
    file_index: usize,
    total_files: usize,
    tracker: Arc<GlobalDownloadTracker>,
    file_size: u64,
    segment_count: usize,
) -> Result<(), String> {
    let encoded_url = url
        .replace(" ", "%20")
        .replace("[", "%5D")
        .replace("]", "%5D")
        .replace("#", "%23")
        .replace("%", "%25");

    let _ = app_handle.emit("download-log", format!(
        "[分段] {} - 分{}段下载",
        save_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        segment_count
    ));

    if let Some(parent) = save_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(save_path)
        .await
        .map_err(|e| format!("创建文件失败：{}", e))?;

    file.set_len(file_size).await.map_err(|e| format!("设置文件大小失败：{}", e))?;
    drop(file);

    let min_segment_size = 10 * 1024 * 1024;
    let calculated_segment_size = (file_size + segment_count as u64 - 1) / segment_count as u64;
    let segment_size = calculated_segment_size.max(min_segment_size);

    let mut handles = Vec::new();

    for seg_idx in 0..segment_count {
        let start_byte = seg_idx as u64 * segment_size;
        let end_byte = ((seg_idx + 1) as u64 * segment_size).min(file_size);
        let seg_size = end_byte - start_byte;

        if seg_size == 0 {
            continue;
        }

        let target_path = save_path.clone();
        let app_handle_clone = app_handle.clone();
        let tracker_clone = tracker.clone();
        let encoded_url = encoded_url.clone();
        let _file_name = save_path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let handle = tokio::spawn(async move {
            let retry_times = 3;
            let mut retry = 0;

            while retry < retry_times {
                match download_segment_single(&encoded_url, &target_path, start_byte, end_byte, &tracker_clone).await {
                    Ok(_) => {
                        let _ = app_handle_clone.emit("download-log", format!(
                            "[分段完成] {}/{} 分段 {}/{}",
                            file_index + 1, total_files, seg_idx + 1, segment_count
                        ));
                        return Ok::<(), String>(());
                    }
                    Err(e) => {
                        retry += 1;
                        if retry >= retry_times {
                            return Err(format!("分段 {}/{} 下载重试{}次失败：{}", seg_idx + 1, segment_count, retry_times, e));
                        }
                        let backoff = 1 << retry;
                        tokio::time::sleep(Duration::from_secs(backoff)).await;
                    }
                }
            }
            Ok(())
        });

        handles.push(handle);
    }

    for handle in handles {
        if let Err(e) = handle.await {
            return Err(e.to_string());
        }
    }

    Ok(())
}

pub async fn download_file_with_resume(
    url: &str,
    save_path: &std::path::PathBuf,
    app_handle: &AppHandle,
    file_index: usize,
    total_files: usize,
    total_size_all: u64,
    downloaded_size_all: u64,
) -> Result<usize, String> {
    use std::time::Instant;

    let client = reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{}", e))?;

    let mut start_pos = 0u64;
    if save_path.exists() {
        let metadata = tokio::fs::metadata(save_path).await
            .map_err(|e| format!("获取文件元数据失败：{}", e))?;
        start_pos = metadata.len();
    }

    let mut request_builder = client.get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Accept", "*/*");

    if start_pos > 0 {
        request_builder = request_builder.header("Range", format!("bytes={}-", start_pos));
    }

    let response = request_builder.send().await
        .map_err(|e| format!("请求失败：{}", e))?;

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(save_path)
        .await
        .map_err(|e| format!("打开文件失败：{}", e))?;

    let mut stream = response.bytes_stream();
    let mut total_bytes = 0usize;
    let mut last_report_time = Instant::now();
    let mut last_reported_bytes = 0usize;
    let mut speed_history: Vec<f64> = Vec::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取数据失败：{}", e))?;
        let len = chunk.len();
        file.write_all(&chunk).await.map_err(|e| format!("写入文件失败：{}", e))?;
        total_bytes += len;

        let now = Instant::now();
        let elapsed = now.duration_since(last_report_time).as_secs_f64();

        if elapsed >= 0.5 || total_bytes % (512 * 1024) < 32768 {
            let current_file_downloaded = start_pos as usize + total_bytes;
            let progress_mb = current_file_downloaded as f64 / (1024.0 * 1024.0);

            let overall_downloaded = downloaded_size_all + current_file_downloaded as u64;

            let bytes_since_last = (start_pos as usize + total_bytes) - (start_pos as usize + last_reported_bytes);
            let instant_speed = if elapsed > 0.0 { bytes_since_last as f64 / elapsed } else { 0.0 };

            speed_history.push(instant_speed);
            if speed_history.len() > 5 {
                speed_history.remove(0);
            }
            let avg_speed = speed_history.iter().sum::<f64>() / speed_history.len() as f64;
            let speed_str = format_speed(avg_speed as u64);

            if total_size_all > 0 {
                let percent = overall_downloaded as f64 / total_size_all as f64 * 100.0;
                let remaining_bytes = total_size_all - overall_downloaded;
                let eta_secs = if avg_speed > 0.0 { remaining_bytes as f64 / avg_speed } else { 0.0 };
                let eta_str = format_eta(eta_secs);

                let _ = app_handle.emit("download-progress", serde_json::json!({
                    "file_index": file_index,
                    "total_files": total_files,
                    "file_name": save_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    "downloaded": overall_downloaded,
                    "total": total_size_all,
                    "speed": speed_str,
                    "eta": eta_str,
                    "percent": percent
                }));

                let _ = app_handle.emit("download-log", format!(
                    "[进度] {}/{} - {:.2} MB ({:.2}%) - {} - ETA: {}",
                    file_index + 1, total_files, progress_mb, percent, speed_str, eta_str
                ));
            } else {
                let _ = app_handle.emit("download-progress", serde_json::json!({
                    "file_index": file_index,
                    "total_files": total_files,
                    "file_name": save_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    "downloaded": overall_downloaded,
                    "total": 0,
                    "speed": speed_str,
                    "eta": "--",
                    "percent": 0.0
                }));

                let _ = app_handle.emit("download-log", format!(
                    "[进度] {}/{} - {:.2} MB - {}",
                    file_index + 1, total_files, progress_mb, speed_str
                ));
            }

            last_report_time = now;
            last_reported_bytes = start_pos as usize + total_bytes;
        }
    }

    file.flush().await.map_err(|e| format!("刷新文件失败：{}", e))?;
    Ok(total_bytes)
}

pub fn format_eta(secs: f64) -> String {
    if secs.is_infinite() || secs > 86400.0 * 365.0 {
        return "--".to_string();
    }

    if secs < 60.0 {
        format!("{:.0}s", secs)
    } else if secs < 3600.0 {
        format!("{:.0}m {:.0}s", secs / 60.0, secs % 60.0)
    } else if secs < 86400.0 {
        format!("{:.0}h {:.0}m", secs / 3600.0, (secs % 3600.0) / 60.0)
    } else {
        format!("{:.0}d {:.0}h", secs / 86400.0, (secs % 86400.0) / 3600.0)
    }
}

pub async fn download_files_from_urls(
    base_url: &str,
    files: &[FileInfo],
    save_path: &str,
    app_handle: AppHandle,
    incomplete_marker: std::path::PathBuf,
) -> Result<String, String> {
    let files_clone = files.to_vec();
    let total_files = files.len();
    let total_size_all: u64 = files.iter().map(|f| f.size).sum();

    let _ = app_handle.emit("download-log", format!("[下载] 准备下载 {} 个文件，总大小：{}", total_files, format_bytes(total_size_all)));

    let bandwidth = BANDWIDTH_CACHE.load(Ordering::SeqCst);
    let connections_per_file = if bandwidth > 0 {
        (((bandwidth as f64 * 0.8) / 200_000.0) as usize).max(5).min(MAX_CONNECTIONS_PER_FILE)
    } else {
        5
    };

    let _ = app_handle.emit("download-log", format!("[测速] 带宽: {}，目标速度: {}/s，每文件 {} 连接 (上限{})",
        format_speed(bandwidth), format_speed((bandwidth as f64 * 1.6) as u64), connections_per_file, MAX_CONNECTIONS_PER_FILE));

    let tracker = Arc::new(GlobalDownloadTracker::new());
    let tracker_clone = tracker.clone();
    let app_handle_progress = app_handle.clone();

    let monitor_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let (speed, _) = tracker_clone.calculate_speed();
            let total_downloaded = tracker_clone.get_total_downloaded();
            let speed_str = format_speed(speed);

            let percent = if total_size_all > 0 {
                (total_downloaded as f64 / total_size_all as f64 * 100.0).min(100.0)
            } else {
                0.0
            };

            let remaining = total_size_all.saturating_sub(total_downloaded);
            let eta_secs = if speed > 0 {
                (remaining as f64 / speed as f64) as u64
            } else {
                0
            };
            let eta_str = format_eta(eta_secs as f64);

            let _ = app_handle_progress.emit("download-progress", serde_json::json!({
                "file_index": 0,
                "total_files": total_files,
                "file_name": "多文件下载",
                "downloaded": total_downloaded,
                "total": total_size_all,
                "speed": speed_str,
                "eta": eta_str,
                "percent": percent
            }));

            let _ = app_handle_progress.emit("download-log", format!(
                "[进度] {:.2} MB / {:.2} MB ({:.2}%) - {} - ETA: {}",
                total_downloaded as f64 / (1024.0 * 1024.0),
                total_size_all as f64 / (1024.0 * 1024.0),
                percent,
                speed_str,
                eta_str
            ));
        }
    });

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_DOWNLOADS));
    let _ = app_handle.emit("download-log", format!("[下载] 同时下载文件数上限: {}", MAX_CONCURRENT_DOWNLOADS));

    let mut tasks = Vec::new();
    for (i, file_info) in files_clone.into_iter().enumerate() {
        let url = format!("{}/{}", base_url, file_info.path);
        let target_path = std::path::PathBuf::from(save_path).join(&file_info.path);
        let app_handle_clone = app_handle.clone();
        let tracker_clone = tracker.clone();
        let file_size = file_info.size;
        let conn_count = connections_per_file;
        let sem_clone = semaphore.clone();

        let task = tokio::spawn(async move {
            let _permit = sem_clone.acquire().await.unwrap();
            if let Some(parent) = target_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }

            if file_size > 100 * 1024 * 1024 && conn_count > 1 {
                match download_file_with_segments(&url, &target_path, &app_handle_clone, i, total_files, tracker_clone, file_size, conn_count).await {
                    Ok(_) => {
                        let _ = app_handle_clone.emit("download-log", format!("[完成] {}", file_info.path));
                        1
                    }
                    Err(e) => {
                        let _ = app_handle_clone.emit("download-log", format!("[失败] {} - {}", file_info.path, e));
                        0
                    }
                }
            } else {
                match download_single_file_with_tracker(&url, &target_path, &app_handle_clone, i, total_files, tracker_clone, file_size).await {
                    Ok(_) => {
                        let _ = app_handle_clone.emit("download-log", format!("[完成] {}", file_info.path));
                        1
                    }
                    Err(e) => {
                        let _ = app_handle_clone.emit("download-log", format!("[失败] {} - {}", file_info.path, e));
                        0
                    }
                }
            }
        });

        tasks.push(task);
    }

    let results = futures::future::join_all(tasks).await;
    let success_count = results.iter().filter(|r| r.as_ref().ok().copied().unwrap_or(0) == 1).count();
    let fail_count = total_files - success_count;

    monitor_task.abort();

    let (speed, _) = tracker.calculate_speed();
    let total_downloaded = tracker.get_total_downloaded();
    let _ = app_handle.emit("download-log", format!("[完成] 下载速度: {}", format_speed(speed)));
    let _ = app_handle.emit("download-log", format!("[完成] 总下载量: {:.2} MB", total_downloaded as f64 / (1024.0 * 1024.0)));

    if fail_count == 0 {
        let _ = std::fs::remove_file(&incomplete_marker);
        let _ = app_handle.emit("download-log", format!("[完成] 模型下载完成，移除未完成标记"));
    }

    if fail_count > 0 {
        Ok(format!("下载完成：成功 {} 个，失败 {} 个", success_count, fail_count))
    } else {
        Ok(format!("下载完成：{} 个文件", success_count))
    }
}
