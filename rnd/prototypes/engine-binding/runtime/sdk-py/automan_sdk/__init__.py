"""Thin Python SDK over the prototype engine: in-process (PyO3) or RPC (engined)."""
import time


def now_us():
    return time.time_ns() / 1000.0


def read_body(page):
    """Inline body if delivered, otherwise read from the content-addressed blob."""
    b = page.get("body")
    if b is not None:
        return b
    with open(page["blob"]["path"], "rb") as f:
        return f.read()
