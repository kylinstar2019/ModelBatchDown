from dataclasses import dataclass
from typing import Optional, List

@dataclass
class DownloadConfig:
    HF_ENDPOINT: str = "https://hf-mirror.com"
    MS_ENDPOINT: str = "https://modelscope.cn"
    DEFAULT_SAVE_PATH: str = "./models"
    GGUF_QUANT_VERSIONS: List[str] = None

    def __post_init__(self):
        if self.GGUF_QUANT_VERSIONS is None:
            self.GGUF_QUANT_VERSIONS = [
                "Q3_K_M",
                "Q4_K_S",
                "Q4_K_M",
                "Q5_K_S",
                "Q5_K_M",
                "Q6_K",
                "Q8_0"
            ]

config = DownloadConfig()
