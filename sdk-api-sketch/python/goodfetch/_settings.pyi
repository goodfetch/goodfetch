"""Settings in code (ADR 0083, decided in #470).

One `settings` keyword on each value that opens a scope (`connect()`, starting a Run, declaring a crawl's Label (ADR
0093), opening a Session or a page); the nearest scope wins and is inherited beneath it. A call takes only its task's inputs and `timeout`. There are no setters
(G2). The types are generated from the settings schema (ADR 0028) as nested `TypedDict`s, as openai-python types its
request parameters; this file shows a sample of its keys. Deployment variables are no keys here: `connect()` reads them
from its environment map. A missing key: inherited. `None`: cleared, where the type
allows it. A value: set. The engine resolves the layers, so the SDK sends only what the program set.
"""

from dataclasses import dataclass
from datetime import timedelta
from collections.abc import Mapping, Sequence
from typing import Literal, NotRequired, Required, TypeAlias, TypedDict, final

from goodfetch._crawl import FetchTier, UrlPattern
from goodfetch._politeness import (
    CollieSitePreferencesSettings,
    PolitenessSettings,
    SitePreferencesSettings,
)
from goodfetch._sessions import Permission, ProxySettings, SecretsSettings, SessionLoginSettings, SessionScopeLoginSettings
from goodfetch._results import ProgramError, RefusedError, RunLayerOrigin, SettingsLayer, UnavailableError
from goodfetch._models import AiSettings, PageAiSettings

class TimeoutSettings(TypedDict, total=False):
    """`timeout.*`: ADR 0080's phases, one family. `None` clears a phase's timeout for the whole scope. `timeout.connect`
    is set only in the project file, which `connect()` applies; a connection's own is `connect()`'s `timeout` (S2)."""

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
    human_pause: timedelta | None
    """`timeout.human_pause`: how long a Human pause may wait for a person; 24 hours by default (ADR 0092)."""
    ai: timedelta | None
    """`timeout.ai`: an AI verb from its call to its return, model calls and a Repair included, 120 s (ADR 0091)."""
    model: timedelta | None
    """`timeout.model`: one attempt of one model request, the program's own provider included, 60 s; an attempt past it
    is retried, within the call that made it (ADR 0101)."""
    agent: timedelta | None
    """`timeout.agent`: an Agent's task, 10 minutes; reaching it stops the task with the reason `timeout` (ADR 0100)."""

class RunTimeoutSettings(TypedDict, total=False):
    """The phases a Run or a Session can set: not `run`, which starting the Run spells as its `timeout` (S2)."""

    action: timedelta | None
    navigation: timedelta | None
    wait: timedelta | None
    handler: timedelta | None
    hook: timedelta | None
    sink_write: timedelta | None
    close: timedelta | None
    human_pause: timedelta | None
    ai: timedelta | None
    model: timedelta | None
    agent: timedelta | None

class PageTimeoutSettings(TypedDict, total=False):
    """The phases a page can set."""

    action: timedelta | None
    navigation: timedelta | None
    wait: timedelta | None
    human_pause: timedelta | None
    ai: timedelta | None
    model: timedelta | None
    agent: timedelta | None

class PlanSettings(TypedDict, total=False):
    """`plan.*` (ADR 0091)."""

    frozen: bool
    """Replay only: never call a model and never write the Plan; a missing step or a Drift raises. Default: `False`."""

class ExtractSettings(TypedDict, total=False):
    """`extract.*` (ADR 0091)."""

    mode: Literal["compiled", "model"]
    """`compiled` (the default) or `model`; `extract()` takes it for one call as `mode`."""

class ObservationSettings(TypedDict, total=False):
    """`observation.*` (ADR 0091)."""

    max_size: int
    """The most characters an `ai` aria snapshot holds before its deepest subtrees collapse. Default: 40,000."""

class Viewport(TypedDict):
    width: int
    height: int

class Geolocation(TypedDict):
    latitude: float
    longitude: float
    accuracy: NotRequired[float]

class BrowserSettings(TypedDict, total=False):
    """`browser.*`: part of a Session's identity, fixed for its life (ADR 0021), so set where a Session opens or above
    it, never on a page. Playwright's names where the meaning matches (S3). A value set where a Session opens that conflicts
    with an identity fixed elsewhere (a `stealth` Identity profile, an imported login, a Profile directory used in place)
    is refused with `setting_locked`; an inherited one yields, and `explain_settings()` names what fixed it."""

    viewport: Viewport | None
    device_scale_factor: float
    locale: str | None
    """`None`: matched to the proxy exit, the `standard` Access mode's default."""
    timezone_id: str | None
    """`None`: matched to the proxy exit, as `locale`."""
    color_scheme: Literal["light", "dark", "no-preference"]
    java_script_enabled: bool
    geolocation: Geolocation | None
    headless: bool
    """`browser.headless`: `True` by default, `False` under `stealth` (ADR 0035). A person at the keyboard sees the window
    (ADR 0092)."""
    permissions: Sequence[Permission]
    """`browser.permissions`: granted to every page of the Session; none by default, no setter later (ADR 0092)."""

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

CrawlScopeStrategy: TypeAlias = Literal["same_hostname", "same_domain", "same_origin", "all"]
"""`crawl.scope.strategy`: which hosts a crawl may reach, measured against every seed after the Frontier's URL rules, so
`www.` is no other host (ADR 0005)."""

class CrawlScopeSettings(TypedDict, total=False):
    """`crawl.scope.*`: checked on every enqueue and every redirect (ADR 0093)."""

    strategy: CrawlScopeStrategy
    """Built-in: `same_hostname`, Crawlee's default."""
    include: list[UrlPattern]
    """Only URLs matching one of these, which must match as well as the strategy, never replacing it."""
    exclude: list[UrlPattern]
    """No URL matching any of these."""

class CrawlRetrySettings(TypedDict, total=False):
    """`crawl.retry.*` (ADR 0093)."""

    max: int
    """Retries after the first attempt, for retryable errors and `ctx.retry()`. Built-in: 2, Scrapy's."""
    handler_failed: bool
    """Retry `handler_failed` too, as Crawlee does. Built-in: False: a bug fails the same way again (G5)."""

class CrawlHandlerSettings(TypedDict, total=False):
    concurrency: int
    """`crawl.handler.concurrency`: handlers running at once in the program, the engine's leases to it (ADR 0081). On a
    Label, within the Run's. Built-in: 16."""

class CrawlSettings(TypedDict, total=False):
    payload: PayloadSettings
    scope: CrawlScopeSettings
    max_depth: int | None
    """`crawl.max_depth`: seeds are at 0. Reaching it refuses further enqueues with `max_depth` and ends the Run
    `incomplete`. Built-in: none."""
    max_requests: int | None
    """`crawl.max_requests`: requests admitted to the Frontier. Reaching it refuses further enqueues with `max_requests`
    and ends the Run `incomplete`. Built-in: none."""
    retry: CrawlRetrySettings
    handle_statuses: list[int]
    """`crawl.handle_statuses`: statuses outside 2xx whose responses go to the handler instead of failing with
    `response_not_ok`. Built-in: none."""
    handler: CrawlHandlerSettings
    keep_alive: bool
    """`crawl.keep_alive`: the Run stays `running` with an empty Frontier, waiting for `add_seed`, until `finish()` lets it
    end once drained (ADR 0102, ADR 0037's RT-12). Built-in: False."""

class CheckpointSettings(TypedDict, total=False):
    """`checkpoint.*` (ADRs 0002, 0102)."""

    interval: timedelta | None
    """`checkpoint.interval`: how often the Run writes a Checkpoint. Built-in: none locally."""

class EffectOutputSettings(TypedDict, total=False):
    max_size: int
    """`effect.output.max_size`, in bytes: the largest output of a declared Effect the journal keeps (ADR 0106).
    Built-in: 64 KiB."""

class EffectSettings(TypedDict, total=False):
    """`effect.*` (ADR 0106): set on a connection or a Run."""

    output: EffectOutputSettings

class FetchSettings(TypedDict, total=False):
    """`fetch.*`: the fetch tiers of ADR 0020 (ADR 0093)."""

    tier: Literal["auto"] | FetchTier
    """`auto`, the built-in: learned per Label on a domain by sample comparison. Any other value forces the tier (ADR
    0023)."""
    tier_race: bool
    """ADR 0040's `tier_race`. Built-in: False."""

SinkWriteMode: TypeAlias = Literal["append", "upsert", "insert_only", "replace", "history"]
"""A Sink's write mode (ADR 0026). The keyed ones need a key: the Sink's own or its Datasets'."""

class SinkSettings(TypedDict, total=False):
    """`sinks.<name>.*`: one Sink each (ADRs 0026, 0103), set on a connection or a crawl's Run, never on a Label. A
    sample: each Sink type's own keys (its destination as a Secret reference, its table or bucket) are area 6's spec."""

    type: Required[str]
    datasets: Required[Sequence[str]]
    """The Datasets it receives, by name. One the crawl does not declare is refused when the Run starts, with
    `setting_invalid` naming the closest."""
    mode: SinkWriteMode
    """Built-in: `append`."""
    key: Sequence[str]
    """Built-in: each Dataset's own `key`."""

class AgentSettings(TypedDict, total=False):
    """`agent.*` (ADR 0100). `timeout.agent` is in the timeout family."""

    max_steps: int
    """Steps a task may take: 20 by default. `max_steps=` sets it for one task."""
    max_stalled_steps: int
    """Stalled steps in a row that stop a task: 3 by default."""
    domains: Sequence[str] | None
    """Where a task may go: by default the registrable domain of the page it starts on."""
    human_pause: bool
    """Whether the model may ask a person through a Human pause: `False` by default (ADR 0092)."""

class Settings(TypedDict, total=False):
    """Every setting, as a connection takes them. A sample of the generated type."""

    timeout: TimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    crawl: CrawlSettings
    plan: PlanSettings
    extract: ExtractSettings
    observation: ObservationSettings
    ai: AiSettings
    """`ai.*`: the model, Model providers and structured output (ADR 0101)."""
    agent: AgentSettings
    """`agent.*`: the Agent's step cap, stall rule, domains and Human pause (ADR 0100)."""
    fetch: FetchSettings
    """ADR 0093."""
    politeness: PolitenessSettings
    """ADR 0086."""
    site_preferences: SitePreferencesSettings
    """ADR 0086."""
    session: SessionLoginSettings
    """`session.*` (ADRs 0056, 0006, 0092)."""
    secrets: SecretsSettings
    """`secrets.<name>`: the Secret placeholders' bindings (ADR 0092)."""
    proxy: ProxySettings
    """`proxy.*` (ADRs 0021, 0092)."""
    sinks: Mapping[str, SinkSettings]
    """`sinks.<name>.*`, keyed by the Sink's name (ADR 0103). Foxhound's: a Collie Run has no Datasets."""
    checkpoint: CheckpointSettings
    effect: EffectSettings
    """`effect.*`: what the Effect journal keeps of a declared Effect (ADR 0106)."""

class CrawlRunSettings(TypedDict, total=False):
    """A crawl Run's settings: every setting but `timeout.run`, which starting the Run spells as its `timeout` (S2)."""

    timeout: RunTimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    crawl: CrawlSettings
    fetch: FetchSettings
    politeness: PolitenessSettings
    site_preferences: SitePreferencesSettings
    ai: AiSettings
    plan: PlanSettings
    extract: ExtractSettings
    observation: ObservationSettings
    sinks: Mapping[str, SinkSettings]
    checkpoint: CheckpointSettings
    effect: EffectSettings

class LabelTimeoutSettings(PageTimeoutSettings, total=False):
    """The phases a Label can set: a page's, and its handler's."""

    handler: timedelta | None
    hook: timedelta | None
    """Each call of a Hook on the Label's requests (ADR 0104)."""

class LabelFetchSettings(TypedDict, total=False):
    tier: Literal["auto"] | FetchTier

class LabelCrawlSettings(TypedDict, total=False):
    retry: CrawlRetrySettings
    handle_statuses: list[int]
    handler: CrawlHandlerSettings

class LabelLocatorSettings(TypedDict, total=False):
    test_id_attribute: str
    """`locator.test_id_attribute`, settable at any scope (ADR 0085)."""

class LabelSettings(TypedDict, total=False):
    """A Label's settings (ADR 0093): every setting a page takes, what shapes one crawl request, every `session.*` key
    and `secrets.*`. `session.source`, `session.http_credentials`, `secrets.*` and a `login` step give the Label's
    requests Sessions of a pool of their own (ADR 0092); `session.page_check` and `session.recovery.human_pause` judge
    its pages in whichever pool serves them. The Session's identity (`browser.*`, `proxy.*`, `network.*`), Politeness,
    Site preferences, scope and Run-wide keys are refused with `setting_not_applicable`: two Labels on one domain share
    its Politeness with every run of the user (ADR 0053)."""

    locator: LabelLocatorSettings
    timeout: LabelTimeoutSettings
    extract: ExtractSettings
    observation: ObservationSettings
    ai: PageAiSettings
    fetch: LabelFetchSettings
    crawl: LabelCrawlSettings
    session: SessionLoginSettings
    secrets: SecretsSettings

class RunSettings(TypedDict, total=False):
    """A Collie Run's settings: no `crawl.*`, no `timeout.run`, and no Politeness, which is Foxhound's (ADRs 0084,
    0086). A setting the type cannot rule out, such as `timeout.handler`, is refused with
    `setting_not_applicable`."""

    timeout: RunTimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    plan: PlanSettings
    extract: ExtractSettings
    observation: ObservationSettings
    ai: AiSettings
    agent: AgentSettings
    site_preferences: CollieSitePreferencesSettings
    session: SessionLoginSettings
    secrets: SecretsSettings
    proxy: ProxySettings
    checkpoint: CheckpointSettings
    effect: EffectSettings

class SessionSettings(TypedDict, total=False):
    """A Session's settings: Politeness and Site preferences are a Run's, and `session.source` is `new_session()`'s
    `source`, one spelling (S2). Set where a Borrowed browser's Session opens, any `browser.*` value is refused with
    `setting_not_applicable`."""

    timeout: RunTimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    session: SessionScopeLoginSettings
    secrets: SecretsSettings
    proxy: ProxySettings
    ai: AiSettings
    agent: AgentSettings

class PageSettings(TypedDict, total=False):
    """A page's settings: no part of the Session's identity."""

    timeout: PageTimeoutSettings
    extract: ExtractSettings
    observation: ObservationSettings
    ai: PageAiSettings
    agent: AgentSettings

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
    "timeout.human_pause",
    "timeout.ai",
    "timeout.model",
    "timeout.agent",
    "agent.max_steps",
    "agent.max_stalled_steps",
    "agent.domains",
    "agent.human_pause",
    "ai.model",
    "ai.structured_output",
    "plan.frozen",
    "extract.mode",
    "observation.max_size",
    "browser.viewport",
    "browser.device_scale_factor",
    "browser.locale",
    "browser.timezone_id",
    "browser.color_scheme",
    "browser.java_script_enabled",
    "browser.geolocation",
    "browser.headless",
    "browser.permissions",
    "network.throttle",
    "crawl.payload.max_size",
    "effect.output.max_size",
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
    "site_preferences.robots_txt_user_agent",
    "site_preferences.respect_use_signals",
    "session.source",
    "session.page_check",
    "session.recovery.human_pause",
    "session.recovery.agent",
    "session.http_credentials",
    "proxy.server",
    "proxy.bypass",
    "proxy.username",
    "proxy.password",
    "crawl.scope.strategy",
    "crawl.scope.include",
    "crawl.scope.exclude",
    "crawl.max_depth",
    "crawl.max_requests",
    "crawl.retry.max",
    "crawl.retry.handler_failed",
    "crawl.handle_statuses",
    "crawl.handler.concurrency",
    "fetch.tier",
    "fetch.tier_race",
]
"""A setting's canonical key: dotted snake_case, area first, the same in configuration files, errors and `config
explain`. A sample."""

SettingScope: TypeAlias = Literal["connection", "run", "label", "session", "page"]
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
    fixed_by: str | None
    """The setting that fixed the Session's identity, such as `session.source`, when an inherited `browser.*` value yielded
    to it; `value` is then the identity's, and `layer` that setting's."""

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
