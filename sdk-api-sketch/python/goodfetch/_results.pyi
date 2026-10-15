"""Results, outcomes and errors (ADR 0079, decided in #466).

Failures raise a `GoodfetchError`: one class per Error code under one of seven groups by remedy. Expected outcomes are
values in the return type, and no argument changes which (G1, S2). The sync and async APIs raise the same classes.
The codes below are a representative subset: each area's spec lists its codes, their groups and their details in
`goodfetch-protocol`, and codegen writes one class per code.
"""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import timedelta
from typing import Generic, Literal, TypeAlias, TypeVar, final

ErrorGroup: TypeAlias = Literal["program", "refused", "page", "model", "deadline_exceeded", "unavailable", "internal"]
"""A group of Error codes sharing one remedy, carried on the wire as the JSON-RPC numeric class."""

KnownErrorCode: TypeAlias = Literal[
    "closed",
    "removed",
    "call_cancelled",
    "handler_failed",
    "listener_failed",
    "hook_failed",
    "limit_reached",
    "page_check_failed",
    "target_not_unique",
    "blocked",
    "not_verified",
    "deadline_exceeded",
    "plugin_crashed",
    "payload_invalid",
    "payload_schema_changed",
    "schema_validator_failed",
    "setting_unknown",
    "setting_invalid",
    "setting_not_applicable",
    "setting_locked",
    "capability_unavailable",
    "secret_unbound",
    "secret_unavailable",
    "secret_domain_not_allowed",
    "secret_input_refused",
    "logged_out",
    "human_pause_declined",
    "session_not_savable",
    "label_duplicate",
    "label_unhandled",
    "label_unknown",
    "attempt_ended",
    "browser_required",
    "retry_requested",
    "response_not_ok",
    "site_preferences_refused",
    "internal",
]
ErrorCode: TypeAlias = KnownErrorCode | str
"""A code a newer engine sends and this SDK does not know is kept as received (G3)."""

SettingsLayer: TypeAlias = Literal["default", "operator_policy", "configuration", "recipe", "run"]
"""The settings layers of ADR 0028, lowest first. A program that is not a Recipe sets the `run` layer (ADR 0083)."""

RunLayerOrigin: TypeAlias = Literal["argument", "settings", "flag", "form"]
"""What set a value at the `run` layer; a call's argument beats `settings`. The environment sets no setting (ADR 0083)."""

@final
@dataclass(frozen=True, kw_only=True)
class LimitSource:
    """Where the limit that ended a call came from (G6)."""

    setting: str
    """The setting's canonical key, such as `timeout.action`."""
    value: object
    layer: SettingsLayer
    """The layer that set the value."""
    origin: RunLayerOrigin | None
    """Set when `layer` is `run`: a call's own argument, a `settings` keyword, a CLI flag or a form (ADRs 0080, 0083)."""

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
    """The deadline passed. Raise the timeout its `limit` names, or wait for the right thing (ADR 0080).

    A code named like its group (`deadline_exceeded`, `internal`) shares the group's class. Every timeout is
    the code `deadline_exceeded`, whatever the phase, except a target still ambiguous at its deadline, which raises
    `TargetNotUniqueError` (ADR 0085).
    """

    @property
    def details(self) -> DeadlineExceededDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class EndedBy:
    """The enclosing call whose timeout ended this one."""

    operation: str
    call_id: str

@final
@dataclass(frozen=True, kw_only=True)
class DeadlineExceededDetails:
    """Details of `deadline_exceeded`.

    The error's `limit` names the phase's setting and its layer; a call's own `timeout` argument is that
    setting at the `run` layer (ADR 0028).
    """

    timeout: timedelta
    elapsed: timedelta
    waited_for: str
    """What the call waited for, rendered in Python."""
    ended_by: EndedBy | None
    """The enclosing call whose timeout ended this one, when it was not the call's own."""

class UnavailableError(GoodfetchError):
    """Something Goodfetch depends on is unreachable or gone. Usually retryable: retry or fix the setup."""

class InternalError(GoodfetchError):
    """A bug in Goodfetch. Report it."""

# One generated class per code: the code in PascalCase plus `Error`.

@final
class ClosedError(ProgramError): ...

@final
class CallCancelledError(UnavailableError):
    """Someone else stopped the call in flight: a person, another Client, or the program closing a parent value.

    Not retryable; `observed` says who, and `fix` names resuming the Run, or the close that stopped the call. The
    caller's own cancellation propagates `asyncio.CancelledError` instead and never raises this (ADR 0080).
    """

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
    """A crawl handler raised something other than a `GoodfetchError`; `__cause__` holds it in the process that raised it."""

    @property
    def details(self) -> HandlerFailedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class ListenerFailedDetails:
    """Which listener's exception removed it."""

    event: str
    """The event the listener was registered for."""

@final
class ListenerFailedError(ProgramError):
    """A listener raised (ADR 0081). It was removed, and this is raised once, by the next call on the value it was
    registered on or by the close that closes that value, its own or a parent's, with the listener's exception as
    `__cause__`. A call that raises it is not sent, and the value stays open; a close that raises it closes the value
    first. If the value is released before either, the SDK logs it at error level instead."""

    @property
    def details(self) -> ListenerFailedDetails: ...

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
    Literal[
        "out_of_scope",
        "domain_policy",
        "duplicate",
        "robots_txt",
        "crawl_delay",
        "use_signals",
        "hook",
        "handler",
    ]
    | str
)
"""Why a crawl request was skipped (ADR 0093's full list); the outcome's `message` carries a Hook's or a handler's own text.

- `out_of_scope`: a redirect left the crawl's scope;
- `domain_policy`: a seed, a redirect or a Hook's rewrite reached a host the run's domain policy refuses (ADRs 0014, 0017);
- `duplicate`: a URL rule the Frontier learned after admitting it merged it with another (ADR 0005);
- `robots_txt`: a Disallow line of the origin's robots.txt, or a robots.txt answered with a 429 or a 5xx, or
  unreachable, with no earlier copy, until the request's retries ran out (ADR 0086); also a handler's own
  navigation refused for it whose `SitePreferencesRefusedError` escaped the handler;
- `crawl_delay`: a Crawl-delay above `site_preferences.max_crawl_delay` (ADR 0086);
- `use_signals`: an AI-use signal that refuses the run's Purpose while `site_preferences.respect_use_signals` is on;
- `hook`: a before-request Hook (ADR 0033);
- `handler`: the handler called `ctx.skip()`.
"""

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestRef:
    """Which crawl request an outcome is about. Outcomes span Labels, so the Payload is not typed here."""

    id: str
    url: str
    label: str
    """The Label's name."""
    depth: int

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestSucceeded:
    request: CrawlRequestRef
    attempts: int
    """Retries are not an outcome: every outcome counts its attempts and keeps the failed ones."""
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestFailed:
    request: CrawlRequestRef
    error: GoodfetchError
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestSkipped:
    request: CrawlRequestRef
    reason: SkipReason
    message: str | None
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestBlocked:
    request: CrawlRequestRef
    block_cause: BlockCause
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestOutcomeUnknown:
    """A kind a newer engine sent that this SDK does not know, kept as received (G3)."""

    request: CrawlRequestRef
    raw: str
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

CrawlRequestOutcome: TypeAlias = (
    CrawlRequestSucceeded | CrawlRequestFailed | CrawlRequestSkipped | CrawlRequestBlocked | CrawlRequestOutcomeUnknown
)
"""A crawl request's ending (ADR 0079; the noun is ADR 0093's). How a program reads outcomes is #478's."""

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
