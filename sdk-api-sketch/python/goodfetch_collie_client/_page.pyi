"""Collie's page API (ADR 0085, decided in #472).

Playwright's page API (ADR 0019) under Playwright for Python's names and forms where the meaning is the same, with
the guard's guarantees:

- Pages, Sessions and frames are values. The Session is Playwright's context: pages open with `session.new_page()`
  or arrive as popups, and frames are reached through frame locators. There is no active page, no frame pointer and
  no element handle: a locator is resolved again at each use.
- Every action, and every read of one element, waits through the guard until exactly one element matches and is
  actionable, or its deadline passes; a match still ambiguous then raises `TargetNotUniqueError` listing every match.
- The engine runs a page's gestures (actions, keyboard, mouse and touch input, navigations) one at a time in arrival
  order; reads, waits, `evaluate` and answers to routes and dialogs run concurrently.
- A page function is JavaScript source run in the page, with one explicit argument; values cross as JSON.
- Routes and dialog handlers answer requests: later ones first, the first to settle wins, one that settles nothing
  falls through, and past the last one a request goes to the network and a dialog gets Playwright's default, so a
  handler that answers nothing never stalls the page.
- A gesture that waits on the program (a locator handler's turn before an attempt, or a route's answer to a request
  it waits on, after its input) gives up its page's turn, only between its own inputs, so a handler's own gestures run
  and no two gestures' inputs interleave. An open dialog blocks input, so a dialog handler answers before it acts.

Waits come from ADR 0080 (`_time`), events and subscriptions from ADR 0081 (`_concurrency`).
"""

import re
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from datetime import timedelta
from pathlib import Path
from types import TracebackType
from typing import Literal, Self, TypeAlias, TypedDict, final, overload

from goodfetch_collie_client._ai import AIVerbs, AsyncAIVerbs
from goodfetch_collie_client._catalog import AsyncRawCall, RawCall
from goodfetch_collie_client._concurrency import AsyncListener, Listener, Subscription
from goodfetch_collie_client._data import JSONObject, JSONValue
from goodfetch_collie_client._lifecycle import AsyncSession, Session
from goodfetch_collie_client._results import PageError, ProgramError, RefusedError, UnavailableError
from goodfetch_collie_client._sessions import HeadersInput, JSONValueWithSecrets, SecretPlaceholder
from goodfetch_collie_client._time import Count, LoadState, NotGiven, Timeout, URLMatcher

AriaRole: TypeAlias = Literal[
    "alert",
    "button",
    "checkbox",
    "combobox",
    "dialog",
    "heading",
    "link",
    "listbox",
    "listitem",
    "menuitem",
    "option",
    "radio",
    "row",
    "searchbox",
    "tab",
    "table",
    "textbox",
]
"""ARIA roles, as `get_by_role` takes them. A sample: the SDK lists every ARIA role."""

TextMatch: TypeAlias = str | re.Pattern[str]
"""A string matches as a case-insensitive substring after whitespace is normalized, as in Playwright, or exactly
with `exact=True`. A compiled pattern runs in the engine, in the Rust `regex` syntax (ADR 0081)."""

Modifier: TypeAlias = Literal["Alt", "Control", "ControlOrMeta", "Meta", "Shift"]
MouseButton: TypeAlias = Literal["left", "right", "middle"]
World: TypeAlias = Literal["main", "isolated"]
"""`main`, the default, sees the page's own globals and is marked in Provenance (ADR 0019); `isolated` runs where
the page can neither see nor hook the code."""

class Position(TypedDict):
    x: float
    y: float

class BoundingBox(TypedDict):
    x: float
    y: float
    width: float
    height: float

@final
@dataclass(frozen=True, kw_only=True)
class FilePayload:
    """A file held in memory."""

    name: str
    mime_type: str
    buffer: bytes

InputFiles: TypeAlias = str | Path | FilePayload | Sequence[str | Path] | Sequence[FilePayload]
"""Paths on the program's machine, which the SDK reads and streams over the connection, or files in memory. An
empty list clears the input."""

ResourceType: TypeAlias = (
    Literal[
        "document",
        "stylesheet",
        "image",
        "media",
        "font",
        "script",
        "texttrack",
        "xhr",
        "fetch",
        "eventsource",
        "websocket",
        "manifest",
        "other",
    ]
    | str
)
"""A newer engine's resource type is kept as received (G3)."""

AbortReason: TypeAlias = Literal[
    "aborted",
    "accessdenied",
    "addressunreachable",
    "blockedbyclient",
    "blockedbyresponse",
    "connectionaborted",
    "connectionclosed",
    "connectionfailed",
    "connectionrefused",
    "connectionreset",
    "internetdisconnected",
    "namenotresolved",
    "timedout",
    "failed",
]

ElementState: TypeAlias = Literal["attached", "detached", "visible", "hidden"]

@final
@dataclass(frozen=True, kw_only=True)
class ConsoleMessageLocation:
    url: str
    line_number: int
    column_number: int

@final
@dataclass(frozen=True, kw_only=True)
class ConsoleMessage:
    """A console message: Playwright's properties."""

    type: Literal["log", "debug", "info", "error", "warning", "dir", "table", "trace", "assert", "count", "timeEnd"] | str
    text: str
    location: ConsoleMessageLocation

@final
@dataclass(frozen=True, kw_only=True)
class WebError:
    """An exception the page's own scripts did not catch (`pageerror`). Playwright's name; never `PageError`, which is
    a group of Goodfetch's errors."""

    name: str
    message: str
    stack: str | None

@final
@dataclass(frozen=True, kw_only=True)
class WebSocketFrame:
    payload: str | bytes

# The page API's own Error codes (ADR 0079's naming rule).

class SelectorInvalidError(ProgramError):
    """`selector_invalid`: a selector that does not parse, or a Playwright-only engine, `>>` or a Playwright CSS
    extension (`:has-text()`, `:visible`, …); `fix` names the replacing method."""

class TargetUnsupportedError(PageError):
    """`target_unsupported`: the target is of the wrong kind for the action, raised at once."""

class PageFunctionFailedError(ProgramError):
    """`page_function_failed`: the page function raised; a `ReferenceError`'s fix names the argument."""

class PageValueInvalidError(ProgramError):
    """`page_value_invalid`: a value crossing to or from the page is not JSON; its details name the path and kind."""

class PageHandlerFailedError(ProgramError):
    """`page_handler_failed`: a route, dialog or locator handler raised; the original is its `__cause__`."""

class RouteAlreadyHandledError(ProgramError):
    """`route_already_handled`: a route settled twice, or after its handler ended."""

class NavigationInterruptedError(PageError):
    """`navigation_interrupted`: superseded by another navigation before it committed."""

class PageNavigatedDuringCallError(PageError):
    """`page_navigated_during_call` (ADR 0114): on a page other code also drives (a page borrowed with `run.borrow()`, a
    Lent page while a library is attached, a page with an open CDP session), an AI verb's page committed a navigation to
    a new document that the verb did not start, between its first read of the page and its return. Nothing was written
    to the Plan, and an `act` sent no input. Retryable: await each Collie call before the program's own driver acts on
    the page; the page may also have navigated by itself. `details` carries `url` and `navigated_to`."""

class ResponseNotJSONError(PageError):
    """`response_not_json`: `json()` on a body that is not JSON."""

class NavigationFailedError(PageError):
    """`navigation_failed`: the site never answered, with the browser's network error. Retryable."""

@final
@dataclass(frozen=True, kw_only=True)
class TooManyRedirectsDetails:
    url: str
    """The URL the chain started from."""
    chain: Sequence[str]
    """Each hop followed, in order."""

@final
class TooManyRedirectsError(PageError):
    """`too_many_redirects` (ADR 0118): a redirect chain longer than the browser's 20, `page.request`'s `max_redirects`
    or a crawl's `fetch.redirect.max`. A loop answers the same way again, so not retryable."""

    @property
    def details(self) -> TooManyRedirectsDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class ResponseTruncatedDetails:
    url: str
    received_bytes: int
    expected_bytes: int | None
    """`None` when the body had no length, such as a chunked or an HTTP/2 one."""

@final
class ResponseTruncatedError(PageError):
    """`response_truncated` (ADR 0118): a body that broke off, short of its `Content-Length`, a chunked body missing its
    last chunk, a reset stream or a compressed stream cut before its end. Raised by a navigation waiting for
    `domcontentloaded` or `load` whose main document was cut, by a cut response's body read, and by a cut
    `page.request`; never part of a body returned as the whole. Retryable."""

    @property
    def details(self) -> ResponseTruncatedDetails: ...

class PageCrashedError(UnavailableError):
    """`page_crashed`: the renderer crashed."""

class BodyUnavailableError(UnavailableError):
    """`body_unavailable`: the browser no longer holds the body; `fix` names `wait_for_request_finished`."""

class DownloadFailedError(UnavailableError):
    """`download_failed`."""

class CDPCommandRefusedError(RefusedError):
    """`cdp_command_refused`: a command beyond the session's page, or one that would close a Borrowed browser."""

# Async API.

class _AsyncLocates:
    """The locator constructors that pages, frame locators and locators share. None does I/O."""

    def locator(
        self,
        selector: str,
        *,
        has_text: TextMatch | None = None,
        has_not_text: TextMatch | None = None,
        has: AsyncLocator | None = None,
        has_not: AsyncLocator | None = None,
    ) -> AsyncLocator:
        """CSS, or XPath when the string starts with `//`, `..` or `xpath=`. CSS pierces open shadow roots; XPath
        does not. Playwright's other selector engines, `>>` chains and its CSS extensions (`:has-text()`, `:text()`,
        `:visible`, `:nth-match()`) raise `SelectorInvalidError`."""

    def get_by_role(
        self,
        role: AriaRole,
        *,
        name: TextMatch | None = None,
        exact: bool = False,
        checked: bool | None = None,
        disabled: bool | None = None,
        expanded: bool | None = None,
        include_hidden: bool = False,
        level: int | None = None,
        pressed: bool | None = None,
        selected: bool | None = None,
    ) -> AsyncLocator: ...
    def get_by_text(self, text: TextMatch, *, exact: bool = False) -> AsyncLocator: ...
    def get_by_label(self, text: TextMatch, *, exact: bool = False) -> AsyncLocator: ...
    def get_by_placeholder(self, text: TextMatch, *, exact: bool = False) -> AsyncLocator: ...
    def get_by_alt_text(self, text: TextMatch, *, exact: bool = False) -> AsyncLocator: ...
    def get_by_title(self, text: TextMatch, *, exact: bool = False) -> AsyncLocator: ...
    def get_by_test_id(self, test_id: TextMatch) -> AsyncLocator:
        """Matches the attribute named by the setting `locator.test_id_attribute`, `data-testid` by default."""

    def frame_locator(self, selector: str | None = None) -> AsyncFrameLocator:
        """Enters an iframe, same-origin or cross-origin alike; without a selector, every frame of the page."""

@final
class AsyncFrameLocator(_AsyncLocates):
    """An iframe's content, resolved again at each use. Strict: its `iframe` must be unique."""

    @property
    def owner(self) -> AsyncLocator: ...

@final
class AsyncLocator(_AsyncLocates, AsyncAIVerbs, AsyncRawCall):
    """A lazy description of elements, built without I/O and resolved again at each use."""

    def filter(
        self,
        *,
        has_text: TextMatch | None = None,
        has_not_text: TextMatch | None = None,
        has: AsyncLocator | None = None,
        has_not: AsyncLocator | None = None,
        visible: bool | None = None,
    ) -> AsyncLocator: ...
    def and_(self, locator: AsyncLocator) -> AsyncLocator: ...
    def or_(self, locator: AsyncLocator) -> AsyncLocator: ...
    @property
    def first(self) -> AsyncLocator:
        """Opts out of strictness: the first match."""

    @property
    def last(self) -> AsyncLocator: ...
    def nth(self, index: int) -> AsyncLocator: ...
    @property
    def content_frame(self) -> AsyncFrameLocator: ...
    async def all(self) -> list[AsyncLocator]:
        """One locator per element matching now. Answers at once, with no timeout."""

    # Gestures: guarded, and run one at a time on the page. `force=True` skips the actionability checks and the
    # replay, keeps uniqueness and marks the action unguarded: the setting `action.force` for this call (ADR 0023).
    async def click(
        self,
        *,
        button: MouseButton = "left",
        click_count: int = 1,
        delay: timedelta | None = None,
        modifiers: Sequence[Modifier] = (),
        position: Position | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> None:
        """Waits until the target is unique, visible, stable, enabled and receiving the click, then clicks; a click
        that lands elsewhere is cancelled and replayed."""

    async def dblclick(
        self,
        *,
        button: MouseButton = "left",
        modifiers: Sequence[Modifier] = (),
        position: Position | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> None: ...
    async def tap(self, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    async def hover(
        self, *, modifiers: Sequence[Modifier] = (), position: Position | None = None, force: bool = False, timeout: Timeout = ...
    ) -> None: ...
    async def check(self, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    async def uncheck(self, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    async def set_checked(
        self, checked: bool, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...
    ) -> None: ...
    async def drag_to(
        self,
        target: AsyncLocator,
        *,
        source_position: Position | None = None,
        target_position: Position | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> None: ...
    async def fill(self, value: str | SecretPlaceholder, *, force: bool = False, timeout: Timeout = ...) -> None:
        """Clears the field and inserts the text as typed text arrives; on a `<label>`, into its control. A target that
        cannot hold text raises `TargetUnsupportedError` at once; a read-only one is waited for. A Secret placeholder is
        typed by the engine, checked against the domain of the field's frame (ADR 0092); reads of the field then show
        `%name%`."""

    async def clear(self, *, force: bool = False, timeout: Timeout = ...) -> None: ...
    async def press(self, key: str, *, delay: timedelta | None = None, force: bool = False, timeout: Timeout = ...) -> None:
        """A key or a combination, such as `Enter` or `ControlOrMeta+A`."""

    async def press_sequentially(
        self, text: str | SecretPlaceholder, *, delay: timedelta | None = None, force: bool = False, timeout: Timeout = ...
    ) -> None:
        """Types key by key, for fields that react to each keystroke."""

    async def select_option(
        self,
        value: str | Sequence[str] | None = None,
        *,
        index: int | Sequence[int] | None = None,
        label: str | Sequence[str] | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> list[str]:
        """On a `<select>`, or the control a `<label>` points to. Returns the values selected."""

    async def select_text(self, *, force: bool = False, timeout: Timeout = ...) -> None: ...
    async def set_input_files(self, files: InputFiles, *, force: bool = False, timeout: Timeout = ...) -> None:
        """On a file input, or the one a `<label>` points to."""

    async def focus(self, *, timeout: Timeout = ...) -> None: ...
    async def blur(self, *, timeout: Timeout = ...) -> None: ...
    async def scroll_into_view_if_needed(self, *, timeout: Timeout = ...) -> None: ...

    # Reads: one round trip each, whatever the number of matches.
    async def text_content(self, *, timeout: Timeout = ...) -> str:
        """Waits for one element, as an action does, under the action timeout."""

    async def inner_text(self, *, timeout: Timeout = ...) -> str: ...
    async def inner_html(self, *, timeout: Timeout = ...) -> str: ...
    async def input_value(self, *, timeout: Timeout = ...) -> str: ...
    async def get_attribute(self, name: str, *, timeout: Timeout = ...) -> str | None:
        """`None` when the attribute is absent: an answer, not a failure."""

    async def bounding_box(self, *, timeout: Timeout = ...) -> BoundingBox | None: ...
    async def is_enabled(self, *, timeout: Timeout = ...) -> bool: ...
    async def is_disabled(self, *, timeout: Timeout = ...) -> bool: ...
    async def is_checked(self, *, timeout: Timeout = ...) -> bool: ...
    async def is_editable(self, *, timeout: Timeout = ...) -> bool: ...
    async def is_visible(self) -> bool:
        """Answers at once, with no timeout: no match is not visible; two matches raise `TargetNotUniqueError`."""

    async def is_hidden(self) -> bool: ...
    async def count(self) -> int:
        """Answers at once. To wait for a count, `wait_for_count`."""

    async def all_text_contents(self) -> list[str]: ...
    async def all_inner_texts(self) -> list[str]: ...
    async def screenshot(
        self, *, type: Literal["png", "jpeg"] = "png", mask: Sequence[AsyncLocator] = (), path: str | Path | None = None, timeout: Timeout = ...
    ) -> bytes: ...

    # Page functions.
    async def evaluate(
        self, expression: str, arg: JSONValue | NotGiven = ..., *, world: World = "main", timeout: Timeout = ...
    ) -> JSONValue:
        """JavaScript source given the element first and then `arg`; values cross as JSON."""

    async def evaluate_all(
        self, expression: str, arg: JSONValue | NotGiven = ..., *, world: World = "main", timeout: Timeout = ...
    ) -> JSONValue: ...

    # Waits (ADR 0080).
    async def wait_for_function(self, expression: str, arg: JSONValue | NotGiven = ..., *, timeout: Timeout = ...) -> JSONValue:
        """Waits until the page function, given the element first, returns a truthy value; resolved again at each check."""

    async def wait_for(self, *, state: ElementState = "visible", timeout: Timeout = ...) -> None:
        """Playwright's `wait_for`: one element in the given state."""

    async def wait_for_count(self, count: int | Count, *, timeout: Timeout = ...) -> int:
        """Waits until the number of matches is in range: a different behaviour, so a different name (S3)."""

@final
class AsyncRequest:
    """A request the page sent, under Playwright for Python's properties."""

    @property
    def url(self) -> str: ...
    @property
    def method(self) -> str: ...
    @property
    def headers(self) -> Mapping[str, str]:
        """Names lower-cased, as the browser sent them."""

    @property
    def post_data(self) -> str | None: ...
    @property
    def resource_type(self) -> ResourceType: ...
    @property
    def redirected_from(self) -> AsyncRequest | None: ...
    @property
    def failure(self) -> str | None:
        """The browser's network error for a failed request, such as `net::ERR_CONNECTION_RESET`."""

    def is_navigation_request(self) -> bool: ...
    async def response(self, *, timeout: Timeout = ...) -> AsyncResponse | None: ...

@final
class AsyncResponse:
    """A response, from the page, from `route.fetch()` or from `page.request`."""

    @property
    def url(self) -> str: ...
    @property
    def status(self) -> int: ...
    @property
    def status_text(self) -> str: ...
    @property
    def ok(self) -> bool:
        """`True` for a status from 200 to 299. It says nothing about the page itself: a login page that answers 200
        is `ok`, and the Page check judges pages (ADR 0006)."""

    @property
    def headers(self) -> Mapping[str, str]: ...
    @property
    def request(self) -> AsyncRequest: ...
    @property
    def from_service_worker(self) -> bool:
        """`True` when a site's service worker answered the request itself, typically from its own cache, so it
        reached no network and no route answered it (ADR 0121). Playwright's property."""

    async def body(self, *, timeout: Timeout = ...) -> bytes:
        """Read on demand. Never fails for a response from `wait_for_request_finished`, `route.fetch()` or
        `page.request`; elsewhere a body the browser has dropped raises `BodyUnavailableError`. A body that broke off
        raises `ResponseTruncatedError` (ADR 0118)."""

    async def text(self, *, timeout: Timeout = ...) -> str:
        """The body decoded with the response's own encoding, where Playwright always decodes UTF-8 (ADR 0119): a
        navigation's document response with the encoding its document was decoded with, so it agrees with
        `page.content()`; any other with its BOM, its `Content-Type` charset, the `<meta>` prescan for HTML or the XML
        declaration for XML, then UTF-8. Bad bytes become U+FFFD; it never raises on them."""

    async def json(self, *, timeout: Timeout = ...) -> JSONValue:
        """UTF-8 whatever the charset (RFC 8259), a leading BOM ignored. A body that is not JSON raises
        `ResponseNotJSONError`. Narrow it with the program's own model."""

@final
class AsyncRoute:
    """An intercepted request waiting for the program's answer. The first route to settle it wins: `continue_`,
    `fulfill`, `abort`, or `fetch` then `fulfill`. `fallback` passes it to the route registered before this one; a
    handler that ends without settling falls back too, and past the last route the request goes to the network.
    Settling twice, or after the handler ended, raises `RouteAlreadyHandledError`. What a route sends on still passes
    the run's domain policy (ADR 0014) and Filter lists (ADR 0047)."""

    @property
    def request(self) -> AsyncRequest:
        """The request as the page sent it, before any override."""

    async def continue_(
        self,
        *,
        url: str | None = None,
        method: str | None = None,
        headers: HeadersInput | None = None,
        post_data: str | bytes | None = None,
    ) -> None: ...
    async def fulfill(
        self,
        *,
        status: int | None = None,
        headers: Mapping[str, str] | None = None,
        content_type: str | None = None,
        body: str | bytes | None = None,
        json: JSONValue | NotGiven = ...,
        response: AsyncResponse | None = None,
    ) -> None: ...
    async def abort(self, error_code: AbortReason = "failed") -> None: ...
    async def fallback(
        self,
        *,
        url: str | None = None,
        method: str | None = None,
        headers: HeadersInput | None = None,
        post_data: str | bytes | None = None,
    ) -> None: ...
    async def fetch(
        self,
        *,
        url: str | None = None,
        method: str | None = None,
        headers: HeadersInput | None = None,
        post_data: str | bytes | None = None,
        timeout: Timeout = ...,
    ) -> AsyncResponse:
        """Sends the request through the engine and returns its response, body kept, for `fulfill`."""

AsyncRouteHandler: TypeAlias = Callable[[AsyncRoute], Awaitable[None]]
"""Answers matching requests; its return value is ignored. If it raises, its request is aborted, never sent
half-changed, and the route is removed; `PageHandlerFailedError` is raised by a call in flight that waited on that
request, or else by the next call on the value it was registered on, as ADR 0081's listeners do."""

@final
class AsyncDialog:
    """A JavaScript dialog waiting for the program's answer."""

    @property
    def type(self) -> Literal["alert", "confirm", "prompt", "beforeunload"]: ...
    @property
    def message(self) -> str: ...
    @property
    def default_value(self) -> str: ...
    async def accept(self, prompt_text: str | None = None) -> None: ...
    async def dismiss(self) -> None: ...

AsyncDialogHandler: TypeAlias = Callable[[AsyncDialog], Awaitable[None]]
"""Answers dialogs under the routes' rules; past the last handler, or with none, a dialog gets Playwright's default: an
`alert`, `confirm` or `prompt` is dismissed (ADR 0014) and a `beforeunload` dialog is accepted."""

@final
class AsyncDownload:
    """A download, kept by the engine with its Run."""

    @property
    def url(self) -> str: ...
    @property
    def suggested_filename(self) -> str: ...
    async def failure(self, *, timeout: Timeout = ...) -> str | None:
        """Waits for the download to end: the reason it failed, or `None`. An answer, not a failure."""

    async def save_as(self, path: str | Path, *, timeout: Timeout = ...) -> None:
        """Streams the file to this path on the program's machine; a failed download raises `DownloadFailedError`.

        A failed write raises `PathNotWritableError` or `FileWriteFailedError`, leaving the path as it was (ADR 0116).
        """

    async def cancel(self, *, timeout: Timeout = ...) -> None: ...

@final
class AsyncFileChooser:
    """A file chooser the page opened, answered with `set_files`, a gesture."""

    def is_multiple(self) -> bool: ...
    async def set_files(self, files: InputFiles, *, timeout: Timeout = ...) -> None: ...

@final
class AsyncWebSocket:
    @property
    def url(self) -> str: ...
    def is_closed(self) -> bool: ...
    @overload
    def on(self, event: Literal["framesent", "framereceived"], listener: AsyncListener[WebSocketFrame]) -> Subscription: ...
    @overload
    def on(self, event: Literal["close"], listener: AsyncListener[None]) -> Subscription: ...

class AsyncKeyboard:
    """Input with no target: nothing to guard, so marked unguarded in Provenance."""

    async def press(self, key: str, *, delay: timedelta | None = None, timeout: Timeout = ...) -> None: ...
    async def down(self, key: str, *, timeout: Timeout = ...) -> None: ...
    async def up(self, key: str, *, timeout: Timeout = ...) -> None: ...
    async def type(self, text: str, *, delay: timedelta | None = None, timeout: Timeout = ...) -> None: ...
    async def insert_text(self, text: str, *, timeout: Timeout = ...) -> None: ...

class AsyncMouse:
    """Input with no target: nothing to guard, so marked unguarded in Provenance."""

    async def move(self, x: float, y: float, *, steps: int = 1, timeout: Timeout = ...) -> None: ...
    async def down(self, *, button: MouseButton = "left", click_count: int = 1, timeout: Timeout = ...) -> None: ...
    async def up(self, *, button: MouseButton = "left", click_count: int = 1, timeout: Timeout = ...) -> None: ...
    async def click(
        self, x: float, y: float, *, button: MouseButton = "left", click_count: int = 1, timeout: Timeout = ...
    ) -> None: ...
    async def dblclick(self, x: float, y: float, *, button: MouseButton = "left", timeout: Timeout = ...) -> None: ...
    async def wheel(self, delta_x: float, delta_y: float, *, timeout: Timeout = ...) -> None: ...

class AsyncTouchscreen:
    async def tap(self, x: float, y: float, *, timeout: Timeout = ...) -> None: ...

class AsyncPageRequest:
    """Playwright's request API on the page's Session, sent through the browser-fetch tier so the Session's identity
    does not change (ADR 0020). Its responses keep their bodies."""

    async def fetch(
        self,
        url: str,
        *,
        method: str = "GET",
        headers: HeadersInput | None = None,
        params: Mapping[str, str | int | bool] | None = None,
        data: str | bytes | JSONValueWithSecrets | NotGiven = ...,
        max_redirects: int | None = None,
        timeout: Timeout = ...,
    ) -> AsyncResponse: ...
    async def get(self, url: str, *, headers: HeadersInput | None = None, timeout: Timeout = ...) -> AsyncResponse: ...
    async def post(
        self,
        url: str,
        *,
        headers: HeadersInput | None = None,
        data: str | bytes | JSONValueWithSecrets | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AsyncResponse: ...

    async def put(
        self,
        url: str,
        *,
        headers: HeadersInput | None = None,
        data: str | bytes | JSONValueWithSecrets | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AsyncResponse: ...
    async def patch(
        self,
        url: str,
        *,
        headers: HeadersInput | None = None,
        data: str | bytes | JSONValueWithSecrets | NotGiven = ...,
        timeout: Timeout = ...,
    ) -> AsyncResponse: ...
    async def delete(self, url: str, *, headers: HeadersInput | None = None, timeout: Timeout = ...) -> AsyncResponse: ...
    async def head(self, url: str, *, headers: HeadersInput | None = None, timeout: Timeout = ...) -> AsyncResponse: ...

@final
class AsyncCDPSession:
    """Raw Chrome DevTools Protocol for one page, over the program's own connection: a raw CDP attach under ADR 0017
    (egress proxy, or ADR 0056's interception on a Borrowed browser; no Secret typed; results marked unguarded; the
    attach audited). It reaches only its page's target;
    commands beyond it, and anything that would close a Borrowed browser, raise `CDPCommandRefusedError`. Semver covers
    this shape, never the protocol, which is Chromium's."""

    async def send(self, method: str, params: JSONObject | None = None, *, timeout: Timeout = ...) -> JSONValue: ...
    def on(self, event: str, listener: AsyncListener[JSONValue]) -> Subscription: ...
    async def close(self, *, timeout: Timeout = ...) -> None: ...
    async def __aenter__(self) -> Self: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

class AsyncPageAPI(_AsyncLocates):
    """The page API beside the waits (ADR 0080) and events (ADR 0081)."""

    # Navigations are gestures; their timeout is the navigation phase's.
    async def goto(
        self, url: str, *, wait_until: LoadState = "load", referer: str | None = None, timeout: Timeout = ...
    ) -> AsyncResponse | None:
        """A 404 or a 500 is a response, a value; `None` for a navigation within the same document."""

    async def reload(self, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> AsyncResponse | None: ...
    async def go_back(self, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> AsyncResponse | None: ...
    async def go_forward(self, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> AsyncResponse | None: ...
    async def set_content(self, html: str, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> None: ...
    @property
    def url(self) -> str:
        """Read without a round trip, kept current by the engine's navigation events."""

    async def title(self, *, timeout: Timeout = ...) -> str: ...
    async def content(self, *, timeout: Timeout = ...) -> str: ...
    def is_closed(self) -> bool: ...
    @property
    def session(self) -> AsyncSession:
        """The Session this page belongs to: Playwright's `page.context`."""

    @property
    def keyboard(self) -> AsyncKeyboard: ...
    @property
    def mouse(self) -> AsyncMouse: ...
    @property
    def touchscreen(self) -> AsyncTouchscreen: ...
    @property
    def request(self) -> AsyncPageRequest: ...
    async def evaluate(
        self, expression: str, arg: JSONValue | NotGiven = ..., *, world: World = "main", timeout: Timeout = ...
    ) -> JSONValue:
        """JavaScript source run in the page with one explicit argument: a function if it parses as one, otherwise an
        expression. Values cross as JSON both ways; anything else raises `PageValueInvalidError` naming its path."""

    async def wait_for_function(
        self,
        expression: str,
        arg: JSONValue | NotGiven = ...,
        *,
        polling: Literal["raf"] | timedelta = "raf",
        timeout: Timeout = ...,
    ) -> JSONValue:
        """Waits until the page function returns a truthy value, and returns it."""

    def add_init_script(self, script: str, arg: JSONValue | NotGiven = ...) -> Subscription:
        """Runs before the page's own scripts in every new document, marked in Provenance; registered when called."""

    async def screenshot(
        self,
        *,
        full_page: bool = False,
        clip: BoundingBox | None = None,
        type: Literal["png", "jpeg"] = "png",
        mask: Sequence[AsyncLocator] = (),
        path: str | Path | None = None,
        timeout: Timeout = ...,
    ) -> bytes:
        """Secrets are masked by the engine (ADR 0014); `path` also writes it on the program's machine.

        That file is written whole or not at all; a failed write raises `PathNotWritableError` or
        `FileWriteFailedError` and returns no bytes (ADR 0116).
        """

    async def pdf(self, *, format: str | None = None, path: str | Path | None = None, timeout: Timeout = ...) -> bytes:
        """Chromium only."""

    def route(self, url: URLMatcher, handler: AsyncRouteHandler) -> Subscription:
        """Answers requests matching `url`, registered when called; the handler runs as a task in the page's group. A
        glob or a compiled pattern matches in the engine; a predicate runs here and, like a route on every URL, has
        every request wait for the program (ADR 0081). Page routes run before the Session's."""

    def add_locator_handler(
        self,
        locator: AsyncLocator,
        handler: Callable[[AsyncLocator], Awaitable[None]],
        *,
        no_wait_after: bool = False,
        times: int | None = None,
    ) -> Subscription:
        """Before each attempt of a guarded action, when `locator` is visible, the guard calls the handler, then waits
        for the locator to be hidden. Handlers run one at a time, their own actions trigger none, and their time
        counts toward the action's timeout; one that raises fails the action it interrupted with `PageHandlerFailedError`
        and stays registered. While a handler runs, the interrupted action gives up the page's turn."""

    async def new_cdp_session(self, *, timeout: Timeout = ...) -> AsyncCDPSession:
        """Chromium only: elsewhere it raises `CapabilityUnavailableError`."""

# Sync API: the same concepts, generated from the same catalog (ADR 0038).

class _Locates:
    def locator(
        self,
        selector: str,
        *,
        has_text: TextMatch | None = None,
        has_not_text: TextMatch | None = None,
        has: Locator | None = None,
        has_not: Locator | None = None,
    ) -> Locator: ...
    def get_by_role(
        self,
        role: AriaRole,
        *,
        name: TextMatch | None = None,
        exact: bool = False,
        checked: bool | None = None,
        disabled: bool | None = None,
        expanded: bool | None = None,
        include_hidden: bool = False,
        level: int | None = None,
        pressed: bool | None = None,
        selected: bool | None = None,
    ) -> Locator: ...
    def get_by_text(self, text: TextMatch, *, exact: bool = False) -> Locator: ...
    def get_by_label(self, text: TextMatch, *, exact: bool = False) -> Locator: ...
    def get_by_placeholder(self, text: TextMatch, *, exact: bool = False) -> Locator: ...
    def get_by_alt_text(self, text: TextMatch, *, exact: bool = False) -> Locator: ...
    def get_by_title(self, text: TextMatch, *, exact: bool = False) -> Locator: ...
    def get_by_test_id(self, test_id: TextMatch) -> Locator: ...
    def frame_locator(self, selector: str | None = None) -> FrameLocator: ...

@final
class FrameLocator(_Locates):
    @property
    def owner(self) -> Locator: ...

@final
class Locator(_Locates, AIVerbs, RawCall):
    """A lazy description of elements, built without I/O and resolved again at each use."""

    def filter(
        self,
        *,
        has_text: TextMatch | None = None,
        has_not_text: TextMatch | None = None,
        has: Locator | None = None,
        has_not: Locator | None = None,
        visible: bool | None = None,
    ) -> Locator: ...
    def and_(self, locator: Locator) -> Locator: ...
    def or_(self, locator: Locator) -> Locator: ...
    @property
    def first(self) -> Locator: ...
    @property
    def last(self) -> Locator: ...
    def nth(self, index: int) -> Locator: ...
    @property
    def content_frame(self) -> FrameLocator: ...
    def all(self) -> list[Locator]: ...
    def click(
        self,
        *,
        button: MouseButton = "left",
        click_count: int = 1,
        delay: timedelta | None = None,
        modifiers: Sequence[Modifier] = (),
        position: Position | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> None: ...
    def dblclick(
        self,
        *,
        button: MouseButton = "left",
        modifiers: Sequence[Modifier] = (),
        position: Position | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> None: ...
    def tap(self, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    def hover(
        self, *, modifiers: Sequence[Modifier] = (), position: Position | None = None, force: bool = False, timeout: Timeout = ...
    ) -> None: ...
    def check(self, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    def uncheck(self, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    def set_checked(self, checked: bool, *, position: Position | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    def drag_to(
        self,
        target: Locator,
        *,
        source_position: Position | None = None,
        target_position: Position | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> None: ...
    def fill(self, value: str | SecretPlaceholder, *, force: bool = False, timeout: Timeout = ...) -> None: ...
    def clear(self, *, force: bool = False, timeout: Timeout = ...) -> None: ...
    def press(self, key: str, *, delay: timedelta | None = None, force: bool = False, timeout: Timeout = ...) -> None: ...
    def press_sequentially(
        self, text: str | SecretPlaceholder, *, delay: timedelta | None = None, force: bool = False, timeout: Timeout = ...
    ) -> None: ...
    def select_option(
        self,
        value: str | Sequence[str] | None = None,
        *,
        index: int | Sequence[int] | None = None,
        label: str | Sequence[str] | None = None,
        force: bool = False,
        timeout: Timeout = ...,
    ) -> list[str]: ...
    def select_text(self, *, force: bool = False, timeout: Timeout = ...) -> None: ...
    def set_input_files(self, files: InputFiles, *, force: bool = False, timeout: Timeout = ...) -> None: ...
    def focus(self, *, timeout: Timeout = ...) -> None: ...
    def blur(self, *, timeout: Timeout = ...) -> None: ...
    def scroll_into_view_if_needed(self, *, timeout: Timeout = ...) -> None: ...
    def text_content(self, *, timeout: Timeout = ...) -> str: ...
    def inner_text(self, *, timeout: Timeout = ...) -> str: ...
    def inner_html(self, *, timeout: Timeout = ...) -> str: ...
    def input_value(self, *, timeout: Timeout = ...) -> str: ...
    def get_attribute(self, name: str, *, timeout: Timeout = ...) -> str | None: ...
    def bounding_box(self, *, timeout: Timeout = ...) -> BoundingBox | None: ...
    def is_enabled(self, *, timeout: Timeout = ...) -> bool: ...
    def is_disabled(self, *, timeout: Timeout = ...) -> bool: ...
    def is_checked(self, *, timeout: Timeout = ...) -> bool: ...
    def is_editable(self, *, timeout: Timeout = ...) -> bool: ...
    def is_visible(self) -> bool: ...
    def is_hidden(self) -> bool: ...
    def count(self) -> int: ...
    def all_text_contents(self) -> list[str]: ...
    def all_inner_texts(self) -> list[str]: ...
    def screenshot(
        self, *, type: Literal["png", "jpeg"] = "png", mask: Sequence[Locator] = (), path: str | Path | None = None, timeout: Timeout = ...
    ) -> bytes: ...
    def evaluate(self, expression: str, arg: JSONValue | NotGiven = ..., *, world: World = "main", timeout: Timeout = ...) -> JSONValue: ...
    def evaluate_all(
        self, expression: str, arg: JSONValue | NotGiven = ..., *, world: World = "main", timeout: Timeout = ...
    ) -> JSONValue: ...
    def wait_for_function(self, expression: str, arg: JSONValue | NotGiven = ..., *, timeout: Timeout = ...) -> JSONValue: ...
    def wait_for(self, *, state: ElementState = "visible", timeout: Timeout = ...) -> None: ...
    def wait_for_count(self, count: int | Count, *, timeout: Timeout = ...) -> int: ...

@final
class Request:
    @property
    def url(self) -> str: ...
    @property
    def method(self) -> str: ...
    @property
    def headers(self) -> Mapping[str, str]: ...
    @property
    def post_data(self) -> str | None: ...
    @property
    def resource_type(self) -> ResourceType: ...
    @property
    def redirected_from(self) -> Request | None: ...
    @property
    def failure(self) -> str | None: ...
    def is_navigation_request(self) -> bool: ...
    def response(self, *, timeout: Timeout = ...) -> Response | None: ...

@final
class Response:
    @property
    def url(self) -> str: ...
    @property
    def status(self) -> int: ...
    @property
    def status_text(self) -> str: ...
    @property
    def ok(self) -> bool: ...
    @property
    def headers(self) -> Mapping[str, str]: ...
    @property
    def request(self) -> Request: ...
    @property
    def from_service_worker(self) -> bool: ...
    def body(self, *, timeout: Timeout = ...) -> bytes: ...
    def text(self, *, timeout: Timeout = ...) -> str: ...
    def json(self, *, timeout: Timeout = ...) -> JSONValue: ...

@final
class Route:
    @property
    def request(self) -> Request: ...
    def continue_(
        self,
        *,
        url: str | None = None,
        method: str | None = None,
        headers: HeadersInput | None = None,
        post_data: str | bytes | None = None,
    ) -> None: ...
    def fulfill(
        self,
        *,
        status: int | None = None,
        headers: Mapping[str, str] | None = None,
        content_type: str | None = None,
        body: str | bytes | None = None,
        json: JSONValue | NotGiven = ...,
        response: Response | None = None,
    ) -> None: ...
    def abort(self, error_code: AbortReason = "failed") -> None: ...
    def fallback(
        self,
        *,
        url: str | None = None,
        method: str | None = None,
        headers: HeadersInput | None = None,
        post_data: str | bytes | None = None,
    ) -> None: ...
    def fetch(
        self,
        *,
        url: str | None = None,
        method: str | None = None,
        headers: HeadersInput | None = None,
        post_data: str | bytes | None = None,
        timeout: Timeout = ...,
    ) -> Response: ...

RouteHandler: TypeAlias = Callable[[Route], None]

@final
class Dialog:
    @property
    def type(self) -> Literal["alert", "confirm", "prompt", "beforeunload"]: ...
    @property
    def message(self) -> str: ...
    @property
    def default_value(self) -> str: ...
    def accept(self, prompt_text: str | None = None) -> None: ...
    def dismiss(self) -> None: ...

DialogHandler: TypeAlias = Callable[[Dialog], None]

@final
class Download:
    @property
    def url(self) -> str: ...
    @property
    def suggested_filename(self) -> str: ...
    def failure(self, *, timeout: Timeout = ...) -> str | None: ...
    def save_as(self, path: str | Path, *, timeout: Timeout = ...) -> None: ...
    def cancel(self, *, timeout: Timeout = ...) -> None: ...

@final
class FileChooser:
    def is_multiple(self) -> bool: ...
    def set_files(self, files: InputFiles, *, timeout: Timeout = ...) -> None: ...

@final
class WebSocket:
    @property
    def url(self) -> str: ...
    def is_closed(self) -> bool: ...
    @overload
    def on(self, event: Literal["framesent", "framereceived"], listener: Listener[WebSocketFrame]) -> Subscription: ...
    @overload
    def on(self, event: Literal["close"], listener: Listener[None]) -> Subscription: ...

class Keyboard:
    def press(self, key: str, *, delay: timedelta | None = None, timeout: Timeout = ...) -> None: ...
    def down(self, key: str, *, timeout: Timeout = ...) -> None: ...
    def up(self, key: str, *, timeout: Timeout = ...) -> None: ...
    def type(self, text: str, *, delay: timedelta | None = None, timeout: Timeout = ...) -> None: ...
    def insert_text(self, text: str, *, timeout: Timeout = ...) -> None: ...

class Mouse:
    def move(self, x: float, y: float, *, steps: int = 1, timeout: Timeout = ...) -> None: ...
    def down(self, *, button: MouseButton = "left", click_count: int = 1, timeout: Timeout = ...) -> None: ...
    def up(self, *, button: MouseButton = "left", click_count: int = 1, timeout: Timeout = ...) -> None: ...
    def click(self, x: float, y: float, *, button: MouseButton = "left", click_count: int = 1, timeout: Timeout = ...) -> None: ...
    def dblclick(self, x: float, y: float, *, button: MouseButton = "left", timeout: Timeout = ...) -> None: ...
    def wheel(self, delta_x: float, delta_y: float, *, timeout: Timeout = ...) -> None: ...

class Touchscreen:
    def tap(self, x: float, y: float, *, timeout: Timeout = ...) -> None: ...

class PageRequest:
    def fetch(
        self,
        url: str,
        *,
        method: str = "GET",
        headers: HeadersInput | None = None,
        params: Mapping[str, str | int | bool] | None = None,
        data: str | bytes | JSONValueWithSecrets | NotGiven = ...,
        max_redirects: int | None = None,
        timeout: Timeout = ...,
    ) -> Response: ...
    def get(self, url: str, *, headers: HeadersInput | None = None, timeout: Timeout = ...) -> Response: ...
    def post(
        self, url: str, *, headers: HeadersInput | None = None, data: str | bytes | JSONValueWithSecrets | NotGiven = ..., timeout: Timeout = ...
    ) -> Response: ...

    def put(
        self, url: str, *, headers: HeadersInput | None = None, data: str | bytes | JSONValueWithSecrets | NotGiven = ..., timeout: Timeout = ...
    ) -> Response: ...
    def patch(
        self, url: str, *, headers: HeadersInput | None = None, data: str | bytes | JSONValueWithSecrets | NotGiven = ..., timeout: Timeout = ...
    ) -> Response: ...
    def delete(self, url: str, *, headers: HeadersInput | None = None, timeout: Timeout = ...) -> Response: ...
    def head(self, url: str, *, headers: HeadersInput | None = None, timeout: Timeout = ...) -> Response: ...

@final
class CDPSession:
    def send(self, method: str, params: JSONObject | None = None, *, timeout: Timeout = ...) -> JSONValue: ...
    def on(self, event: str, listener: Listener[JSONValue]) -> Subscription: ...
    def close(self, *, timeout: Timeout = ...) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

class PageAPI(_Locates):
    def goto(self, url: str, *, wait_until: LoadState = "load", referer: str | None = None, timeout: Timeout = ...) -> Response | None: ...
    def reload(self, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> Response | None: ...
    def go_back(self, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> Response | None: ...
    def go_forward(self, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> Response | None: ...
    def set_content(self, html: str, *, wait_until: LoadState = "load", timeout: Timeout = ...) -> None: ...
    @property
    def url(self) -> str: ...
    def title(self, *, timeout: Timeout = ...) -> str: ...
    def content(self, *, timeout: Timeout = ...) -> str: ...
    def is_closed(self) -> bool: ...
    @property
    def session(self) -> Session: ...
    @property
    def keyboard(self) -> Keyboard: ...
    @property
    def mouse(self) -> Mouse: ...
    @property
    def touchscreen(self) -> Touchscreen: ...
    @property
    def request(self) -> PageRequest: ...
    def evaluate(self, expression: str, arg: JSONValue | NotGiven = ..., *, world: World = "main", timeout: Timeout = ...) -> JSONValue: ...
    def wait_for_function(
        self,
        expression: str,
        arg: JSONValue | NotGiven = ...,
        *,
        polling: Literal["raf"] | timedelta = "raf",
        timeout: Timeout = ...,
    ) -> JSONValue: ...
    def add_init_script(self, script: str, arg: JSONValue | NotGiven = ...) -> Subscription: ...
    def screenshot(
        self,
        *,
        full_page: bool = False,
        clip: BoundingBox | None = None,
        type: Literal["png", "jpeg"] = "png",
        mask: Sequence[Locator] = (),
        path: str | Path | None = None,
        timeout: Timeout = ...,
    ) -> bytes: ...
    def pdf(self, *, format: str | None = None, path: str | Path | None = None, timeout: Timeout = ...) -> bytes: ...
    def route(self, url: URLMatcher, handler: RouteHandler) -> Subscription:
        """Answers requests matching `url`, registered when called; the handler runs on the connection's pool."""

    def add_locator_handler(
        self, locator: Locator, handler: Callable[[Locator], None], *, no_wait_after: bool = False, times: int | None = None
    ) -> Subscription: ...
    def new_cdp_session(self, *, timeout: Timeout = ...) -> CDPSession: ...
