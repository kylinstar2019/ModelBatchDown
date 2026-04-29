from typing import Optional, Callable, List
import time
import os
import httpx
from huggingface_hub import snapshot_download


HF_ENDPOINT = os.environ.get("HF_ENDPOINT", "https://hf-mirror.com")


def download_hf_model(
    repo_id: str,
    local_dir: str,
    allow_patterns: Optional[List[str]] = None,
    progress_callback: Optional[Callable] = None,
) -> str:
    try:
        os.makedirs(local_dir, exist_ok=True)
    except Exception as e:
        return f"❌ 下载失败：无法创建目录 {local_dir} ({e})"

    max_retries = 3
    for attempt in range(1, max_retries + 1):
        try:
            print(
                f"开始下载 {repo_id} 到 {local_dir} (尝试 {attempt}/{max_retries})",
                flush=True,
            )
            snapshot_download(
                repo_id=repo_id,
                local_dir=local_dir,
                resume_download=True,
                force_download=False,
                allow_patterns=allow_patterns,
                endpoint=HF_ENDPOINT,
            )

            if not os.path.exists(local_dir):
                return f"❌ 下载失败：无法创建目录 {local_dir}"

            downloaded_files = os.listdir(local_dir)
            if not downloaded_files:
                return f"❌ 下载失败：目录为空，请检查网络连接或模型是否存在"

            return f"✅ HF 模型下载完成：{repo_id}\n📁 已下载 {len(downloaded_files)} 个文件到 {local_dir}"
        except httpx.HTTPError as e:
            if attempt == max_retries:
                return f"❌ 网络错误：无法连接 HuggingFace，请检查网络或代理设置 (来源: {e})"
            time.sleep(1.5)
            continue
        except Exception as e:
            error_msg = str(e)
            if attempt == max_retries:
                if "404" in error_msg or "Repository not found" in error_msg:
                    return f"❌ 仓库不存在：{repo_id}，请确认模型名称是否正确"
                if isinstance(e, OSError) and e.errno is not None:
                    return f"❌ 下载失败：{error_msg}"
                return f"❌ 下载失败：{error_msg}"
            time.sleep(1.5)
            continue

    return f"❌ 下载失败：重试次数用尽"
