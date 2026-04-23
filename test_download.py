#!/usr/bin/env python3
import os
import sys

os.environ["HF_ENDPOINT"] = "https://hf-mirror.com"
os.environ["MS_ENDPOINT"] = "https://modelscope.cn"

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app.platform.router import download_model, validate_model_url

test_url = "https://huggingface.co/unsloth/Llama-3.2-1B-Instruct-GGUF"
test_save = "./test_download_model"

print("测试 URL 验证:")
result, msg = validate_model_url(test_url)
print(f"  URL: {test_url}")
print(f"  结果: {result}, 类型: {msg}")

print("\n测试下载 (小模型):")
try:
    result = download_model(test_url, test_save, "Q4_K_M")
    print(f"  返回结果: {result}")
except Exception as e:
    print(f"  错误: {e}")

if os.path.exists(test_save):
    files = os.listdir(test_save)
    print(f"  下载目录内容: {files}")
