"""`connect()` beside a busy Collie Daemon of the same commit (ADR 0098).

A Collie program on `engine="daemon"` starts Collie's executable as the
Daemon of the test's home and stays connected, so the Daemon is busy,
refuses a Foxhound Client by Product and cannot hand over; the Foxhound
program's environment points at Foxhound's own executable from the test
tool manifest, three ways. The Daemon is stopped while the Collie program
is still connected: once Foxhound's executable has run in the home, an idle
Collie Daemon would hand over to it. TypeScript's `collie-daemon.test.ts`
of Foxhound's package and the Rust crate's `tests/it/daemon.rs` are its
twins.
"""

from __future__ import annotations

import logging
from contextlib import asynccontextmanager
from typing import TYPE_CHECKING

import pytest
from collie_calls import CLIENT_LOGGER, exit_code, failure
from goodfetch_collie import AsyncCollie
from goodfetch_collie_client.generated.errors import NotInProductError
from goodfetch_foxhound import AsyncFoxhound, Foxhound
from goodfetch_test_support import Product as ToolProduct
from goodfetch_test_support import TestEnv, Way, three_ways

if TYPE_CHECKING:
    from collections.abc import AsyncGenerator, Mapping


@asynccontextmanager
async def _beside_a_busy_collie_daemon(
    env: Mapping[str, str], test_env: TestEnv
) -> AsyncGenerator[str]:
    """A Collie Daemon a Collie program keeps busy, stopped after; its URL."""
    busy = await AsyncCollie.connect(env=env, engine="daemon")
    try:
        yield busy.engine.url or ""
    finally:
        stop = test_env.start(
            [test_env.tools.path(ToolProduct.COLLIE.tool), "daemon", "stop"], env=env
        )
        stopped = await exit_code(lambda: stop.wait(30))
        await busy.close()
    assert stopped == 0


ENGINE_STDERR = "engine stderr"
"""The `source` of the warnings that carry the Attached engine's stderr lines."""


def _foxhound_env(test_env: TestEnv) -> dict[str, str]:
    """The test's environment, its engine path Foxhound's executable."""
    foxhound = ToolProduct.FOXHOUND
    return {
        **test_env.env,
        foxhound.engine_path_variable: str(test_env.tools.path(foxhound.tool)),
    }


def _restart(path: str) -> str:
    """The sentence a refusal's `fix` names Foxhound's `daemon restart` in."""
    return f"Run `{path} daemon restart` to restart the Daemon on its major's pick."


@pytest.mark.covers("story:11-1006", "error:not_in_product")
@three_ways
async def test_auto_runs_foxhounds_attached_engine_says_why_and_warns_once(
    way: Way,
    env: Mapping[str, str],
    test_env: TestEnv,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """At `auto`, the busy Collie Daemon's refusal leaves an Attached Foxhound."""
    caplog.set_level(logging.WARNING, logger=CLIENT_LOGGER)
    foxhound_env = _foxhound_env(test_env)
    path = foxhound_env[ToolProduct.FOXHOUND.engine_path_variable]

    async with _beside_a_busy_collie_daemon(env, test_env) as daemon_url:
        foxhound = await way.open(
            lambda: AsyncFoxhound.connect(env=foxhound_env),
            lambda: Foxhound.connect(env=foxhound_env),
        )
        engine = foxhound.engine
        await foxhound.close()

    assert (engine.kind, engine.product, engine.engine_path, engine.url) == (
        "attached",
        "foxhound",
        path,
        None,
    )
    assert engine.fallback is not None
    assert "not_in_product" in engine.fallback.cause
    assert engine.fallback.fix.endswith(_restart(path))
    [warning] = [
        record
        for record in caplog.records
        if record.name == CLIENT_LOGGER
        and record.levelno == logging.WARNING
        and record.__dict__.get("source") != ENGINE_STDERR
    ]
    assert daemon_url in warning.getMessage()
    assert warning.__dict__["fallback"] == engine.fallback


@pytest.mark.covers("story:11-1006", "error:not_in_product")
@three_ways
async def test_daemon_fails_with_not_in_product_its_fix_naming_foxhounds_restart(
    way: Way,
    env: Mapping[str, str],
    test_env: TestEnv,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """At `daemon`, the busy Collie Daemon's refusal fails the connect."""
    caplog.set_level(logging.WARNING, logger=CLIENT_LOGGER)
    foxhound_env = _foxhound_env(test_env)
    path = foxhound_env[ToolProduct.FOXHOUND.engine_path_variable]

    async with _beside_a_busy_collie_daemon(env, test_env):
        refused = await failure(
            way.open(
                lambda: AsyncFoxhound.connect(env=foxhound_env, engine="daemon"),
                lambda: Foxhound.connect(env=foxhound_env, engine="daemon"),
            )
        )

    assert (type(refused), refused.code) == (NotInProductError, "not_in_product")
    assert (refused.fix or "").endswith(_restart(path))
    assert [record for record in caplog.records if record.name == CLIENT_LOGGER] == []
