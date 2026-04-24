#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    HuggingFace,
    ModelScope,
}

pub fn detect_platform(url: &str) -> Platform {
    let url = url.trim();
    if url.contains("huggingface.co") {
        Platform::HuggingFace
    } else if url.contains("modelscope.cn") {
        Platform::ModelScope
    } else {
        panic!("Unsupported platform for URL: {}", url);
    }
}

pub fn parse_repo_id(url: &str) -> (&str, String) {
    let url = url.trim();

    if url.contains("huggingface.co") {
        let path = url.split("huggingface.co/").last().unwrap_or("");
        let part = path.split(&[' ', '?'][..]).next().unwrap_or("");
        let parts: Vec<&str> = part.split('/').collect();
        if parts.len() >= 2 {
            return ("huggingface", format!("{}/{}", parts[0], parts[1]));
        }
    } else if url.contains("modelscope.cn") {
        let path = url.split("modelscope.cn/").last().unwrap_or("");
        let part = path.split(&[' ', '?'][..]).next().unwrap_or("");
        let parts: Vec<&str> = part.split('/').collect();
        if parts.len() >= 3 && parts[0] == "models" {
            return ("modelscope", format!("{}/{}", parts[1], parts[2]));
        } else if parts.len() >= 2 {
            return ("modelscope", format!("{}/{}", parts[0], parts[1]));
        }
    }

    panic!("Failed to parse repo ID from URL: {}", url);
}
