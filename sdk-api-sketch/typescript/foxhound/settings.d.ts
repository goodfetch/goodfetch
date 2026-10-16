// Foxhound's settings (ADR 0083, decided in #470): the keys only a crawl's Run honours (`crawl.*`,
// `fetch.*`, `sinks.*`, the crawl phases of `timeout.*`, Politeness toward sites and robots.txt),
// added to Collie's, which are the Collie client package's (ADRs 0088, 0095). A Collie engine refuses
// them with `not_in_product`. `Settings`, `TimeoutSettings` and `SettingKey` here are the whole of a
// Foxhound program's: Collie's and Foxhound's own.

import type {
  CallOptions,
  ExplainedSetting,
  PageSettings,
  SecretsSettings,
  SessionLoginSettings,
  SettingKey as CollieSettingKey,
  Settings as CollieSettings,
  Timeout,
  TimeoutSettings as CollieTimeoutSettings,
} from "@goodfetch/collie-client";
import type { FetchTier, LinkPattern } from "./crawl.js";
import type { PolitenessSettings, SitePreferencesSettings } from "./politeness.js";

/** `timeout.*`: Collie's phases and a crawl's own. */
export interface TimeoutSettings extends CollieTimeoutSettings {
  readonly handler?: Timeout;
  readonly hook?: Timeout;
  readonly sinkWrite?: Timeout;
}

/** `crawl.scope.strategy`: which hosts a crawl may reach, measured against every seed, hosts compared as the WHATWG parser leaves them and one leading `www.` label ignored (ADR 0120). */
export type CrawlScopeStrategy = "same_hostname" | "same_domain" | "same_origin" | "all";

/** `crawl.scope.*`: checked on every enqueue and every redirect (ADR 0093). */
export interface CrawlScopeSettings {
  /** Built-in: `same_hostname`, Crawlee's default. */
  readonly strategy?: CrawlScopeStrategy;
  /** Only URLs matching one of these, which must match as well as the strategy, never replacing it. */
  readonly include?: readonly LinkPattern[];
  /** No URL matching any of these. */
  readonly exclude?: readonly LinkPattern[];
}

/** `crawl.retry.*` (ADR 0093). */
export interface CrawlRetrySettings {
  /** Retries after the first attempt, for retryable errors and `ctx.retry()`. Built-in: 2, Scrapy's. */
  readonly max?: number;
  /** Retry `handler_failed` too, as Crawlee does. Built-in: false: a bug fails the same way again, and each retry fetches the page again (G5). */
  readonly handlerFailed?: boolean;
}

export interface CrawlSettings {
  readonly payload?: {
    /** `crawl.payload.max_size`, in bytes (ADR 0082). */
    readonly maxSize?: number;
  };
  readonly scope?: CrawlScopeSettings;
  /** `crawl.max_depth`: seeds are at 0. Reaching it refuses further enqueues with `max_depth` and ends the Run `incomplete`. Built-in: none. */
  readonly maxDepth?: number | null;
  /** `crawl.max_requests`: requests admitted to the Frontier. Reaching it refuses further enqueues with `max_requests` and ends the Run `incomplete`. Built-in: none. */
  readonly maxRequests?: number | null;
  /** `crawl.max_url_length`: characters of a URL as parsed, its fragment not counted; a longer one is not added with `max_url_length`, and the finished Run is `unverified`. Built-in: 2,083, Scrapy's; `null` for none (ADR 0120). */
  readonly maxUrlLength?: number | null;
  /** `crawl.max_path_segments`: segments of a URL's path as parsed, an empty last one (a trailing slash) not counted; a URL with a longer path is not added with `max_path_segments`, and the finished Run is `unverified`. Built-in: 20, Heritrix's; `null` for none (ADR 0120). */
  readonly maxPathSegments?: number | null;
  /** `crawl.max_segment_repeats`: how many times a path segment, or a run of segments, may appear back to back, so `/a/a` passes and `/a/a/a` or `/a/b/a/b/a/b` does not; past it a URL is not added with `max_segment_repeats`, and the finished Run is `unverified`. Built-in: 2, Heritrix's; `null` for none (ADR 0120). */
  readonly maxSegmentRepeats?: number | null;
  /**
   * `crawl.keep_url_fragment`: the fragment stays in a request's key, for a Label whose requests are
   * hash routes, Crawlee's `keepUrlFragment`. Such a Label is served only on `light_browser` or `browser`:
   * forcing another tier fails the Run's start with `setting_not_applicable`, and a resume may not change
   * it. Built-in: false (ADR 0120).
   */
  readonly keepUrlFragment?: boolean;
  readonly retry?: CrawlRetrySettings;
  /**
   * `crawl.handle_statuses`: statuses outside 2xx whose responses go to the handler instead of failing
   * with `response_not_ok`; a redirect status listed here is not followed (ADR 0118). Built-in: none.
   */
  readonly handleStatuses?: readonly number[];
  /**
   * `crawl.content_types`: the media types the handler takes (`type/subtype`, `type/*`, or any type), the
   * type computed by MIME sniffing as Chrome computes it. Any other response that would reach the
   * handler ends `skipped` with `content_type` before its body is read. For an accepted type that is
   * no HTML document, the page holds no document and `ctx.response` holds the body (ADR 0119).
   * Built-in: `text/html` and `application/xhtml+xml`.
   */
  readonly contentTypes?: readonly string[];
  readonly handler?: {
    /** `crawl.handler.concurrency`: handlers running at once in the program, the engine's leases to it (ADR 0081). On a Label, within the Run's. Built-in: 16. */
    readonly concurrency?: number;
  };
  /**
   * `crawl.keep_alive`: the Run stays `running` with an empty Frontier, waiting for `addSeed`, until
   * `finish()` lets it end once drained (ADR 0102, ADR 0037's RT-12). Built-in: false.
   */
  readonly keepAlive?: boolean;
}

/** `fetch.*`: the fetch tiers of ADR 0020 (ADR 0093). */
export interface FetchSettings {
  /** `auto`, the built-in: learned per Label on a domain by sample comparison. Any other value forces the tier (ADR 0023). */
  readonly tier?: "auto" | FetchTier;
  /** `fetch.tier_race`, ADR 0040's `tier_race`. Built-in: false. */
  readonly tierRace?: boolean;
  /** `fetch.kept_responses.*`: the responses a handler reads with `keptResponse` and `keptResponses` (ADR 0110). */
  readonly keptResponses?: KeptResponsesSettings;
  /** `fetch.redirect.*` (ADR 0118). */
  readonly redirect?: RedirectSettings;
  /** `fetch.refresh.*` (ADR 0118). */
  readonly refresh?: RefreshSettings;
  /**
   * `fetch.encoding`: a WHATWG Encoding label forced on the crawl request's document on every tier,
   * after its BOM; an unknown label is refused with `setting_invalid`. With none, every tier reads a
   * BOM, the `Content-Type` charset, the `<meta>` prescan, then UTF-8 for a valid UTF-8 body, and a
   * body it would have to guess escalates to the browser (ADR 0119). Built-in: none.
   */
  readonly encoding?: string | null;
}

/**
 * `fetch.redirect.*` (ADR 0118): every tier follows Fetch's redirect statuses (301, 302, 303, 307,
 * 308) with Fetch's method and header rules, each hop through the navigation gate. A redirect status
 * listed in `crawl.handle_statuses` reaches the handler unfollowed: the one way not to follow.
 */
export interface RedirectSettings {
  /** `fetch.redirect.max`: from 1 to 20, the browser's most; past it, `too_many_redirects`. Built-in: 20. */
  readonly max?: number;
}

/**
 * `fetch.refresh.*` (ADR 0118): a crawl request's document that refreshes to another URL (a `Refresh`
 * header, or `<meta http-equiv="refresh">` outside `<noscript>`) is one more hop, made at once on every
 * tier. A refresh to the document itself is a reload, never followed.
 */
export interface RefreshSettings {
  /** `fetch.refresh.max_delay`, in milliseconds: a longer refresh is left on the page. `null`: any. Built-in: 100_000, Scrapy's. */
  readonly maxDelay?: number | null;
}

/** `export.*`: options of the Export formats (ADR 0103). Every text format is UTF-8 (ADR 0119). */
export interface ExportSettings {
  readonly csv?: {
    /** `export.csv.bom`: start a CSV Export with a UTF-8 BOM, for spreadsheets. Built-in: false. */
    readonly bom?: boolean;
  };
}

/**
 * `fetch.kept_responses.*` (ADR 0110), on a crawl's Run or its Labels. A Label that sets `urls` while
 * forcing `http` or `browser_fetch` fails the Run's start with `setting_not_applicable`: those tiers
 * run no script.
 */
export interface KeptResponsesSettings {
  /** The URLs of the responses kept on a request's page from the start of its navigation, matched in the engine. Built-in: none. */
  readonly urls?: readonly LinkPattern[];
  /** Bytes of kept bodies per attempt; a body past it reads as `body_unavailable`. Built-in: 16 MiB. */
  readonly maxSize?: number;
}

/**
 * A Sink's write mode (ADR 0026). The keyed ones use the Sink's own key, or else each Dataset's key.
 * The Run's start refuses with `setting_invalid` naming `sinks.<name>.key` a Sink in a keyed mode
 * with no key of its own while one of its Datasets has none, and a Sink key naming a field that a
 * schema of one of its Datasets does not require.
 */
export type SinkWriteMode = "append" | "upsert" | "insert_only" | "replace" | "history";

/**
 * `sinks.<name>.*`: one Sink each (ADRs 0026, 0103), set on a connection or a crawl's Run, never on a
 * Label. A sample: each Sink type's own keys (its destination as a Secret reference, its table or
 * bucket) are area 6's spec.
 */
export interface SinkSettings {
  readonly type: string;
  /**
   * The Datasets it receives, by name. Set on the Run, one the crawl does not declare is refused at
   * start with `setting_invalid` naming the closest; inherited from the connection or configuration,
   * the Sink delivers only the crawl's declared Datasets, and is inactive when there are none.
   */
  readonly datasets: readonly string[];
  /** Built-in: `append`. */
  readonly mode?: SinkWriteMode;
  /** Built-in: each Dataset's own `key`. */
  readonly key?: readonly string[];
}

/** Every setting of a Foxhound program, as a connection takes them: Collie's, with a crawl's own keys. A sample of the generated type. */
export interface Settings extends Omit<CollieSettings, "timeout" | "politeness" | "sitePreferences"> {
  readonly timeout?: TimeoutSettings;
  readonly crawl?: CrawlSettings;
  /** ADR 0093. */
  readonly fetch?: FetchSettings;
  /** ADR 0086, with Collie's per-proxy limits. */
  readonly politeness?: PolitenessSettings;
  /** ADR 0086. */
  readonly sitePreferences?: SitePreferencesSettings;
  /** `sinks.<name>.*`, keyed by the Sink's name (ADR 0103). */
  readonly sinks?: Readonly<Record<string, SinkSettings>>;
  /** ADR 0103. */
  readonly export?: ExportSettings;
}

/** A setting's canonical key: Collie's, and Foxhound's own. A sample. */
export type SettingKey =
  | CollieSettingKey
  | "timeout.handler"
  | "timeout.hook"
  | "timeout.sink_write"
  | "crawl.payload.max_size"
  | "politeness.domain.concurrency"
  | "politeness.domain.delay"
  | "politeness.backoff.enabled"
  | "politeness.backoff.initial"
  | "politeness.backoff.max"
  | "politeness.ip.concurrency"
  | "politeness.degraded_max"
  | "site_preferences.respect_robots_txt"
  | "site_preferences.max_crawl_delay"
  | "site_preferences.robots_txt_user_agent"
  | "crawl.scope.strategy"
  | "crawl.scope.include"
  | "crawl.scope.exclude"
  | "crawl.max_depth"
  | "crawl.max_requests"
  | "crawl.max_url_length"
  | "crawl.max_path_segments"
  | "crawl.max_segment_repeats"
  | "crawl.keep_url_fragment"
  | "crawl.retry.max"
  | "crawl.retry.handler_failed"
  | "crawl.handle_statuses"
  | "crawl.content_types"
  | "crawl.handler.concurrency"
  | "fetch.tier"
  | "fetch.tier_race"
  | "fetch.kept_responses.urls"
  | "fetch.kept_responses.max_size"
  | "fetch.redirect.max"
  | "fetch.refresh.max_delay"
  | "politeness.retry_after.max"
  | "fetch.encoding"
  | "export.csv.bom";

/** A crawl Run's settings: every setting but `timeout.run`, which starting the Run spells as its `timeout` (S2). */
export type CrawlRunSettings = Omit<Settings, "timeout"> & { readonly timeout?: Omit<TimeoutSettings, "run"> };

/**
 * A Label's settings (ADR 0093): every setting a page takes, what shapes one crawl request, every
 * `session.*` key and `secrets.*`. `session.source`, `session.httpCredentials`, `secrets.*` and a
 * `login` step give the Label's requests Sessions of a pool of their own (ADR 0092);
 * `session.pageCheck` and `session.recovery.humanPause` judge its pages in whichever pool serves
 * them. The Session's identity (`browser.*`, `proxy.*`, `network.*`), Politeness, Site preferences,
 * scope and Run-wide keys are refused with `setting_not_applicable`: two Labels on one domain share
 * its Politeness with every run of the user (ADR 0053).
 */
export interface LabelSettings extends Omit<PageSettings, "timeout"> {
  readonly timeout?: NonNullable<PageSettings["timeout"]> & Pick<TimeoutSettings, "handler" | "hook">;
  readonly fetch?: Pick<FetchSettings, "tier" | "keptResponses" | "encoding" | "redirect" | "refresh">;
  readonly crawl?: Pick<CrawlSettings, "retry" | "handleStatuses" | "contentTypes" | "handler" | "keepUrlFragment">;
  readonly session?: SessionLoginSettings;
  readonly secrets?: SecretsSettings;
}

