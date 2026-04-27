use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::constants::{API_BASE_URL, STORE_PATH, USER_INFO_KEY};
use crate::logging::write_log;

#[tauri::command]
pub async fn create_wechat_qrcode(_app: AppHandle) -> Result<serde_json::Value, String> {
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
pub async fn check_wechat_login(scene_str: String) -> Result<serde_json::Value, String> {
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
pub async fn save_user_info(app: AppHandle, user_info: serde_json::Value) -> Result<(), String> {
    write_log(&format!("[WechatAPI] 保存用户信息: {:?}", user_info));

    let store = app.store(STORE_PATH).map_err(|e| e.to_string())?;
    store.set(USER_INFO_KEY, user_info);
    store.save().map_err(|e| e.to_string())?;

    write_log("[WechatAPI] 用户信息保存成功");
    Ok(())
}

pub fn find_project_root(exe_dir: &std::path::Path) -> Option<std::path::PathBuf> {
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
