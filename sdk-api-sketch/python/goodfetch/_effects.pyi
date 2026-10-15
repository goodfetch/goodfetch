"""Effects a program declares in code (ADR 0106, decided in #500).

A program's own calls are Effects only when it declares them (ADR 0010): `run.effect(key, body)` wraps a function of
its own, and the JSON value the body returns is kept in the Run's Effect journal. In a crawl handler it is
`ctx.run.effect(...)`, whose key's namespace is the crawl request, so every retry, escalation, sample comparison, lost
lease and `retry_failed` reaches the same entry; elsewhere it is the Run, so a Collie program started again with its
Run's ID finds what it already did.

The engine judges a body by what it sent for it while the body ran: a body that returned is done and never runs again;
one that ended before anything was sent was not performed; one that ended after a send is uncertain
(`EffectUncertainError`) until a person reconciles it (ADR 0029), unless it is repeatable. A body counts as sent from its
start while the engine cannot see every command (a raw CDP attach, a lent page, a browser shared with other code).
Prepare the page before the declaration: the body holds the action that commits the Effect and the reads of its
result. There is no separate check: a body that checks first, or sends `effect.idempotency_key`, is repeatable. There
is no `with` form, since a context manager cannot skip its own body.
"""

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from datetime import datetime
from typing import Generic, Literal, TypeAlias, TypeVar, final

from goodfetch._data import JsonValue
from goodfetch._results import CrawlRequestRef, ProgramError

J = TypeVar("J", bound=JsonValue)
T = TypeVar("T")

@final
@dataclass(frozen=True, kw_only=True)
class Effect:
    """The Effect a body performs, handed to the body."""

    key: str
    """The key it was declared with."""
    idempotency_key: str
    """An opaque key, the same on every attempt and resume of this Effect, on any machine, and different in every Run
    and every install. Send it as a site's `Idempotency-Key` header, or to the program's own service, to make the body
    safe to repeat. It reveals neither the Run's ID nor the key."""

@final
@dataclass(frozen=True, kw_only=True)
class EffectDone(Generic[T]):
    """The Effect is done: `performed`, the body ran now; or `journal`, the Effect journal already held it from an
    earlier attempt or start, so the body did not run, and `output` is what it returned then."""

    source: Literal["performed", "journal"]
    output: T
    performed_at: datetime

@final
@dataclass(frozen=True, kw_only=True)
class EffectReconciled:
    """A person answered "it happened" (ADR 0029): the body did not run and no output is known."""

    source: Literal["reconciled"]
    reconciled_by: str
    reconciled_at: datetime

@final
@dataclass(frozen=True, kw_only=True)
class EffectSourceUnknown:
    """A source a newer engine sends, kept as received (ADR 0079)."""

    source: str

EffectResult: TypeAlias = EffectDone[T] | EffectReconciled | EffectSourceUnknown
"""What `run.effect` returns: narrow it with `isinstance` or `match`."""

UncertainWhy: TypeAlias = Literal["failed_after_send", "interrupted", "journal_incomplete"] | str
"""Why an Effect is uncertain: its body raised after something was sent, an earlier attempt ended inside it after a
send, or the Run resumed from a Checkpoint after which it may have gone on and the journal lacks the key (ADR 0002)."""

@final
@dataclass(frozen=True, kw_only=True)
class UncertainEffect:
    """An Effect the journal holds as uncertain, until a person reconciles it (ADR 0029)."""

    journal_id: str
    key: str | None
    """A declared Effect's key; `None` for an AI action."""
    instruction: str | None
    """An AI action's instruction (ADR 0091); `None` for a declared Effect."""
    request: CrawlRequestRef | None
    """The crawl request whose namespace holds it; `None` outside a handler."""
    why: UncertainWhy
    first_sent: str | None
    """The first command sent for it, in Python, such as `page.get_by_role("button", name="Place order").click()`;
    `None` when the journal lost it."""
    since: datetime

class DeclaresEffects:
    def effect(self, key: str, body: Callable[[Effect], J], *, repeatable: bool = False) -> EffectResult[J]:
        """Performs `body` as the Effect `key`, unless the Run's Effect journal already answers it.

        The key, at most 255 code points, names one Effect in its namespace: the crawl request in a handler or a Hook,
        the Run elsewhere; declaring it again while its body runs, or after this attempt got its answer, raises
        `EffectKeyReusedError`, except that on a borrowed browser (ADR 0089) a key declared again after its answer
        returns that answer, since the other crawler's retries reach the Run as new calls. While the body runs in an
        attempt, staging an output with it (`enqueue`, `add_seed`, `ctx.push`, a Key-value store write) raises `EffectStagingRefusedError`:
        stage it after the declaration, from the output. A body that raises before the engine sent anything raises its
        own error unchanged; one
        that raises after a send raises `EffectUncertainError` from it, unless `repeatable`, which declares the body
        safe to run again (ADR 0010). `CancelledError`, `KeyboardInterrupt` and `AttemptEndedError` propagate unchanged,
        and the next declaration of the key acts on what was sent. The program's own I/O inside the body is invisible
        to the journal. No `timeout`: the body's own calls carry theirs (ADR 0080). The body runs on the calling
        thread.
        """

class AsyncDeclaresEffects:
    async def effect(self, key: str, body: Callable[[Effect], Awaitable[J]], *, repeatable: bool = False) -> EffectResult[J]:
        """Performs `body` as the Effect `key`, unless the Run's Effect journal already answers it; as the sync
        client's, with the body awaited in the calling task."""

@final
@dataclass(frozen=True, kw_only=True)
class EffectKeyInvalidDetails:
    key: str

@final
class EffectKeyInvalidError(ProgramError):
    """An empty key, or one longer than 255 code points."""

    @property
    def details(self) -> EffectKeyInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class EffectKeyReusedDetails:
    key: str
    answer: Literal["performed", "journal", "reconciled", "running"] | str
    """`running`: its body is still running in this attempt."""

@final
class EffectKeyReusedError(ProgramError):
    """A key declared again while its body runs or after this attempt got its answer: a loop whose key lacks its
    varying part."""

    @property
    def details(self) -> EffectKeyReusedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class EffectOutputInvalidDetails:
    key: str
    problem: Literal["not_json", "too_large"] | str
    size: int | None

@final
class EffectOutputInvalidError(ProgramError):
    """The body returned a value that is not JSON or is larger than `effect.output.max_size`: raised as such when nothing
    was sent, and as the cause of `EffectUncertainError` after a send."""

    @property
    def details(self) -> EffectOutputInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class EffectStagingRefusedDetails:
    key: str
    """The running body's key."""
    staged: Literal["enqueue", "seed", "record", "key_value"] | str

@final
class EffectStagingRefusedError(ProgramError):
    """A call staged an output with an attempt while a declared Effect's body ran in it: a re-run that gets `journal`
    does not run the body again, so the output would be lost when the attempt ends early."""

    @property
    def details(self) -> EffectStagingRefusedDetails: ...
