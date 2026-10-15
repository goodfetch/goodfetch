// Crawl politeness defaults (ADR 0086, decided in #480).
// With nothing set, a crawl obeys robots.txt (RFC 9309) and keeps one request in flight per
// domain, one second between starts (G5). These settings govern the requests a crawl's Frontier
// admits and every navigation a crawl handler makes from its page. Politeness is Foxhound's
// (ADR 0084): a program's own navigation outside a handler (`page.goto`, an AI verb or an Agent in
// its Session) is Collie's, directed by the program like a browser's, and nothing of Politeness
// gates or counts it. The per-proxy limits and `politeness.sharing` are Collie's (ADR 0088), in the
// client package. A value that lets a crawl press a site harder than its built-in default is the
// program's explicit choice: the Audit log records it, the run's result lists it, and the run logs
// it once as a warning through the host's logging. An Operator policy or a Recipe may lock any of them (ADR 0028).

import type {
  PolitenessSettings as ColliePolitenessSettings,
  SitePreferencesSettings as CollieSitePreferencesSettings,
} from "@goodfetch/collie-client";

/** `politeness.domain.*`: per domain, keyed as in ADR 0004, and shared by every run of the user (ADR 0053). */
export interface DomainPolitenessSettings {
  /**
   * Requests in flight per domain, each from its start until its response (a browser-tier page until its
   * load event), whatever its page loads. Built-in: 1. At 1, a crawl's Run that switches hedges or `fetch.tier_race` on is refused.
   */
  readonly concurrency?: number;
  /**
   * The least time between two request starts to one domain, in milliseconds; `0` for none. Built-in: 1_000.
   * While robots.txt is respected, a host's Crawl-delay raises it for the host's whole domain key.
   */
  readonly delay?: number;
}

/** `politeness.backoff.*`: on 429, 502, 503, 504, an HTTP/2 `ENHANCE_YOUR_CALM`, and a connection refused or timed out. */
export interface BackoffSettings {
  /** Built-in: true. Off, the run records that it ignored them and `Retry-After` (ADR 0023). */
  readonly enabled?: boolean;
  /**
   * The least wait, in milliseconds. Each wait is drawn from the `Entropy` handle between this and
   * three times the previous one, at most `max`: decorrelated jitter, never below it (ADR 0118). Built-in: 2_000.
   */
  readonly initial?: number;
  /** The longest computed wait, in milliseconds; a `Retry-After` may ask for more. Built-in: 60_000. */
  readonly max?: number;
}

/**
 * `politeness.retry_after.*` (ADR 0118): a `Retry-After` in seconds or as an HTTP-date, on a 429,
 * 502, 503, 504 or a 3xx, holds its domain for the longer of it and the backoff. Nothing is sent to
 * the domain before the site's time, whatever this is set to, so it presses no site harder.
 */
export interface RetryAfterSettings {
  /**
   * The longest `Retry-After` a domain is waited for, in milliseconds. Past it, the requests the wait
   * holds end `failed` with `response_unavailable`, its `retryAfter` set, for `retryFailed()` later.
   * Built-in: 900_000 (15 minutes).
   */
  readonly max?: number;
}

/** `politeness.ip.*`: keyed by the IP the engine resolves (ADR 0021). */
export interface IPPolitenessSettings {
  /** Requests in flight per resolved IP. `null`, the built-in: no limit beyond the domain's. */
  readonly concurrency?: number | null;
}

/** `politeness.*`: one family, so `goodfetch-foxhound config explain politeness` shows every limit. */
export interface PolitenessSettings extends ColliePolitenessSettings {
  readonly domain?: DomainPolitenessSettings;
  readonly backoff?: BackoffSettings;
  readonly retryAfter?: RetryAfterSettings;
  readonly ip?: IPPolitenessSettings;
}

/** `site_preferences.*`: what the engine does with the Site preferences it always reads and records in Provenance (ADR 0016). */
export interface SitePreferencesSettings extends CollieSitePreferencesSettings {
  /** Obey robots.txt for crawl requests, its Crawl-delay included, start URLs too. Built-in: true. */
  readonly respectRobotsTxt?: boolean;
  /**
   * The longest Crawl-delay a crawl waits out, in milliseconds; a host asking for more has its requests
   * skipped with `crawl_delay`. `null`: wait out any. Built-in: 30_000.
   */
  readonly maxCrawlDelay?: number | null;
  /**
   * The operator's own product token, as the crawl's user agent names it: its robots.txt groups apply,
   * merged with `goodfetch-foxhound`'s (ADR 0090), before `*`. Built-in: none. Setting it is recorded.
   */
  readonly robotsTxtUserAgent?: string | null;
}
