"""The fixture Product's `tests/integration`, which the install harness runs unchanged."""

import goodfetch_collie
import pytest
from goodfetch_collie_client import greet


def test_the_client_it_pins_greets() -> None:
    """The pinned client is installed beside it."""
    assert greet("collie") == "hello, collie"


def test_its_executable_prints_its_name_and_version(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """Its entry point reads the installed version."""
    goodfetch_collie.main()

    assert capsys.readouterr().out == "goodfetch-collie 0.2.0\n"
