"""Imports what ruff's TID251 must refuse (ADR 0062); ci/proofs/run.sh expects both."""

from unittest.mock import MagicMock

from pytest import MonkeyPatch

__all__ = ["MagicMock", "MonkeyPatch"]
