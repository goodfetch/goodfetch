"""`goodfetch_collie` (ADRs 0090, 0095): Collie's Python SDK and its `goodfetch-collie` executable.

Where the main types live (ADR 0095): `goodfetch_collie_client` holds `Page`, `Locator`, `Run`, `Session` and the
results and errors; `goodfetch_collie` holds `Collie`; `goodfetch_foxhound` holds `Foxhound`, `Crawl`, `Label`,
`Dataset` and `CrawlContext`, and re-exports `goodfetch_collie_client`. An async class adds `Async`: `AsyncPage`.

It re-exports the Collie client package, whose values (Run, Session, Page, errors) a Foxhound handler gets too, and adds
the connection, which finds Collie's executable in its own wheel and reads only `GOODFETCH_COLLIE_ENGINE_PATH` (ADR
0090).
"""

from collections.abc import Generator, Mapping
from os import PathLike
from types import TracebackType
from typing import Generic, TypeVar, final

from goodfetch_collie_client import *

_C = TypeVar("_C")

@final
class _AsyncConnecting(Generic[_C]):
    """What `connect()` returns: awaited, or used with `async with`."""

    def __await__(self) -> Generator[None, None, _C]: ...
    async def __aenter__(self) -> _C: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AsyncCollie(AsyncConnection):
    """A connection to an engine, Collie's or Foxhound's (ADR 0095): the entry point (ADR 0078). Each one is
    independent, and many may be open in one process. It announces Collie in `hello`, so a Foxhound engine accepts it too.
    """

    @classmethod
    def connect(
        cls,
        *,
        engine: EngineChoice = "auto",
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None | NotGiven = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        resolve_secret: AsyncResolveSecret | None = None,
        model_providers: Mapping[str, AsyncModelProviderHandler] | None = None,
        proxy_providers: Mapping[str, AsyncProxyProviderHandler] | None = None,
    ) -> _AsyncConnecting[AsyncCollie]:
        """Starts or finds the engine and completes `hello` before the connection is returned.

        Left at `auto`, `engine` is filled by `GOODFETCH_URL` when set; a value written here wins over it. `settings` holds
        every other setting for this connection and everything beneath it. `env` is the environment map, read once here
        and never at import, by default the process's own (ADRs 0062, 0069): only the deployment variables are read, an
        unknown `GOODFETCH_` name fails with `SettingUnknownError`, and an Attached engine gets exactly this map.
        `project` is the project configuration file: left out, found upward from the working directory as the CLI does;
        `None`, none (ADR 0083). `resolve_secret` answers `caller:` Secret references from the program's own store, at
        the moment of use (ADR 0092). `model_providers` are Model providers answered by the program's own code, by the
        name Model references use: `"mine/any-model"` for `{"mine": handler}`, from the Runs this connection opens; a
        name taken by a built-in, a declared provider or a Plugin raises `SettingInvalidError` (ADR 0101).
        `proxy_providers` are the program's own Proxy providers, SDK handlers by the name settings choose them by (ADR
        0104): the only one becomes its choosing setting's value with the run-layer origin `registered`, which every
        other run-layer value replaces, and two fail unless `settings` choose one. Collie has no Fetch backend or Sink
        kind (ADR 0088).
        """

@final
class Collie(Connection):
    """`AsyncCollie` in the sync API: a connection to an engine, Collie's or Foxhound's.

    It runs the async core on a private background thread of its own, so it works on any thread, including one whose event
    loop is running (a notebook cell); its values are safe to call from any thread (ADR 0081).
    """

    @classmethod
    def connect(
        cls,
        *,
        engine: EngineChoice = "auto",
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None | NotGiven = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        resolve_secret: ResolveSecret | None = None,
        model_providers: Mapping[str, ModelProviderHandler] | None = None,
        proxy_providers: Mapping[str, ProxyProviderHandler] | None = None,
    ) -> Collie:
        """Starts or finds the engine and completes `hello` before returning; use it with `with`.

        Left at `auto`, `engine` is filled by `GOODFETCH_URL` when set; a value written here wins over it. `settings` holds
        every other setting for this connection and everything beneath it. `env` is the environment map, read once here
        and never at import, by default the process's own (ADRs 0062, 0069): only the deployment variables are read, an
        unknown `GOODFETCH_` name fails with `SettingUnknownError`, and an Attached engine gets exactly this map.
        `project` is the project configuration file: left out, found upward from the working directory as the CLI does;
        `None`, none (ADR 0083). `model_providers` are Model providers answered by the program's own code, by the name
        Model references use, from the Runs this connection opens; a name taken by a built-in, a declared provider or a
        Plugin raises `SettingInvalidError` (ADR 0101). `proxy_providers` are the program's own Proxy providers,
        SDK handlers (ADR 0104). All of them run on the connection's thread pool.
        """
