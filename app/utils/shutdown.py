import subprocess
import platform

def schedule_shutdown(delay_seconds: int = 60):
    system = platform.system().lower()

    if system == "windows":
        subprocess.run(
            ["shutdown", "/s", "/t", str(delay_seconds)],
            shell=True,
            check=False
        )
    elif system in ("linux", "darwin"):
        subprocess.run(
            ["shutdown", "-h", f"+{delay_seconds // 60}"],
            check=False
        )

def cancel_shutdown():
    system = platform.system().lower()

    if system == "windows":
        subprocess.run(
            ["shutdown", "/a"],
            shell=True,
            check=False
        )
    elif system in ("linux", "darwin"):
        subprocess.run(
            ["shutdown", "-c"],
            check=False
        )
