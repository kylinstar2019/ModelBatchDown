import os
import re
from typing import List, Optional, Callable
from app.platform.router import download_model
from app.utils.shutdown import schedule_shutdown

def extract_model_name_from_url(url: str) -> str:
    url = url.strip()
    if "huggingface.co" in url:
        path_part = url.split("huggingface.co/")[-1].split(" ")[0]
        parts = path_part.split("/")
        return parts[1] if len(parts) >= 2 else "model"
    elif "modelscope.cn" in url:
        path_part = url.split("modelscope.cn/")[-1].split(" ")[0]
        parts = path_part.split("/")
        if parts[0] == "models" and len(parts) >= 3:
            return parts[2]
        elif len(parts) >= 2:
            return parts[1]
        return "model"
    return "model"

def sanitize_dir_name(name: str) -> str:
    name = re.sub(r'[<>:"/\\|?*]', '_', name)
    name = name.strip('. ')
    if not name:
        name = "model"
    return name[:100]

class BatchDownloader:
    def __init__(self):
        self.tasks: List[str] = []
        self.current_index = 0

    def add_task(self, url: str):
        self.tasks.append(url.strip())

    def execute_single(self, url: str, save_path: str, gguf_quant: Optional[str], progress_callback: Optional[Callable] = None) -> str:
        url = url.strip()
        if not os.path.exists(save_path):
            os.makedirs(save_path)
        return download_model(url, save_path, gguf_quant, progress_callback)

    def execute(
        self,
        save_root: str,
        gguf_quant: Optional[str],
        auto_shutdown: bool,
        progress_callback: Optional[Callable] = None
    ) -> str:
        if not self.tasks:
            return "❌ 任务列表为空"

        results = []
        for idx, url in enumerate(self.tasks, 1):
            model_name = extract_model_name_from_url(url)
            safe_name = sanitize_dir_name(model_name)
            save_path = os.path.join(save_root, safe_name)

            if not os.path.exists(save_path):
                os.makedirs(save_path)

            result = download_model(
                url, save_path, gguf_quant, progress_callback
            )
            results.append(f"-------- 第 {idx} 个任务 --------\n{result}\n")

        if auto_shutdown:
            schedule_shutdown(60)
            results.append("\n🔌 所有任务下载完成，准备自动关机...")

        self.tasks.clear()
        return "\n".join(results)