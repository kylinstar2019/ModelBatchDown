use anyhow::{Context, Result};
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::path::Path;
use tokio::io::AsyncWriteExt;

const HF_ENDPOINT: &str = "https://hf-mirror.com";
const HF_API: &str = "https://huggingface.co";
const MS_API: &str = "https://modelscope.cn/api/v1/models";

#[derive(Debug, serde::Deserialize)]
struct HfRepoTreeItem {
    #[serde(rename = "type")]
    item_type: String,
    path: String,
    size: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct MsFileInfo {
    Name: String,
    Size: String,
}

pub async fn download_model(
    url: &str,
    save_path: &Path,
    quant: Option<&str>,
    mode: Option<&str>,
) -> Result<()> {
    let platform = crate::platform::detect_platform(url);
    let (_, repo_id) = crate::platform::parse_repo_id(url);

    let is_gguf = repo_id.to_lowercase().contains("gguf");

    let allow_patterns = if is_gguf {
        quant.map(|q| format!("*{}*.gguf", q))
    } else if mode == Some("main") {
        Some("*,config.json,*.safetensors,tokenizer*".to_string())
    } else {
        None
    };

    match platform {
        crate::platform::Platform::HuggingFace => {
            download_hf_model(&repo_id, save_path, allow_patterns.as_deref()).await
        }
        crate::platform::Platform::ModelScope => {
            download_ms_model(&repo_id, save_path, allow_patterns.as_deref()).await
        }
    }
}

async fn download_hf_model(repo_id: &str, save_path: &Path, allow_patterns: Option<&str>) -> Result<()> {
    println!("开始下载 {} 到 {}", repo_id, save_path.display());

    std::fs::create_dir_all(save_path).context("Failed to create directory")?;

    let client = Client::builder()
        .user_agent("model-download-cli/1.0")
        .build()
        .context("Failed to create HTTP client")?;

    let tree_url = format!("{}/api/models/{}/tree/main", HF_API, repo_id);
    let files: Vec<HfRepoTreeItem> = client
        .get(&tree_url)
        .send()
        .await
        .context("Failed to fetch file list")?
        .json()
        .await
        .context("Failed to parse file list")?;

    let files: Vec<HfRepoTreeItem> = if let Some(patterns) = allow_patterns {
        let glob_pattern = glob::Pattern::new(patterns)
            .context("Invalid glob pattern")?;
        files
            .into_iter()
            .filter(|f| {
                if f.item_type == "directory" {
                    return true;
                }
                if let Ok(pat) = glob::Pattern::new(&f.path.replace("**/", "*")) {
                    return glob_pattern.matches(&f.path) || pat.matches(patterns);
                }
                glob_pattern.matches(&f.path)
            })
            .collect()
    } else {
        files
    };

    if files.is_empty() {
        anyhow::bail!("No files found matching the pattern");
    }

    let total_size: u64 = files.iter().filter_map(|f| f.size).sum();
    let multi_progress = MultiProgress::new();

    let pb = multi_progress.add(ProgressBar::new(total_size));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?
            .progress_chars("██ "),
    );

    let client = Client::builder()
        .user_agent("model-download-cli/1.0")
        .build()
        .context("Failed to create HTTP client")?;

    for file in files {
        if file.item_type == "directory" {
            continue;
        }

        let file_path = save_path.join(&file.path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create directory")?;
        }

        let file_url = format!("{}/{} resolve/main/{}", HF_ENDPOINT, repo_id, file.path);

        download_file(&client, &file_url, &file_path, &pb).await?;
    }

    pb.finish_with_message("下载完成");
    println!("✅ HF 模型下载完成：{}", repo_id);

    Ok(())
}

async fn download_ms_model(repo_id: &str, save_path: &Path, allow_patterns: Option<&str>) -> Result<()> {
    println!("开始下载 {} 到 {}", repo_id, save_path.display());

    std::fs::create_dir_all(save_path).context("Failed to create directory")?;

    let client = Client::builder()
        .user_agent("model-download-cli/1.0")
        .build()
        .context("Failed to create HTTP client")?;

    let repo_api = if repo_id.contains('/') {
        repo_id.to_string()
    } else {
        format!("AI-ModelScope/{}", repo_id)
    };

    let tree_url = format!("{}/{}/tree/main", MS_API, repo_api);
    let response = client
        .get(&tree_url)
        .send()
        .await
        .context("Failed to fetch file list")?;

    let files: Vec<MsFileInfo> = if response.status().is_success() {
        response.json().await.unwrap_or_default()
    } else {
        let search_url = format!("{}/{}/repo/files", MS_API, repo_api);
        client
            .get(&search_url)
            .send()
            .await
            .context("Failed to fetch file list")?
            .json()
            .await
            .context("Failed to parse file list")?
    };

    if files.is_empty() {
        anyhow::bail!("No files found for model: {}", repo_id);
    }

    let total_size: u64 = files.iter().filter_map(|f| f.Size.parse().ok()).sum();
    let multi_progress = MultiProgress::new();

    let pb = multi_progress.add(ProgressBar::new(total_size));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?
            .progress_chars("██ "),
    );

    for file in files {
        let file_path = save_path.join(&file.Name);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create directory")?;
        }

        let file_url = format!("https://modelscope.cn/models/{}/resolve/main/{}", repo_api, file.Name);

        download_file(&client, &file_url, &file_path, &pb).await?;
    }

    pb.finish_with_message("下载完成");
    println!("✅ 魔塔模型下载完成：{}", repo_id);

    Ok(())
}

async fn download_file(
    client: &Client,
    url: &str,
    file_path: &Path,
    progress: &ProgressBar,
) -> Result<()> {
    let existing_size = if file_path.exists() {
        std::fs::metadata(file_path)?.len()
    } else {
        0
    };

    let mut request = client.get(url);
    if existing_size > 0 {
        request = request.header("Range", format!("bytes={}-", existing_size));
    }

    let response = request.send().await.context("Failed to send download request")?;

    let status = response.status();
    if status == 404 {
        anyhow::bail!("File not found: {}", url);
    }

    if !status.is_success() && status.as_u16() != 206 {
        anyhow::bail!("Download failed with status: {}", status);
    }

    let total_size: u64 = response
        .content_length()
        .map(|c| c + existing_size)
        .unwrap_or(0);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .await
        .context("Failed to open file")?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = existing_size;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Failed to read chunk")?;
        file.write_all(&chunk).await.context("Failed to write chunk")?;
        downloaded += chunk.len() as u64;
        if total_size > 0 {
            progress.set_position(downloaded);
        }
    }

    Ok(())
}
