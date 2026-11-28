"""ScrapeGraphAI bench adapter entrypoint (ADR 0025, issue 183, fake model in CI)."""

from __future__ import annotations

import runpy
import sys
from pathlib import Path

if __name__ == "__main__":
    here = Path(__file__).resolve().parent
    shared = here.parent / "_shared" / "http_adapter.py"
    sys.argv = [str(shared), "--config", str(here / "configuration.json")]
    runpy.run_path(str(shared), run_name="__main__")
