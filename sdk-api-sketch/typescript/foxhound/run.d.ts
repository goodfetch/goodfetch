// A crawl's Run in code (ADR 0102, decided in #478): what its progress and its report add to every
// Run's, which are the Collie client package's. `CrawlRun` itself is in crawl.d.ts.

import type {
  BlockCause,
  LimitSource,
  NothingDone,
  RunProgress,
  RunReport,
  UncertainEffect,
} from "@goodfetch/collie-client";
import type { NotAddedReason, ReportedFetchTier } from "./crawl.js";
import type { CrawlRequestRef, SkipReason } from "./outcomes.js";
import type { RecordCounts } from "./output.js";

/**
 * Why a due crawl request is held back. A reason a newer engine sends is kept as received (G3).
 * - `domain_concurrency`, `domain_delay`: the domain's slots and delay (ADR 0086);
 * - `crawl_delay`: a host's Crawl-delay, which raised its domain's delay;
 * - `backoff`: after a 429, 502, 503, 504, or a connection refused or timed out (ADRs 0021, 0086);
 * - `robots_txt`: the origin's robots.txt, not served yet;
 * - `handler_concurrency`: `crawl.handler.concurrency` handlers already running in the program;
 * - `governor`: ADR 0040's resource governor, admitting nothing new under memory pressure;
 * - `session`: waiting for a Session of the pool, such as one logging in;
 * - `human_pause`: its Session waits for a person.
 */
export type WaitReason =
  | "domain_concurrency"
  | "domain_delay"
  | "crawl_delay"
  | "backoff"
  | "robots_txt"
  | "handler_concurrency"
  | "governor"
  | "session"
  | "human_pause"
  | (string & {});

/** The due requests one reason holds back. */
export interface WaitGroup {
  readonly reason: WaitReason;
  readonly requests: number;
  /** The domains it holds, the busiest first, with when each is next admitted; `null`: once something else happens, such as a person ending a pause. */
  readonly domains: readonly { readonly domain: string; readonly requests: number; readonly until: Date | null }[];
  /** Domains held for this reason beyond those listed. */
  readonly moreDomains: number;
  /** The setting behind the wait, with its value and layer, where one sets it. */
  readonly setting: LimitSource | null;
}

/** A crawl request being fetched or handled. */
export interface InFlightRequest {
  readonly request: CrawlRequestRef;
  /** `fetching`: from its start until its response arrives (ADR 0086); `handling`: its handler runs in the program. */
  readonly phase: "fetching" | "handling" | (string & {});
  readonly tier: ReportedFetchTier;
  readonly attempt: number;
  readonly since: Date;
  /** While its handler's own navigation or fetch waits for a permit, which counts against no timeout (ADR 0093): why, and the setting behind it. */
  readonly waiting: { readonly reason: WaitReason; readonly setting: LimitSource | null } | null;
}

/** A sitemap the engine could not read. */
export interface UnreadSitemap {
  readonly url: string;
  /** `fetch_failed`, with the response status when one came; `robots_txt`, which disallows it; `unparsable`. */
  readonly reason: "fetch_failed" | "robots_txt" | "unparsable" | (string & {});
  readonly status: number | null;
}

/** A sitemap seed's sitemaps, read while the Run works (ADR 0093's R3): its entries counted, never listed one by one. */
export interface SitemapTotals {
  readonly sitemapsRead: number;
  /** At most a fixed number, the rest counted in `moreSitemapsUnread`. */
  readonly sitemapsUnread: readonly UnreadSitemap[];
  readonly moreSitemapsUnread: number;
  readonly entriesAdded: number;
  /** The entries not added, by reason: `no_rule` for one that matched no rule, and an enqueue's other reasons (ADR 0093). */
  readonly entriesNotAdded: Readonly<Partial<Record<NotAddedReason, number>>>;
}

/** Why the crawl runs as many requests at once as it does (P47). */
export interface CrawlConcurrency {
  readonly handlers: { readonly running: number; readonly limit: LimitSource };
  /** ADR 0040's resource governor. */
  readonly governor: {
    readonly level: "normal" | "pressure" | "critical" | (string & {});
    readonly openPages: number;
    readonly allowedPages: number;
    /** What set the memory allowance: the process's cgroup, a Windows Job object, `memory_allowance`, or half of physical memory. */
    readonly allowanceFrom: "cgroup" | "job_object" | "memory_allowance" | "physical_memory" | (string & {});
  };
}

/** The ended requests counted by skip reason, Error code and Block cause. */
export interface EndedRequestBreakdown {
  readonly bySkipReason: Readonly<Partial<Record<SkipReason, number>>>;
  readonly byErrorCode: Readonly<Record<string, number>>;
  readonly byBlockCause: Readonly<Partial<Record<BlockCause, number>>>;
}

/** A crawl's requests by state: live ones, then ended ones by outcome. */
export interface CrawlRequestCounts {
  readonly pending: number;
  readonly fetching: number;
  readonly handling: number;
  readonly paused: number;
  readonly succeeded: number;
  readonly failed: number;
  readonly skipped: number;
  readonly blocked: number;
}

/** A crawl's progress: what is in flight, what waits and why. */
export interface CrawlRunProgress extends RunProgress {
  /** Its requests by state, the ended ones also by skip reason, Error code and Block cause. */
  readonly requests: CrawlRequestCounts & EndedRequestBreakdown;
  readonly waiting: readonly WaitGroup[];
  readonly inFlight: readonly InFlightRequest[];
  readonly concurrency: CrawlConcurrency;
  /** `true` while `crawl.keep_alive` holds the Run `running` with an empty Frontier and nothing in flight, until `addSeed` gives it work; idle time counts toward the Run's own timeout. */
  readonly idle: boolean;
  /** Per sitemap seed's URL. */
  readonly sitemaps: Readonly<Record<string, SitemapTotals>>;
}

/** A URL rule the Frontier learned, with its evidence (ADR 0005). */
export interface LearnedRule {
  readonly kind: "no_effect_parameter" | "trap_path" | "cursor_pagination" | (string & {});
  readonly domain: string;
  readonly rule: string;
  readonly evidence: string;
  readonly urlsMerged: number;
}

/** A crawl's requests when its Run ended; its Records per Dataset are the report's `records`. */
export interface CrawlRequestTotals {
  readonly succeeded: number;
  readonly failed: number;
  readonly skipped: number;
  readonly blocked: number;
  /** Left in the Frontier by a Run that stopped. */
  readonly pending: number;
  readonly bySkipReason: Readonly<Partial<Record<SkipReason, number>>>;
  readonly byErrorCode: Readonly<Record<string, number>>;
  readonly byBlockCause: Readonly<Partial<Record<BlockCause, number>>>;
}

/** A Dataset's Records over the Run (ADR 0103). */
export interface DatasetTotals extends RecordCounts {
  /** The dropped ones, by the Hook's reason. */
  readonly droppedByReason: Readonly<Record<string, number>>;
}

/** A Sink's delivery (ADR 0026), as it stood when the report was read: a Sink may go on delivering after the Run ends. */
export interface SinkDeliveryStatus {
  /** ADR 0026's, or `inactive` for a Sink inherited from the connection or configuration that names none of the crawl's Datasets, which delivers nothing (ADR 0103). */
  readonly status: "complete" | "complete_with_rejections" | "behind" | "failed" | "inactive" | (string & {});
  /** For an `inactive` Sink, the Dataset names it gave that the crawl does not declare. */
  readonly undeclaredDatasets: readonly string[];
  /** ADR 0026's Guarantee class. */
  readonly guaranteeClass: string;
  readonly deadLetters: number;
  /** Deliveries that fell outside the destination's deduplication window, so were at-least-once. */
  readonly atLeastOnce: number;
}

/**
 * An uncertain Effect of a crawl's Run: one declared in a handler or a Hook sits in its crawl request's
 * namespace (ADR 0106), a field only Foxhound fills, so it is Foxhound's type (ADR 0099).
 */
export interface CrawlUncertainEffect extends UncertainEffect {
  /** The crawl request whose namespace holds it; `null` for one the program declared outside a handler. */
  readonly request: CrawlRequestRef | null;
}

/** A crawl's Run report. */
export interface CrawlRunReport extends RunReport {
  readonly uncertainEffects: readonly CrawlUncertainEffect[];
  readonly requests: CrawlRequestTotals;
  /** Per Dataset name. */
  readonly records: Readonly<Record<string, DatasetTotals>>;
  /** Per Sink name. */
  readonly sinks: Readonly<Record<string, SinkDeliveryStatus>>;
  readonly learnedRules: readonly LearnedRule[];
  /** Per sitemap seed's URL, as the progress has them. */
  readonly sitemaps: Readonly<Record<string, SitemapTotals>>;
}

