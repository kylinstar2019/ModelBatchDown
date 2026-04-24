use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use tokio::task;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod platform;
mod download;

use platform::{detect_platform, Platform};

#[derive(Parser, Debug)]
#[command(name = "model-download")]
#[command(about = "Model downloader for HuggingFace and ModelScope", long_about = None)]
struct Args {
    #[arg(long, required = true, help = "Model URLs, separated by newlines")]
    urls: String,

    #[arg(long, required = true, help = "Root directory to save models")]
    save_root: String,

    #[arg(long, default_value = "", help = "GGUF quantization version (deprecated, use URL encoding)")]
    gguf_quant: String,

    #[arg(long, default_value = "0", help = "Auto shutdown after completion (1/0)")]
    auto_shutdown: String,
}

#[derive(ValueEnum, Debug, Clone)]
enum DownloadMode {
    All,
    Quant,
    Main,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let args = Args::parse();

    let urls: Vec<&str> = args.urls.split('\n').filter(|u| !u.trim().is_empty()).collect();

    if urls.is_empty() {
        anyhow::bail!("Please provide at least one model URL");
    }

    if args.save_root.is_empty() {
        anyhow::bail!("Please provide save directory");
    }

    let save_root = PathBuf::from(&args.save_root);
    if !save_root.exists() {
        std::fs::create_dir_all(&save_root)?;
    }

    let auto_shutdown = args.auto_shutdown == "1";

    for (idx, raw_url) in urls.iter().enumerate() {
        let url = raw_url.trim();
        let (model_url, quant, mode) = parse_url_entry(url);

        let platform = detect_platform(model_url);
        let model_name = extract_model_name(model_url);

        let save_path = if let Some(name) = model_name {
            save_root.join(&name)
        } else {
            save_root.join(format!("model_{}", idx + 1))
        };

        println!("[INFO] -------- 第 {}/{} 个任务 ({}) --------", idx + 1, urls.len(), model_name.unwrap_or("unknown"));
        println!("[INFO] SAVE_PATH:{}", save_path.display());

        if let Some(ref q) = quant {
            println!("[INFO] [量化: {}]", q);
        }

        let result = task::spawn_blocking({
            let model_url = model_url.to_string();
            let save_path = save_path.clone();
            let quant = quant.clone();
            let mode = mode.clone();
            move || {
                tokio::runtime::Handle::current().block_on(async {
                    download::download_model(&model_url, &save_path, quant.as_deref(), mode.as_deref())
                        .await
                })
            }
        })
        .await??;
    }

    println!("[DONE]");

    if auto_shutdown {
        println!("[INFO] 所有任务下载完成，准备自动关机...");
        #[cfg(windows)]
        {
            std::thread::spawn(|| {
                std::process::Command::new("shutdown")
                    .args(["/s", "/t", "60"])
                    .spawn()
                    .ok();
            });
        }
    }

    Ok(())
}

fn parse_url_entry(entry: &str) -> (&str, Option<&str>, Option<&str>) {
    if let Some((url, suffix)) = entry.split_once("::") {
        if suffix.starts_with("QUANT:") {
            return (url.trim(), Some(&suffix[6..]), Some("quant"));
        } else if suffix.starts_with("MODE:") {
            return (url.trim(), None, Some(&suffix[5..]));
        } else {
            return (url.trim(), Some(suffix), Some("quant"));
        }
    }
    (entry.trim(), None, Some("all"))
}

fn extract_model_name(url: &str) -> Option<String> {
    let url = url.trim();

    if url.contains("huggingface.co") {
        let path = url.split("huggingface.co/").last()?;
        let part = path.split(&[' ', '?'][..]).next()?;
        let parts: Vec<&str> = part.split('/').collect();
        if parts.len() >= 2 {
            return Some(parts[1].to_string());
        }
    } else if url.contains("modelscope.cn") {
        let path = url.split("modelscope.cn/").last()?;
        let part = path.split(&[' ', '?'][..]).next()?;
        let parts: Vec<&str> = part.split('/').collect();
        if parts.len() >= 3 && parts[0] == "models" {
            return Some(parts[2].to_string());
        } else if parts.len() >= 2 {
            return Some(parts[1].to_string());
        }
    }
    None
}
