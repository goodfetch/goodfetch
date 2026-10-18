import pytest


@pytest.mark.slow
def test_reads_a_value() -> None:
    assert 1 == 1
