use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use futures::StreamExt;
use tauri::{AppHandle, Emitter};

/// 下载进度信息
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub file_index: usize,
    pub total_files: usize,
    pub file_name: String,
    pub downloaded: u64,
    pub total: u64,
    pub speed: String,
    pub eta: String,
    pub percent: f64,
}

/// 进度计算器 - 负责计算下载进度和速度
pub struct ProgressCalculator {
    total_size: u64,
    downloaded_size: Arc<AtomicU64>,
    start_time: Instant,
    last_report_time: Instant,
    last_reported_bytes: u64,
    speed_history: Vec<f64>,
}

impl ProgressCalculator {
    pub fn new(total_size: u64, downloaded_size: Arc<AtomicU64>) -> Self {
        let now = Instant::now();
        Self {
            total_size,
            downloaded_size,
            start_time: now,
            last_report_time: now,
            last_reported_bytes: 0,
            speed_history: Vec::new(),
        }
    }

    /// 更新进度并返回进度信息
    pub fn update(&mut self, current_file_downloaded: u64, file_index: usize, total_files: usize, file_name: &str) -> DownloadProgress {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_report_time).as_secs_f64();
        
        // 计算速度（使用滑动平均）
        let bytes_since_last = current_file_downloaded.saturating_sub(self.last_reported_bytes);
        let instant_speed = if elapsed > 0.0 { bytes_since_last as f64 / elapsed } else { 0.0 };
        
        self.speed_history.push(instant_speed);
        if self.speed_history.len() > 5 {
            self.speed_history.remove(0);
        }
        let avg_speed = self.speed_history.iter().sum::<f64>() / self.speed_history.len() as f64;
        
        // 计算整体进度
        let overall_downloaded = self.downloaded_size.load(Ordering::SeqCst);
        let percent = if self.total_size > 0 {
            (overall_downloaded as f64 / self.total_size as f64 * 100.0).min(100.0)
        } else {
            0.0
        };
        
        // 计算 ETA
        let remaining_bytes = self.total_size.saturating_sub(overall_downloaded);
        let eta_secs = if avg_speed > 0.0 { remaining_bytes as f64 / avg_speed } else { 0.0 };
        let eta_str = Self::format_eta(eta_secs);
        
        self.last_report_time = now;
        self.last_reported_bytes = current_file_downloaded;
        
        DownloadProgress {
            file_index,
            total_files,
            file_name: file_name.to_string(),
            downloaded: overall_downloaded,
            total: self.total_size,
            speed: Self::format_speed(avg_speed as u64),
            eta: eta_str,
            percent,
        }
    }

    fn format_speed(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * 1024;
        const GB: u64 = 1024 * 1024 * 1024;
        
        if bytes >= GB {
            format!("{:.2} GB/s", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.2} MB/s", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.2} KB/s", bytes as f64 / KB as f64)
        } else {
            format!("{} B/s", bytes)
        }
    }

    fn format_eta(secs: f64) -> String {
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
}

/// 文件下载器 - 负责下载单个文件
pub struct FileDownloader {
    app_handle: AppHandle,
    progress_calculator: ProgressCalculator,
    file_index: usize,
    total_files: usize,
}

impl FileDownloader {
    pub fn new(
        app_handle: AppHandle,
        total_size: u64,
        downloaded_size: Arc<AtomicU64>,
        file_index: usize,
        total_files: usize,
    ) -> Self {
        let progress_calculator = ProgressCalculator::new(total_size, downloaded_size);
        Self {
            app_handle,
            progress_calculator,
            file_index,
            total_files,
        }
    }

    /// 下载文件（支持断点续传）
    pub async fn download(
        &mut self,
        url: &str,
        save_path: &std::path::PathBuf,
        file_size: u64,
    ) -> Result<u64, String> {
        use tokio::io::AsyncSeekExt;
        
        // 检查文件是否已存在
        let start_pos = if save_path.exists() {
            let metadata = std::fs::metadata(save_path)
                .map_err(|e| format!("读取文件失败：{}", e))?;
            let len = metadata.len();
            if len >= file_size {
                let _ = self.app_handle.emit("download-log", format!("[跳过] {} 已存在", save_path.display()));
                return Ok(len);
            }
            len
        } else {
            0
        };

        // 创建或打开文件
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(save_path)
            .await
            .map_err(|e| format!("创建文件失败：{}", e))?;

        // 移动到文件末尾
        if start_pos > 0 {
            file.seek(tokio::io::SeekFrom::End(0)).await
                .map_err(|e| format!("定位文件失败：{}", e))?;
        }

        // 构建带 Range 头的请求
        let client = reqwest::Client::new();
        let mut request = client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept", "*/*")
            .header("Connection", "keep-alive");
        
        if start_pos > 0 {
            request = request.header("Range", format!("bytes={}-", start_pos));
        }

        // 发送请求（带重试）
        let max_retries = 3;
        let mut retry_count = 0;
        let mut response = None;
        
        while retry_count < max_retries {
            match request.try_clone().unwrap().send().await {
                Ok(resp) => {
                    response = Some(resp);
                    break;
                }
                Err(e) => {
                    retry_count += 1;
                    if retry_count < max_retries {
                        let wait_time = std::time::Duration::from_secs(2u64.pow(retry_count));
                        let _ = self.app_handle.emit("download-log", format!("[重试] 第 {} 次重试，等待 {} 秒", retry_count, wait_time.as_secs()));
                        tokio::time::sleep(wait_time).await;
                    } else {
                        return Err(format!("请求失败（重试 {} 次）: {}", max_retries, e));
                    }
                }
            }
        }

        let response = response.ok_or("请求失败")?;
        let status = response.status();
        if !status.is_success() && status.as_u16() != 206 {
            return Err(format!("HTTP 错误：{}", status));
        }

        // 下载数据
        let mut stream = response.bytes_stream();
        let mut total_bytes = 0usize;
        
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("读取数据失败：{}", e))?;
            let len = chunk.len();
            file.write_all(&chunk).await.map_err(|e| format!("写入文件失败：{}", e))?;
            total_bytes += len;
            
            // 更新进度
            let current_downloaded = start_pos as u64 + total_bytes as u64;
            let file_name = save_path.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            
            let progress = self.progress_calculator.update(
                current_downloaded,
                self.file_index,
                self.total_files,
                &file_name,
            );
            
            // 发送进度事件
            let _ = self.app_handle.emit("download-progress", serde_json::json!({
                "file_index": progress.file_index,
                "total_files": progress.total_files,
                "file_name": progress.file_name,
                "downloaded": progress.downloaded,
                "total": progress.total,
                "speed": progress.speed,
                "eta": progress.eta,
                "percent": progress.percent,
            }));
        }

        file.flush().await.map_err(|e| format!("刷新文件失败：{}", e))?;
        Ok(start_pos + total_bytes as u64)
    }
}
