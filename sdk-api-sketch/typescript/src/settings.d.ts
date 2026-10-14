// Settings in code (ADR 0083, decided in #470).
// One `settings` option on each value that opens a scope (`connect()`, starting a Run, opening a
// Session or a page); the nearest scope wins and is inherited beneath it. A call takes only its
// task's inputs and `timeout`. There are no setters (G2).
// The types are generated from the settings schema (ADR 0028), each segment of a key camelCased;
// this file shows a sample of its keys. Deployment variables are no keys here: `connect()` reads
// them from its environment map. Left out: inherited. `null`: cleared, where the type allows
// it. A value: set. The engine resolves the layers, so the SDK sends only what the program set.

import type { CallOptions, Timeout } from "./time.js";
import type { ProgramError, RefusedError, RunLayerOrigin, SettingsLayer, UnavailableError } from "./results.js";
import type {
  ColliePolitenessSettings,
  CollieSitePreferencesSettings,
  PolitenessSettings,
  SitePreferencesSettings,
} from "./politeness.js";

/**
 * `timeout.*`: ADR 0080's phases, one family. `null` clears a phase's timeout for the whole scope.
 * `timeout.connect` is set only in configuration: a connection's own is `connect()`'s `timeout` (S2).
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

export interface CrawlSettings {
  readonly payload?: {
    /** `crawl.payload.max_size`, in bytes (ADR 0082). */
    readonly maxSize?: number;
  };
}

/** Every setting, as a connection takes them. A sample of the generated type. */
export interface Settings {
  readonly timeout?: TimeoutSettings;
  readonly browser?: BrowserSettings;
  readonly network?: NetworkSettings;
  readonly crawl?: CrawlSettings;
  /** ADR 0086. */
  readonly politeness?: PolitenessSettings;
  /** ADR 0086. */
  readonly sitePreferences?: SitePreferencesSettings;
}

/**
 * A Collie Run's settings: no `crawl.*`, no `timeout.run`, which starting the Run spells as its
 * `timeout` (S2), and none of the politeness a crawl's Frontier alone uses (ADR 0086). A setting the
 * type cannot rule out, such as `timeout.handler`, is refused with `setting_not_applicable`.
 */
export interface RunSettings {
  readonly timeout?: Omit<TimeoutSettings, "run">;
  readonly browser?: BrowserSettings;
  readonly network?: NetworkSettings;
  readonly politeness?: ColliePolitenessSettings;
  readonly sitePreferences?: CollieSitePreferencesSettings;
}

/**
 * A Session's settings: Politeness and Site preferences are a Run's. On a Borrowed browser, any
 * `browser.*` value is refused with `setting_not_applicable`.
 */
export type SessionSettings = Omit<RunSettings, "politeness" | "sitePreferences">;

/** A page's settings: no part of the Session's identity. */
export interface PageSettings {
  readonly timeout?: Pick<TimeoutSettings, "action" | "navigation" | "wait">;
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
  | "browser.viewport"
  | "browser.device_scale_factor"
  | "browser.locale"
  | "browser.timezone_id"
  | "browser.color_scheme"
  | "browser.java_script_enabled"
  | "browser.geolocation"
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
  | "site_preferences.respect_use_signals";

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
}

/** Every scope value reads its effective settings; none sets them. */
export interface ExplainsSettings {
  /** All the scope's settings when `keys` is left out. */
  explainSettings(keys?: readonly SettingKey[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
}

/** The scopes a setting can be set at. */
export type SettingScope = "connection" | "run" | "session" | "page";

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
