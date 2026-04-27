use std::process::{Command, Child, Stdio};
use std::sync::Mutex;
use std::io::{BufRead, Write};
use std::fs::OpenOptions;
use std::path::PathBuf;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder, http::Response};
use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;
use tokio::io::AsyncSeekExt;
use futures::StreamExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

const FRONTEND_HTML: &str = include_str!("../../src/index.html");

#[derive(Clone, Debug)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelFilesResult {
    is_gguf: bool,
    files: Vec<String>,
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
        .map_err(|e| format!("读取任务文件失败: {}", e))?;
    let tasks: Vec<DownloadTask> = serde_json::from_str(&content)
        .unwrap_or_default();
    Ok(tasks)
}

#[tauri::command]
fn save_tasks(tasks: Vec<DownloadTask>) -> Result<(), String> {
    let path = get_tasks_file_path();
    let content = serde_json::to_string_pretty(&tasks)
        .map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("写入任务文件失败: {}", e))?;
    Ok(())
}

#[tauri::command]
fn clear_tasks() -> Result<(), String> {
    let path = get_tasks_file_path();
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("删除任务文件失败: {}", e))?;
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
            write_log(&format!("[WechatLogin] 收到登录成功事件! payload: {:?}", event.payload()));
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                write_log(&format!("[WechatLogin] 解析payload成功: {:?}", payload));
                if let Some(user_info) = payload.get("userInfo") {
                    write_log(&format!("[WechatLogin] 获取到userInfo: {:?}", user_info));
                    if let Ok(store) = app_handle2.store(STORE_PATH) {
                        store.set(USER_INFO_KEY, user_info.clone());
                        if let Err(e) = store.save() {
                            write_log(&format!("[WechatLogin] 保存用户信息失败: {}", e));
                        } else {
                            write_log("[WechatLogin] 用户信息保存成功");
                        }
                    } else {
                        write_log("[WechatLogin] 获取store失败");
                    }
                    let _ = app_handle2.emit("login-success", user_info.clone());
                    write_log("[WechatLogin] 已发送login-success到前端");
                } else {
                    write_log("[WechatLogin] 未找到userInfo字段");
                }
            } else {
                write_log(&format!("[WechatLogin] 解析payload失败: {}", event.payload()));
            }
        });

        window.listen("close-webview", move |_| {
            write_log("[WechatLogin] 收到close-webview事件");
            if let Some(w) = app_handle3.get_webview_window("wechat-login") {
                let _ = w.close();
                write_log("[WechatLogin] 登录窗口已关闭");
            }
        });

        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                write_log("[WechatLogin] 用户请求关闭登录窗口");
                if let Some(st) = app_handle4.try_state::<AppState>() {
                    if let Ok(mut is_open) = st.login_window_open.lock() {
                        *is_open = false;
                    }
                }
            }
        });
    } else {
        write_log("[WechatLogin] 未找到登录窗口");
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
async fn get_debug_info(app: AppHandle) -> Result<serde_json::Value, String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    let user_info = store.get(USER_INFO_KEY);
    
    let login_window = app.get_webview_window("wechat-login");
    
    let info = serde_json::json!({
        "isLoggedIn": user_info.is_some(),
        "userInfo": user_info,
        "loginWindowExists": login_window.is_some(),
    });
    
    write_log(&format!("[DEBUG] get_debug_info: {}", info));
    
    Ok(info)
}

#[tauri::command]
async fn notify_login_success(app: AppHandle, user_info: serde_json::Value) -> Result<(), String> {
    write_log(&format!("[Notify] notify_login_success 被调用, user_info: {:?}", user_info));
    
    if let Err(e) = app.emit("login-success", user_info.clone()) {
        write_log(&format!("[Notify] emit失败: {}", e));
        return Err(e.to_string());
    }
    
    write_log("[Notify] login-success 已发送到前端");
    
    if let Some(window) = app.get_webview_window("wechat-login") {
        if let Err(e) = window.close() {
            write_log(&format!("[Notify] 关闭窗口失败: {}", e));
        } else {
            write_log("[Notify] 登录窗口已关闭");
        }
    }
    
    Ok(())
}

#[tauri::command]
async fn close_login_window(app: AppHandle) -> Result<(), String> {
    write_log("[Notify] close_login_window 被调用");

    if let Some(window) = app.get_webview_window("wechat-login") {
        if let Err(e) = window.close() {
            write_log(&format!("[Notify] 关闭窗口失败: {}", e));
        } else {
            write_log("[Notify] 登录窗口已关闭");
        }
    } else {
        write_log("[Notify] 未找到登录窗口");
    }

    Ok(())
}

#[tauri::command]
async fn create_wechat_qrcode(app: AppHandle) -> Result<serde_json::Value, String> {
    write_log("[WechatAPI] 调用生成二维码接口");

    let url = API_BASE_URL.to_string();
    write_log(&format!("[WechatAPI] 请求URL: {}", url));

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
            write_log(&format!("[WechatAPI] 响应: {}", text));

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
        Err(e) => {
            write_log(&format!("[WechatAPI] 请求失败: {}", e));
            Err(format!("请求失败: {}", e))
        }
    }
}

#[tauri::command]
async fn check_wechat_login(scene_str: String) -> Result<serde_json::Value, String> {
    write_log(&format!("[WechatAPI] 查询登录状态, scene_str: {}", scene_str));

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
            write_log(&format!("[WechatAPI] 登录状态响应: {}", text));

            let result: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

            Ok(serde_json::json!({
                "code": result["code"],
                "msg": result["msg"],
                "token": result["token"],
                "userInfo": result["userInfo"]
            }))
        }
        Err(e) => {
            write_log(&format!("[WechatAPI] 查询请求失败: {}", e));
            Err(format!("查询请求失败: {}", e))
        }
    }
}

#[tauri::command]
async fn save_user_info(app: AppHandle, user_info: serde_json::Value) -> Result<(), String> {
    write_log(&format!("[WechatAPI] 保存用户信息: {:?}", user_info));

    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    store.set(USER_INFO_KEY, user_info);
    store.save().map_err(|e| e.to_string())?;

    write_log("[WechatAPI] 用户信息保存成功");
    Ok(())
}

fn find_project_root(exe_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let script_name = "run_download_cli.exe";
    let mut current = exe_dir.to_path_buf();
    for _ in 0..8 {
        if current.join(script_name).exists() {
            return Some(current.to_path_buf());
        }
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }
    None
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
    // 清理旧的下载
    {
        let mut dl = state.download_child.lock().map_err(|e| e.to_string())?;
        if let Some(ref mut c) = *dl {
            let _ = c.kill();
            let _ = c.wait();
        }
        *dl = None;
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
                
                // 自动创建模型文件夹
                let model_name = extract_model_name(&url_str);
                let model_save_path = std::path::PathBuf::from(&save_root_clone).join(&model_name);
                
                // 创建模型文件夹
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
    // Kill the download process
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
                    // 检查目录是否为空（未完成的下载）
                    if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        let has_files = sub_entries.flatten().next().is_some();
                        if !has_files {
                            // 空目录，直接删除
                            if let Err(e) = std::fs::remove_dir(&path) {
                                eprintln!("清理空目录失败：{:?}, 错误：{}", path, e);
                                failed_count += 1;
                            } else {
                                cleaned_count += 1;
                            }
                        } else {
                            // 非空目录，检查是否有 .incomplete 标记
                            let incomplete_marker = path.join(".incomplete");
                            if incomplete_marker.exists() {
                                // 有未完成的标记，删除整个目录
                                if let Err(e) = std::fs::remove_dir_all(&path) {
                                    eprintln!("清理未完成目录失败：{:?}, 错误：{}", path, e);
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
    #[cfg(not(target_os = "windows"))]
    { Command::new("shutdown").args(["-h", "+1"]).spawn().unwrap(); }
}

#[tauri::command]
fn cancel_shutdown() {
    #[cfg(target_os = "windows")]
    { Command::new("shutdown").args(["/a"]).spawn().unwrap(); }
    #[cfg(not(target_os = "windows"))]
    { Command::new("shutdown").args(["-c"]).spawn().unwrap(); }
}

#[tauri::command]
fn get_model_files(url: String) -> Result<ModelFilesResult, String> {
    let url_lower = url.to_lowercase();
    let is_gguf = url_lower.contains("gguf");
    let model_id = extract_model_id(&url).ok_or("无法解析模型ID")?;

    let files = if url.contains("huggingface.co") {
        let hf_files = get_hf_files_with_size(&model_id)?;
        hf_files.into_iter().map(|f| f.path).collect()
    } else if url.contains("modelscope.cn") {
        let ms_files = get_ms_files_with_size(&model_id)?;
        ms_files.into_iter().map(|f| f.path).collect()
    } else {
        return Err("不支持的链接".into());
    };

    Ok(ModelFilesResult { is_gguf, files })
}

fn get_hf_all_files(model_id: &str) -> Result<Vec<String>, String> {
    let endpoint = std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".to_string());
    let url = format!("{}/api/models/{}", endpoint, model_id);

    let response = ureq::get(&url)
        .call()
        .map_err(|e| format!("请求失败: {}", e))?;

    let json: serde_json::Value = response.into_json()
        .map_err(|e| format!("解析失败: {}", e))?;

    let siblings = json.get("siblings")
        .and_then(|s| s.as_array())
        .ok_or("无法获取文件列表")?;

    let files: Vec<String> = siblings.iter()
        .filter_map(|s| s.get("rfilename").and_then(|f| f.as_str()).map(String::from))
        .collect();

    Ok(files)
}

/// 获取 ModelScope 模型文件列表（包含大小信息）
fn get_ms_files_with_size(model_id: &str) -> Result<Vec<FileInfo>, String> {
    let url = format!("https://modelscope.cn/api/v1/models/{}/repo/files?Recursive=true", model_id);

    let response = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/89.0.4389.90 Safari/537.36")
        .call()
        .map_err(|e| format!("请求失败：{}", e))?;

    let json: serde_json::Value = response.into_json()
        .map_err(|e| format!("解析失败：{}", e))?;

    let success = json.get("Success").and_then(|s| s.as_bool()).unwrap_or(false);
    if !success {
        let message = json.get("Message").and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("API 返回错误：{}", message));
    }

    let files_data = json.get("Data")
        .and_then(|d| d.get("Files"))
        .and_then(|f| f.as_array())
        .ok_or("无法获取文件列表")?;

    let mut files: Vec<FileInfo> = Vec::new();

    for file_entry in files_data {
        let file_type = file_entry.get("Type").and_then(|t| t.as_str()).unwrap_or("");
        if file_type == "blob" {
            if let (Some(path), Some(size)) = (
                file_entry.get("Path").and_then(|p| p.as_str()),
                file_entry.get("Size").and_then(|s| s.as_u64())
            ) {
                files.push(FileInfo {
                    path: path.to_string(),
                    size,
                });
            }
        }
    }

    if files.is_empty() {
        Err("未找到模型文件".into())
    } else {
        Ok(files)
    }
}

/// 获取 HF 模型文件列表（包含大小信息）
fn get_hf_files_with_size(model_id: &str) -> Result<Vec<FileInfo>, String> {
    let endpoint = std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".to_string());
    let url = format!("{}/api/models/{}", endpoint, model_id);

    let response = ureq::get(&url)
        .call()
        .map_err(|e| format!("请求失败：{}", e))?;

    let json: serde_json::Value = response.into_json()
        .map_err(|e| format!("解析失败：{}", e))?;

    let siblings = json.get("siblings")
        .and_then(|s| s.as_array())
        .ok_or("无法获取文件列表")?;

    let mut files: Vec<FileInfo> = Vec::new();

    for sibling in siblings {
        if let Some(rfilename) = sibling.get("rfilename").and_then(|f| f.as_str()) {
            // size 字段可能不存在，使用 0 作为默认值
            let size = sibling.get("size")
                .and_then(|s| s.as_u64())
                .unwrap_or(0);
            
            files.push(FileInfo {
                path: rfilename.to_string(),
                size,
            });
        }
    }

    if files.is_empty() {
        Err("未找到模型文件".into())
    } else {
        Ok(files)
    }
}

fn extract_model_name(url: &str) -> String {
    let url = url.trim();
    
    // 从 URL 中提取模型名称
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



async fn download_model_rust(
    url: &str,
    save_path: &str,
    gguf_quant: Option<String>,
    mode: &str,
    app_handle: AppHandle,
) -> Result<String, String> {
    let url_lower = url.to_lowercase();
    let is_gguf = url_lower.contains("gguf");
    let model_id = extract_model_id(url).ok_or("无法解析模型 ID")?;

    let allow_patterns = if is_gguf {
        gguf_quant.map(|q| vec![format!("*{}*.gguf", q)])
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

async fn huggingface_download(
    model_id: &str,
    save_path: &str,
    allow_patterns: Option<Vec<String>>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let _ = app_handle.emit("download-log", format!("[HF] 开始下载模型：{}", model_id));
    
    // 创建.incomplete 标记
    let incomplete_marker = std::path::PathBuf::from(save_path).join(".incomplete");
    if let Err(e) = std::fs::File::create(&incomplete_marker) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建未完成标记失败：{}", e));
    }
    
    // 获取文件列表（带大小信息）
    let files = get_hf_files_with_size(model_id)?;
    let files_to_download = filter_files_with_size(&files, &allow_patterns);
    
    if files_to_download.is_empty() {
        let _ = std::fs::remove_file(&incomplete_marker);
        return Err("没有找到需要下载的文件".to_string());
    }

    let _ = app_handle.emit("download-log", format!("[HF] 准备下载 {} 个文件", files_to_download.len()));
    
    // 使用 HTTP 直接下载（和 ModelScope 一样的逻辑）
    let endpoint = std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".to_string());
    let base_url = format!("{}/{}/resolve/main", endpoint, model_id);
    
    download_files_from_urls(&base_url, &files_to_download, save_path, app_handle, incomplete_marker).await
}

/// 并发下载模型文件
async fn modelscope_download(
    model_id: &str,
    save_path: &str,
    allow_patterns: Option<Vec<String>>,
    app_handle: AppHandle,
) -> Result<String, String> {
    use std::time::Duration;
    
    let _ = app_handle.emit("download-log", format!("[MS] 开始下载模型：{}", model_id));
    
    // 创建.incomplete 标记
    let incomplete_marker = std::path::PathBuf::from(save_path).join(".incomplete");
    if let Err(e) = std::fs::File::create(&incomplete_marker) {
        let _ = app_handle.emit("download-log", format!("[警告] 创建未完成标记失败：{}", e));
    }
    
    // 获取带大小的文件列表
    let files = get_ms_files_with_size(model_id)?;
    let files_to_download = filter_files_with_size(&files, &allow_patterns);
    
    if files_to_download.is_empty() {
        let _ = std::fs::remove_file(&incomplete_marker);
        return Err("没有找到需要下载的文件".to_string());
    }

    // 计算总大小
    let total_size_all: u64 = files_to_download.iter().map(|f| f.size).sum();
    let total_files = files_to_download.len();
    
    let _ = app_handle.emit("download-log", format!("[MS] 准备下载 {} 个文件，总大小：{}", total_files, format_bytes(total_size_all)));
    
    // 并发下载所有文件
    let mut success_count = 0;
    let mut fail_count = 0;
    let downloaded_size_all = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let speed_samples = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    
    // 克隆文件列表，避免生命周期问题
    let files_clone = files_to_download.clone();
    let save_path_clone = save_path.to_string();
    let app_handle_progress = app_handle.clone();
    let downloaded_scan = downloaded_size_all.clone();
    let total_speed_scan = speed_samples.clone();
    
    // 启动定期扫描任务（每 30 秒）
    let scan_task = tokio::spawn(async move {
        let save_path = std::path::PathBuf::from(save_path_clone);
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        let mut last_total_size = 0u64;
        let mut last_time = std::time::Instant::now();
        
        // 构建目标文件路径列表
        let mut target_files = std::collections::HashSet::new();
        for file_info in &files_to_download {
            target_files.insert(file_info.path.clone());
        }
        
        loop {
            interval.tick().await;
            
            // 扫描文件夹，只计算目标文件的大小
            if let Ok(scanned_size) = scan_folder_size_filtered(&save_path, &target_files) {
                downloaded_scan.store(scanned_size, std::sync::atomic::Ordering::SeqCst);
                
                // 计算平均速度
                let current_time = std::time::Instant::now();
                let elapsed = current_time.duration_since(last_time).as_secs_f64();
                if elapsed > 0.0 {
                    let size_diff = if scanned_size >= last_total_size {
                        scanned_size - last_total_size
                    } else {
                        0
                    };
                    let avg_speed = size_diff as f64 / elapsed;
                    
                    let mut samples = total_speed_scan.lock().unwrap();
                    samples.push(avg_speed);
                    if samples.len() > 10 {
                        samples.remove(0);
                    }
                    
                    // 发送进度更新
                    let percent = if total_size_all > 0 {
                        (scanned_size as f64 / total_size_all as f64 * 100.0).min(100.0)
                    } else {
                        0.0
                    };
                    
                    let _ = app_handle_progress.emit("download-progress", serde_json::json!({
                        "downloaded": scanned_size,
                        "total": total_size_all,
                        "speed": format_speed(avg_speed as u64),
                        "percent": percent
                    }));
                    
                    last_total_size = scanned_size;
                    last_time = current_time;
                }
            }
        }
    });
    
    // 创建下载任务
    let mut tasks = Vec::new();
    for (i, file_info) in files_clone.into_iter().enumerate() {
        let url = format!("https://modelscope.cn/{}/resolve/master/{}", model_id, file_info.path);
        let target_path = std::path::PathBuf::from(save_path).join(&file_info.path);
        let app_handle_clone = app_handle.clone();
        let downloaded_clone = downloaded_size_all.clone();
        let file_size = file_info.size;
        let speed_samples_clone = speed_samples.clone();
        
        let task = tokio::spawn(async move {
            // 创建目录
            if let Some(parent) = target_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            
            // 下载文件
            match download_single_file(&url, &target_path, &app_handle_clone, i, total_files, total_size_all, downloaded_clone.clone(), file_size).await {
                Ok(_) => {
                    let _ = app_handle_clone.emit("download-log", format!("[完成] {}", file_info.path));
                    1
                }
                Err(e) => {
                    let _ = app_handle_clone.emit("download-log", format!("[下载失败] {} - {}", file_info.path, e));
                    0
                }
            }
        });
        
        tasks.push(task);
    }
    
    // 等待所有任务完成
    for task in tasks {
        if let Ok(result) = task.await {
            if result == 1 {
                success_count += 1;
            } else {
                fail_count += 1;
            }
        }
    }
    
    // 停止扫描任务
    scan_task.abort();
    
    // 最后一次扫描
    if let Ok(final_size) = scan_folder_size(&std::path::PathBuf::from(save_path)) {
        downloaded_size_all.store(final_size, std::sync::atomic::Ordering::SeqCst);
        
        let _ = app_handle.emit("download-progress", serde_json::json!({
            "downloaded": final_size,
            "total": total_size_all,
            "speed": "0 B/s",
            "percent": 100.0
        }));
    }
    
    // 下载完成，删除.incomplete 标记
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

/// 扫描文件夹大小
fn scan_folder_size(path: &std::path::PathBuf) -> Result<u64, std::io::Error> {
    let mut total_size = 0u64;
    if path.exists() {
        for entry in std::fs::read_dir(path)? {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(metadata) = entry.metadata() {
                        total_size += metadata.len();
                    }
                } else if path.is_dir() {
                    total_size += scan_folder_size(&path.to_path_buf()).unwrap_or(0);
                }
            }
        }
    }
    Ok(total_size)
}

/// 扫描文件夹大小（只计算目标文件）
fn scan_folder_size_filtered(path: &std::path::PathBuf, target_files: &std::collections::HashSet<String>) -> Result<u64, std::io::Error> {
    let mut total_size = 0u64;
    if path.exists() {
        for entry in std::fs::read_dir(path)? {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    // 检查是否是目标文件
                    if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                        if target_files.iter().any(|target| target.ends_with(file_name)) {
                            if let Ok(metadata) = entry.metadata() {
                                total_size += metadata.len();
                            }
                        }
                    }
                } else if path.is_dir() {
                    // 递归扫描子目录
                    total_size += scan_folder_size_filtered(&path.to_path_buf(), target_files).unwrap_or(0);
                }
            }
        }
    }
    Ok(total_size)
}

async fn get_file_size(url: &str) -> Result<u64, String> {
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

fn filter_files_with_size(files: &[FileInfo], patterns: &Option<Vec<String>>) -> Vec<FileInfo> {
    match patterns {
        Some(patterns) => {
            files.iter()
                .filter(|f| patterns.iter().any(|p| {
                    if p.starts_with('*') && p.ends_with('*') {
                        f.path.contains(&p[1..p.len()-1])
                    } else if p.starts_with('*') {
                        f.path.ends_with(&p[1..])
                    } else if p.ends_with('*') {
                        f.path.starts_with(&p[..p.len()-1])
                    } else {
                        f.path == *p
                    }
                }))
                .cloned()
                .collect()
        }
        None => files.to_vec(),
    }
}

fn filter_files(files: &[String], patterns: &Option<Vec<String>>) -> Vec<String> {
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

/// 测试网速（下载前 1MB）
async fn test_download_speed(url: &str) -> Result<f64, String> {
    let client = reqwest::Client::new();
    let response = client.get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Range", "bytes=0-1048575") // 下载前 1MB
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

/// 下载单个文件（支持断点续传）
async fn download_single_file(
    url: &str,
    save_path: &std::path::PathBuf,
    app_handle: &AppHandle,
    file_index: usize,
    total_files: usize,
    total_size_all: u64,
    downloaded_size_all: std::sync::Arc<std::sync::atomic::AtomicU64>,
    file_size: u64,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    use std::time::Instant;
    
    // 检查文件是否已存在
    let start_pos = if save_path.exists() {
        let metadata = std::fs::metadata(save_path).map_err(|e| format!("读取文件失败：{}", e))?;
        let len = metadata.len();
        if len >= file_size {
            let _ = app_handle.emit("download-log", format!("[跳过] {} 已存在", save_path.display()));
            downloaded_size_all.fetch_add(file_size, std::sync::atomic::Ordering::SeqCst);
            return Ok(());
        }
        len
    } else {
        0
    };
    
    // 创建文件
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(save_path)
        .await
        .map_err(|e| format!("创建文件失败：{}", e))?;
    
    file.set_len(file_size).await.map_err(|e| format!("设置文件大小失败：{}", e))?;
    
    // 发送请求（支持断点续传）
    let client = reqwest::Client::new();
    
    // 记录原始 URL 用于调试
    let _ = app_handle.emit("download-log", format!("[URL] {}", url));
    
    // 对 URL 进行完整编码，处理所有特殊字符
    let encoded_url = url
        .replace(" ", "%20")
        .replace("[", "%5B")
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
    let mut last_report_time = Instant::now();
    let mut last_reported_bytes = 0usize;
    
    // 下载数据
    let mut last_overall_reported = 0u64; // 记录上次报告的总下载量
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取数据失败：{}", e))?;
        let len = chunk.len();
        file.seek(std::io::SeekFrom::Start(start_pos as u64 + total_bytes as u64))
            .await
            .map_err(|e| format!("定位失败：{}", e))?;
        file.write_all(&chunk).await.map_err(|e| format!("写入文件失败：{}", e))?;
        total_bytes += len;
        
        let now = Instant::now();
        let elapsed = now.duration_since(last_report_time).as_secs_f64();
        
        // 每 0.5 秒报告一次进度
        if elapsed >= 0.5 {
            // 当前文件本次下载的增量（不包括 start_pos，因为那是之前就有的）
            let current_increment = total_bytes as u64;
            // 总下载量 = 其他已完成文件的下载量 + 当前文件本次下载的增量
            let overall_downloaded = downloaded_size_all.load(std::sync::atomic::Ordering::Relaxed) + current_increment;
            
            if total_size_all > 0 {
                let percent = overall_downloaded as f64 / total_size_all as f64 * 100.0;
                
                let _ = app_handle.emit("download-progress", serde_json::json!({
                    "file_index": file_index,
                    "total_files": total_files,
                    "file_name": save_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    "downloaded": overall_downloaded,
                    "total": total_size_all,
                    "speed": "计算中...",
                    "percent": percent
                }));
                
                let progress_mb = (start_pos as u64 + total_bytes as u64) as f64 / (1024.0 * 1024.0);
                let _ = app_handle.emit("download-log", format!(
                    "[进度] {}/{} - {:.2} MB ({:.2}%)",
                    file_index + 1, total_files, progress_mb, percent
                ));
            }
            
            last_report_time = now;
            last_reported_bytes = start_pos as usize + total_bytes;
            last_overall_reported = overall_downloaded;
        }
    }
    
    // 更新总下载量：只增加本次下载的增量（total_bytes 是本次实际下载的字节数）
    downloaded_size_all.fetch_add(total_bytes as u64, std::sync::atomic::Ordering::SeqCst);
    
    Ok(())
}

async fn download_file_with_resume(
    url: &str,
    save_path: &std::path::PathBuf,
    app_handle: &AppHandle,
    file_index: usize,
    total_files: usize,
    total_size_all: u64,
    downloaded_size_all: u64,
) -> Result<usize, String> {
    use tokio::io::AsyncWriteExt;
    use futures::StreamExt;
    use std::time::Instant;
    
    let client = reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{}", e))?;

    // 检查已存在的文件，获取已下载的大小
    let mut start_pos = 0u64;
    if save_path.exists() {
        let metadata = tokio::fs::metadata(save_path).await
            .map_err(|e| format!("获取文件元数据失败：{}", e))?;
        start_pos = metadata.len();
        let _ = app_handle.emit("download-log", format!("[续传] 已下载 {} bytes", start_pos));
    }

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
    let mut request = client.get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/89.0.4389.90 Safari/537.36");
    
    if start_pos > 0 {
        request = request.header("Range", format!("bytes={}-", start_pos));
    }

    // 发送请求，带重试机制
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
                    let _ = app_handle.emit("download-log", format!("[重试] 第 {} 次重试，等待 {} 秒", retry_count, wait_time.as_secs()));
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

    // 获取文件大小（如果有）
    let total_size = response.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let mut stream = response.bytes_stream();
    let mut total_bytes = 0usize;
    let start_time = Instant::now();
    let mut last_report_time = start_time;
    let mut last_reported_bytes = 0usize;
    let mut speed_history: Vec<f64> = Vec::new(); // 速度历史，用于平滑

    // 下载数据
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取数据失败：{}", e))?;
        let len = chunk.len();
        file.write_all(&chunk).await.map_err(|e| format!("写入文件失败：{}", e))?;
        total_bytes += len;
        
        let now = Instant::now();
        let elapsed = now.duration_since(last_report_time).as_secs_f64();
        
        // 每 0.5 秒报告一次进度和速度
        if elapsed >= 0.5 || total_bytes % (512 * 1024) < 32768 {
            let current_file_downloaded = start_pos as usize + total_bytes;
            let progress_mb = current_file_downloaded as f64 / (1024.0 * 1024.0);
            
            // 计算整体进度（包括之前下载的文件）
            let overall_downloaded = downloaded_size_all + current_file_downloaded as u64;
            
            // 计算速度（使用更长的时间窗口）
            let bytes_since_last = (start_pos as usize + total_bytes) - (start_pos as usize + last_reported_bytes);
            let instant_speed = if elapsed > 0.0 { bytes_since_last as f64 / elapsed } else { 0.0 };
            
            // 使用滑动平均平滑速度
            speed_history.push(instant_speed);
            if speed_history.len() > 5 {
                speed_history.remove(0);
            }
            let avg_speed = speed_history.iter().sum::<f64>() / speed_history.len() as f64;
            let speed_str = format_speed(avg_speed as u64);
            
            if total_size_all > 0 {
                // 使用整体大小计算百分比
                let percent = overall_downloaded as f64 / total_size_all as f64 * 100.0;
                // 计算 ETA
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

/// 并发下载文件（用于 HF 和 MS）
async fn download_files_from_urls(
    base_url: &str,
    files: &[FileInfo],
    save_path: &str,
    app_handle: AppHandle,
    incomplete_marker: std::path::PathBuf,
) -> Result<String, String> {
    use std::time::Duration;
    
    let files_clone = files.to_vec();
    let save_path_clone = save_path.to_string();
    let total_files = files.len();
    
    // 计算总大小
    let total_size_all: u64 = files.iter().map(|f| f.size).sum();
    
    // 共享的已下载大小（原子操作）
    let downloaded_size_all = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let downloaded_scan = downloaded_size_all.clone();
    
    // 扫描文件夹计算已下载大小（用于进度显示）
    let mut target_files = std::collections::HashSet::new();
    for file_info in files {
        target_files.insert(file_info.path.clone());
    }
    
    // 启动定期扫描任务（每 5 秒）
    let app_handle_scan_task = app_handle.clone();
    let scan_task = tokio::spawn(async move {
        let save_path = std::path::PathBuf::from(save_path_clone);
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        let mut last_total_size = 0u64;
        let mut last_time = std::time::Instant::now();
        let app_handle_scan = app_handle_scan_task.clone();
        
        loop {
            interval.tick().await;
            
            if let Ok(scanned_size) = scan_folder_size_filtered(&save_path, &target_files) {
                downloaded_scan.store(scanned_size, std::sync::atomic::Ordering::SeqCst);
                
                let current_time = std::time::Instant::now();
                let elapsed = current_time.duration_since(last_time).as_secs_f64();
                if elapsed > 0.0 {
                    let size_diff = if scanned_size >= last_total_size {
                        scanned_size - last_total_size
                    } else {
                        0
                    };
                    let avg_speed = size_diff as f64 / elapsed;
                    
                    let percent = if total_size_all > 0 {
                        (scanned_size as f64 / total_size_all as f64 * 100.0).min(100.0)
                    } else {
                        0.0
                    };
                    
                    let _ = app_handle_scan.emit("download-progress", serde_json::json!({
                        "downloaded": scanned_size,
                        "total": total_size_all,
                        "speed": format_speed(avg_speed as u64),
                        "percent": percent
                    }));
                    
                    last_total_size = scanned_size;
                    last_time = current_time;
                }
            }
        }
    });
    
    // 创建下载任务
    let mut tasks = Vec::new();
    for (i, file_info) in files_clone.into_iter().enumerate() {
        let url = format!("{}/{}", base_url, file_info.path);
        let target_path = std::path::PathBuf::from(save_path).join(&file_info.path);
        let app_handle_clone = app_handle.clone();
        let downloaded_clone = downloaded_size_all.clone();
        let file_size = file_info.size;
        
        let task = tokio::spawn(async move {
            if let Some(parent) = target_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            
            match download_single_file(&url, &target_path, &app_handle_clone, i, total_files, total_size_all, downloaded_clone.clone(), file_size).await {
                Ok(_) => {
                    let _ = app_handle_clone.emit("download-log", format!("[完成] {}", file_info.path));
                    1
                }
                Err(e) => {
                    let _ = app_handle_clone.emit("download-log", format!("[失败] {} - {}", file_info.path, e));
                    0
                }
            }
        });
        
        tasks.push(task);
    }
    
    // 等待所有任务完成
    let results = futures::future::join_all(tasks).await;
    let success_count = results.iter().filter(|r| r.as_ref().ok().copied().unwrap_or(0) == 1).count();
    let fail_count = total_files - success_count;
    
    // 停止扫描任务
    scan_task.abort();
    
    // 下载完成，删除.incomplete 标记
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(AppState {
            download_child: Mutex::new(None),
            current_save_path: Mutex::new(None),
            login_window_open: Mutex::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            start_download, stop_download, stop_download_with_cleanup, shutdown_system, cancel_shutdown, get_model_files,
            load_tasks, save_tasks, clear_tasks,
            check_login_status, get_user_info, open_wechat_login_window, logout, get_debug_info, notify_login_success, close_login_window,
            create_wechat_qrcode, check_wechat_login, save_user_info
        ])
        .register_uri_scheme_protocol("app", move |_ctx, _req| {
            Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .body(FRONTEND_HTML.as_bytes().to_vec())
                .expect("Failed to build HTML response")
        })
        .setup(|app| {
            let _window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::App("app://index.html".into())
            )
            .title("ModelBatchDown - 模型批量下载器")
            .inner_size(1000.0, 720.0)
            .resizable(true)
            .center()
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
