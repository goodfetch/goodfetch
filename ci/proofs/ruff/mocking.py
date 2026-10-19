"""Imports what ruff's TID251 must refuse (ADR 0062); ci/proofs/run.sh expects each."""

from unittest.mock import MagicMock

from _pytest.monkeypatch import MonkeyPatch as PrivateMonkeyPatch
from pytest import MonkeyPatch

__all__ = ["MagicMock", "MonkeyPatch", "PrivateMonkeyPatch"]
