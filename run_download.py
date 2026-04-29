import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app.env_setup import *
from app.config import config
from app.platform.router import download_model, parse_url_entry


def extract_model_name(url):
    url = url.strip()
    if "huggingface.co" in url:
        path_part = url.split("huggingface.co/")[-1].split(" ")[0].split("?")[0]
        parts = path_part.split("/")
        return parts[1] if len(parts) >= 2 else None
    elif "modelscope.cn" in url:
        path_part = url.split("modelscope.cn/")[-1].split(" ")[0].split("?")[0]
        parts = path_part.split("/")
        if parts[0] == "models" and len(parts) >= 3:
            return parts[2]
        elif len(parts) >= 2:
            return parts[1]
        return None
    return None


def main():
    parser = argparse.ArgumentParser(description="ModelBatchDown CLI")
    parser.add_argument("--urls", required=True, help="模型链接，用换行分隔")
    parser.add_argument("--save-root", required=True, help="保存根目录")
    parser.add_argument(
        "--gguf-quant", default="", help="GGUF量化版本(已弃用，使用URL编码)"
    )
    parser.add_argument("--auto-shutdown", default="0", help="完成后自动关机 1/0")
    args = parser.parse_args()

    raw_urls = [u.strip() for u in args.urls.splitlines() if u.strip()]
    save_root = args.save_root.strip()
    auto_shutdown = args.auto_shutdown == "1"

    if not raw_urls:
        print("[ERROR] 请输入至少一个模型链接", flush=True)
        return

    if not save_root:
        print("[ERROR] 请输入保存目录", flush=True)
        return

    if not os.path.exists(save_root):
        os.makedirs(save_root)

    total = len(raw_urls)

    for idx, raw_url in enumerate(raw_urls, 1):
        url, gguf_quant, mode = parse_url_entry(raw_url)
        model_name = extract_model_name(url)

        if model_name:
            save_path = os.path.join(save_root, model_name)
            print(
                f"[INFO] -------- 第 {idx}/{total} 个任务 ({model_name}) --------",
                flush=True,
            )
        else:
            save_path = os.path.join(save_root, f"model_{idx}")
            print(f"[INFO] -------- 第 {idx}/{total} 个任务 --------", flush=True)

        mode_str = f" [模式: {mode}]" if mode != "all" else ""
        quant_str = f" [量化: {gguf_quant}]" if gguf_quant else ""
        print(f"[INFO] SAVE_PATH:{save_path}{mode_str}{quant_str}", flush=True)

        try:
            result = download_model(url, save_path, gguf_quant, None, mode)
            print(result, flush=True)
        except Exception as e:
            print(f"[ERROR] 错误：{e}", flush=True)

    if auto_shutdown:
        from app.utils.shutdown import schedule_shutdown

        schedule_shutdown(60)
        print("[INFO] 所有任务下载完成，准备自动关机...", flush=True)

    print("[DONE]", flush=True)


if __name__ == "__main__":
    main()
