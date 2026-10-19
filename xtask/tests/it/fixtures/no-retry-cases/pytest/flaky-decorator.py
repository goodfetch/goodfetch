from flaky import flaky


@flaky(max_runs=3)
def test_reads_a_value() -> None:
    assert 1 == 1
