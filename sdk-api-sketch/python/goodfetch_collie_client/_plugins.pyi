"""SDK handlers both Products take (ADR 0104): a program's own Proxy provider, registered on `connect()` by name as ADR
0101 registers `model_providers` (`_models`) and ADR 0092 takes `resolve_secret` (`_sessions`). The Fetch backend and
Sink kinds are Foxhound's (ADR 0088), in `goodfetch_foxhound`'s `_hooks`, which states the rules every SDK handler
follows.
"""

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import TypeAlias, final

from goodfetch_collie_client._results import BlockCause, ProgramError
from goodfetch_collie_client._sessions import SecretReference

@final
@dataclass(frozen=True, kw_only=True)
class ProxyRequest:
    """One proxy per Session (ADRs 0021, 0092)."""

    session_id: str
    domain: str
    """The registrable domain of the Session's first request."""
    replacing: BlockCause | None
    """Set when the Session replaces one retired for this Block cause."""

@final
@dataclass(frozen=True, kw_only=True)
class ProxyAnswer:
    """A proxy, with Playwright's fields as `proxy.*` has them (ADR 0092)."""

    server: str
    username: str | None = None
    password: SecretReference | None = None
    """A Secret reference, never a value: `caller:` resolves through `resolve_secret`."""

AsyncProxyProviderHandler: TypeAlias = Callable[[ProxyRequest], Awaitable[ProxyAnswer]]
ProxyProviderHandler: TypeAlias = Callable[[ProxyRequest], ProxyAnswer]

@final
@dataclass(frozen=True, kw_only=True)
class SdkHandlerFailedDetails:
    """The handler, then the exception as `handler_failed` carries a handler's."""

    handler: str
    """The name it was registered under."""
    exception_type: str
    exception_message: str
    stack: Sequence[str]

@final
class ProxyHandlerFailedError(ProgramError):
    """`proxy_handler_failed`: the Session does not open. In a crawl, the requests that needed it are retried only when
    `crawl.retry.handler_failed` is on for their Label; in a Collie Run, the call fails."""

    @property
    def details(self) -> SdkHandlerFailedDetails: ...

