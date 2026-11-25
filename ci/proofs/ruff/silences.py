"""Swallows two exceptions and keeps dead code, which ruff must refuse.

ci/proofs/run.sh expects one S110, one S112 and one ERA001.
"""


def swallows_and_passes(text: str) -> int:
    """Hides a failed parse behind `pass` (S110)."""
    number = 0
    try:
        number = int(text)
    except Exception:
        pass
    return number


def swallows_and_continues(texts: list[str]) -> int:
    """Hides each failed parse behind `continue` (S112)."""
    total = 0
    for text in texts:
        try:
            total += int(text)
        except Exception:
            continue
    return total


# total = swallows_and_continues(["1", "two"])
