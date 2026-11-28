"""J-013, one-off script on an Attached engine, then two scripts sharing the Daemon.

"Run a one-off SDK script on an Attached engine, then two scripts that share
the Daemon", on the `sdk-python` surface (ADRs 0063, 0075 and 0094): a
one-off script with no Daemon running starts an Attached engine over stdio
that stops when it ends without starting a Daemon; then once the Daemon is
started on the CLI, two scripts run at once and share that one Daemon
without starting an Attached engine. Rust's `tests/it/journeys/j013.rs` and
TypeScript's `tests/journeys/J-013.test.ts` are its twins.
"""

import json
from collections.abc import Mapping
from dataclasses import dataclass
from typing import cast

import anyio
import anyio.to_thread
from goodfetch_collie import AsyncCollie, EngineInfo
from goodfetch_collie_client.generated.models import PageField
from goodfetch_test_support import JourneyRun, journey

PRODUCT_FIELDS = {
    "id": PageField.model_validate({"locator": {"kind": "css", "selector": "span.id"}}),
    "name": PageField.model_validate(
        {"locator": {"kind": "css", "selector": "h1.name"}}
    ),
    "price": PageField.model_validate(
        {
            "locator": {"kind": "css", "selector": "span.price"},
            "attribute": "data-price",
            "transform": {"kind": "number", "locale": "en"},
        }
    ),
}
"""The product's ID, name and price, as task `catalog/2` names them."""


@dataclass(frozen=True)
class _ScriptOutcome:
    engine: EngineInfo
    record: Mapping[str, object]


async def _cli_json(run: JourneyRun, args: list[str]) -> Mapping[str, object]:
    """Runs Collie's CLI with `args` and `--json`, requiring exit 0 and JSON output."""
    proc = run.command([*args, "--json"])
    stdout, stderr = await anyio.to_thread.run_sync(proc.communicate)
    assert proc.returncode == 0, (
        f"collie {' '.join(args)} exited with {proc.returncode}: "
        f"{stderr.decode('utf-8', errors='replace')}"
    )
    parsed = json.loads(stdout.decode("utf-8"))
    assert isinstance(parsed, dict), f"expected JSON object, got: {parsed!r}"
    return cast("Mapping[str, object]", parsed)


async def _extract_product(env: Mapping[str, str], product_url: str) -> _ScriptOutcome:
    """Runs one SDK script that extracts the catalog's product page and closes."""
    collie = await AsyncCollie.connect(env=env)
    try:
        engine = collie.engine
        started = await collie.start()
        page = await started.new_page()
        await page.goto(product_url)
        read = await page.read(PRODUCT_FIELDS)
        await started.finish()
        return _ScriptOutcome(engine=engine, record=read.values)
    finally:
        await collie.close()


@journey("J-013", "sdk-python", browser=True)
async def test_j013(run: JourneyRun) -> None:
    """Covers: row:RT-19, story:3-1, story:3-2."""
    proving_ground = run.proving_ground
    assert proving_ground is not None, "J-013 has a Proving ground"
    product = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/2")
    daemon_started = False
    status_after_one_off: Mapping[str, object] | None = None
    daemon_start_pid: object = None
    status_after_shared: Mapping[str, object] | None = None

    try:

        async def run_one_off() -> _ScriptOutcome:
            nonlocal status_after_one_off
            outcome = await _extract_product(run.env, product)
            status_after_one_off = await _cli_json(run, ["daemon", "status"])
            return outcome

        one_off = await run.step(
            "With no Daemon running and nothing configured, run a one-off script "
            "that extracts the catalog's product page",
            run_one_off,
        )

        async def start_daemon() -> None:
            nonlocal daemon_started, daemon_start_pid
            started = await _cli_json(run, ["daemon", "start"])
            daemon_started = True
            daemon_start_pid = started["pid"]
            assert isinstance(daemon_start_pid, int), f"{started!r}"

        await run.step("Start the Daemon", start_daemon)

        async def run_two_shared() -> tuple[_ScriptOutcome, _ScriptOutcome]:
            nonlocal status_after_shared
            results: list[_ScriptOutcome | None] = [None, None]

            async def worker(index: int) -> None:
                results[index] = await _extract_product(run.env, product)

            async with anyio.create_task_group() as group:
                group.start_soon(worker, 0)
                group.start_soon(worker, 1)

            status_after_shared = await _cli_json(run, ["daemon", "status"])
            first, second = results[0], results[1]
            assert first is not None and second is not None
            return first, second

        shared = await run.step(
            "Run two scripts at once, each extracting the catalog's product page",
            run_two_shared,
        )

        async def check_one_off() -> None:
            assert one_off is not None
            assert status_after_one_off is not None
            assert one_off.engine.kind == "attached"
            assert one_off.engine.engine_path is not None
            assert status_after_one_off["pid"] is None

        await run.check(
            "The one-off script ran on an Attached engine it started over stdio, "
            "which stopped when the script ended, and no Daemon was started",
            check_one_off,
        )

        await run.expect_task(
            "The oracle verifies the one-off script's result",
            {
                "status": "success",
                "records": [] if one_off is None else [one_off.record],
            },
        )

        async def check_shared() -> None:
            assert shared is not None
            assert status_after_shared is not None
            first, second = shared
            assert first.engine.kind == "daemon"
            assert second.engine.kind == "daemon"
            assert first.engine.engine_path is None
            assert second.engine.engine_path is None
            assert first.engine.url is not None
            assert second.engine.url == first.engine.url
            assert status_after_shared["pid"] == daemon_start_pid

        await run.check(
            "Both scripts ran on the one running Daemon, and neither started an "
            "Attached engine",
            check_shared,
        )

        await run.expect_task(
            "The oracle verifies the first script's result",
            {
                "status": "success",
                "records": [] if shared is None else [shared[0].record],
            },
        )

        await run.expect_task(
            "The oracle verifies the second script's result",
            {
                "status": "success",
                "records": [] if shared is None else [shared[1].record],
            },
        )
    finally:
        if daemon_started:
            stop = run.command(["daemon", "stop", "--json"])
            await anyio.to_thread.run_sync(stop.communicate)
