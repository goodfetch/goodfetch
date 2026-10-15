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
    """`crawl.scope.*`: checked on every enqueue and every redirect (ADR 0093), hosts compared as the WHATWG parser
    leaves them and one leading `www.` label ignored (ADR 0120)."""

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
    max_url_length: int | None
    """`crawl.max_url_length`: characters of a URL as parsed; a longer one is not added with `max_url_length`, and the
    finished Run is `unverified`. Built-in: 2,083, Scrapy's; `None` for none (ADR 0120)."""
    max_path_segments: int | None
    """`crawl.max_path_segments`: a URL with a longer path is not added with `max_path_segments`, and the finished Run is
    `unverified`. Built-in: 20, Heritrix's; `None` for none (ADR 0120)."""
    max_segment_repeats: int | None
    """`crawl.max_segment_repeats`: how many times a path segment, or a run of segments, may follow itself; past it a URL
    is not added with `max_segment_repeats`, and the finished Run is `unverified`. Built-in: 2, Heritrix's; `None` for
    none (ADR 0120)."""
    keep_url_fragment: bool
    """`crawl.keep_url_fragment`: the fragment stays in a request's key, for a Label whose requests are hash routes. Such
    a Label is served only on `light_browser` or `browser`, and a resume may not change it. Built-in: False (ADR 0120)."""
    retry: CrawlRetrySettings
    handle_statuses: list[int]
    """`crawl.handle_statuses`: statuses outside 2xx whose responses go to the handler instead of failing with
    `response_not_ok`; a redirect status listed here is not followed (ADR 0118). Built-in: none."""
    content_types: list[str]
    """`crawl.content_types`: the media types the handler takes (`type/subtype`, `type/*`, `*/*`), the type computed by
    MIME sniffing as Chrome computes it. Any other response that would reach the handler ends `skipped` with
    `content_type` before its body is read. For an accepted type that is no HTML document, the page holds no document
    and `ctx.response` holds the body (ADR 0119). Built-in: `text/html` and `application/xhtml+xml`."""
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
    redirect: RedirectSettings
    """`fetch.redirect.*` (ADR 0118)."""
    refresh: RefreshSettings
    """`fetch.refresh.*` (ADR 0118)."""
    encoding: str | None
    """`fetch.encoding`: a WHATWG Encoding label forced on the crawl request's document on every tier, after its BOM; an
    unknown label is refused with `setting_invalid`. With none, every tier reads a BOM, the `Content-Type` charset, the
    `<meta>` prescan, then UTF-8 for a valid UTF-8 body, and a body it would have to guess escalates to the browser (ADR
    0119). Built-in: none."""

class RedirectSettings(TypedDict, total=False):
    """`fetch.redirect.*` (ADR 0118): every tier follows Fetch's redirect statuses (301, 302, 303, 307, 308) with Fetch's
    method and header rules, each hop through the navigation gate. A redirect status listed in `crawl.handle_statuses`
    reaches the handler unfollowed: the one way not to follow."""

    max: int
    """`fetch.redirect.max`: from 1 to 20, the browser's most; past it, `too_many_redirects`. Built-in: 20."""

class RefreshSettings(TypedDict, total=False):
    """`fetch.refresh.*` (ADR 0118): a crawl request's document that refreshes to another URL (a `Refresh` header, or
    `<meta http-equiv="refresh">` outside `<noscript>`) is one more hop, made at once on every tier. A refresh to the
    document itself is a reload, never followed."""

    max_delay: timedelta | None
    """`fetch.refresh.max_delay`: a longer refresh is left on the page. `None`: any. Built-in: 100 seconds, Scrapy's."""

class ExportCSVSettings(TypedDict, total=False):
    bom: bool
    """`export.csv.bom`: start a CSV Export with a UTF-8 BOM, for spreadsheets. Built-in: False."""

class ExportSettings(TypedDict, total=False):
    """`export.*`: options of the Export formats (ADR 0103). Every text format is UTF-8 (ADR 0119)."""

    csv: ExportCSVSettings

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
    export: ExportSettings
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
    redirect: RedirectSettings
    refresh: RefreshSettings
    encoding: str | None

class LabelCrawlSettings(TypedDict, total=False):
    retry: CrawlRetrySettings
    handle_statuses: list[int]
    content_types: list[str]
    handler: CrawlHandlerSettings
    keep_url_fragment: bool

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
    export: ExportSettings
    """`export.*`: the Export formats' options (ADRs 0103, 0119)."""
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
    "crawl.max_url_length",
    "crawl.max_path_segments",
    "crawl.max_segment_repeats",
    "crawl.keep_url_fragment",
    "crawl.retry.max",
    "crawl.retry.handler_failed",
    "crawl.handle_statuses",
    "crawl.content_types",
    "crawl.handler.concurrency",
    "fetch.tier",
    "fetch.tier_race",
    "fetch.kept_responses.urls",
    "fetch.kept_responses.max_size",
    "fetch.redirect.max",
    "fetch.refresh.max_delay",
    "politeness.retry_after.max",
    "fetch.encoding",
    "export.csv.bom",
]
