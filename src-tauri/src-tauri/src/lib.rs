mod constants;
mod logging;
mod tasks;
mod login;
mod wechat_api;
mod model_info;
mod download_core;
mod network_speed;

pub use constants::*;
pub use logging::*;
pub use tasks::*;
pub use login::*;
pub use wechat_api::*;
pub use model_info::*;
pub use download_core::*;
pub use network_speed::*;

use tauri::{http::Response, WebviewUrl, WebviewWindowBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            get_model_files,
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
            .build()
            .expect("Failed to create main window");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
