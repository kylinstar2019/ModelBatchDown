use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_store::StoreExt;

use crate::constants::{WEIBO_LOGIN_URL, STORE_PATH, USER_INFO_KEY};
use crate::logging::write_log;

pub struct AppState {
    pub download_child: Mutex<Option<std::process::Child>>,
    pub current_save_path: Mutex<Option<String>>,
    pub login_window_open: Mutex<bool>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            download_child: Mutex::new(None),
            current_save_path: Mutex::new(None),
            login_window_open: Mutex::new(false),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[tauri::command]
pub async fn check_login_status(app: AppHandle) -> Result<bool, String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    let user_info = store.get(USER_INFO_KEY);
    Ok(user_info.is_some())
}

#[tauri::command]
pub async fn get_user_info(app: AppHandle) -> Result<Option<serde_json::Value>, String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    let user_info = store.get(USER_INFO_KEY);
    Ok(user_info.map(|v| v.clone()))
}

#[tauri::command]
pub async fn open_wechat_login_window(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
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
pub async fn logout(app: AppHandle) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    store.delete(USER_INFO_KEY);
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_debug_info(app: AppHandle) -> Result<serde_json::Value, String> {
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
pub async fn notify_login_success(app: AppHandle, user_info: serde_json::Value) -> Result<(), String> {
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
pub async fn close_login_window(app: AppHandle) -> Result<(), String> {
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
