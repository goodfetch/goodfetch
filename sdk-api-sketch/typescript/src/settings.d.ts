// Settings in code (ADR 0083, decided in #470).
// One `settings` option on each value that opens a scope (`connect()`, starting a Run, declaring a
// crawl's Label (ADR 0093), opening a Session or a page); the nearest scope wins and is inherited
// beneath it. A call takes only its task's inputs and `timeout`. There are no setters (G2).
// The types are generated from the settings schema (ADR 0028), each segment of a key camelCased;
// this file shows a sample of its keys. Deployment variables are no keys here: `connect()` reads
// them from its environment map. Left out: inherited. `null`: cleared, where the type allows
// it. A value: set. The engine resolves the layers, so the SDK sends only what the program set.

import type { CallOptions, Timeout } from "./time.js";
import type { ProgramError, RefusedError, RunLayerOrigin, SettingsLayer, UnavailableError } from "./results.js";
import type { FetchTier, UrlPattern } from "./crawl.js";
import type {
  CollieSitePreferencesSettings,
  PolitenessSettings,
  SitePreferencesSettings,
} from "./politeness.js";
import type { Permission, ProxySettings, SecretsSettings, SessionLoginSettings } from "./sessions.js";
import type { AgentSettings } from "./agent.js";
import type { AiSettings } from "./models.js";

/**
 * `timeout.*`: ADR 0080's phases, one family. `null` clears a phase's timeout for the whole scope.
 * `timeout.connect` is set only in the project file, which `connect()` applies; a connection's own
 * is `connect()`'s `timeout` (S2).
 */
export interface TimeoutSettings {
  readonly action?: Timeout;
  readonly navigation?: Timeout;
  readonly wait?: Timeout;
  readonly handler?: Timeout;
  readonly hook?: Timeout;
  readonly sinkWrite?: Timeout;
  readonly close?: Timeout;
  /** The default for each Run's own timeout, given at the connection; its built-in default is none. A Run's own is starting it with `timeout`. */
  readonly run?: Timeout;
  /** `timeout.human_pause`: how long a Human pause may wait for a person; 24 hours by default (ADR 0092). */
  readonly humanPause?: Timeout;
  /** `timeout.ai`: an AI verb from its call to its return, model calls and a Repair included, 120 s (ADR 0091). */
  readonly ai?: Timeout;
  /** `timeout.model`: one attempt of one model request, the program's own provider included, 60 s; an attempt past it is retried, within the call that made it (ADR 0101). */
  readonly model?: Timeout;
  /** `timeout.agent`: an Agent's task, 10 minutes; reaching it stops the task with the reason `timeout` (ADR 0100). */
  readonly agent?: Timeout;
}

/** `plan.*` (ADR 0091). */
export interface PlanSettings {
  /** Replay only: never call a model and never write the Plan; a missing step or a Drift throws. Default: `false`. */
  readonly frozen?: boolean;
}

/** `extract.*` (ADR 0091). */
export interface ExtractSettings {
  /** `compiled` (the default) or `model`; `extract()` takes it for one call as `mode`. */
  readonly mode?: "compiled" | "model";
}

/** `observation.*` (ADR 0091). */
export interface ObservationSettings {
  /** The most characters an `ai` aria snapshot holds before its deepest subtrees collapse. Default: 40,000. */
  readonly maxSize?: number;
}

export interface Viewport {
  readonly width: number;
  readonly height: number;
}

export interface Geolocation {
  readonly latitude: number;
  readonly longitude: number;
  readonly accuracy?: number;
}

/**
 * `browser.*`: part of a Session's identity, fixed for its life (ADR 0021), so set where a Session
 * opens or above it, never on a page. Playwright's names where the meaning matches (S3). A value
 * set where a Session opens that conflicts with an identity fixed elsewhere (a `stealth` Identity
 * profile, an imported login, a Profile directory used in place) is refused with `setting_locked`;
 * an inherited one yields, and `explainSettings()` names what fixed it.
 */
export interface BrowserSettings {
  readonly viewport?: Viewport | null;
  readonly deviceScaleFactor?: number;
  /** `null`: matched to the proxy exit, the `standard` Access mode's default. */
  readonly locale?: string | null;
  /** `null`: matched to the proxy exit, as `locale`. */
  readonly timezoneId?: string | null;
  readonly colorScheme?: "light" | "dark" | "no-preference";
  readonly javaScriptEnabled?: boolean;
  readonly geolocation?: Geolocation | null;
  /** `browser.headless`: `true` by default, `false` under `stealth` (ADR 0035). A person at the keyboard sees the window (ADR 0092). */
  readonly headless?: boolean;
  /** `browser.permissions`: granted to every page of the Session; none by default, no setter later (ADR 0092). */
  readonly permissions?: readonly Permission[];
}

/** Emulated network conditions, which Playwright closed as not planned (#8622). */
export interface NetworkThrottle {
  readonly downloadBytesPerSecond: number;
  readonly uploadBytesPerSecond: number;
  /** Added latency, in milliseconds. */
  readonly latency: number;
}

export interface NetworkSettings {
  readonly throttle?: NetworkThrottle | null;
}

/** `crawl.scope.strategy`: which hosts a crawl may reach, measured against every seed after the Frontier's URL rules, so `www.` is no other host (ADR 0005). */
export type CrawlScopeStrategy = "same_hostname" | "same_domain" | "same_origin" | "all";

/** `crawl.scope.*`: checked on every enqueue and every redirect (ADR 0093). */
export interface CrawlScopeSettings {
  /** Built-in: `same_hostname`, Crawlee's default. */
  readonly strategy?: CrawlScopeStrategy;
  /** Only URLs matching one of these, which must match as well as the strategy, never replacing it. */
  readonly include?: readonly UrlPattern[];
  /** No URL matching any of these. */
  readonly exclude?: readonly UrlPattern[];
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
  readonly retry?: CrawlRetrySettings;
  /** `crawl.handle_statuses`: statuses outside 2xx whose responses go to the handler instead of failing with `response_not_ok`. Built-in: none. */
  readonly handleStatuses?: readonly number[];
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

/** `checkpoint.*` (ADRs 0002, 0102). */
export interface CheckpointSettings {
  /** `checkpoint.interval`: how often the Run writes a Checkpoint, in milliseconds. Built-in: none locally (`null`). */
  readonly interval?: number | null;
}

/** `fetch.*`: the fetch tiers of ADR 0020 (ADR 0093). */
export interface FetchSettings {
  /** `auto`, the built-in: learned per Label on a domain by sample comparison. Any other value forces the tier (ADR 0023). */
  readonly tier?: "auto" | FetchTier;
  /** `fetch.tier_race`, ADR 0040's `tier_race`. Built-in: false. */
  readonly tierRace?: boolean;
}

/** A Sink's write mode (ADR 0026). The keyed ones need a key: the Sink's own or its Datasets'. */
export type SinkWriteMode = "append" | "upsert" | "insert_only" | "replace" | "history";

/**
 * `sinks.<name>.*`: one Sink each (ADRs 0026, 0103), set on a connection or a crawl's Run, never on a
 * Label. A sample: each Sink type's own keys (its destination as a Secret reference, its table or
 * bucket) are area 6's spec.
 */
export interface SinkSettings {
  readonly type: string;
  /** The Datasets it receives, by name. One the crawl does not declare is refused when the Run starts, with `setting_invalid` naming the closest. */
  readonly datasets: readonly string[];
  /** Built-in: `append`. */
  readonly mode?: SinkWriteMode;
  /** Built-in: each Dataset's own `key`. */
  readonly key?: readonly string[];
}

/** Every setting, as a connection takes them. A sample of the generated type. */
export interface Settings {
  readonly timeout?: TimeoutSettings;
  readonly browser?: BrowserSettings;
  readonly network?: NetworkSettings;
  readonly crawl?: CrawlSettings;
  readonly plan?: PlanSettings;
  readonly extract?: ExtractSettings;
  readonly observation?: ObservationSettings;
  /** `ai.*`: the model, Model providers and structured output (ADR 0101). */
  readonly ai?: AiSettings;
  /** `agent.*`: the Agent's step cap, stall rule, domains and Human pause (ADR 0100). */
  readonly agent?: AgentSettings;
  /** ADR 0093. */
  readonly fetch?: FetchSettings;
  /** ADR 0086. */
  readonly politeness?: PolitenessSettings;
  /** ADR 0086. */
  readonly sitePreferences?: SitePreferencesSettings;
  /** `session.*` (ADRs 0056, 0006, 0092). */
  readonly session?: SessionLoginSettings;
  /** `secrets.<name>`: the Secret placeholders' bindings (ADR 0092). */
  readonly secrets?: SecretsSettings;
  /** `proxy.*` (ADRs 0021, 0092). */
  readonly proxy?: ProxySettings;
  /** `sinks.<name>.*`, keyed by the Sink's name (ADR 0103). Foxhound's: a Collie Run has no Datasets. */
  readonly sinks?: Readonly<Record<string, SinkSettings>>;
  readonly checkpoint?: CheckpointSettings;
}

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
  /** `locator.test_id_attribute`, settable at any scope (ADR 0085). */
  readonly locator?: { readonly testIdAttribute?: string };
  readonly fetch?: Pick<FetchSettings, "tier">;
  readonly crawl?: Pick<CrawlSettings, "retry" | "handleStatuses" | "handler">;
  readonly session?: SessionLoginSettings;
  readonly secrets?: SecretsSettings;
}

/**
 * A Collie Run's settings: no `crawl.*`, no `timeout.run`, which starting the Run spells as its
 * `timeout` (S2), and no Politeness, which is Foxhound's (ADRs 0084, 0086). A setting the
 * type cannot rule out, such as `timeout.handler`, is refused with `setting_not_applicable`.
 */
export interface RunSettings {
  readonly timeout?: Omit<TimeoutSettings, "run">;
  readonly browser?: BrowserSettings;
  readonly network?: NetworkSettings;
  readonly plan?: PlanSettings;
  readonly extract?: ExtractSettings;
  readonly observation?: ObservationSettings;
  readonly ai?: AiSettings;
  readonly agent?: AgentSettings;
  readonly sitePreferences?: CollieSitePreferencesSettings;
  readonly session?: SessionLoginSettings;
  readonly secrets?: SecretsSettings;
  readonly proxy?: ProxySettings;
  readonly checkpoint?: CheckpointSettings;
}

/**
 * A Session's settings: Site preferences are a Run's, and `session.source` is opening the Session's
 * `source` argument, one spelling (S2). Set where a Borrowed browser's Session opens, any `browser.*`
 * value is refused with `setting_not_applicable`.
 */
export interface SessionSettings extends Omit<RunSettings, "sitePreferences" | "session" | "plan" | "checkpoint"> {
  readonly session?: Omit<SessionLoginSettings, "source">;
}

/** A page's settings: no part of the Session's identity. */
export interface PageSettings {
  readonly timeout?: Pick<TimeoutSettings, "action" | "navigation" | "wait" | "humanPause" | "ai" | "model" | "agent">;
  readonly extract?: ExtractSettings;
  readonly observation?: ObservationSettings;
  /** The model and the strategy for the page's AI calls; Model providers are declared above a page. */
  readonly ai?: Pick<AiSettings, "model" | "structuredOutput">;
  readonly agent?: AgentSettings;
}

/** Options of opening a page. Where pages and Sessions are opened is #472's and #476's. */
export interface NewPageOptions extends CallOptions {
  readonly settings?: PageSettings;
}

/** A setting's canonical key: dotted snake_case, area first, the same in configuration files, errors and `config explain`. A sample. */
export type SettingKey =
  | "timeout.connect"
  | "timeout.action"
  | "timeout.navigation"
  | "timeout.wait"
  | "timeout.handler"
  | "timeout.hook"
  | "timeout.sink_write"
  | "timeout.close"
  | "timeout.run"
  | "timeout.human_pause"
  | "timeout.ai"
  | "timeout.model"
  | "timeout.agent"
  | "agent.max_steps"
  | "agent.max_stalled_steps"
  | "agent.domains"
  | "agent.human_pause"
  | "plan.frozen"
  | "extract.mode"
  | "observation.max_size"
  | "ai.model"
  | "ai.structured_output"
  | `ai.providers.${string}`
  | "browser.viewport"
  | "browser.device_scale_factor"
  | "browser.locale"
  | "browser.timezone_id"
  | "browser.color_scheme"
  | "browser.java_script_enabled"
  | "browser.geolocation"
  | "browser.headless"
  | "browser.permissions"
  | "network.throttle"
  | "crawl.payload.max_size"
  | "politeness.domain.concurrency"
  | "politeness.domain.delay"
  | "politeness.backoff.enabled"
  | "politeness.backoff.initial"
  | "politeness.backoff.max"
  | "politeness.ip.concurrency"
  | "politeness.proxy.concurrency"
  | "politeness.proxy.rate"
  | "politeness.sharing"
  | "politeness.degraded_max"
  | "site_preferences.respect_robots_txt"
  | "site_preferences.max_crawl_delay"
  | "site_preferences.robots_txt_user_agent"
  | "site_preferences.respect_use_signals"
  | "session.source"
  | "session.page_check"
  | "session.recovery.human_pause"
  | "session.recovery.agent"
  | "session.http_credentials"
  | "proxy.server"
  | "proxy.bypass"
  | "proxy.username"
  | "proxy.password"
  | "crawl.scope.strategy"
  | "crawl.scope.include"
  | "crawl.scope.exclude"
  | "crawl.max_depth"
  | "crawl.max_requests"
  | "crawl.retry.max"
  | "crawl.retry.handler_failed"
  | "crawl.handle_statuses"
  | "crawl.handler.concurrency"
  | "fetch.tier"
  | "fetch.tier_race"
  | `secrets.${string}`;

/** One effective value, as `goodfetch config explain` shows it for the scope. */
export interface ExplainedSetting {
  /** A key this SDK does not know is kept as received (G3). */
  readonly key: SettingKey | (string & {});
  /** A Secret reference shows as the reference. */
  readonly value: unknown;
  readonly layer: SettingsLayer;
  /** Set when `layer` is `run`. */
  readonly origin?: RunLayerOrigin;
  /** The configuration file, when `layer` is `configuration`. */
  readonly file?: string;
  /** The layer that locked or capped the value, if any. */
  readonly lockedBy?: SettingsLayer;
  /** The setting that fixed the Session's identity, such as `session.source`, when an inherited `browser.*` value yielded to it; `value` is then the identity's, and `layer` that setting's. */
  readonly fixedBy?: string;
}

/** Every scope value reads its effective settings; none sets them. */
export interface ExplainsSettings {
  /** All the scope's settings when `keys` is left out. */
  explainSettings(keys?: readonly SettingKey[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
}

/** The scopes a setting can be set at. */
export type SettingScope = "connection" | "run" | "label" | "session" | "page";

// The Error codes of settings (ADR 0083). Each is checked when the scope opens, or when a call
// with a per-call argument starts, so it points at the line that set the value; nothing is clamped,
// dropped or ignored (G3).

export interface SettingUnknownDetails {
  /** The key as written, or the `GOODFETCH_` variable. */
  readonly key: string;
  /** The closest valid key or listed variable, when one is close. */
  readonly closest: string | null;
}
/** An unknown key, or an unknown `GOODFETCH_` variable at `connect()`. */
export declare class SettingUnknownError extends ProgramError {
  readonly code: "setting_unknown";
  readonly details: SettingUnknownDetails;
}

export interface SettingInvalidDetails {
  readonly key: string;
  /** The type or range the setting accepts, such as `a positive number of milliseconds, or null`. */
  readonly expected: string;
}
/** A wrong type or an out-of-range value, or an empty or unparsable `GOODFETCH_` variable. */
export declare class SettingInvalidError extends ProgramError {
  readonly code: "setting_invalid";
  readonly details: SettingInvalidDetails;
}

export interface SettingNotApplicableDetails {
  readonly key: string;
  /** Where the program set it. */
  readonly scope: SettingScope;
  /** Where the setting takes effect. */
  readonly appliesTo: readonly SettingScope[];
}
/** A setting no scope beneath can use, such as a `crawl.*` setting on a Collie Run or `browser.viewport` on a page. */
export declare class SettingNotApplicableError extends ProgramError {
  readonly code: "setting_not_applicable";
  readonly details: SettingNotApplicableDetails;
}

export interface SettingLockedDetails {
  readonly key: string;
  /** The layer that locked or capped the value, or set the setting that fixed the identity. */
  readonly lockedBy: SettingsLayer;
  /** The setting that fixed a Session's identity, such as `session.source` or the Access mode's; `null` for a lock or a cap. */
  readonly fixedBy: string | null;
}
/**
 * A value a lower layer locked or capped, or outside a Recipe's exposed bounds, or a `browser.*`
 * value that conflicts with an identity fixed elsewhere; `limit` names that layer, or the setting
 * that fixed the identity, and its value.
 */
export declare class SettingLockedError extends RefusedError {
  readonly code: "setting_locked";
  readonly details: SettingLockedDetails;
}

export interface CapabilityUnavailableDetails {
  readonly capability: string;
  readonly platform: string;
}
/** A capability the platform cannot provide; `fix` names the setting that turns it off (ADRs 0038, 0057). */
export declare class CapabilityUnavailableError extends UnavailableError {
  readonly code: "capability_unavailable";
  readonly details: CapabilityUnavailableDetails;
}
