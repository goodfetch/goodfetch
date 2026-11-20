"""A scope's settings in code (ADR 0083) on an Attached engine.

`connect()`'s own reach the engine in `hello`, a Run's own sit over them in
their three states, a key left out inheriting, `None` clearing and a value
setting, and `explain_settings()` on the Connection and on the Run answers
what each runs with; each setting the engine refuses says how to fix it, a
configuration-only key passed in code naming the file that sets it. The
cases play on the Scripted engine started as the executable, which checks
every message the SDK sends, three ways: async on asyncio, async on Trio
and through the sync `Collie`. TypeScript's `settings.test.ts` is its twin.
"""

from collections.abc import Sequence
from datetime import timedelta

import pytest
from collie_calls import engine_warnings, failure
from goodfetch_collie import (
    AsyncCollie,
    Collie,
    ExplainedSetting,
    Settings,
    SettingsLayer,
)
from goodfetch_collie_client.generated.errors import (
    ProgramError,
    RefusedError,
    SettingInvalidError,
    SettingLockedError,
    SettingNotApplicableError,
    SettingUnknownError,
)
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

HOME = "/home/user/.config/goodfetch/1"
"""The Scripted engine's per-major home, which holds both settings files."""


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


@pytest.mark.covers(
    "story:11-71",
    "story:11-76",
    "error:setting_not_applicable",
    "error:setting_unknown",
    "error:setting_invalid",
    "error:setting_locked",
)
@pytest.mark.client_case(id="settings-refusals-say-how-to-fix", mode="attached")
@three_ways
async def test_each_setting_the_engine_refuses_says_how_to_fix_it(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A configuration-only key in code names its file; each refusal its fix."""
    refused_env = scripted_engine_env(
        test_env,
        product,
        "sdk-python/run/settings-configuration-only-key-refused.json",
    )
    configuration_only: dict[str, object] = {"storage": {"low_free": 1_000}}
    not_applicable = await failure(
        way.open(
            lambda: AsyncCollie.connect(
                env=refused_env,
                settings=configuration_only,  # type: ignore[arg-type]
            ),
            lambda: Collie.connect(
                env=refused_env,
                settings=configuration_only,  # type: ignore[arg-type]
            ),
        )
    )
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/settings-refusals-say-how-to-fix.json"
    )
    collie = await way.open(
        lambda: AsyncCollie.connect(env=env), lambda: Collie.connect(env=env)
    )
    explained = await collie.explain_settings(["storage.low_free"])
    misspelled: dict[str, object] = {"timeout": {"navigations": 1_000}}
    unknown = await failure(collie.start(settings=misspelled))  # type: ignore[arg-type]
    wrong_type: dict[str, object] = {"timeout": {"wait": "soon"}}
    invalid = await failure(collie.start(settings=wrong_type))  # type: ignore[arg-type]
    locked = await failure(
        collie.start(settings={"timeout": {"navigation": timedelta(seconds=1)}})
    )
    await collie.close()

    with client_case.check(
        "a configuration-only key passed in code is refused at connect() "
        "with setting_not_applicable naming its file"
    ):
        assert (
            type(not_applicable),
            isinstance(not_applicable, ProgramError),
            not_applicable.details.get("key"),
            not_applicable.fix,
        ) == (
            SettingNotApplicableError,
            True,
            "storage.low_free",
            f"set `storage.low_free` in user configuration, "
            f"`{HOME}/configuration.toml`, or an Operator policy, "
            f"`{HOME}/operator-policy.toml`",
        )
    with client_case.check(
        "explain_settings() still reports the configuration-only key, from its file"
    ):
        assert [
            (each.key, each.value, each.layer) for each in explained.settings or []
        ] == [("storage.low_free", 1_000, "user_configuration")]
    with client_case.check("a misspelled key names its closest neighbour"):
        assert (
            type(unknown),
            isinstance(unknown, ProgramError),
            unknown.details.get("closest"),
        ) == (SettingUnknownError, True, "timeout.navigation")
    with client_case.check(
        "a value of the wrong type names the type the setting takes"
    ):
        assert (
            type(invalid),
            isinstance(invalid, ProgramError),
            "expected a whole number of milliseconds"
            in str(invalid.details.get("reason")),
        ) == (SettingInvalidError, True, True)
    with client_case.check("a locked key names the layer that locked it"):
        assert locked.limit is not None
        assert (
            type(locked),
            isinstance(locked, RefusedError),
            (locked.limit.key, locked.limit.value, locked.limit.layer),
        ) == (
            SettingLockedError,
            True,
            ("timeout.navigation", 10_000, SettingsLayer.operator_policy),
        )
    with client_case.check(
        "the engine read each message as the script expects, so it wrote no failure"
    ):
        assert engine_warnings(caplog) == []
