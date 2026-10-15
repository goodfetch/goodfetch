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
import type { PolitenessSettings, SitePreferencesSettings } from "./politeness.js";
import type { Permission, ProxySettings, SecretsSettings, SessionLoginSettings } from "./sessions.js";
import type { AgentSettings } from "./agent.js";
import type { AISettings } from "./models.js";
import type { EffectSettings } from "./effects.js";

/**
 * `timeout.*`: ADR 0080's phases, one family. `null` clears a phase's timeout for the whole scope.
 * `timeout.connect` is set only in the project file, which `connect()` applies; a connection's own
 * is `connect()`'s `timeout` (S2).
 */
export interface TimeoutSettings {
  readonly action?: Timeout;
  readonly navigation?: Timeout;
  readonly wait?: Timeout;
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
  /**
   * `browser.block_resources`: resource types the engine's interception blocks on every page of the
   * Session, before any navigation, so every browser fetch of a crawl is covered (ADRs 0047, 0117).
   * Empty by default. Set directly on a Label whose `fetch.tier` runs no browser, it is refused with
   * `setting_not_applicable`. In the `standard` and `stealth` Access modes it warns once, since a site
   * can see the missing loads. Finer blocking, by URL or by page, is a before-navigation Hook's route.
   */
  readonly blockResources?: readonly BlockedResourceType[];
}

/** A resource type `browser.block_resources` takes: only these four, any other a type error (G3, ADR 0117). */
export type BlockedResourceType = "image" | "media" | "font" | "stylesheet";

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

/** `checkpoint.*` (ADRs 0002, 0102). */
export interface CheckpointSettings {
  /** `checkpoint.interval`: how often the Run writes a Checkpoint, in milliseconds. Built-in: none locally (`null`). */
  readonly interval?: number | null;
}

/** Every setting of Collie, as a connection takes them. A sample of the generated type; Foxhound's adds its own keys. */
export interface Settings {
  readonly locator?: LocatorSettings;
  readonly timeout?: TimeoutSettings;
  readonly browser?: BrowserSettings;
  readonly network?: NetworkSettings;
  readonly plan?: PlanSettings;
  readonly extract?: ExtractSettings;
  readonly observation?: ObservationSettings;
  /** `ai.*`: the model, Model providers and structured output (ADR 0101). */
  readonly ai?: AISettings;
  /** `agent.*`: the Agent's step cap, stall rule, domains and Human pause (ADR 0100). */
  readonly agent?: AgentSettings;
  /** `politeness.proxy.*` and `politeness.sharing`: the per-proxy limits, which Collie carries (ADR 0088). */
  readonly politeness?: PolitenessSettings;
  /** `site_preferences.respect_use_signals` (ADR 0086). */
  readonly sitePreferences?: SitePreferencesSettings;
  /** `session.*` (ADRs 0056, 0006, 0092). */
  readonly session?: SessionLoginSettings;
  /** `secrets.<name>`: the Secret placeholders' bindings (ADR 0092). */
  readonly secrets?: SecretsSettings;
  /** `proxy.*` (ADRs 0021, 0092). */
  readonly proxy?: ProxySettings;
  readonly checkpoint?: CheckpointSettings;
  /** `effect.*`: what the Effect journal keeps of a declared Effect (ADR 0106). */
  readonly effect?: EffectSettings;
}

/**
 * A Collie Run's settings: no `timeout.run`, which starting the Run spells as its `timeout` (S2), and
 * no Politeness toward sites, which is Foxhound's (ADRs 0086, 0088). A Foxhound key given here on a
 * Foxhound engine is refused with `setting_not_applicable`, and on a Collie engine with `not_in_product`.
 */
export interface RunSettings {
  readonly locator?: LocatorSettings;
  readonly timeout?: Omit<TimeoutSettings, "run">;
  readonly browser?: BrowserSettings;
  readonly network?: NetworkSettings;
  readonly plan?: PlanSettings;
  readonly extract?: ExtractSettings;
  readonly observation?: ObservationSettings;
  readonly ai?: AISettings;
  readonly agent?: AgentSettings;
  readonly sitePreferences?: SitePreferencesSettings;
  readonly session?: SessionLoginSettings;
  readonly secrets?: SecretsSettings;
  readonly proxy?: ProxySettings;
  readonly checkpoint?: CheckpointSettings;
  readonly effect?: EffectSettings;
}

/**
 * A Session's settings: Site preferences are a Run's, and `session.source` is opening the Session's
 * `source` argument, one spelling (S2). Set where a Borrowed browser's Session opens, any `browser.*`
 * value is refused with `setting_not_applicable`.
 */
export interface SessionSettings extends Omit<RunSettings, "sitePreferences" | "session" | "plan" | "checkpoint" | "effect" | "ai"> {
  readonly session?: Omit<SessionLoginSettings, "source">;
  /** A Run's Model allowance is counted per Run, so a Session takes none (ADR 0107). */
  readonly ai?: Omit<AISettings, "allowance">;
}

/** `locator.*` (ADR 0085), settable at any scope. */
export interface LocatorSettings {
  /** `locator.test_id_attribute`: the attribute `getByTestId` matches, `data-testid` by default. */
  readonly testIdAttribute?: string;
}

/** A page's settings: no part of the Session's identity. */
export interface PageSettings {
  readonly locator?: LocatorSettings;
  readonly timeout?: Pick<TimeoutSettings, "action" | "navigation" | "wait" | "humanPause" | "ai" | "model" | "agent">;
  readonly extract?: ExtractSettings;
  readonly observation?: ObservationSettings;
  /** The model and the strategy for the page's AI calls; Model providers are declared above a page. */
  readonly ai?: Pick<AISettings, "model" | "structuredOutput">;
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
  | "ai.allowance.price"
  | "ai.allowance.tokens"
  | "ai.allowance.calls"
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
  | "browser.block_resources"
  | "network.throttle"
  | "effect.output.max_size"
  | "politeness.proxy.concurrency"
  | "politeness.proxy.rate"
  | "politeness.sharing"
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
  | `secrets.${string}`;

/** One effective value, as `goodfetch-collie config explain` shows it for the scope. */
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
export interface ExplainsSettings<K extends string = SettingKey> {
  /** All the scope's settings when `keys` is left out. A Foxhound value takes Foxhound's keys too. */
  explainSettings(keys?: readonly K[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
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
