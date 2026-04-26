use std::process::{Command, Child};
use std::sync::Mutex;
use std::io::Write;
use std::fs::OpenOptions;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder};
use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;

mod download_manager;
mod api_service;

use download_manager::FileDownloader;
use api_service::{ModelApiService, HuggingFaceService, ModelScopeService, FileFilter};

const CREATE_NO_WINDOW: u32 = 0x08000000;
const WEIBO_LOGIN_URL: &str = "https://vlogger.org.cn/wechatLogin";
const API_BASE_URL: &str = "https://fc-mp-67853879-23c3-42bc-907b-6042038c9906.next.bspapp.com/http/router";
const STORE_PATH: &str = "user_store.json";
const USER_INFO_KEY: &str = "user_info";
const LOG_FILE: &str = "app_debug.log";

fn get_log_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    exe_dir.join(LOG_FILE)
}

fn write_log(msg: &str) {
    let log_path = get_log_path();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let timestamp = chrono_lite_timestamp();
        let _ = writeln!(file, "[{}] {}", timestamp, msg);
    }
}

fn chrono_lite_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let secs = now.as_secs();
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DownloadTask {
    url: String,
    quant: Option<String>,
    mode: Option<String>,
    total_size: u64,
}

struct AppState {
    download_child: Mutex<Option<Child>>,
    current_save_path: Mutex<Option<String>>,
    login_window_open: Mutex<bool>,
}

fn get_tasks_file_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    exe_dir.join("download_tasks.json")
}

#[tauri::command]
fn load_tasks() -> Result<Vec<DownloadTask>, String> {
    let path = get_tasks_file_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取任务文件失败：{}", e))?;
    let tasks: Vec<DownloadTask> = serde_json::from_str(&content)
        .unwrap_or_default();
    Ok(tasks)
}

#[tauri::command]
fn save_tasks(tasks: Vec<DownloadTask>) -> Result<(), String> {
    let path = get_tasks_file_path();
    let content = serde_json::to_string_pretty(&tasks)
        .map_err(|e| format!("序列化失败：{}", e))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("写入任务文件失败：{}", e))?;
    Ok(())
}

#[tauri::command]
fn clear_tasks() -> Result<(), String> {
    let path = get_tasks_file_path();
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("删除任务文件失败：{}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn check_login_status(app: AppHandle) -> Result<bool, String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    let user_info = store.get(USER_INFO_KEY);
    Ok(user_info.is_some())
}

#[tauri::command]
async fn get_user_info(app: AppHandle) -> Result<Option<serde_json::Value>, String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    let user_info = store.get(USER_INFO_KEY);
    Ok(user_info.map(|v| v.clone()))
}

#[tauri::command]
async fn open_wechat_login_window(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    {
        let mut is_open = state.login_window_open.lock().map_err(|e| e.to_string())?;
        if *is_open {
            return Ok(());
        }
        *is_open = true;
    }

    let _app_handle = app.clone();
    let _login_window = WebviewWindowBuilder::new(
        &app,
        "wechat-login",
        WebviewUrl::External(WEIBO_LOGIN_URL.parse().unwrap())
    )
    .title("微信登录 - 贝仓创业研习社")
    .inner_size(480.0, 680.0)
    .center()
    .resizable(false)
    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 BEIDesktopApp/1.0")
    .build()
    .map_err(|e| e.to_string())?;

    let app_handle2 = app.clone();
    let app_handle3 = app.clone();
    let app_handle4 = app.clone();
    if let Some(window) = app.get_webview_window("wechat-login") {
        write_log("[WechatLogin] 开始监听登录成功事件");
        
        window.listen("wechat-login-success", move |event| {
            write_log(&format!("[WechatLogin] 收到登录成功事件！payload: {:?}", event.payload()));
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                write_log(&format!("[WechatLogin] 解析 payload 成功：{:?}", payload));
                if let Some(user_info) = payload.get("userInfo") {
                    write_log(&format!("[WechatLogin] 获取到 userInfo: {:?}", user_info));
                    if let Ok(store) = app_handle2.store(STORE_PATH) {
                        store.set(USER_INFO_KEY, user_info.clone());
                        if let Err(e) = store.save() {
                            write_log(&format!("[WechatLogin] 保存用户信息失败：{}", e));
                        } else {
                            write_log("[WechatLogin] 用户信息保存成功");
                        }
                    }
                    let _ = app_handle2.emit("login-success", user_info.clone());
                    write_log("[WechatLogin] 已发送 login-success 到前端");
                }
            }
        });

        window.listen("close-webview", move |_| {
            write_log("[WechatLogin] 收到 close-webview 事件");
            if let Some(w) = app_handle3.get_webview_window("wechat-login") {
                let _ = w.close();
            }
        });

        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                write_log("[WechatLogin] 用户请求关闭登录窗口");
                if let Ok(mut is_open) = app_handle4.state::<AppState>().login_window_open.lock() {
                    *is_open = false;
                }
            }
        });
    }

    Ok(())
}

#[tauri::command]
async fn logout(app: AppHandle) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    store.delete(USER_INFO_KEY);
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn notify_login_success(app: AppHandle, user_info: serde_json::Value) -> Result<(), String> {
    write_log(&format!("[Notify] notify_login_success 被调用，user_info: {:?}", user_info));
    let _ = app.emit("login-success", user_info.clone());
    if let Some(window) = app.get_webview_window("wechat-login") {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
async fn close_login_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("wechat-login") {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
async fn create_wechat_qrcode() -> Result<serde_json::Value, String> {
    write_log("[WechatAPI] 调用生成二维码接口");
    let url = API_BASE_URL.to_string();
    let body = serde_json::json!({
        "$url": "client/apiForRes/user/pub/weixinCreateQRCode",
        "data": {}
    });

    let response = ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("vk-platform", "h5")
        .set("Unicloud-S2s-Authorization", "CONNECTCODE s2uqpb0h958vhhom0hi1ug5bt88r29bcg")
        .send_json(body);

    match response {
        Ok(resp) => {
            let text = resp.into_string().map_err(|e| e.to_string())?;
            let result: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if result["code"] == 0 {
                Ok(serde_json::json!({
                    "success": true,
                    "url": result["url"],
                    "ticket": result["ticket"],
                    "scene_str": result["scene_str"],
                    "expire_seconds": result["expire_seconds"]
                }))
            } else {
                Err(result["msg"].as_str().unwrap_or("未知错误").to_string())
            }
        }
        Err(e) => Err(format!("请求失败：{}", e))
    }
}

#[tauri::command]
async fn check_wechat_login(scene_str: String) -> Result<serde_json::Value, String> {
    write_log(&format!("[WechatAPI] 查询登录状态，scene_str: {}", scene_str));
    let url = API_BASE_URL.to_string();
    let body = serde_json::json!({
        "$url": "client/apiForRes/user/pub/weixinCheckLogin",
        "data": { "scene_str": scene_str }
    });

    let response = ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("vk-platform", "h5")
        .set("Unicloud-S2s-Authorization", "CONNECTCODE s2uqpb0h958vhhom0hi1ug5bt88r29bcg")
        .send_json(body);

    match response {
        Ok(resp) => {
            let text = resp.into_string().map_err(|e| e.to_string())?;
            let result: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "code": result["code"],
                "msg": result["msg"],
                "token": result["token"],
                "userInfo": result["userInfo"]
            }))
        }
        Err(e) => Err(format!("查询请求失败：{}", e))
    }
}

#[tauri::command]
async fn save_user_info(app: AppHandle, user_info: serde_json::Value) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    store.set(USER_INFO_KEY, user_info);
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn start_download(
    app: AppHandle,
    urls: String,
    save_root: String,
    gguf_quant: String,
    auto_shutdown: bool,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    {
        let mut dl = state.download_child.lock().map_err(|e| e.to_string())?;
        if let Some(ref mut c) = *dl {
            let _ = c.kill();
            let _ = c.wait();
            *dl = None;
        }
    }

    {
        let mut path_guard = state.current_save_path.lock().map_err(|e| e.to_string())?;
        *path_guard = Some(save_root.clone());
    }

    let app_handle = app.clone();
    let urls_clone = urls.clone();
    let save_root_clone = save_root.clone();
    let gguf_quant_clone = gguf_quant.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let app_handle_after = app_handle.clone();
        let app_handle_shutdown = app_handle.clone();
        let result = rt.block_on(async move {
            let url_list: Vec<&str> = urls_clone.split('\n').filter(|s| !s.is_empty()).collect();
            
            for (i, url) in url_list.iter().enumerate() {
                let _ = app_handle.emit("download-log", format!("-------- 第 {} 个任务 --------", i));
                let url_str = url.to_string();
                let model_name = extract_model_name(&url_str);
                let model_save_path = std::path::PathBuf::from(&save_root_clone).join(&model_name);
                
                tokio::fs::create_dir_all(&model_save_path).await
                    .map_err(|e| format!("创建模型文件夹失败：{}", e))?;
                
                let _ = app_handle.emit("download-log", format!("📁 模型文件夹：{}", model_save_path.display()));
                
                let gguf = if gguf_quant_clone.is_empty() { None } else { Some(gguf_quant_clone.clone()) };
                let mode = if gguf.is_some() { "quant" } else { "main" };

                let result = download_model_rust(&url_str, model_save_path.to_str().unwrap(), gguf, mode, app_handle.clone()).await;
                
                match result {
                    Ok(msg) => {
                        let _ = app_handle.emit("download-log", format!("✅ {}", msg));
                    }
                    Err(e) => {
                        let _ = app_handle.emit("download-log", format!("❌ 下载失败：{}", e));
                    }
                }
            }
            Ok::<(), String>(())
        });

        let _ = app_handle_after.emit("download-finished", "");
        
        if auto_shutdown && result.is_ok() {
            let _ = app_handle_shutdown.emit("download-log", "🌙 下载完成，准备关机...".to_string());
            std::thread::sleep(std::time::Duration::from_secs(2));
            #[cfg(target_os = "windows")]
            { std::process::Command::new("shutdown").args(["/s", "/t", "0"]).spawn().ok(); }
        }
    });

    Ok("下载已启动".into())
}

#[tauri::command]
fn stop_download(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let mut dl = state.download_child.lock().map_err(|e| e.to_string())?;
    if let Some(ref mut c) = *dl {
        let _ = c.kill();
        let _ = c.try_wait();
        *dl = None;
    }
    Ok("已停止".into())
}

#[tauri::command]
fn stop_download_with_cleanup(save_root: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    {
        let mut dl = state.download_child.lock().map_err(|e| e.to_string())?;
        if let Some(ref mut c) = *dl {
            let _ = c.kill();
            let _ = c.try_wait();
            *dl = None;
        }
    }

    let save_root_clone = save_root.clone();
    std::thread::spawn(move || {
        let mut cleaned_count = 0;
        let mut failed_count = 0;
        
        if let Ok(entries) = std::fs::read_dir(&save_root_clone) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        let has_files = sub_entries.flatten().next().is_some();
                        if !has_files {
                            if let Err(_) = std::fs::remove_dir(&path) {
                                failed_count += 1;
                            } else {
                                cleaned_count += 1;
                            }
                        } else {
                            let incomplete_marker = path.join(".incomplete");
                            if incomplete_marker.exists() {
                                if let Err(_) = std::fs::remove_dir_all(&path) {
                                    failed_count += 1;
                                } else {
                                    cleaned_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        eprintln!("清理完成：删除 {} 个目录，失败 {} 个", cleaned_count, failed_count);
    });

    Ok("已停止并清理未完成的下载".into())
}

#[tauri::command]
fn shutdown_system() {
    #[cfg(target_os = "windows")]
    { Command::new("shutdown").args(["/s", "/t", "60"]).spawn().unwrap(); }
}

#[tauri::command]
fn cancel_shutdown() {
    #[cfg(target_os = "windows")]
    { Command::new("shutdown").args(["/a"]).spawn().unwrap(); }
}

#[tauri::command]
fn get_model_files(url: String) -> Result<api_service::ModelFilesResult, String> {
    let url_lower = url.to_lowercase();
    let is_gguf = url_lower.contains("gguf");
    let model_id = extract_model_id(&url).ok_or("无法解析模型 ID")?;

    let result = if url.contains("huggingface.co") {
        let hf_service = HuggingFaceService::new();
        hf_service.get_model_files(&model_id)?
    } else if url.contains("modelscope.cn") {
        let ms_service = ModelScopeService::new();
        ms_service.get_model_files(&model_id)?
    } else {
        return Err("不支持的链接".into());
    };

    Ok(api_service::ModelFilesResult {
        is_gguf,
        files: result.files,
        total_size: result.total_size,
    })
}

async fn download_model_rust(
    url: &str,
    save_path: &str,
    gguf_quant: Option<String>,
    mode: &str,
    app_handle: AppHandle,
) -> Result<String, String> {
    let model_id = extract_model_id(url).ok_or("无法解析模型 ID")?;

    let allow_patterns = if let Some(quant) = gguf_quant {
        Some(FileFilter::gguf_pattern(&quant))
    } else if mode == "main" {
        Some(FileFilter::main_files_pattern())
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

async fn huggingface_download(
    model_id: &str,
    save_path: &str,
    allow_patterns: Option<Vec<String>>,
    app_handle: AppHandle,
) -> Result<String, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    
    let _ = app_handle.emit("download-log", format!("[HF] 开始下载模型：{}", model_id));
    
    let incomplete_marker = std::path::PathBuf::from(save_path).join(".incomplete");
    if let Err(e) = std::fs::File::create(&incomplete_marker) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建未完成标记失败：{}", e));
    }
    
    let hf_service = HuggingFaceService::new();
    let result = hf_service.get_model_files(model_id)?;
    let files_to_download = FileFilter::filter_files(&result.files, &allow_patterns);
    
    if files_to_download.is_empty() {
        let _ = std::fs::remove_file(&incomplete_marker);
        return Err("没有找到需要下载的文件".to_string());
    }

    let total_size = result.total_size;
    let total_files = files_to_download.len();
    
    let _ = app_handle.emit("download-log", format!("[HF] 准备下载 {} 个文件，总大小：{}", total_files, format_bytes(total_size)));
    
    let base_url = hf_service.build_resolve_url(model_id, "main");
    
    let mut success_count = 0;
    let mut fail_count = 0;
    let downloaded_size_all = Arc::new(AtomicU64::new(0));
    
    let mut tasks = Vec::new();
    for (i, file_info) in files_to_download.into_iter().enumerate() {
        let url = format!("{}/{}", base_url, file_info.path);
        let target_path = std::path::PathBuf::from(save_path).join(&file_info.path);
        let app_handle_clone = app_handle.clone();
        let downloaded_clone = downloaded_size_all.clone();
        let file_size = file_info.size;
        
        let task = tokio::spawn(async move {
            if let Some(parent) = target_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            
            let mut downloader = FileDownloader::new(
                app_handle_clone.clone(),
                total_size,
                downloaded_clone.clone(),
                i,
                total_files,
            );
            
            match downloader.download(&url, &target_path, file_size).await {
                Ok(bytes) => {
                    downloaded_clone.fetch_add(bytes, Ordering::SeqCst);
                    let _ = app_handle_clone.emit("download-log", format!("[完成] {}", target_path.display()));
                    1
                }
                Err(e) => {
                    let _ = app_handle_clone.emit("download-log", format!("[下载失败] {} - {}", target_path.display(), e));
                    0
                }
            }
        });
        
        tasks.push(task);
    }
    
    for task in tasks {
        if let Ok(result) = task.await {
            if result == 1 {
                success_count += 1;
            } else {
                fail_count += 1;
            }
        }
    }
    
    let _ = std::fs::remove_file(&incomplete_marker);
    
    if fail_count > 0 {
        Ok(format!("下载完成：成功 {} 个，失败 {} 个", success_count, fail_count))
    } else {
        Ok(format!("下载完成：{} 个文件", success_count))
    }
}

async fn modelscope_download(
    model_id: &str,
    save_path: &str,
    allow_patterns: Option<Vec<String>>,
    app_handle: AppHandle,
) -> Result<String, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    
    let _ = app_handle.emit("download-log", format!("[MS] 开始下载模型：{}", model_id));
    
    let incomplete_marker = std::path::PathBuf::from(save_path).join(".incomplete");
    if let Err(e) = std::fs::File::create(&incomplete_marker) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建未完成标记失败：{}", e));
    }
    
    let ms_service = ModelScopeService::new();
    let result = ms_service.get_model_files(model_id)?;
    let files_to_download = FileFilter::filter_files(&result.files, &allow_patterns);
    
    if files_to_download.is_empty() {
        let _ = std::fs::remove_file(&incomplete_marker);
        return Err("没有找到需要下载的文件".to_string());
    }

    let total_size = result.total_size;
    let total_files = files_to_download.len();
    
    let _ = app_handle.emit("download-log", format!("[MS] 准备下载 {} 个文件，总大小：{}", total_files, format_bytes(total_size)));
    
    let mut success_count = 0;
    let mut fail_count = 0;
    let downloaded_size_all = Arc::new(AtomicU64::new(0));
    
    let mut tasks = Vec::new();
    for (i, file_info) in files_to_download.into_iter().enumerate() {
        let url = format!("https://modelscope.cn/{}/resolve/master/{}", model_id, file_info.path);
        let target_path = std::path::PathBuf::from(save_path).join(&file_info.path);
        let app_handle_clone = app_handle.clone();
        let downloaded_clone = downloaded_size_all.clone();
        let file_size = file_info.size;
        
        let task = tokio::spawn(async move {
            if let Some(parent) = target_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            
            let mut downloader = FileDownloader::new(
                app_handle_clone.clone(),
                total_size,
                downloaded_clone.clone(),
                i,
                total_files,
            );
            
            match downloader.download(&url, &target_path, file_size).await {
                Ok(bytes) => {
                    downloaded_clone.fetch_add(bytes, Ordering::SeqCst);
                    let _ = app_handle_clone.emit("download-log", format!("[完成] {}", target_path.display()));
                    1
                }
                Err(e) => {
                    let _ = app_handle_clone.emit("download-log", format!("[下载失败] {} - {}", target_path.display(), e));
                    0
                }
            }
        });
        
        tasks.push(task);
    }
    
    for task in tasks {
        if let Ok(result) = task.await {
            if result == 1 {
                success_count += 1;
            } else {
                fail_count += 1;
            }
        }
    }
    
    let _ = std::fs::remove_file(&incomplete_marker);
    
    if fail_count > 0 {
        Ok(format!("下载完成：成功 {} 个，失败 {} 个", success_count, fail_count))
    } else {
        Ok(format!("下载完成：{} 个文件", success_count))
    }
}

fn extract_model_name(url: &str) -> String {
    let url = url.trim();
    if url.contains("huggingface.co") {
        let path = url.split("huggingface.co/").nth(1).unwrap_or("");
        let parts: Vec<&str> = path.split('?').next().unwrap_or(path).split('/').collect();
        if parts.len() >= 2 {
            format!("{}_{}", parts[0], parts[1])
        } else {
            "unknown_model".to_string()
        }
    } else if url.contains("modelscope.cn") {
        let path = url.split("modelscope.cn/").nth(1).unwrap_or("");
        let parts: Vec<&str> = path.split('?').next().unwrap_or(path).split('/').collect();
        if parts.len() >= 2 {
            if parts[0] == "models" && parts.len() >= 3 {
                format!("{}_{}", parts[1], parts[2])
            } else {
                format!("{}_{}", parts[0], parts[1])
            }
        } else {
            "unknown_model".to_string()
        }
    } else {
        "unknown_model".to_string()
    }
}

fn extract_model_id(url: &str) -> Option<String> {
    let url = url.trim();
    if url.contains("huggingface.co") {
        let path = url.split("huggingface.co/").nth(1)?;
        let parts: Vec<&str> = path.split('?').next().unwrap_or(path).split('/').collect();
        if parts.len() >= 2 {
            Some(format!("{}/{}", parts[0], parts[1]))
        } else {
            None
        }
    } else if url.contains("modelscope.cn") {
        let path = url.split("modelscope.cn/").nth(1)?;
        let parts: Vec<&str> = path.split('?').next().unwrap_or(path).split('/').collect();
        if parts.len() >= 3 && parts[0] == "models" {
            Some(format!("{}/{}", parts[1], parts[2]))
        } else if parts.len() >= 2 {
            Some(format!("{}/{}", parts[0], parts[1]))
        } else {
            None
        }
    } else {
        None
    }
}

fn format_bytes(bytes: u64) -> String {
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

pub fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(AppState {
            download_child: Mutex::new(None),
            current_save_path: Mutex::new(None),
            login_window_open: Mutex::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            start_download, stop_download, stop_download_with_cleanup, shutdown_system, cancel_shutdown, get_model_files,
            load_tasks, save_tasks, clear_tasks,
            check_login_status, get_user_info, open_wechat_login_window, logout, notify_login_success, close_login_window,
            create_wechat_qrcode, check_wechat_login, save_user_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub fn run() {
    main();
}
