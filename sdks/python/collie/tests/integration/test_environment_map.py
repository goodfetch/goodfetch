"""`connect()`'s environment map (ADR 0083) on an Attached engine.

The Scripted engine, started as the executable, writes the environment it
was started with: it is exactly the `env` map, and with none the process
environment, shown in a program the test starts with an environment of its
own, so no test patches its own (ADR 0062). A `GOODFETCH_` variable off
the list, an empty one and one the engine cannot parse fail `connect()`
before any engine starts. Three ways: async on asyncio, async on Trio and
through the sync `Collie`, the program too. TypeScript's
`environment-map.test.ts` is its twin.
"""

import functools
import json
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import anyio
import pytest
from collie_calls import failure
from goodfetch_collie import AsyncCollie, Collie
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    scripted_engine_environment,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from collections.abc import Mapping

SCRIPT = "sdk-python/connect/reached-attached.json"
"""The Attached engine's script: `hello`, then the Client closes."""

MARK = "ENVIRONMENT_MAP_MARK"
"""A variable outside the reserved prefix, which the map carries as it is."""

PROGRAM = Path(__file__).parents[1] / "programs" / "collie_environment.py"
"""The program that connects with no `env`."""

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long the program may take to connect and close."""

REFUSALS: dict[str, dict[str, str]] = {
    "unknown": {"GOODFETCH_HOEM": "/home/goodfetch"},
    "empty": {"GOODFETCH_URL": ""},
    "channel": {"GOODFETCH_CHANNEL": "nightly"},
    "drain_grace": {"GOODFETCH_DAEMON_DRAIN_GRACE": "5s"},
}
"""Each refused variable, by the home its `connect()` would have started in."""


@pytest.mark.covers("story:11-73", "error:setting_unknown", "error:setting_invalid")
@pytest.mark.client_case(id="connect-takes-the-environment-map", mode="attached")
@three_ways
async def test_the_engine_gets_exactly_the_env_map_or_the_process_environment(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
) -> None:
    """And a bad `GOODFETCH_` variable fails `connect()` before any engine."""
    given_home = test_env.folder / "given"
    given = {
        **scripted_engine_env(test_env, product, SCRIPT, given_home),
        MARK: "given",
    }
    collie = await way.open(
        lambda: AsyncCollie.connect(env=given), lambda: Collie.connect(env=given)
    )
    await collie.close()

    process_home = test_env.folder / "process"
    out = test_env.folder / "environment.json"
    program_env = {
        **scripted_engine_env(test_env, product, SCRIPT, process_home),
        MARK: "process",
    }
    with anyio.fail_after(PROGRAM_DEADLINE_SECONDS):
        program = await anyio.run_process(
            [sys.executable, str(PROGRAM), str(out), way.value],
            cwd=test_env.folder,
            env=program_env,
            check=False,
        )

    refused: dict[str, tuple[str, Mapping[str, object], dict[str, str] | None]] = {}
    for name, variable in REFUSALS.items():
        home = test_env.folder / name
        env = {**scripted_engine_env(test_env, product, SCRIPT, home), **variable}
        error = await failure(
            way.open(
                functools.partial(AsyncCollie.connect, env=env),
                functools.partial(Collie.connect, env=env),
            )
        )
        refused[name] = (error.code, error.details, scripted_engine_environment(home))

    with client_case.check("the engine was started with exactly the env map"):
        assert scripted_engine_environment(given_home) == given
    with client_case.check("the program with no env map ended with exit code 0"):
        assert program.returncode == 0, program.stderr.decode(errors="replace")
    with client_case.check(
        "with no env map, the engine was started with the process environment"
    ):
        own = json.loads(out.read_text("utf-8"))
        assert (own[MARK], own["GOODFETCH_HOME"]) == ("process", str(process_home))
        assert scripted_engine_environment(process_home) == own
    with client_case.check(
        "a GOODFETCH_ variable off the list is setting_unknown, naming the closest"
    ):
        assert refused["unknown"][:2] == (
            "setting_unknown",
            {"key": "GOODFETCH_HOEM", "closest": "GOODFETCH_HOME"},
        )
    with client_case.check(
        "an empty value and one the engine cannot parse are setting_invalid"
    ):
        assert [
            (code, details.get("key"))
            for name, (code, details, _) in refused.items()
            if name != "unknown"
        ] == [
            ("setting_invalid", "GOODFETCH_URL"),
            ("setting_invalid", "GOODFETCH_CHANNEL"),
            ("setting_invalid", "GOODFETCH_DAEMON_DRAIN_GRACE"),
        ]
    with client_case.check("no refused connect() started an engine"):
        started = {name: environment for name, (_, _, environment) in refused.items()}
        assert started == {
            "unknown": None,
            "empty": None,
            "channel": None,
            "drain_grace": None,
        }
