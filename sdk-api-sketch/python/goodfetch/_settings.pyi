"""Settings in code (ADR 0083, decided in #470).

One `settings` keyword on each value that opens a scope (`connect()`, starting a Run, opening a Session or a page); the
nearest scope wins and is inherited beneath it. A call takes only its task's inputs and `timeout`. There are no setters
(G2). The types are generated from the settings schema (ADR 0028) as nested `TypedDict`s, as openai-python types its
request parameters; this file shows a sample of its keys. Deployment variables are no keys here: `connect()` reads them
from its environment map. A missing key: inherited. `None`: cleared, where the type
allows it. A value: set. The engine resolves the layers, so the SDK sends only what the program set.
"""

from dataclasses import dataclass
from datetime import timedelta
from typing import Literal, NotRequired, TypeAlias, TypedDict, final

from goodfetch._politeness import (
    ColliePolitenessSettings,
    CollieSitePreferencesSettings,
    PolitenessSettings,
    SitePreferencesSettings,
)
from goodfetch._results import ProgramError, RefusedError, RunLayerOrigin, SettingsLayer, UnavailableError

class TimeoutSettings(TypedDict, total=False):
    """`timeout.*`: ADR 0080's phases, one family. `None` clears a phase's timeout for the whole scope. `timeout.connect`
    is set only in configuration: a connection's own is `connect()`'s `timeout` (S2)."""

    action: timedelta | None
    navigation: timedelta | None
    wait: timedelta | None
    handler: timedelta | None
    hook: timedelta | None
    sink_write: timedelta | None
    close: timedelta | None
    run: timedelta | None
    """The default for each Run's own timeout, given at the connection; its built-in default is none. A Run's own is
    starting it with `timeout`."""

class RunTimeoutSettings(TypedDict, total=False):
    """The phases a Run or a Session can set: not `run`, which starting the Run spells as its `timeout` (S2)."""

    action: timedelta | None
    navigation: timedelta | None
    wait: timedelta | None
    handler: timedelta | None
    hook: timedelta | None
    sink_write: timedelta | None
    close: timedelta | None

class PageTimeoutSettings(TypedDict, total=False):
    """The phases a page can set."""

    action: timedelta | None
    navigation: timedelta | None
    wait: timedelta | None

class Viewport(TypedDict):
    width: int
    height: int

class Geolocation(TypedDict):
    latitude: float
    longitude: float
    accuracy: NotRequired[float]

class BrowserSettings(TypedDict, total=False):
    """`browser.*`: part of a Session's identity, fixed for its life (ADR 0021), so set where a Session opens or above
    it, never on a page. Playwright's names where the meaning matches (S3). A value that conflicts with an identity fixed
    elsewhere (a `stealth` Identity profile, an imported login, a Profile directory used in place) is refused with
    `setting_locked`."""

    viewport: Viewport | None
    device_scale_factor: float
    locale: str | None
    """`None`: matched to the proxy exit, the `standard` Access mode's default."""
    timezone_id: str | None
    """`None`: matched to the proxy exit, as `locale`."""
    color_scheme: Literal["light", "dark", "no-preference"]
    java_script_enabled: bool
    geolocation: Geolocation | None

class NetworkThrottle(TypedDict):
    """Emulated network conditions, which Playwright closed as not planned (#8622)."""

    download_bytes_per_second: int
    upload_bytes_per_second: int
    latency: timedelta

class NetworkSettings(TypedDict, total=False):
    throttle: NetworkThrottle | None

class PayloadSettings(TypedDict, total=False):
    max_size: int
    """`crawl.payload.max_size`, in bytes (ADR 0082)."""

class CrawlSettings(TypedDict, total=False):
    payload: PayloadSettings

class Settings(TypedDict, total=False):
    """Every setting, as a connection takes them. A sample of the generated type."""

    timeout: TimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    crawl: CrawlSettings
    politeness: PolitenessSettings
    """ADR 0086."""
    site_preferences: SitePreferencesSettings
    """ADR 0086."""

class RunSettings(TypedDict, total=False):
    """A Collie Run's settings: no `crawl.*`, no `timeout.run`, and none of the politeness a crawl's Frontier alone
    uses (ADR 0086). A setting the type cannot rule out, such as `timeout.handler`, is refused with
    `setting_not_applicable`."""

    timeout: RunTimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    politeness: ColliePolitenessSettings
    site_preferences: CollieSitePreferencesSettings

class SessionSettings(TypedDict, total=False):
    """A Session's settings: Politeness and Site preferences are a Run's. On a Borrowed browser, any `browser.*` value
    is refused with `setting_not_applicable`."""

    timeout: RunTimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings

class PageSettings(TypedDict, total=False):
    """A page's settings: no part of the Session's identity."""

    timeout: PageTimeoutSettings

SettingKey: TypeAlias = Literal[
    "timeout.connect",
    "timeout.action",
    "timeout.navigation",
    "timeout.wait",
    "timeout.handler",
    "timeout.hook",
    "timeout.sink_write",
    "timeout.close",
    "timeout.run",
    "browser.viewport",
    "browser.device_scale_factor",
    "browser.locale",
    "browser.timezone_id",
    "browser.color_scheme",
    "browser.java_script_enabled",
    "browser.geolocation",
    "network.throttle",
    "crawl.payload.max_size",
    "politeness.domain.concurrency",
    "politeness.domain.delay",
    "politeness.backoff.enabled",
    "politeness.backoff.initial",
    "politeness.backoff.max",
    "politeness.ip.concurrency",
    "politeness.proxy.concurrency",
    "politeness.proxy.rate",
    "politeness.sharing",
    "politeness.degraded_max",
    "site_preferences.respect_robots_txt",
    "site_preferences.max_crawl_delay",
    "site_preferences.respect_use_signals",
]
"""A setting's canonical key: dotted snake_case, area first, the same in configuration files, errors and `config
explain`. A sample."""

SettingScope: TypeAlias = Literal["connection", "run", "session", "page"]
"""The scopes a setting can be set at."""

@final
@dataclass(frozen=True, kw_only=True)
class ExplainedSetting:
    """One effective value, as `goodfetch config explain` shows it for the scope."""

    key: SettingKey | str
    """A key this SDK does not know is kept as received (G3)."""
    value: object
    """A Secret reference shows as the reference."""
    layer: SettingsLayer
    origin: RunLayerOrigin | None
    """Set when `layer` is `run`."""
    file: str | None
    """The configuration file, when `layer` is `configuration`."""
    locked_by: SettingsLayer | None
    """The layer that locked or capped the value, if any."""

# The Error codes of settings (ADR 0083). Each is checked when the scope opens, or when a call with a per-call argument
# starts, so it points at the line that set the value; nothing is clamped, dropped or ignored (G3).

@final
@dataclass(frozen=True, kw_only=True)
class SettingUnknownDetails:
    key: str
    """The key as written, or the `GOODFETCH_` variable."""
    closest: str | None
    """The closest valid key or listed variable, when one is close."""

@final
class SettingUnknownError(ProgramError):
    """An unknown key, or an unknown `GOODFETCH_` variable at `connect()`."""

    @property
    def details(self) -> SettingUnknownDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SettingInvalidDetails:
    key: str
    expected: str
    """The type or range the setting accepts, such as `a positive timedelta, or None`."""

@final
class SettingInvalidError(ProgramError):
    """A wrong type or an out-of-range value, or an empty or unparsable `GOODFETCH_` variable."""

    @property
    def details(self) -> SettingInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SettingNotApplicableDetails:
    key: str
    scope: SettingScope
    """Where the program set it."""
    applies_to: tuple[SettingScope, ...]
    """Where the setting takes effect."""

@final
class SettingNotApplicableError(ProgramError):
    """A setting no scope beneath can use, such as a `crawl.*` setting on a Collie Run or `browser.viewport` on a
    page."""

    @property
    def details(self) -> SettingNotApplicableDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SettingLockedDetails:
    key: str
    locked_by: SettingsLayer
    """The layer that locked or capped the value, or set the setting that fixed the identity."""
    fixed_by: str | None
    """The setting that fixed a Session's identity, such as `session.source` or the Access mode's; `None` for a lock or a
    cap."""

@final
class SettingLockedError(RefusedError):
    """A value a lower layer locked or capped, or outside a Recipe's exposed bounds, or a `browser.*` value that
    conflicts with an identity fixed elsewhere; `limit` names that layer, or the setting that fixed the identity, and its
    value."""

    @property
    def details(self) -> SettingLockedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class CapabilityUnavailableDetails:
    capability: str
    platform: str

@final
class CapabilityUnavailableError(UnavailableError):
    """A capability the platform cannot provide; `fix` names the setting that turns it off (ADRs 0038, 0057)."""

    @property
    def details(self) -> CapabilityUnavailableDetails: ...
