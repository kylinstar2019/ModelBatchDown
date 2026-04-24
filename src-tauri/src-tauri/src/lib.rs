use std::process::{Command, Child, Stdio};
use std::sync::Mutex;
use std::io::{BufRead, BufReader, Write};
use std::fs::OpenOptions;
use std::path::PathBuf;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder};
use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;

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

    let exe_dir = std::env::current_exe()
        .ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let project_root = find_project_root(&exe_dir)
        .ok_or_else(|| "找不到 run_download_cli.exe".to_string())?;

    let cli_exe = project_root.join("run_download_cli.exe");

    let mut cmd = Command::new(&cli_exe);
    cmd.current_dir(&project_root)
       .arg("--urls").arg(&urls)
       .arg("--save-root").arg(&save_root)
       .arg("--gguf-quant").arg(&gguf_quant)
       .arg("--auto-shutdown").arg(if auto_shutdown { "1" } else { "0" })
       .stdout(Stdio::piped())
       .stderr(Stdio::piped());

    #[cfg(windows)]
    { cmd.creation_flags(CREATE_NO_WINDOW); }

    match cmd.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            {
                let mut dl = state.download_child.lock().unwrap();
                *dl = Some(child);
            }

            let app_handle = app.clone();
            if let Some(stdout) = stdout {
                std::thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(l) = line {
                            let _ = app_handle.emit("download-log", l);
                        }
                    }
                });
            }

            if let Some(stderr) = stderr {
                let app_handle = app.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(l) = line {
                            let _ = app_handle.emit("download-log", format!("[STDERR] {}", l));
                        }
                    }
                });
            }

            let app_handle2 = app.clone();
            std::thread::spawn(move || {
                if let Some(st) = app_handle2.try_state::<AppState>() {
                    loop {
                        if let Ok(mut dl_guard) = st.download_child.lock() {
                            if let Some(ref mut c) = *dl_guard {
                                match c.try_wait() {
                                    Ok(Some(_)) => { *dl_guard = None; break; }
                                    Ok(None) => { drop(dl_guard); std::thread::sleep(std::time::Duration::from_millis(100)); continue; }
                                    Err(_) => { *dl_guard = None; break; }
                                }
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                let _ = app_handle2.emit("download-finished", "");
            });

            Ok("下载已启动".into())
        }
        Err(e) => Err(format!("启动失败: {}", e)),
    }
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
        if let Ok(entries) = std::fs::read_dir(&save_root_clone) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        eprintln!("清理目录失败: {:?}, 错误: {}", path, e);
                    }
                }
            }
        }
    });

    Ok("已停止并清理".into())
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
        get_hf_all_files(&model_id)?
    } else if url.contains("modelscope.cn") {
        get_ms_all_files(&model_id)?
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

fn get_ms_all_files(model_id: &str) -> Result<Vec<String>, String> {
    let url = format!("https://modelscope.cn/api/v1/models/{}", model_id);

    let response = ureq::get(&url)
        .call()
        .map_err(|e| format!("请求失败: {}", e))?;

    let json: serde_json::Value = response.into_json()
        .map_err(|e| format!("解析失败: {}", e))?;

    let mut files: Vec<String> = Vec::new();

    // GGUF models
    if let Some(gguf_list) = json.pointer("/Data/ModelInfos/gguf/gguf_file_list").and_then(|g| g.as_array()) {
        for entry in gguf_list {
            if let Some(file_infos) = entry.get("file_info").and_then(|f| f.as_array()) {
                for file_info in file_infos {
                    if let Some(name) = file_info.get("name").and_then(|n| n.as_str()) {
                        files.push(name.to_string());
                    }
                }
            }
        }
    }

    // Standard safetensor models
    if files.is_empty() {
        if let Some(safetensor_files) = json.pointer("/Data/ModelInfos/safetensor/files").and_then(|f| f.as_array()) {
            for file_entry in safetensor_files {
                if let Some(name) = file_entry.get("name").and_then(|n| n.as_str()) {
                    files.push(name.to_string());
                }
            }
        }
    }

    if files.is_empty() {
        Err("未找到模型文件".into())
    } else {
        Ok(files)
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
