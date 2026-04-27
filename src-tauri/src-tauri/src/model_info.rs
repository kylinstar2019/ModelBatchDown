use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFilesResult {
    pub is_gguf: bool,
    pub files: Vec<String>,
}

#[tauri::command]
pub fn get_model_files(url: String) -> Result<ModelFilesResult, String> {
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

pub fn get_hf_all_files(model_id: &str) -> Result<Vec<String>, String> {
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

pub fn get_ms_files_with_size(model_id: &str) -> Result<Vec<FileInfo>, String> {
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

pub fn get_hf_files_with_size(model_id: &str) -> Result<Vec<FileInfo>, String> {
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
            let size = sibling.get("size")
                .and_then(|s| s.as_u64())
                .or_else(|| sibling.get("sizeBytes").and_then(|s| s.as_u64()))
                .or_else(|| sibling.get("size_in_bytes").and_then(|s| s.as_u64()))
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
        let total_size: u64 = files.iter().map(|f| f.size).sum();
        eprintln!("[HF] 获取到 {} 个文件，总大小: {} bytes", files.len(), total_size);
        Ok(files)
    }
}

pub fn extract_model_name(url: &str) -> String {
    let url = url.trim();

    if url.contains("huggingface.co") {
        let path = url.split("huggingface.co/").nth(1).unwrap_or("");
        let parts: Vec<&str> = path.split('?').next().unwrap_or(path).split('/').collect();
        if parts.len() >= 2 {
            format!("{}/{}", parts[0], parts[1])
        } else {
            "unknown_model".to_string()
        }
    } else if url.contains("modelscope.cn") {
        let path = url.split("modelscope.cn/").nth(1).unwrap_or("");
        let parts: Vec<&str> = path.split('?').next().unwrap_or(path).split('/').collect();
        if parts.len() >= 2 {
            if parts[0] == "models" && parts.len() >= 3 {
                format!("{}/{}", parts[1], parts[2])
            } else {
                format!("{}/{}", parts[0], parts[1])
            }
        } else {
            "unknown_model".to_string()
        }
    } else {
        "unknown_model".to_string()
    }
}

pub fn extract_model_id(url: &str) -> Option<String> {
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
