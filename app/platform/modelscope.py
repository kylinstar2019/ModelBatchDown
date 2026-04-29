from typing import Optional, Callable, List
import time
import os
import httpx
from modelscope.hub.snapshot_download import snapshot_download as ms_download


def download_ms_model(
    model_id: str,
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
            ms_download(
                model_id=model_id,
                local_dir=local_dir,
                allow_patterns=allow_patterns,
            )
            if not os.path.exists(local_dir):
                return f"❌ 下载失败：目录未创建 {local_dir}"
            files = [
                f
                for f in os.listdir(local_dir)
                if os.path.isfile(os.path.join(local_dir, f))
            ]
            subdirs = [
                d
                for d in os.listdir(local_dir)
                if os.path.isdir(os.path.join(local_dir, d))
            ]
            all_files = []
            for d in subdirs:
                for root, _, fs in os.walk(os.path.join(local_dir, d)):
                    all_files.extend([os.path.join(root, f) for f in fs])
            if not files and not all_files:
                return f"❌ 下载失败：未找到任何文件 {local_dir}"
            return f"✅ 魔塔模型下载完成：{model_id}"
        except httpx.HTTPError as e:
            if attempt == max_retries:
                return (
                    f"❌ 网络错误：无法连接魔塔模型，请检查网络或代理设置 (来源: {e})"
                )
            time.sleep(1.5)
            continue
        except Exception as e:
            if attempt == max_retries:
                return f"❌ 下载失败：{e}"
            time.sleep(1.5)
            continue
    return f"❌ 下载失败：未知错误"
