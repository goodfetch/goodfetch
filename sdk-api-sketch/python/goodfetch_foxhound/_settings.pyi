"""Foxhound's settings (ADR 0083, decided in #470): the keys only a crawl's Run honours (`crawl.*`, `fetch.*`,
`sinks.*`, the crawl phases of `timeout.*`, Politeness toward sites and robots.txt), added to Collie's, which are the
Collie client package's (ADRs 0088, 0095). A Collie engine refuses them with `not_in_product`. `Settings`,
`TimeoutSettings`, `RunTimeoutSettings` and `SettingKey` here are the whole of a Foxhound program's: Collie's and
Foxhound's own, generated as one `TypedDict` each.
"""

from collections.abc import Mapping, Sequence
from datetime import timedelta
from typing import Literal, Required, TypeAlias, TypedDict

from goodfetch_collie_client import (
    AgentSettings,
    BrowserSettings,
    CheckpointSettings,
    EffectSettings,
    ExtractSettings,
    LocatorSettings,
    NetworkSettings,
    ObservationSettings,
    PageAISettings,
    PageTimeoutSettings,
    PlanSettings,
    ProxySettings,
    RunAISettings,
    SecretsSettings,
    SessionLoginSettings,
)
from goodfetch_collie_client._settings import SettingKey as CollieSettingKey
from goodfetch_foxhound._crawl import FetchTier, LinkPattern
from goodfetch_foxhound._politeness import PolitenessSettings, SitePreferencesSettings

class TimeoutSettings(TypedDict, total=False):
    """`timeout.*`: Collie's phases and a crawl's own (`handler`, `hook`, `sink_write`)."""

    action: timedelta | None
    navigation: timedelta | None
    wait: timedelta | None
    handler: timedelta | None
    hook: timedelta | None
    sink_write: timedelta | None
    close: timedelta | None
    run: timedelta | None
    human_pause: timedelta | None
    ai: timedelta | None
    model: timedelta | None
    agent: timedelta | None

class RunTimeoutSettings(TypedDict, total=False):
    """A crawl Run's phases: every one but `run`, which starting the Run spells as its `timeout` (S2)."""

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

class PayloadSettings(TypedDict, total=False):
    max_size: int
    """`crawl.payload.max_size`, in bytes (ADR 0082)."""

CrawlScopeStrategy: TypeAlias = Literal["same_hostname", "same_domain", "same_origin", "all"]
class CrawlScopeSettings(TypedDict, total=False):
    """`crawl.scope.*`: checked on every enqueue and every redirect (ADR 0093)."""

    strategy: CrawlScopeStrategy
    """Built-in: `same_hostname`, Crawlee's default."""
    include: list[LinkPattern]
    """Only URLs matching one of these, which must match as well as the strategy, never replacing it."""
    exclude: list[LinkPattern]
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

class KeptResponsesSettings(TypedDict, total=False):
    """`fetch.kept_responses.*` (ADR 0110), on a crawl's Run or its Labels. A Label that sets `urls` while forcing `http` or
    `browser_fetch` fails the Run's start with `setting_not_applicable`: those tiers run no script."""

    urls: list[LinkPattern]
    """The URLs of the responses kept on a request's page from the start of its navigation, matched in the engine.
    Built-in: none."""
    max_size: int
    """Bytes of kept bodies per attempt; a body past it reads as `body_unavailable`. Built-in: 16 MiB."""

class FetchSettings(TypedDict, total=False):
    """`fetch.*`: the fetch tiers of ADR 0020 (ADR 0093)."""

    tier: Literal["auto"] | FetchTier
    """`auto`, the built-in: learned per Label on a domain by sample comparison. Any other value forces the tier (ADR
    0023)."""
    tier_race: bool
    """ADR 0040's `tier_race`. Built-in: False."""
    kept_responses: KeptResponsesSettings
    """The responses a handler reads with `kept_response` and `kept_responses` (ADR 0110)."""

SinkWriteMode: TypeAlias = Literal["append", "upsert", "insert_only", "replace", "history"]
class SinkSettings(TypedDict, total=False):
    """`sinks.<name>.*`: one Sink each (ADRs 0026, 0103), set on a connection or a crawl's Run, never on a Label. A
    sample: each Sink type's own keys (its destination as a Secret reference, its table or bucket) are area 6's spec."""

    type: Required[str]
    datasets: Required[Sequence[str]]
    """The Datasets it receives, by name. Set on the Run, one the crawl does not declare is refused at start with
    `setting_invalid` naming the closest; inherited from the connection or configuration, the Sink delivers only the
    crawl's declared Datasets, and is inactive when there are none."""
    mode: SinkWriteMode
    """Built-in: `append`."""
    key: Sequence[str]
    """Built-in: each Dataset's own `key`."""

class CrawlRunSettings(TypedDict, total=False):
    """A crawl Run's settings: every setting but `timeout.run`, which starting the Run spells as its `timeout` (S2)."""

    locator: LocatorSettings
    timeout: RunTimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    crawl: CrawlSettings
    fetch: FetchSettings
    politeness: PolitenessSettings
    site_preferences: SitePreferencesSettings
    ai: RunAISettings
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
    kept_responses: KeptResponsesSettings

class LabelCrawlSettings(TypedDict, total=False):
    retry: CrawlRetrySettings
    handle_statuses: list[int]
    handler: CrawlHandlerSettings

class LabelSettings(TypedDict, total=False):
    """A Label's settings (ADR 0093): every setting a page takes, what shapes one crawl request, every `session.*` key
    and `secrets.*`. `session.source`, `session.http_credentials`, `secrets.*` and a `login` step give the Label's
    requests Sessions of a pool of their own (ADR 0092); `session.page_check` and `session.recovery.human_pause` judge
    its pages in whichever pool serves them. The Session's identity (`browser.*`, `proxy.*`, `network.*`), Politeness,
    Site preferences, scope and Run-wide keys are refused with `setting_not_applicable`: two Labels on one domain share
    its Politeness with every run of the user (ADR 0053)."""

    locator: LocatorSettings
    timeout: LabelTimeoutSettings
    extract: ExtractSettings
    observation: ObservationSettings
    ai: PageAISettings
    agent: AgentSettings
    fetch: LabelFetchSettings
    crawl: LabelCrawlSettings
    session: SessionLoginSettings
    secrets: SecretsSettings


class Settings(TypedDict, total=False):
    """Every setting, as a connection takes them. A sample of the generated type."""

    locator: LocatorSettings
    timeout: TimeoutSettings
    browser: BrowserSettings
    network: NetworkSettings
    crawl: CrawlSettings
    plan: PlanSettings
    extract: ExtractSettings
    observation: ObservationSettings
    ai: RunAISettings
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

SettingKey: TypeAlias = CollieSettingKey | Literal[
    "timeout.handler",
    "timeout.hook",
    "timeout.sink_write",
    "crawl.payload.max_size",
    "politeness.domain.concurrency",
    "politeness.domain.delay",
    "politeness.backoff.enabled",
    "politeness.backoff.initial",
    "politeness.backoff.max",
    "politeness.ip.concurrency",
    "politeness.degraded_max",
    "site_preferences.respect_robots_txt",
    "site_preferences.max_crawl_delay",
    "site_preferences.robots_txt_user_agent",
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
    "fetch.kept_responses.urls",
    "fetch.kept_responses.max_size",
]
