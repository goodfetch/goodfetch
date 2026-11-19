"""The bases of every error a Goodfetch SDK raises (ADR 0079).

Each Error code's generated class extends `GoodfetchError` through its
group's, except Foxhound's `AttemptEndedError`, which extends
`BaseGoodfetchError` alone, outside `Exception` (ADR 0093). Hand-written:
only the seven groups and the codes' classes come from the catalog.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Protocol, TypedDict, TypeVar

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence
    from typing import Unpack

    from goodfetch_collie_client.generated.errors import ErrorGroup
    from goodfetch_collie_client.generated.models import ModelUsage, SettingLimit


class ErrorFields(TypedDict, total=False):
    """What an error carries beyond its code, group and retryable flag (ADR 0079).

    The cause is Python's own chaining, `raise ... from`.
    """

    operation: str | None
    """The operation the call made, by its catalog name, such as `locator.click`."""
    call_id: str | None
    """The call's ID, which finds it in the Run trace."""
    run_id: str | None
    """The Run the call belonged to, inside a Run."""
    target: str | None
    """What the call acted on, as the engine rendered it in Python."""
    url: str | None
    """The page's URL, where one applies."""
    observed: str | None
    """What was there instead of what the call needed."""
    limit: SettingLimit | None
    """The setting that decided the outcome: its key, value, layer and origin."""
    fix: str | None
    """What to do about it, when the engine or the runtime knows."""
    call_log: Sequence[str] | None
    """The engine's steps for the call, as Playwright's call log."""
    usage: ModelUsage | None
    """The usage of the model calls made before the failure, if any (ADR 0101)."""
    agent_task: Mapping[str, object] | None
    """What the task did, on an error `page.agent` raises after it took steps."""
    details: Mapping[str, object] | None
    """The code's details, as the engine sent them, in wire names."""


class BaseGoodfetchError(BaseException):
    """What every error a Goodfetch SDK raises carries, `AttemptEndedError`'s included.

    As `BaseExceptionGroup` is to `ExceptionGroup`: `GoodfetchError` adds
    `Exception`, and Foxhound's `AttemptEndedError`, which ends a crawl
    handler's attempt, does not, so a handler's `except Exception:` lets it
    through, as it lets `asyncio.CancelledError` through (ADR 0093). A
    program catches `GoodfetchError`; catching this class also stops the
    end of an attempt.
    """

    code: str
    """The Error code, as the engine sent it."""
    group: ErrorGroup
    """The code's group, named for its remedy."""
    retryable: bool
    """Whether the same call may succeed when made again."""
    operation: str | None
    """The operation the call made, by its catalog name; the message spells it."""
    call_id: str | None
    """The call's ID, which finds it in the Run trace."""
    run_id: str | None
    """The Run the call belonged to, inside a Run."""
    target: str | None
    """What the call acted on, as the engine rendered it in Python."""
    url: str | None
    """The page's URL, where one applies."""
    observed: str | None
    """What was there instead of what the call needed."""
    limit: SettingLimit | None
    """The setting that decided the outcome, where one did."""
    fix: str | None
    """What to do about it, when the engine or the runtime knows."""
    call_log: list[str]
    """The engine's steps for the call; empty when it sent none (ADR 0015)."""
    usage: ModelUsage | None
    """The usage of the model calls the call made before it failed, when it made any."""
    agent_task: Mapping[str, object] | None
    """On an error `page.agent` raises after its task took steps, what the task did."""
    details: Mapping[str, object]
    """The code's details, in wire names; empty when it has none."""

    def __init__(
        self,
        message: str,
        *,
        code: str,
        group: ErrorGroup,
        retryable: bool,
        **fields: Unpack[ErrorFields],
    ) -> None:
        """Takes the message, the code's fields and what the error carries."""
        super().__init__(message)
        self.code = code
        self.group = group
        self.retryable = retryable
        self.operation = fields.get("operation")
        self.call_id = fields.get("call_id")
        self.run_id = fields.get("run_id")
        self.target = fields.get("target")
        self.url = fields.get("url")
        self.observed = fields.get("observed")
        self.limit = fields.get("limit")
        self.fix = fields.get("fix")
        self.call_log = list(fields.get("call_log") or [])
        self.usage = fields.get("usage")
        self.agent_task = fields.get("agent_task")
        details = fields.get("details")
        self.details = {} if details is None else dict(details)

    @property
    def name(self) -> str:
        """The error's class name, such as `SettingInvalidError`."""
        return type(self).__name__


class GoodfetchError(BaseGoodfetchError, Exception):
    """The base of every error a Goodfetch SDK raises, `AttemptEndedError` aside.

    ADRs 0079 and 0093. The sync and the async clients raise the same
    classes.
    """


_Error = TypeVar("_Error", bound=BaseGoodfetchError)


def runtime_message(
    what_failed: str, *, operation: str | None = None, fix: str | None = None
) -> str:
    """The message of an error the runtime raises itself, in ADR 0079's layout.

    The operation in the caller's form, what failed, then the fix: Python's
    form of a catalog name is the catalog's own (ADR 0099). It never holds
    a chained cause's text, which Python's own reporting prints.
    """
    head = what_failed if operation is None else f"{operation}: {what_failed}"
    return head if fix is None else f"{head}. {fix}"


def in_call(error: _Error, operation: str) -> _Error:
    """`error`, raised by the runtime, as the call `operation` fails with it.

    Its message is headed by the operation and its `operation` field set,
    unless it already names one; its cause is kept. Each call gets its own
    copy, so one failure of the Connection reads as each call it failed.
    """
    if error.operation is not None:
        return error
    copied = type(error).__new__(type(error))
    copied.__dict__.update(error.__dict__)
    copied.args = (f"{operation}: {error}",)
    copied.operation = operation
    copied.__cause__ = error.__cause__
    return copied


class ErrorClass(Protocol):
    """The class of one Error code: its message, then what it carries."""

    def __call__(
        self, message: str, **fields: Unpack[ErrorFields]
    ) -> BaseGoodfetchError:
        """Makes the error from the message the engine rendered."""
        ...
