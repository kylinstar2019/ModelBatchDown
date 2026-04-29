from datetime import datetime
from typing import List

class DownloadLogger:
    def __init__(self):
        self.logs: List[str] = []

    def log(self, message: str, level: str = "INFO"):
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        entry = f"[{timestamp}] {level} - {message}"
        self.logs.append(entry)
        print(entry)

    def info(self, message: str):
        self.log(message, "INFO")

    def error(self, message: str):
        self.log(message, "ERROR")

    def success(self, message: str):
        self.log(message, "SUCCESS")

    def get_logs(self) -> str:
        return "\n".join(self.logs)

    def clear(self):
        self.logs.clear()
