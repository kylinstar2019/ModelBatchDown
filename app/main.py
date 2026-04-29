from app.env_setup import *

import gradio as gr
import os
import json
from app.config import config

GGUF_QUANT_VERSIONS = config.GGUF_QUANT_VERSIONS

JS_INVOKE_START = """
async function startDownload(urls, saveRoot, ggufQuant, autoShutdown) {
    if (!urls || !urls.trim()) return '❌ 请输入至少一个模型链接';
    if (!saveRoot || !saveRoot.trim()) return '❌ 请输入或选择保存目录';
    try {
        const result = await window.__TAURI__.core.invoke('start_download', {
            urls: urls,
            saveRoot: saveRoot,
            ggufQuant: ggufQuant,
            autoShutdown: autoShutdown
        });
        logListener = true;
        return result;
    } catch(e) { return '❌ ' + e; }
}
"""

JS_INVOKE_STOP = """
async function stopDownload() {
    try {
        const result = await window.__TAURI__.core.invoke('stop_download');
        logListener = false;
        return result;
    } catch(e) { return '❌ ' + e; }
}
"""

JS_LISTEN_LOG = """
let logBuffer = '';
let logListener = false;

function setupLogListener() {
    if (logListener) return;
    const unlisten = window.__TAURI__.event.listen('download-log', (event) => {
        logBuffer += event.payload + '\\n';
        const el = document.getElementById('log_output');
        if (el) el.value = logBuffer;
    });
    window.__TAURI__.event.listen('download-finished', () => {
        logListener = false;
        logBuffer += '\\n✅ 所有下载任务完成\\n';
        const el = document.getElementById('log_output');
        if (el) el.value = logBuffer;
    });
    logListener = true;
}
"""

with gr.Blocks(title="ModelBatchDown - HF & 魔塔模型批量下载器", css="""
    #log_output textarea { height: calc(12 * 1.5em + 90px) !important; font-family: Consolas, monospace; }
""") as demo:
    gr.Markdown("# 🚀 ModelBatchDown（模型批量下载工具）")
    gr.Markdown("### 🔧 支持 HF/魔塔双平台 | GGUF量化选择 | 实时日志 | 自动关机")

    with gr.Row():
        with gr.Column(scale=2):
            gguf_quant = gr.Dropdown(
                choices=GGUF_QUANT_VERSIONS,
                value="Q4_K_M",
                label="GGUF 量化版本（自动过滤）"
            )
            urls_text = gr.Textbox(
                lines=6,
                label="📥 批量模型链接（一行一个）",
                placeholder="例如：\nhttps://huggingface.co/unsloth/LTX-2.3-GGUF\nhttps://modelscope.cn/xxx/yyy"
            )
            save_root = gr.Textbox(
                label="📁 保存根目录",
                lines=2,
                placeholder="请输入保存路径，例如：D:/Downloads/Models",
                info="程序会自动创建目录（如果不存在）"
            )
            auto_shutdown = gr.Checkbox(
                label="🔌 全部任务下载完成后自动关机（需等待60秒）",
                value=False
            )

        with gr.Column(scale=1):
            log_output = gr.Textbox(
                lines=14,
                label="📜 运行日志",
                interactive=False,
                elem_id="log_output"
            )

    with gr.Row():
        run_btn = gr.Button("▶️ 开始批量下载", variant="primary")
        stop_btn = gr.Button("⏹️ 停止下载", variant="stop")

    run_btn.click(
        fn=None,
        inputs=[urls_text, save_root, gguf_quant, auto_shutdown],
        outputs=[log_output],
        js=JS_INVOKE_START
    )

    stop_btn.click(
        fn=None,
        inputs=[],
        outputs=[log_output],
        js=JS_INVOKE_STOP
    )

    demo.load(
        fn=None,
        inputs=[],
        outputs=[],
        js=JS_LISTEN_LOG
    )


def run_gradio_server():
    demo.launch(
        server_name="127.0.0.1",
        server_port=7860,
        inbrowser=False
    )


if __name__ == "__main__":
    run_gradio_server()
    print("Gradio 服务已启动：http://127.0.0.1:7860")
    input("按回车退出\n")