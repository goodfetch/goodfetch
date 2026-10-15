"""Sessions, logins, the Human pause and secrets in code (ADR 0092, decided in #476).

- A Session opens from its Session source (ADR 0056): `run.new_session(source=...)`. `run.new_page()` opens pages in the
  Run's default Session, reached as `page.session`.
- A login is kept as a Saved identity (`session.save_identity`), never as a clear-text file; a lost one is found by the
  Session's Page check and recovered by ADR 0006's ladder, or raises `LoggedOutError`.
- `page.pause()` is the Human pause: no clock runs while a person acts, in the real window or the Live view, and it ends
  when its condition holds or the person says so.
- A program names a secret by a Secret placeholder, `secret("name")` or `%name%` in an AI instruction, which
  `secrets.<name>` binds to a Secret reference and the domains allowed to receive it. The engine types or sends the
  value; the program never receives it, except what it answers for a `caller:` reference (ADRs 0014, 0015).
"""

import builtins
import re
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime, timedelta
from typing import Literal, NotRequired, TypeAlias, TypedDict, final, overload

from goodfetch._page import AsyncLocator, Locator
from goodfetch._results import PageError, ProgramError, RefusedError, UnavailableError
from goodfetch._time import Timeout, UrlMatcher

# Secrets.

SecretReference: TypeAlias = str
"""A Secret reference, `provider:rest`: `env:QUOTES_PASSWORD`, `keychain:goodfetch/portal`, `op://vault/item/field`, or
`caller:portal` for `connect()`'s `resolve_secret`. It resolves in the engine, at the moment of use: `env:` reads the
engine's environment, which for an Attached engine is `connect()`'s environment map (ADR 0083). A value that is not a
reference is refused with `SettingInvalidError`, so a raw password is never sent."""

@final
class SecretPlaceholder:
    """A Secret placeholder: the name by which the program asks for a secret to be typed or sent. It holds no value;
    `secrets.<name>` binds it, and the engine refuses an unbound one with `SecretUnboundError`."""

    @property
    def name(self) -> str: ...

def secret(name: str) -> SecretPlaceholder:
    """The Secret placeholder `name`, which `%name%` also names in an AI instruction."""

class ReferenceBinding(TypedDict):
    """`secrets.<name>` bound to a Secret reference. `domains` are registrable domains (ADR 0004), `*` only when written,
    and required; they are checked against the frame holding the target field, or a request's URL."""

    reference: SecretReference
    domains: Sequence[str]
    totp: NotRequired[bool]
    """The reference holds a TOTP seed: the engine types the current code. A vault that makes codes itself needs no
    flag."""

class CodeSourceBinding(TypedDict):
    """`secrets.<name>` bound to one of ADR 0006's email or SMS code sources, configured by name; a Human pause takes over
    when no code arrives."""

    code_source: str
    domains: Sequence[str]

SecretBinding: TypeAlias = ReferenceBinding | CodeSourceBinding
"""Set by the program's own code, configuration or an Operator policy, at the connection, a Run or a Session, nearest
winning; a Recipe declares the names it needs and never binds them, and a Client acting for a model never sets one
(`SettingLockedError`)."""

SecretsSettings: TypeAlias = Mapping[str, SecretBinding]
"""`secrets.*`, keyed by placeholder name."""

HeadersInput: TypeAlias = Mapping[str, str | SecretPlaceholder]
"""Header values a request or a route sends: a placeholder is resolved by the engine, checked against the request's
URL, and added at the engine's own interception, where no page script or Service Worker reads it."""

JsonValueWithSecrets: TypeAlias = (
    None | bool | int | float | str | SecretPlaceholder | Sequence["JsonValueWithSecrets"] | Mapping[str, "JsonValueWithSecrets"]
)
"""A JSON body a request sends, whose strings may be placeholders."""

@final
@dataclass(frozen=True, kw_only=True)
class SecretRequest:
    """What `resolve_secret` is asked for."""

    name: str
    """The reference's name after `caller:`."""
    run_id: str
    domain: str | None
    """The registrable domain about to receive it, or `None` for a credential the engine uses itself."""

AsyncResolveSecret: TypeAlias = Callable[[SecretRequest], Awaitable[str]]
"""Answers `caller:` references at the moment of use (ADR 0015's caller provider), on the program's own event loop
(ADR 0081); the value goes straight to the engine, and the SDK keeps no copy. An exception fails the call with
`SecretUnavailableError`."""

ResolveSecret: TypeAlias = Callable[[SecretRequest], str]
"""The sync client's form, run on the connection's thread pool (ADR 0081)."""

# Where a Session's login comes from (ADR 0056). `client_browser` is `run.borrow()`'s, never written.

BrowserName: TypeAlias = Literal["chrome", "edge", "brave", "chromium", "firefox", "safari"]

class SavedIdentitySource(TypedDict):
    kind: Literal["saved_identity"]
    name: str

class StateFileSource(TypedDict):
    kind: Literal["state_file"]
    path: str
    """Read on the program's machine and sent over the connection, so it works with a remote engine."""
    format: NotRequired[Literal["storage_state", "cookies_txt", "cdp_cookies", "session_export"]]
    key: NotRequired[SecretReference]
    """Opens a session export bundle (ADR 0015)."""
    domains: NotRequired[Sequence[str]]

class CookieHeaderSource(TypedDict):
    kind: Literal["cookie_header"]
    header: str
    domain: str

class ProfileDirectorySource(TypedDict):
    kind: Literal["profile_directory"]
    path: str
    browser: BrowserName
    chromium_profile: NotRequired[str]
    domains: Sequence[str]

class InstalledBrowserSource(TypedDict):
    kind: Literal["installed_browser"]
    browser: BrowserName
    chromium_profile: NotRequired[str]
    domains: Sequence[str]

class RunningBrowserSource(TypedDict):
    kind: Literal["running_browser"]
    browser: BrowserName
    domains: Sequence[str]

class CdpUrlSource(TypedDict):
    kind: Literal["cdp_url"]
    url: str
    headers: NotRequired[Mapping[str, SecretReference]]
    domains: NotRequired[Sequence[str]]
    exclusive: NotRequired[bool]
    """Nothing else drives this browser, so secrets may be typed into it (ADR 0056)."""

SessionSource: TypeAlias = (
    SavedIdentitySource
    | StateFileSource
    | CookieHeaderSource
    | ProfileDirectorySource
    | InstalledBrowserSource
    | RunningBrowserSource
    | CdpUrlSource
)
"""`session.source`: exactly one kind (ADR 0056). Only the program's own code, configuration, an Operator policy or a
person sets it; a Recipe never."""

class PageMatch(TypedDict, total=False):
    """What a page looks like, matched in the engine: every key given must match."""

    url: str | re.Pattern[str]
    """A glob or a regular expression, in the Rust `regex` syntax (ADR 0081)."""
    selector: str
    """CSS, or XPath starting with `//`, as `locator()` takes it."""
    text: str | re.Pattern[str]

class PageCheck(TypedDict, total=False):
    """`session.page_check` (ADR 0006): every page of the Session is judged against it. A login page is a lost login,
    which recovery tries to bring back; a page that is neither expected nor a login page fails with
    `page_check_failed`. Without one, the built-in detector applies and pages are weakly checked."""

    login_page: PageMatch
    expected_page: PageMatch

class HttpCredentials(TypedDict):
    """`session.http_credentials`: sent only to a listed domain that challenges, never on a browser other code may read.
    A Recipe and a Client acting for a model never set it."""

    username: str
    password: SecretReference
    domains: Sequence[str]

class SessionRecoverySettings(TypedDict, total=False):
    human_pause: bool
    """`session.recovery.human_pause`: whether ADR 0006's ladder may end in a Human pause; `True` by default."""
    agent: bool
    """`session.recovery.agent`: whether the ladder's AI agent login step runs, the built-in Agent with the Session's
    Secret placeholders; `True` by default, skipped with no model and no login Plan (ADR 0100)."""

class SessionLoginSettings(TypedDict, total=False):
    """`session.*` (ADRs 0056, 0006, 0092)."""

    source: SessionSource
    page_check: PageCheck | None
    recovery: SessionRecoverySettings
    http_credentials: Sequence[HttpCredentials]

class SessionScopeLoginSettings(TypedDict, total=False):
    """`session.*` where a Session opens: its source is `new_session()`'s `source`, one spelling (S2)."""

    page_check: PageCheck | None
    recovery: SessionRecoverySettings
    http_credentials: Sequence[HttpCredentials]

class ProxySettings(TypedDict, total=False):
    """`proxy.*`: part of a Session's identity, one proxy per Session (ADR 0021), with Playwright's fields. Pools by kind
    and provider adapters are this family's too; area 8's spec names their keys."""

    server: str | None
    bypass: str
    """Comma-separated domains that skip the proxy."""
    username: str
    password: SecretReference

Permission: TypeAlias = Literal[
    "clipboard-read", "clipboard-write", "geolocation", "notifications", "camera", "microphone", "local-network-access"
]
"""`browser.permissions`, under Playwright's names. A sample: the SDK lists every one."""

# Cookies and Saved identities.

SameSite: TypeAlias = Literal["Strict", "Lax", "None"]

@final
@dataclass(frozen=True, kw_only=True)
class Cookie:
    """A cookie, with Playwright's fields. `expires` is a time-zone-aware `datetime` in UTC, the SDK's point-in-time
    type, or `None` for a session cookie."""

    name: str
    value: str
    domain: str
    path: str
    expires: datetime | None
    http_only: bool
    secure: bool
    same_site: SameSite
    partition_key: str | None = None

class CookieByUrl(TypedDict):
    name: str
    value: str
    url: str
    expires: NotRequired[datetime | None]
    http_only: NotRequired[bool]
    secure: NotRequired[bool]
    same_site: NotRequired[SameSite]
    partition_key: NotRequired[str]

class CookieByDomain(TypedDict):
    name: str
    value: str
    domain: str
    path: str
    expires: NotRequired[datetime | None]
    http_only: NotRequired[bool]
    secure: NotRequired[bool]
    same_site: NotRequired[SameSite]
    partition_key: NotRequired[str]

CookieInput: TypeAlias = CookieByUrl | CookieByDomain
"""A cookie to add: by `url`, or by `domain` and `path`, as Playwright takes it."""

@final
@dataclass(frozen=True, kw_only=True)
class SavedIdentity:
    """A Saved identity (ADR 0035), as `gf.identities.list()` and `session.save_identity()` return it."""

    name: str
    source: str
    """Where its login came from: an import's source kind, or `session` for a login made in a Session."""
    access_mode: Literal["standard", "identified", "stealth"] | str
    saved_at: datetime
    cookies_expire: Mapping[str, datetime]
    """The earliest cookie expiry per registrable domain: cookies expire then, which says nothing of the site's own
    session (ADR 0056)."""
    bound_to_machine: bool
    """An import on direct egress, or a login saved from a Session that came from one: usable only on this machine
    (ADR 0056)."""

@final
class AsyncIdentities:
    """The install's Saved identities, reached as `gf.identities`. No SDK writes a clear-text `storageState` (ADR 0056)."""

    async def list(self, *, timeout: Timeout = ...) -> builtins.list[SavedIdentity]: ...
    async def import_(self, name: str, source: SessionSource, *, timeout: Timeout = ...) -> SavedIdentity:
        """Imports a login into a Saved identity, as `goodfetch identity import` does. `import` is a keyword (#471)."""
    async def delete(self, name: str, *, timeout: Timeout = ...) -> None: ...
    @overload
    async def export(self, name: str, *, path: str, recipient: str, timeout: Timeout = ...) -> None:
        """ADR 0015's `age`-encrypted bundle, to a public key."""
    @overload
    async def export(self, name: str, *, path: str, passphrase: SecretReference, timeout: Timeout = ...) -> None:
        """ADR 0015's `age`-encrypted bundle, to a passphrase."""

@final
class Identities:
    """The sync client's `gf.identities`."""

    def list(self, *, timeout: Timeout = ...) -> builtins.list[SavedIdentity]: ...
    def import_(self, name: str, source: SessionSource, *, timeout: Timeout = ...) -> SavedIdentity: ...
    def delete(self, name: str, *, timeout: Timeout = ...) -> None: ...
    @overload
    def export(self, name: str, *, path: str, recipient: str, timeout: Timeout = ...) -> None: ...
    @overload
    def export(self, name: str, *, path: str, passphrase: SecretReference, timeout: Timeout = ...) -> None: ...

class AsyncSessionStateApi:
    """A Session's cookies and keeping its identity, beside its page members (ADR 0085)."""

    async def cookies(self, urls: str | Sequence[str] | None = None, *, timeout: Timeout = ...) -> list[Cookie]:
        """Playwright's `context.cookies()`. Offered in the SDKs only, never on the CLI, MCP or Code mode (ADRs 0011,
        0014)."""
    async def add_cookies(self, cookies: Sequence[CookieInput], *, timeout: Timeout = ...) -> None:
        """Kept as session state, encrypted in run state and never logged (ADR 0015). Outside the source's `domains`,
        refused."""
    async def clear_cookies(
        self,
        *,
        name: str | re.Pattern[str] | None = None,
        domain: str | re.Pattern[str] | None = None,
        path: str | re.Pattern[str] | None = None,
        timeout: Timeout = ...,
    ) -> None: ...
    async def save_identity(self, name: str, *, timeout: Timeout = ...) -> SavedIdentity:
        """Keeps this Session's identity, cookies and storage as a Saved identity (ADR 0035), replacing one of that name,
        recorded in the Audit log. A Session whose cookies live outside run state (a Borrowed browser, a Profile
        directory used in place) raises `SessionNotSavableError`."""

class SessionStateApi:
    """The sync client's Session cookies and Saved identity."""

    def cookies(self, urls: str | Sequence[str] | None = None, *, timeout: Timeout = ...) -> list[Cookie]: ...
    def add_cookies(self, cookies: Sequence[CookieInput], *, timeout: Timeout = ...) -> None: ...
    def clear_cookies(
        self,
        *,
        name: str | re.Pattern[str] | None = None,
        domain: str | re.Pattern[str] | None = None,
        path: str | re.Pattern[str] | None = None,
        timeout: Timeout = ...,
    ) -> None: ...
    def save_identity(self, name: str, *, timeout: Timeout = ...) -> SavedIdentity: ...

@final
@dataclass(frozen=True, kw_only=True)
class WebStorageItem:
    name: str
    value: str

@final
class AsyncWebStorage:
    """A page's `localStorage` or `sessionStorage` for its current origin, Playwright's `WebStorage` (1.61). Offered in
    the SDKs only (ADRs 0011, 0014); outside a bound source's `domains`, refused. A value set is session state; a Saved
    identity keeps `localStorage`, and `sessionStorage` travels only in an export bundle."""

    async def items(self, *, timeout: Timeout = ...) -> builtins.list[WebStorageItem]: ...
    async def get_item(self, name: str, *, timeout: Timeout = ...) -> str | None: ...
    async def set_item(self, name: str, value: str, *, timeout: Timeout = ...) -> None: ...
    async def remove_item(self, name: str, *, timeout: Timeout = ...) -> None: ...
    async def clear(self, *, timeout: Timeout = ...) -> None: ...

@final
class WebStorage:
    """The sync client's `WebStorage`."""

    def items(self, *, timeout: Timeout = ...) -> builtins.list[WebStorageItem]: ...
    def get_item(self, name: str, *, timeout: Timeout = ...) -> str | None: ...
    def set_item(self, name: str, value: str, *, timeout: Timeout = ...) -> None: ...
    def remove_item(self, name: str, *, timeout: Timeout = ...) -> None: ...
    def clear(self, *, timeout: Timeout = ...) -> None: ...

class AsyncPageStorage:
    """A page's storage, under Playwright for Python's names."""

    @property
    def local_storage(self) -> AsyncWebStorage: ...
    @property
    def session_storage(self) -> AsyncWebStorage: ...

class PageStorage:
    @property
    def local_storage(self) -> WebStorage: ...
    @property
    def session_storage(self) -> WebStorage: ...

# The Human pause.

@final
@dataclass(frozen=True, kw_only=True)
class HumanPauseResult:
    """How a Human pause ended. A person's "Decline" raises `HumanPauseDeclinedError` instead."""

    resolved_by: Literal["condition", "person"] | str
    person: str | None
    """The person who answered, as the Console or the CLI knows them; `None` when the condition ended it."""
    waited: timedelta

class AsyncPausesForHuman:
    async def pause(
        self, *, reason: str | None = None, until: AsyncLocator | UrlMatcher | None = None, timeout: Timeout = ...
    ) -> HumanPauseResult:
        """Playwright's `page.pause()`, with the Human pause's meaning: the program waits while a person acts in this
        page, in its window when it is headful on their desktop, else in the Live view on any device.

        `reason` is shown in the Console's "Needs you", the Notification and the Live view, and logged at info. `until`
        ends the pause by itself when it holds (a locator visible, or a URL matched as `wait_for_url` matches it); a
        person's "Done" is accepted only while it holds. `timeout` is `timeout.human_pause`, 24 hours by default; `None`
        waits without limit. A paused page takes no Gesture: its later Gestures wait; reads and waits run. No enclosing
        timeout counts its time; Usage still counts toward Limits. Cancelling the task ends it.
        """

class PausesForHuman:
    def pause(
        self, *, reason: str | None = None, until: Locator | UrlMatcher | None = None, timeout: Timeout = ...
    ) -> HumanPauseResult: ...

# Error codes. Each names the placeholder and the reference, never a value.

@final
@dataclass(frozen=True, kw_only=True)
class SecretUnboundDetails:
    placeholder: str
    bound: Sequence[str]
    """The names the scope binds."""

@final
class SecretUnboundError(ProgramError):
    """No scope binds this placeholder."""

    @property
    def details(self) -> SecretUnboundDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SecretUnavailableDetails:
    placeholder: str | None
    reference: str
    provider: str
    """The provider asked, such as `env`, `keychain` or `caller`."""
    failure: Literal["missing", "locked", "unreachable", "resolver_failed"] | str
    """Which failure: the code is not retryable."""

@final
class SecretUnavailableError(UnavailableError):
    """The provider gave no value: a missing variable, a locked keychain, an unreachable vault, or `resolve_secret`
    missing or raising."""

    @property
    def details(self) -> SecretUnavailableDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SecretDomainNotAllowedDetails:
    placeholder: str
    domain: str
    """The domain of the frame holding the field, or of the request."""
    allowed: Sequence[str]

@final
class SecretDomainNotAllowedError(RefusedError):
    """The field's frame or the request is outside the binding's `domains`."""

    @property
    def details(self) -> SecretDomainNotAllowedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SecretInputRefusedDetails:
    placeholder: str
    reason: Literal["shared_browser", "cdp_attached", "lent_page"] | str
    """`shared_browser`: other code also drives this Borrowed browser (ADR 0056); `cdp_attached`: a raw CDP session is
    attached (ADR 0017); `lent_page`: the page is lent to another program (ADR 0089)."""

@final
class SecretInputRefusedError(RefusedError):
    """No secret is typed into or sent from a browser other code may read, and no HTTP authentication challenge is
    answered there."""

    @property
    def details(self) -> SecretInputRefusedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RecoveryStep:
    """One step of ADR 0006's recovery ladder and why it did not bring the login back."""

    step: Literal["refresh", "reimport", "login", "agent_login", "human_pause"] | str
    outcome: Literal["failed", "skipped", "declined", "timed_out"] | str
    reason: str

@final
@dataclass(frozen=True, kw_only=True)
class LoggedOutDetails:
    steps: Sequence[RecoveryStep]

@final
class LoggedOutError(PageError):
    """The Session's Page check found a login page and recovery did not bring the login back, or was off. An exception
    the `login` function raised is chained as the cause."""

    @property
    def details(self) -> LoggedOutDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class HumanPauseDeclinedDetails:
    person: str | None
    note: str | None

@final
class HumanPauseDeclinedError(RefusedError):
    """A person declined the step `page.pause()` asked for."""

    @property
    def details(self) -> HumanPauseDeclinedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SessionNotSavableDetails:
    source: str

@final
class SessionNotSavableError(ProgramError):
    """The Session's cookies live outside run state; `fix` names the import of its source."""

    @property
    def details(self) -> SessionNotSavableDetails: ...
