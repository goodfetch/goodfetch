"""A scope's settings in code (ADR 0083) on an Attached engine.

`connect()`'s own reach the engine in `hello`, a Run's own sit over them in
their three states, a key left out inheriting, `None` clearing and a value
setting, and `explain_settings()` on the Connection and on the Run answers
what each runs with. The case plays on the Scripted engine started as the
executable, which checks every message the SDK sends, three ways: async on
asyncio, async on Trio and through the sync `Collie`. TypeScript's
`settings.test.ts` is its twin.
"""

from collections.abc import Sequence
from datetime import timedelta

import pytest
from collie_calls import engine_warnings
from goodfetch_collie import AsyncCollie, Collie, ExplainedSetting, Settings
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

KEYS = ["checkpoint.interval", "timeout.navigation", "timeout.wait"]
"""The keys both explanations ask for, by their dotted names."""


def keyed(settings: Sequence[ExplainedSetting] | None) -> list[tuple[object, ...]]:
    """Each explained setting as its key, value, layer and origin."""
    return [(each.key, each.value, each.layer, each.origin) for each in settings or []]


@pytest.mark.covers("story:11-70", "story:11-72")
@pytest.mark.client_case(id="settings-three-states", mode="attached")
@three_ways
async def test_a_runs_own_settings_inherit_clear_and_set_over_its_connections(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """`explain_settings()` answers the Connection's, then the Run's over them."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/settings-three-states.json"
    )
    settings: Settings = {
        "timeout": {
            "navigation": timedelta(seconds=1),
            "wait": timedelta(seconds=3),
        },
        "checkpoint": {"interval": timedelta(minutes=1)},
    }
    collie = await way.open(
        lambda: AsyncCollie.connect(env=env, settings=settings),
        lambda: Collie.connect(env=env, settings=settings),
    )

    connection = await collie.explain_settings(KEYS)
    run = await collie.start(
        settings={
            "timeout": {"wait": timedelta(seconds=5)},
            "checkpoint": {"interval": None},
        }
    )
    own = await run.explain_settings(KEYS)
    await run.finish()
    await collie.close()

    with client_case.check("the Connection explains the settings its hello carried"):
        assert keyed(connection.settings) == [
            ("checkpoint.interval", 60_000, "run", "settings"),
            ("timeout.navigation", 1_000, "run", "settings"),
            ("timeout.wait", 3_000, "run", "settings"),
        ]
    with client_case.check(
        "the Run inherits what it leaves out, clears with None and sets with a value"
    ):
        assert keyed(own.settings) == [
            ("checkpoint.interval", None, "run", "settings"),
            ("timeout.navigation", 1_000, "run", "settings"),
            ("timeout.wait", 5_000, "run", "settings"),
        ]
    with client_case.check(
        "the engine read each message as the script expects, so it wrote no failure"
    ):
        assert engine_warnings(caplog) == []
