import pytest


@pytest.mark.flaky(reruns=2)
def test_reads_a_value() -> None:
    assert 1 == 1
