import os
from pathlib import Path

def ensure_save_path(base_path: str, task_index: int) -> str:
    path = Path(base_path) / f"model_{task_index}"
    path.mkdir(parents=True, exist_ok=True)
    return str(path)

def validate_path(path: str) -> bool:
    try:
        Path(path)
        return True
    except Exception:
        return False

def get_available_disk_space(path: str) -> int:
    try:
        stat = os.statvfs(path)
        return stat.f_bavail * stat.f_frsize
    except Exception:
        return 0
