"""Multi-process Crawl workers for Python (ADR 0024; spec #611's #1497).

`workers=N` starts `N - 1` `multiprocessing` processes in the `"spawn"` context,
imports the module-level `AsyncCrawl` or `Crawl` and connects each worker to the
Attached engine's private endpoint or Daemon before `crawl.start`, then directs
each to join the started Run via `crawl.serve` so all `N` processes share work
from the first request, and stops every worker cleanly when the Run ends or the
starter closes the Run.

Each worker communicates with its starter over a private OS `Pipe` (no POSIX
semaphores or `multiprocessing.Event`), so a worker killed with `SIGKILL` never
deadlocks shutdown or leaks semaphores.
"""

from __future__ import annotations

import importlib
import importlib.util
import logging
import multiprocessing
import os
import sys
import time
from contextlib import suppress
from dataclasses import dataclass
from typing import TYPE_CHECKING, cast

import anyio
import anyio.to_thread
from goodfetch_collie_client.generated.errors import SettingInvalidError
from goodfetch_collie_client.generated.models import (
    Lifecycle,
    Product,
    RunId,
    RunParams,
    RunStarted,
)
from goodfetch_collie_client.portal import PrivateLoop
from goodfetch_collie_client.product_connection import open_worker_connection

from goodfetch_foxhound.crawl import AsyncCrawl, Crawl, crawl_origin
from goodfetch_foxhound.generated.errors import CrawlNotImportableError
from goodfetch_foxhound.generated.operations import AsyncFoxhoundOperations

if TYPE_CHECKING:
    from multiprocessing.connection import Connection as MpConnection
    from multiprocessing.context import SpawnProcess

    from goodfetch_collie_client.connection import WorkerTarget
    from goodfetch_collie_client.product_connection import ProductPackage

_WORKER_READY_TIMEOUT_S = 30.0
_WORKER_STOP_GRACE_S = 3.0
_WORKER_KILL_GRACE_S = 1.0


@dataclass(frozen=True)
class WorkerPlan:
    """Validated multi-process worker plan, picklable across `spawn`."""

    count: int
    module: str
    export: str
    module_file: str | None
    is_async: bool


class WorkerPool:
    """Spawned worker processes owned by one crawl Run."""

    def __init__(
        self,
        processes: list[SpawnProcess],
        pipes: list[MpConnection],
    ) -> None:
        """Holds the spawned worker processes and their private pipes."""
        self._processes = processes
        self._pipes = pipes
        self._stopped = False

    async def join(self, run_id: str) -> None:
        """Tells every pre-connected worker to join `run_id` via `crawl.serve`."""
        try:
            await anyio.to_thread.run_sync(self._join_sync, run_id)
        except BaseException:
            await self.stop()
            raise

    def _join_sync(self, run_id: str) -> None:
        for pipe in self._pipes:
            pipe.send(("serve", run_id))
        _wait_all_pipes(self._processes, self._pipes, "ready")

    async def stop(self) -> None:
        """Signals every worker to exit and waits until all have exited."""
        if self._stopped:
            return
        self._stopped = True
        await anyio.to_thread.run_sync(self._stop_sync)

    def _stop_sync(self) -> None:
        for pipe in self._pipes:
            with suppress(Exception):
                pipe.send(("stop", None))
            with suppress(Exception):
                pipe.close()
        for proc in self._processes:
            if proc.is_alive():
                proc.join(timeout=_WORKER_STOP_GRACE_S)
        for proc in self._processes:
            if proc.is_alive():
                proc.terminate()
        for proc in self._processes:
            if proc.is_alive():
                proc.join(timeout=_WORKER_KILL_GRACE_S)
        for proc in self._processes:
            if proc.is_alive():
                proc.kill()
                proc.join(timeout=_WORKER_KILL_GRACE_S)


def prepare_workers(
    crawl: AsyncCrawl | Crawl | None, workers: int
) -> WorkerPlan | None:
    """Validates `workers` and confirms `crawl` is importable at module level.

    Returns `None` when `workers == 1`.

    Raises:
        SettingInvalidError: `workers` is not a whole number >= 1.
        CrawlNotImportableError: `workers > 1` on a Crawl defined inside a
            function or otherwise not importable at module level.
    """
    if isinstance(workers, bool) or type(workers) is not int or workers < 1:
        raise SettingInvalidError(
            f"workers must be a whole number of 1 or more, got {workers!r}",
            fix="Pass an integer >= 1 for workers.",
            details={"key": "workers", "reason": "must be a whole number of 1 or more"},
        )
    if workers == 1:
        return None
    if crawl is None:
        raise CrawlNotImportableError(
            "workers > 1 requires a module-level Crawl with handlers",
            fix=(
                "Define the Crawl at module level, or omit workers for a "
                "code-free crawl."
            ),
            details={"module": None, "export": None},
        )
    module_name, module_file, defined_at_module, handlers = crawl_origin(crawl)
    for handler in handlers:
        qualname = getattr(handler, "__qualname__", "")
        if isinstance(qualname, str) and "<locals>" in qualname:
            raise _not_importable(
                module_name,
                None,
                "a Crawl with a handler defined inside a function cannot be "
                "imported by worker processes",
            )
    if not defined_at_module or module_name is None:
        raise _not_importable(
            module_name,
            None,
            "a Crawl defined inside a function cannot be imported by worker processes",
        )
    module = sys.modules.get(module_name)
    if module is None:
        raise _not_importable(
            module_name,
            None,
            f"module {module_name!r} is not loaded in sys.modules",
        )
    export_name: str | None = None
    for attr_name, value in vars(module).items():
        if value is crawl:
            export_name = attr_name
            break
    if export_name is None:
        raise _not_importable(
            module_name,
            None,
            f"the Crawl is not bound to a module-level variable in {module_name!r}",
        )
    if module_file is None:
        file_attr = getattr(module, "__file__", None)
        if isinstance(file_attr, str):
            module_file = file_attr
    if importlib.util.find_spec(module_name) is None and module_file is None:
        raise _not_importable(
            module_name,
            export_name,
            f"module {module_name!r} cannot be located by spawned worker processes",
        )
    return WorkerPlan(
        count=workers,
        module=module_name,
        export=export_name,
        module_file=module_file,
        is_async=isinstance(crawl, AsyncCrawl),
    )


async def spawn_workers(plan: WorkerPlan, target: WorkerTarget) -> WorkerPool:
    """Starts `plan.count - 1` spawned workers and connects them to `target`."""
    ctx = multiprocessing.get_context("spawn")
    processes: list[SpawnProcess] = []
    pipes: list[MpConnection] = []
    pool = WorkerPool(processes, pipes)
    try:
        for _ in range(plan.count - 1):
            parent_pipe, child_pipe = ctx.Pipe(duplex=True)
            proc = ctx.Process(
                target=_worker_main,
                args=(plan, target, child_pipe),
                daemon=False,
            )
            proc.start()
            child_pipe.close()
            processes.append(proc)
            pipes.append(parent_pipe)
        await anyio.to_thread.run_sync(_wait_all_pipes, processes, pipes, "connected")
    except BaseException:
        await pool.stop()
        raise
    return pool


def _wait_all_pipes(
    processes: list[SpawnProcess],
    pipes: list[MpConnection],
    expected_tag: str,
) -> None:
    deadline = time.monotonic() + _WORKER_READY_TIMEOUT_S
    for proc, pipe in zip(processes, pipes, strict=True):
        remaining = max(0.0, deadline - time.monotonic())
        if not pipe.poll(remaining):
            msg = (
                f"worker process did not reach {expected_tag} within "
                f"{_WORKER_READY_TIMEOUT_S}s (exitcode={proc.exitcode})"
            )
            raise RuntimeError(msg)
        try:
            message: object = pipe.recv()
        except EOFError:
            msg = (
                f"worker process exited before {expected_tag} "
                f"(exitcode={proc.exitcode})"
            )
            raise RuntimeError(msg) from None
        if isinstance(message, tuple):
            parts = cast("tuple[object, ...]", message)
            if len(parts) >= 2:
                tag = parts[0]
                if tag == expected_tag:
                    continue
                if tag == "not_importable" and len(parts) >= 4:
                    raise _not_importable(
                        cast("str | None", parts[1]),
                        cast("str | None", parts[2]),
                        cast("str", parts[3]),
                    )
                if tag == "error":
                    raise RuntimeError(f"worker failed to start: {parts[1]}")
        raise RuntimeError(f"unexpected worker handshake message: {message!r}")


def _not_importable(
    module: str | None, export: str | None, message: str
) -> CrawlNotImportableError:
    return CrawlNotImportableError(
        message,
        fix=(
            "Define the Crawl and its handlers at module level so worker "
            "processes can import them."
        ),
        details={"module": module, "export": export},
    )


def _load_crawl(plan: WorkerPlan) -> AsyncCrawl | Crawl:
    """Imports `plan.module` and returns the module-level `Crawl`."""
    module: object
    try:
        module = importlib.import_module(plan.module)
    except ModuleNotFoundError:
        if plan.module_file is None:
            raise
        spec = importlib.util.spec_from_file_location(plan.module, plan.module_file)
        if spec is None or spec.loader is None:
            raise
        loaded = importlib.util.module_from_spec(spec)
        sys.modules[plan.module] = loaded
        spec.loader.exec_module(loaded)
        module = loaded
    exported = getattr(module, plan.export, None)
    if not isinstance(exported, AsyncCrawl | Crawl):
        raise TypeError(f"attribute {plan.export!r} of {plan.module!r} is not a Crawl")
    return exported


def _wait_pipe_command(pipe: MpConnection) -> tuple[str, str | None]:
    """Reads the next `(cmd, arg)` from the starter pipe, or `("stop", None)` on EOF."""
    try:
        raw: object = pipe.recv()
    except (EOFError, OSError):
        return ("stop", None)
    if isinstance(raw, tuple):
        parts = cast("tuple[object, ...]", raw)
        if len(parts) >= 2 and isinstance(parts[0], str):
            arg = parts[1] if isinstance(parts[1], str) else None
            return (parts[0], arg)
    return ("stop", None)


def _worker_main(
    plan: WorkerPlan,
    target: WorkerTarget,
    pipe: MpConnection,
) -> None:
    """Entry point of each spawned Python worker process."""
    try:
        crawl = _load_crawl(plan)
    except Exception as error:
        with suppress(Exception):
            pipe.send(("not_importable", plan.module, plan.export, str(error)))
        with suppress(Exception):
            pipe.close()
        return

    from goodfetch_foxhound.foxhound import product_package

    product = product_package()
    try:
        if isinstance(crawl, Crawl):
            _run_sync_worker(product, crawl, target, pipe)
        else:
            anyio.run(_run_async_worker, product, crawl, target, pipe)
    except BaseException as error:
        logging.getLogger("goodfetch.client").debug("worker process ended: %s", error)
        with suppress(Exception):
            pipe.send(("error", f"{type(error).__name__}: {error}"))
    finally:
        with suppress(Exception):
            pipe.close()


async def _run_async_worker(
    product: ProductPackage,
    crawl: AsyncCrawl,
    target: WorkerTarget,
    pipe: MpConnection,
) -> None:
    from goodfetch_foxhound.crawl_run import AsyncCrawlRun, CrawlRuns

    connection = await open_worker_connection(product, target)
    try:
        pipe.send(("connected", os.getpid()))
        cmd, run_id = await anyio.to_thread.run_sync(
            _wait_pipe_command, pipe, abandon_on_cancel=True
        )
        if cmd != "serve" or run_id is None:
            return
        crawls = CrawlRuns(connection)
        started = RunStarted.model_construct(
            None,
            run_id=RunId(run_id),
            lifecycle=Lifecycle.running,
            started_as="resumed",
            needs=Product.foxhound,
        )
        run = AsyncCrawlRun(connection, started, crawl, crawls, detached=True)
        ops = AsyncFoxhoundOperations(connection)

        async with anyio.create_task_group() as group:

            async def watch_stop() -> None:
                await anyio.to_thread.run_sync(
                    _wait_pipe_command, pipe, abandon_on_cancel=True
                )
                group.cancel_scope.cancel()

            async def serve_crawl() -> None:
                try:
                    await ops.crawl_serve(RunParams(run_id=RunId(run_id)))
                except BaseException as error:
                    logging.getLogger("goodfetch.client").debug(
                        "worker crawl.serve ended: %s", error
                    )
                finally:
                    group.cancel_scope.cancel()

            group.start_soon(watch_stop)
            group.start_soon(serve_crawl)
            await anyio.lowlevel.checkpoint()
            pipe.send(("ready", os.getpid()))

        await run.close()
    finally:
        with suppress(BaseException):
            await connection.close()


def _run_sync_worker(
    product: ProductPackage,
    crawl: Crawl,
    target: WorkerTarget,
    pipe: MpConnection,
) -> None:
    from goodfetch_foxhound.crawl_run import AsyncCrawlRun, CrawlRun, CrawlRuns

    loop = PrivateLoop()
    try:

        async def _drive() -> None:
            connection = await open_worker_connection(product, target)
            try:
                pipe.send(("connected", os.getpid()))
                cmd, run_id = await anyio.to_thread.run_sync(
                    _wait_pipe_command, pipe, abandon_on_cancel=True
                )
                if cmd != "serve" or run_id is None:
                    return
                crawls = CrawlRuns(connection)
                started = RunStarted.model_construct(
                    None,
                    run_id=RunId(run_id),
                    lifecycle=Lifecycle.running,
                    started_as="resumed",
                    needs=Product.foxhound,
                )
                async_run = AsyncCrawlRun(
                    connection, started, crawl, crawls, detached=True
                )
                _sync_run = CrawlRun(loop, async_run)
                ops = AsyncFoxhoundOperations(connection)

                async with anyio.create_task_group() as group:

                    async def watch_stop() -> None:
                        await anyio.to_thread.run_sync(
                            _wait_pipe_command, pipe, abandon_on_cancel=True
                        )
                        group.cancel_scope.cancel()

                    async def serve_crawl() -> None:
                        try:
                            await ops.crawl_serve(RunParams(run_id=RunId(run_id)))
                        except BaseException as error:
                            logging.getLogger("goodfetch.client").debug(
                                "worker crawl.serve ended: %s", error
                            )
                        finally:
                            group.cancel_scope.cancel()

                    group.start_soon(watch_stop)
                    group.start_soon(serve_crawl)
                    await anyio.lowlevel.checkpoint()
                    pipe.send(("ready", os.getpid()))

                await async_run.close()
                del _sync_run
            finally:
                with suppress(BaseException):
                    await connection.close()

        loop.call(_drive)
    finally:
        loop.stop()
