from typing import Optional, Callable, List, Tuple
import re
from app.platform.huggingface import download_hf_model
from app.platform.modelscope import download_ms_model

MAIN_PATTERNS = [
    "config.json",
    "generation_config.json",
    "tokenizer*.json",
    "tokenizer.model",
    "special_tokens_map.json",
    "*.safetensors",
    "*.safetensors.index.json",
    "preprocessor_config.json",
    "processor_config.json",
]


def parse_url_entry(entry):
    entry = entry.strip()
    if "::" not in entry:
        return entry, None, "all"

    url, suffix = entry.split("::", 1)
    if suffix.startswith("QUANT:"):
        return url, suffix[6:], "quant"
    elif suffix.startswith("MODE:"):
        return url, None, suffix[5:]
    else:
        return url, suffix, "quant"


def get_allow_patterns(
    url: str, gguf_quant: Optional[str], mode: str = "all"
) -> Optional[List[str]]:
    is_gguf = "gguf" in url.lower()

    if is_gguf:
        if gguf_quant:
            return [f"*{gguf_quant}*.gguf"]
        return ["*.gguf"]

    if mode == "main":
        return MAIN_PATTERNS

    return None


def download_model(
    repo_url: str,
    save_path: str,
    gguf_quant: Optional[str] = None,
    progress_callback: Optional[Callable] = None,
    mode: str = "all",
) -> str:
    repo_url = repo_url.strip()
    is_gguf = "gguf" in repo_url.lower()

    if is_gguf:
        allow_patterns = [f"*{gguf_quant}*.gguf"] if gguf_quant else None
    elif mode == "main":
        allow_patterns = MAIN_PATTERNS
    else:
        allow_patterns = None

    if "huggingface.co" in repo_url:
        path_part = repo_url.split("huggingface.co/")[-1].split(" ")[0].split("?")[0]
        parts = path_part.split("/")
        repo_id = f"{parts[0]}/{parts[1]}"
        return download_hf_model(repo_id, save_path, allow_patterns, progress_callback)

    elif "modelscope.cn" in repo_url:
        path_part = repo_url.split("modelscope.cn/")[-1].split(" ")[0].split("?")[0]
        parts = path_part.split("/")
        if parts[0] == "models" and len(parts) >= 3:
            repo_id = f"{parts[1]}/{parts[2]}"
        else:
            repo_id = f"{parts[0]}/{parts[1]}"
        return download_ms_model(repo_id, save_path, allow_patterns, progress_callback)

    else:
        raise ValueError(f"❌ 不支持的链接：{repo_url}")
