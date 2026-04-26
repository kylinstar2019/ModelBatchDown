use serde::{Deserialize, Serialize};
use hf_hub::api::sync::{Api, ApiBuilder};
use hf_hub::{Repo, RepoType};
use std::sync::Arc;

/// 文件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
}

/// 模型文件解析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFilesResult {
    pub is_gguf: bool,
    pub files: Vec<FileInfo>,
    pub total_size: u64,
}

/// API 服务 trait
pub trait ModelApiService {
    /// 获取模型文件列表
    fn get_model_files(&self, model_id: &str) -> Result<ModelFilesResult, String>;
}

/// HuggingFace API 服务
pub struct HuggingFaceService {
    api: Arc<Api>,
    endpoint: String,
}

impl HuggingFaceService {
    pub fn new() -> Self {
        let endpoint = std::env::var("HF_ENDPOINT")
            .unwrap_or_else(|_| "https://hf-mirror.com".to_string());
        
        // 设置环境变量
        std::env::set_var("HF_ENDPOINT", &endpoint);
        
        let api = ApiBuilder::new()
            .build()
            .unwrap_or_else(|_| ApiBuilder::new().build().unwrap());
        
        Self {
            api: Arc::new(api),
            endpoint,
        }
    }

    pub fn get_endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn build_resolve_url(&self, model_id: &str, branch: &str) -> String {
        format!("{}/{}/resolve/{}", self.endpoint, model_id, branch)
    }
}

impl ModelApiService for HuggingFaceService {
    fn get_model_files(&self, model_id: &str) -> Result<ModelFilesResult, String> {
        // 创建 repo 对象
        let repo = Repo::new(model_id.to_string(), RepoType::Model);
        
        // 获取模型信息
        let repo_info = self.api.repo(repo).info()
            .map_err(|e| format!("获取模型信息失败：{}", e))?;
        
        let mut files: Vec<FileInfo> = Vec::new();
        let mut total_size: u64 = 0;
        
        // 遍历文件列表
        for sibling in repo_info.siblings {
            let rfilename = sibling.rfilename;
            files.push(FileInfo {
                path: rfilename.clone(),
                size: 0, // hf-hub 的 siblings 没有 size 字段
            });
            // total_size += size; // 无法获取文件大小
        }
        
        if files.is_empty() {
            return Err("未找到模型文件".into());
        }
        
        Ok(ModelFilesResult {
            is_gguf: false,
            files,
            total_size, // 暂时为 0，需要通过其他方式获取
        })
    }
}

/// ModelScope API 服务
pub struct ModelScopeService;

impl ModelScopeService {
    pub fn new() -> Self {
        Self
    }
}

impl ModelApiService for ModelScopeService {
    fn get_model_files(&self, model_id: &str) -> Result<ModelFilesResult, String> {
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
        let mut total_size: u64 = 0;

        for file_entry in files_data {
            let file_type = file_entry.get("Type").and_then(|t| t.as_str()).unwrap_or("");
            if file_type == "blob" {
                if let (Some(path), Some(size)) = (
                    file_entry.get("Path").and_then(|p| p.as_str()),
                    file_entry.get("Size").and_then(|s| s.as_u64()),
                ) {
                    files.push(FileInfo {
                        path: path.to_string(),
                        size,
                    });
                    total_size += size;
                }
            }
        }

        if files.is_empty() {
            return Err("未找到模型文件".into());
        }

        Ok(ModelFilesResult {
            is_gguf: false,
            files,
            total_size,
        })
    }
}

/// 文件过滤器
pub struct FileFilter;

impl FileFilter {
    /// 根据模式过滤文件
    pub fn filter_files(files: &[FileInfo], patterns: &Option<Vec<String>>) -> Vec<FileInfo> {
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

    /// 获取 GGUF 文件过滤模式
    pub fn gguf_pattern(quant: &str) -> Vec<String> {
        vec![format!("*{}*.gguf", quant)]
    }

    /// 获取标准模型主要文件过滤模式
    pub fn main_files_pattern() -> Vec<String> {
        vec![
            "config.json".to_string(),
            "generation_config.json".to_string(),
            "tokenizer*.json".to_string(),
            "tokenizer.model".to_string(),
            "special_tokens_map.json".to_string(),
            "*.safetensors".to_string(),
            "*.safetensors.index.json".to_string(),
            "preprocessor_config.json".to_string(),
            "processor_config.json".to_string(),
        ]
    }
}
