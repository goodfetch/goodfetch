"""The `test_env` fixture gives the setup `goodfetch-test-env` prepares (ADR 0062)."""

import json
import os
import shutil
from datetime import UTC, datetime

from goodfetch_test_support import TestEnv, redact


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


def test_test_env_folder_redacts_as_written_canonical_and_in_json(
    test_env: TestEnv,
) -> None:
    """The real folder, with its space and `é`, redacts in each printed form."""
    output = " ".join(
        [
            f"{test_env.folder}/a",
            f"{os.path.realpath(test_env.folder)}/b",
            json.dumps({"path": str(test_env.folder / "c")}, ensure_ascii=False),
        ]
    )

    expected_path = json.dumps(
        {"path": os.path.join("[FOLDER]", "c")}, ensure_ascii=False
    )
    assert (
        redact(output, folder=test_env.folder)
        == f"[FOLDER]/a [FOLDER]/b {expected_path}"
    )


def test_teardown_of_a_failed_test_keeps_its_folder() -> None:
    """A failed test's folder stays for its evidence, as a Rust test's does."""
    setup = TestEnv.open(
        "a failed test", {"GOODFETCH_TEST_TOOLS": os.environ["GOODFETCH_TEST_TOOLS"]}
    )
    folder = setup.folder

    setup.close(failed=True)

    assert folder.is_dir()
    shutil.rmtree(folder)
