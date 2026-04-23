import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app.env_setup import *
from app.main import run_gradio_server

if __name__ == "__main__":
    print("启动 Gradio 服务...")
    print("http://127.0.0.1:7860")
    run_gradio_server()