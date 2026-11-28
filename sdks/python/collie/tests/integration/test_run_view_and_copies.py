"""`open_run(run_id)`, `RunView`, KV `keys()`, and `Checkpoint`/`RunTrace` copies.

Played three ways (asyncio, Trio and sync `Collie`), attached and as a Daemon,
against the Scripted engine (ADRs 0082, 0102, 0103 and 0116; issue 1284).
"""

from __future__ import annotations

import json
from collections.abc import AsyncIterable, Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, TypeVar

import anyio.to_thread
import pytest
from collie_calls import exit_code, failure, open_collie
from goodfetch_collie import (
    AsyncCollie,
    GoodfetchError,
    ProgramError,
    RunInterruptedError,
    RunNotFoundError,
    StoredBytes,
    UnavailableError,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie import EngineChoice

COVERS = pytest.mark.covers(
    "story:11-130",
    "operation:run.trace",
    "operation:checkpoint.save_as",
    "operation:checkpoint.stream",
    "operation:trace.save_as",
    "operation:trace.stream",
    "operation:key_value_store.keys",
)


class Book(BaseModel):
    """Schema-typed value stored in the Key-value store."""

    title: str
    pages: int


@dataclass(frozen=True, slots=True)
class Played:
    """What the case's calls answered."""

    live_keys: list[str]
    cp_saved: bytes
    cp_streamed: bytes
    trace_saved: bytes
    trace_streamed: bytes
    finished_progress_last: bool
    finished_report_id: str
    finished_trace_location: str | None
    finished_book: Book | None
    finished_bytes: StoredBytes | None
    finished_keys: list[str]
    interrupted_lifecycle: str
    interrupted_error: GoodfetchError
    missing_error: GoodfetchError


_T = TypeVar("_T")


async def _collect(stream: AsyncIterable[_T] | Iterable[_T]) -> list[_T]:
    if isinstance(stream, AsyncIterable):
        return [item async for item in stream]
    return await anyio.to_thread.run_sync(list, stream)


async def _collect_bytes(stream: AsyncIterable[bytes] | Iterable[bytes]) -> bytes:
    return b"".join(await _collect(stream))


async def _collect_str(stream: AsyncIterable[str] | Iterable[str]) -> list[str]:
    return await _collect(stream)


async def played(
    way: Way,
    env: Mapping[str, str],
    out_dir: Path,
    *,
    engine: EngineChoice = "auto",
) -> Played:
    """Plays `run-view-and-copies` through `way`."""
    collie: AsyncCollie = await open_collie(way, env, engine=engine)
    run = await collie.start()
    kv = way.api(run.key_value_store, run.key_value_store)

    await kv.set("item-a", Book(title="Dune", pages=412), Book)
    await kv.set_bytes("item-bin", b"hello", "application/octet-stream")
    live_keys = await _collect_str(kv.keys(prefix="item-"))

    raw_checkpoint = await run.checkpoint()
    checkpoint = way.api(raw_checkpoint, raw_checkpoint)
    cp_path = out_dir / "nested" / "cp.sqlite"
    await checkpoint.save_as(cp_path)
    cp_saved = cp_path.read_bytes()
    cp_streamed = await _collect_bytes(checkpoint.stream())

    raw_trace = await run.trace()
    assert raw_trace is not None
    trace = way.api(raw_trace, raw_trace)
    trace_path = out_dir / "nested" / "trace.zip"
    await trace.save_as(trace_path)
    trace_saved = trace_path.read_bytes()
    trace_streamed = await _collect_bytes(trace.stream())

    await run.finish()

    finished_view = await collie.open_run("finished-run")
    finished_progress = await _collect(finished_view.progress())
    finished_report = await finished_view.report()
    finished_trace = await finished_view.trace()
    finished_kv = way.api(finished_view.key_value_store, finished_view.key_value_store)
    finished_book = await finished_kv.get("item-a", Book)
    finished_bytes = await finished_kv.get_bytes("item-bin")
    finished_keys = await _collect_str(finished_kv.keys(prefix="item-"))
    await finished_view.close()

    interrupted_view = await collie.open_run("interrupted-run")
    interrupted_progress = await _collect(interrupted_view.progress())
    interrupted_error = await failure(interrupted_view.report())
    await interrupted_view.close()

    missing_view = await collie.open_run("missing-run")
    missing_error = await failure(missing_view.report())
    await missing_view.close()

    await collie.close()
    return Played(
        live_keys=live_keys,
        cp_saved=cp_saved,
        cp_streamed=cp_streamed,
        trace_saved=trace_saved,
        trace_streamed=trace_streamed,
        finished_progress_last=finished_progress[-1].last,
        finished_report_id=finished_report.run_id.root,
        finished_trace_location=(
            None if finished_trace is None else finished_trace.location
        ),
        finished_book=finished_book,
        finished_bytes=finished_bytes,
        finished_keys=finished_keys,
        interrupted_lifecycle=str(interrupted_progress[-1].lifecycle),
        interrupted_error=interrupted_error,
        missing_error=missing_error,
    )


def check_played(client_case: ClientCaseRecord, result: Played) -> None:
    """Checks every assertion of `run-view-and-copies`."""
    with client_case.check(
        "KV keys() yields prefixed keys in order on live Run and RunView"
    ):
        assert result.live_keys == ["item-a", "item-bin"]
        assert result.finished_keys == ["item-a", "item-bin"]
    with client_case.check(
        "Checkpoint.save_as and RunTrace.save_as write whole and stream() matches"
    ):
        assert result.cp_saved == b"cp-bytes!!"
        assert result.cp_streamed == b"cp-bytes!!"
        assert result.trace_saved == b"trace-bytes!"
        assert result.trace_streamed == b"trace-bytes!"
    with client_case.check(
        "RunView on finished Run reads progress, report, trace, and KV"
    ):
        assert result.finished_progress_last is True
        assert result.finished_report_id == "finished-run"
        assert result.finished_trace_location == "/engine/finished-trace.zip"
        assert result.finished_book == Book(title="Dune", pages=412)
        assert result.finished_bytes == StoredBytes(
            content=b"hello", content_type="application/octet-stream"
        )
    with client_case.check(
        "RunView raises run_interrupted on interrupted Run and run_not_found on missing"
    ):
        assert result.interrupted_lifecycle == "stopped"
        assert isinstance(result.interrupted_error, RunInterruptedError)
        assert isinstance(result.interrupted_error, UnavailableError)
        assert result.interrupted_error.code == "run_interrupted"
        assert isinstance(result.missing_error, RunNotFoundError)
        assert isinstance(result.missing_error, ProgramError)
        assert result.missing_error.code == "run_not_found"


CORPUS = (
    Path(__file__).resolve().parents[5] / "test-support" / "fixtures" / "engine-scripts"
)


def _script_with_save_paths(test_env: TestEnv, script: str, out_dir: Path) -> str:
    cp_path = str((out_dir / "nested" / "cp.sqlite").resolve())
    trace_path = str((out_dir / "nested" / "trace.zip").resolve())
    text = (
        (CORPUS / script)
        .read_text("utf-8")
        .replace(json.dumps("/work/out/cp.sqlite"), json.dumps(cp_path))
        .replace(json.dumps("/work/out/trace.zip"), json.dumps(trace_path))
    )
    target = test_env.folder / "save-scripts" / out_dir.name / script
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, "utf-8")
    return str(target)


@COVERS
@pytest.mark.client_case(id="run-view-and-copies", mode="attached")
@three_ways
async def test_run_view_and_copies_attached(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    tmp_path: Path,
    client_case: ClientCaseRecord,
) -> None:
    """`run-view-and-copies` played three ways against the Attached Scripted engine."""
    out_dir = tmp_path / way.value
    script = _script_with_save_paths(
        test_env, "sdk-python/run/run-view-and-copies.json", out_dir
    )
    scripted = scripted_engine_env(test_env, product, script)
    result = await played(way, scripted, out_dir)
    check_played(client_case, result)


@COVERS
@pytest.mark.client_case(id="run-view-and-copies", mode="daemon")
@three_ways
async def test_run_view_and_copies_daemon(
    way: Way,
    test_env: TestEnv,
    tmp_path: Path,
    client_case: ClientCaseRecord,
) -> None:
    """`run-view-and-copies` played three ways against a Scripted Daemon."""
    out_dir = tmp_path / way.value
    script = _script_with_save_paths(
        test_env, "sdk-python/run/run-view-and-copies-from-a-daemon.json", out_dir
    )
    daemon = start_scripted_daemon(test_env, script, "socket")
    result = await played(way, test_env.env, out_dir, engine="daemon")
    code = await exit_code(daemon.wait)
    check_played(client_case, result)
    with client_case.check("the Daemon played its script through"):
        assert code == 0
