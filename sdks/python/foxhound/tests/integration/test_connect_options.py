"""What `connect()` takes (ADRs 0083, 0088 and 0104; issue 3310).

On an Attached engine of Foxhound's executable of the same commit, three
ways: the program's own Fetch backends and Sinks, registered by name,
coroutine functions on `AsyncFoxhound` and plain functions on the sync
`Foxhound`, and the Connection's settings, Collie's and Foxhound's, from
which a key only configuration sets is refused, naming the files that set
it, while `explain_settings()` still reports it. `tests/types` holds what
the type checkers refuse. TypeScript's `connect-options.test.ts` of
Foxhound's package is its twin.
"""

from __future__ import annotations

import re
from typing import TYPE_CHECKING

import pytest
from collie_calls import failure
from goodfetch_collie_client.generated.errors import (
    ProgramError,
    SettingNotApplicableError,
)
from goodfetch_foxhound import (
    AsyncFoxhound,
    BackendRequest,
    BackendResponse,
    Foxhound,
    SinkDelivery,
)
from goodfetch_test_support import Product as ToolProduct
from goodfetch_test_support import Way, three_ways

if TYPE_CHECKING:
    from collections.abc import Mapping

FIX = re.compile(
    r"set `storage\.low_free` in user configuration, "
    r"`<home>/[^/`]+/configuration\.toml`, "
    r"or an Operator policy, `<home>/[^/`]+/operator-policy\.toml`"
)
"""A configuration-only key's fix: its files, in the home's folder of its major."""


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


def _empty_page(request: BackendRequest) -> BackendResponse:
    """A Fetch backend that answers every request with an empty page."""
    return BackendResponse(
        status=200,
        headers={"content-type": "text/html", "x-url": request.url},
        body=b"",
    )


async def _async_empty_page(request: BackendRequest) -> BackendResponse:
    """The coroutine twin of `_empty_page`."""
    return _empty_page(request)


def _discard(delivery: SinkDelivery) -> None:
    """A Sink that keeps nothing."""
    del delivery


async def _async_discard(delivery: SinkDelivery) -> None:
    """The coroutine twin of `_discard`."""
    del delivery


@pytest.mark.covers("story:11-1008")
@three_ways
async def test_accepts_the_programs_own_fetch_backends_and_sinks_each_by_name(
    way: Way, env: Mapping[str, str]
) -> None:
    """`connect()` takes its kind of handler for each way, and Foxhound's settings."""
    foxhound = await way.open(
        lambda: AsyncFoxhound.connect(
            env=env,
            fetch_backends={"empty-page": _async_empty_page},
            sinks={"discard": _async_discard},
            settings={"crawl": {"max_depth": 2}},
        ),
        lambda: Foxhound.connect(
            env=env,
            fetch_backends={"empty-page": _empty_page},
            sinks={"discard": _discard},
            settings={"crawl": {"max_depth": 2}},
        ),
    )
    explained = await foxhound.explain_settings(["crawl.max_depth"])
    await foxhound.close()

    assert [
        (each.key, each.value, each.layer) for each in explained.settings or []
    ] == [("crawl.max_depth", 2, "run")]


@pytest.mark.covers("story:11-1009", "error:setting_not_applicable")
@three_ways
async def test_refuses_a_configuration_only_key_naming_its_files_and_still_explains_it(
    way: Way, env: Mapping[str, str]
) -> None:
    """A key only configuration sets is refused at `connect()`, yet explained."""
    configuration_only: dict[str, object] = {"storage": {"low_free": 1_000}}
    refused = await failure(
        way.open(
            lambda: AsyncFoxhound.connect(
                env=env,
                settings=configuration_only,  # type: ignore[arg-type]
            ),
            lambda: Foxhound.connect(
                env=env,
                settings=configuration_only,  # type: ignore[arg-type]
            ),
        )
    )
    foxhound = await way.open(
        lambda: AsyncFoxhound.connect(env=env), lambda: Foxhound.connect(env=env)
    )
    explained = await foxhound.explain_settings(["storage.low_free"])
    await foxhound.close()

    assert (
        type(refused),
        isinstance(refused, ProgramError),
        refused.code,
        refused.details.get("key"),
    ) == (SettingNotApplicableError, True, "setting_not_applicable", "storage.low_free")
    assert FIX.fullmatch((refused.fix or "").replace(env["GOODFETCH_HOME"], "<home>"))
    assert [(each.key, each.layer) for each in explained.settings or []] == [
        ("storage.low_free", "default")
    ]
