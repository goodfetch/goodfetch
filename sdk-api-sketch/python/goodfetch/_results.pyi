"""Results, outcomes and errors (ADR 0079, decided in #466).

Failures raise a `GoodfetchError`: one class per Error code under one of seven groups by remedy. Expected outcomes are
values in the return type, and no argument changes which (G1, S2). The sync and async APIs raise the same classes.
The codes below are a representative subset: each area's spec lists its codes, their groups and their details in
`goodfetch-protocol`, and codegen writes one class per code.
"""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Generic, Literal, TypeAlias, TypeVar, final

ErrorGroup: TypeAlias = Literal["program", "refused", "page", "model", "deadline_exceeded", "unavailable", "internal"]
"""A group of Error codes sharing one remedy, carried on the wire as the JSON-RPC numeric class."""

KnownErrorCode: TypeAlias = Literal[
    "closed",
    "removed",
    "handler_failed",
    "hook_failed",
    "limit_reached",
    "page_check_failed",
    "target_not_unique",
    "blocked",
    "not_verified",
    "deadline_exceeded",
    "plugin_crashed",
    "internal",
]
ErrorCode: TypeAlias = KnownErrorCode | str
"""A code a newer engine sends and this SDK does not know is kept as received (G3)."""

SettingsLayer: TypeAlias = Literal["default", "operator_policy", "configuration", "recipe", "run"]
"""The settings layers of ADR 0028, lowest first."""

@final
@dataclass(frozen=True, kw_only=True)
class LimitSource:
    """Where the limit that ended a call came from (G6)."""

    setting: str
    """The setting key, such as `timeouts.action`."""
    value: object
    layer: SettingsLayer
    """The layer that set the value."""

@final
@dataclass(frozen=True, kw_only=True)
class ErrorTarget:
    """What a call acted on, rendered by the engine in Python and kept structured too."""

    rendered: str
    """As the program wrote it, such as `page.get_by_role("button", name="Next")`."""
    structured: object

class GoodfetchError(Exception):
    """The base of every Goodfetch failure."""

    @property
    def code(self) -> ErrorCode: ...
    @property
    def group(self) -> ErrorGroup: ...
    @property
    def retryable(self) -> bool: ...
    @property
    def message(self) -> str: ...
    @property
    def operation(self) -> str:
        """The SDK call, such as `locator.click`."""

    @property
    def call_id(self) -> str:
        """Finds the call in the Run trace."""

    @property
    def run_id(self) -> str | None: ...
    @property
    def target(self) -> ErrorTarget | None: ...
    @property
    def url(self) -> str | None: ...
    @property
    def observed(self) -> str | None:
        """What was there instead."""

    @property
    def limit(self) -> LimitSource | None: ...
    @property
    def fix(self) -> str | None:
        """What would change the outcome, naming setting keys or operations."""

    @property
    def call_log(self) -> Sequence[str]:
        """The steps the engine took for this call. Never holds a Secret's value (ADR 0015)."""

    @property
    def details(self) -> object | None:
        """Per-code typed fields; `None` for codes without any."""

# The seven groups by remedy. A code this SDK does not know is raised as its group's class.

class ProgramError(GoodfetchError):
    """The program is wrong: its call, its settings or its own handler code. Fix the program (G3)."""

class RefusedError(GoodfetchError):
    """A policy, a Limit, a lock or a person refused. Change a setting, raise the Limit or ask the operator."""

class PageError(GoodfetchError):
    """The page or site never reached the state the call needed. Look at the Run trace."""

class ModelError(GoodfetchError):
    """The Model provider's answer was unusable. Change the model or the strategy."""

class DeadlineExceededError(GoodfetchError):
    """The deadline passed. Raise it, or wait for the right thing; deadlines are #467's.

    A code named like its group (`deadline_exceeded`, `internal`) shares the group's class.
    """

class UnavailableError(GoodfetchError):
    """Something Goodfetch depends on is unreachable or gone. Usually retryable: retry or fix the setup."""

class InternalError(GoodfetchError):
    """A bug in Goodfetch. Report it."""

# One generated class per code: the code in PascalCase plus `Error`.

@final
class ClosedError(ProgramError): ...

@final
@dataclass(frozen=True, kw_only=True)
class HandlerFailedDetails:
    """The user's own exception, as text, since the outcome outlives the process."""

    exception_type: str
    exception_message: str
    stack: Sequence[str]
    """The stack frames of the user's code."""

@final
class HandlerFailedError(ProgramError):
    """A handler raised something other than a `GoodfetchError`; `__cause__` holds it in the process that raised it."""

    @property
    def details(self) -> HandlerFailedDetails: ...

@final
class LimitReachedError(RefusedError): ...

@final
class PageCheckFailedError(PageError): ...

@final
@dataclass(frozen=True, kw_only=True)
class TargetNotUniqueDetails:
    matches: Sequence[str]
    """Each match, rendered in Python."""

@final
class TargetNotUniqueError(PageError):
    @property
    def details(self) -> TargetNotUniqueDetails: ...

BlockCause: TypeAlias = (
    Literal["rate_limited", "challenge", "ip_blocked", "payment_required", "consent_required"] | str
)
"""The Block causes of ADRs 0021, 0035, 0045 and 0048; a newer engine's cause is kept as received."""

@final
@dataclass(frozen=True, kw_only=True)
class BlockedDetails:
    block_cause: BlockCause

@final
class BlockedError(PageError):
    @property
    def details(self) -> BlockedDetails: ...

# Outcomes are values.

SkipReason: TypeAlias = (
    Literal["out_of_scope", "duplicate", "site_preferences", "skipped_by_hook", "skipped_by_handler"] | str
)
"""Why a request was skipped; the full list is #477's and #480's."""

@final
@dataclass(frozen=True, kw_only=True)
class RequestSucceeded:
    attempts: int
    """Retries are not an outcome: every outcome counts its attempts and keeps the failed ones."""
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class RequestFailed:
    error: GoodfetchError
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class RequestSkipped:
    reason: SkipReason
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class RequestBlocked:
    block_cause: BlockCause
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class RequestOutcomeUnknown:
    """A kind a newer engine sent that this SDK does not know, kept as received (G3)."""

    raw: str
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

RequestOutcome: TypeAlias = RequestSucceeded | RequestFailed | RequestSkipped | RequestBlocked | RequestOutcomeUnknown
"""A crawl request's ending. The noun "request" is #477's to settle."""

Verdict: TypeAlias = Literal["verified", "rejected", "needs-review"] | str
"""A Verdict, given by checks under ADR 0029's rules and never by the model's claim. Any other value is never `verified`."""

@final
@dataclass(frozen=True, kw_only=True)
class Finished: ...

@final
@dataclass(frozen=True, kw_only=True)
class Stopped:
    """AI work that stopped early. A failed ending raises instead."""

    reason: Literal["limit", "timeout", "abort"] | str
    """`timeout`: the Run's own timeout (#467)."""
    limit: str | None
    """The Limit by name, when `reason` is `limit`."""

Ended: TypeAlias = Finished | Stopped

@final
@dataclass(frozen=True, kw_only=True)
class Reported:
    """What the model said about its own work: a signal, never proof (ADR 0013)."""

    claim: Literal["done", "gave_up"] | str
    message: str

T = TypeVar("T")

@final
class AiWorkResult(Generic[T]):
    """The result of AI work that can claim success: an Agent's task, or a Plan replayed toward a goal.

    #474 and #473 build their result types on it and decide `partial`'s exact shape.
    """

    @property
    def ended(self) -> Ended: ...
    @property
    def verdict(self) -> Verdict: ...
    @property
    def reported(self) -> Reported: ...
    @property
    def output(self) -> T | None:
        """Present only once the work finished with a complete `T`; a value of `T` is always complete."""

    @property
    def partial(self) -> Mapping[str, object] | None:
        """What was gathered before an early end, never typed as complete."""

    def require_verified(self) -> T:
        """Returns the output when the Verdict is `verified`; otherwise raises `NotVerifiedError`."""

@final
@dataclass(frozen=True, kw_only=True)
class NotVerifiedDetails:
    verdict: Verdict
    ended: Ended
    reported: Reported
    reasons: Sequence[str]
    """Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects."""

@final
class NotVerifiedError(PageError):
    @property
    def details(self) -> NotVerifiedDetails: ...
