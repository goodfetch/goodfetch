// The Politeness and Site preferences Collie carries (ADRs 0086, 0088): the per-proxy limits, which
// protect the user's own proxy accounts and hold a program's own navigation through them, how far
// they are shared, and whether AI-use signals apply. Politeness toward sites (per domain, backoff,
// robots.txt, Crawl-delay) is a crawl's, in `@goodfetch/foxhound`.

/** `politeness.proxy.*`: per proxy endpoint and credential (ADR 0053), for a crawl and a program's own navigation through a proxy alike (ADR 0088). */
export interface ProxyPolitenessSettings {
  /** Requests in flight through one proxy. `null`, the built-in: only the ceiling learned from its refusals. */
  readonly concurrency?: number | null;
  /** Requests per second through one proxy. `null`, the built-in: no rate. */
  readonly rate?: number | null;
}

/** How far a run shares Politeness (ADR 0053). Anything but `shared` is marked in the run's result. */
export type PolitenessSharing = "shared" | "per_daemon" | "run";

/** `politeness.*` as Collie carries it. */
export interface PolitenessSettings {
  readonly proxy?: ProxyPolitenessSettings;
  /** Built-in: `shared`. */
  readonly sharing?: PolitenessSharing;
}

/** `site_preferences.*` as a Collie Run sets it: Politeness and robots.txt are a crawl's (ADR 0084). */
export interface SitePreferencesSettings {
  /** Apply the AI-use signals that concern the run's Purpose (ADR 0016). Built-in: false. A Collie program's own page is never refused for one (ADR 0085). */
  readonly respectUseSignals?: boolean;
}
