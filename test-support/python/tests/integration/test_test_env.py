"""The `test_env` fixture gives the setup `goodfetch-test-env` prepares (ADR 0062)."""

from datetime import UTC, datetime

from goodfetch_test_support import TestEnv


def test_test_env_gives_a_home_inside_a_folder_named_with_a_non_ascii_letter(
    test_env: TestEnv,
) -> None:
    """The folder holds the home, and its name holds a space and `é` on every OS."""
    assert " é " in test_env.folder.name
    assert test_env.home == test_env.folder / "home"
    assert test_env.home.is_dir()


def test_test_env_points_goodfetch_home_at_the_home(test_env: TestEnv) -> None:
    """The cleared environment map names the test's home as `GOODFETCH_HOME`."""
    assert test_env.env["GOODFETCH_HOME"] == str(test_env.home)


def test_test_env_starts_at_the_shared_start_time(test_env: TestEnv) -> None:
    """Every test starts at 2026-01-05T09:00:00Z, as in Rust."""
    assert test_env.start_time == datetime(2026, 1, 5, 9, 0, tzinfo=UTC)
