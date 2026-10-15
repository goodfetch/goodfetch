"""Collie's testing kit, `goodfetch_collie.testing` (ADRs 0069, 0090 and 0105): public and under semver.

It re-exports the shared kit of `goodfetch_collie_client.testing` and adds a test engine whose connection is
`AsyncCollie` or `Collie`. Crawl handlers and Hooks are Foxhound's, so `run_handler` and `run_hook` are in
`goodfetch_foxhound.testing` (ADR 0084).
"""

from collections.abc import Generator, Mapping
from os import PathLike
from types import TracebackType
from typing import ClassVar, Literal, final

from goodfetch_collie_client.testing import *
from goodfetch_collie import AsyncCollie, Collie
from goodfetch_collie_client import AsyncModelProviderHandler, ModelProviderHandler, Settings

@final
class _AsyncStarting:
    def __await__(self) -> Generator[None, None, AsyncTestEngine]: ...
    async def __aenter__(self) -> AsyncTestEngine: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AsyncTestEngine:
    """A real engine for one test, for `AsyncCollie`: a temporary Goodfetch home removed when it closes, a cleared
    environment, no project file unless named, `offline` (ADR 0043), short durations, and no Model provider unless the
    test registers or configures one, so a call that needs a model with none raises `ModelNotConfiguredError`. Every Run
    started from `connection` that names no Response recording replays an empty one: a request to a site raises
    `ResponseNotRecordedError`, naming it. Browsers come from the browser cache of the caller's Goodfetch home, read and
    never written, or `GOODFETCH_BROWSERS_PATH` (ADRs 0024, 0059, 0105). An Attached engine (ADR 0069)."""

    @classmethod
    def start(
        cls,
        *,
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None = None,
        model_providers: Mapping[str, AsyncModelProviderHandler | ScriptedModel] | None = None,
    ) -> _AsyncStarting:
        """`env` is `connect()`'s environment map (ADRs 0062, 0083), the process's own when left out: only
        `GOODFETCH_HOME` (to read its browser cache, never to write), `GOODFETCH_BROWSERS_PATH` and this Product's
        engine-path variable are read from it, and the engine's own environment stays cleared. Every host its `resolve` setting (ADR 0059) maps to loopback, such as a test's own server, is allowlisted
        (ADR 0017) and reached live even while a Run replays. `model_providers` are named by `ai.model` as
        `"<name>/<any>"` (ADR 0101)."""

    @property
    def connection(self) -> AsyncCollie:
        """A connection to this engine, `AsyncCollie.connect()`'s type: everything a program does works on it."""

    async def reconcile(self, run_id: str, key: str, answer: Literal["happened", "did_not_happen"]) -> None:
        """Stands for a person's answer about an uncertain Effect of a Run on this engine (ADR 0029): the key's next
        declaration returns `reconciled` after `happened`, and performs the body again after `did_not_happen`
        (ADR 0106). Only the testing kit has it."""

    async def close(self) -> None:
        """Closes the connection and the engine and removes the home. Raises `ScriptedModelUnusedError` if a Scripted model
        registered here has replies left."""

    async def __aenter__(self) -> AsyncTestEngine: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class TestEngine:
    """`AsyncTestEngine` for the sync `Collie`, used with `with`."""

    __test__: ClassVar[bool] = False
    """pytest never collects it, as Starlette's `TestClient`."""

    @classmethod
    def start(
        cls,
        *,
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None = None,
        model_providers: Mapping[str, ModelProviderHandler] | None = None,
    ) -> TestEngine: ...
    @property
    def connection(self) -> Collie: ...
    def reconcile(self, run_id: str, key: str, answer: Literal["happened", "did_not_happen"]) -> None: ...
    def close(self) -> None: ...
    def __enter__(self) -> TestEngine: ...
    def __exit__(self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None) -> None: ...
