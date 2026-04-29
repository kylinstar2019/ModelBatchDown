import os
import time
from functools import wraps
from typing import Callable, Any

def retry_on_failure(max_retries: int = 3, delay: int = 5):
    def decorator(func: Callable) -> Callable:
        @wraps(func)
        def wrapper(*args, **kwargs) -> Any:
            for attempt in range(max_retries):
                try:
                    return func(*args, **kwargs)
                except Exception as e:
                    if attempt < max_retries - 1:
                        time.sleep(delay)
                        continue
                    raise
        return wrapper
    return decorator
