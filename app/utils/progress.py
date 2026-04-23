import time
import threading
from dataclasses import dataclass, field

@dataclass
class DownloadProgress:
    downloaded_bytes: int = 0
    total_bytes: int = 0
    speed_mbps: float = 0.0
    progress_percent: float = 0.0
    last_update_time: float = field(default_factory=time.time)
    last_downloaded: int = 0
    is_downloading: bool = False

    def update(self, downloaded: int, total: int):
        current_time = time.time()

        self.downloaded_bytes = downloaded
        self.total_bytes = total
        self.is_downloading = True

        if total > 0:
            self.progress_percent = (downloaded / total) * 100

        elapsed = current_time - self.last_update_time
        if elapsed >= 1.0 and downloaded > self.last_downloaded:
            bytes_delta = downloaded - self.last_downloaded
            self.speed_mbps = (bytes_delta / (1024 * 1024)) / elapsed
            self.last_update_time = current_time
            self.last_downloaded = downloaded

    def reset(self):
        self.downloaded_bytes = 0
        self.total_bytes = 0
        self.speed_mbps = 0.0
        self.progress_percent = 0.0
        self.last_update_time = time.time()
        self.last_downloaded = 0
        self.is_downloading = False


class ProgressMonitor:
    def __init__(self, progress_obj: DownloadProgress):
        self.progress_obj = progress_obj
        self._running = False
        self._thread = None

    def start(self):
        self._running = True
        self._thread = threading.Thread(target=self._monitor_loop, daemon=True)
        self._thread.start()

    def stop(self):
        self._running = False
        if self._thread:
            self._thread.join(timeout=2.0)

    def _monitor_loop(self):
        while self._running:
            time.sleep(0.5)


def create_download_callback(progress_obj: DownloadProgress):
    def callback(downloaded: int, total: int):
        progress_obj.update(downloaded, total)
    return callback
