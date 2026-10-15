// The Run in code (ADR 0102, decided in #478).
// A Run starts with `collie.start()` or `foxhound.startCrawl(crawl, …)`, which return once it is started.
// Its ID, chosen by the program or generated, names it for good: a start whose ID names an existing
// Run resumes it, returns it ended, or refuses, and nothing is inferred from a storage folder.
// `finish()` lets the Run finish and returns its report, `stop()` ends it early with its report, and
// ADR 0078's `close()` is unchanged. Progress is a telemetry sequence (ADRs 0024, 0081); a crawl's
// outcomes are read from its committed state (crawl.d.ts).

import type { RecordingReport } from "./recording.js";
import type { EventIterator } from "./concurrency.js";
import type { NotAddedReason, ReportedFetchTier } from "./crawl.js";
import type { UncertainEffect } from "./effects.js";
import type { Closeable } from "./lifecycle.js";
import type { KeyValueStore, RecordCounts } from "./output.js";
import type { PlanReport } from "./ai.js";
import type { RunUsage } from "./models.js";
import type { ExplainedSetting } from "./settings.js";
import type { BlockCause, CrawlRequestRef, LimitSource, ProgramError, RefusedError, SkipReason, UnavailableError, Verdict } from "./results.js";
import type { CallOptions } from "./time.js";

/**
 * How a start opened its Run: `new`; `resumed` from its committed state, the run file on this machine
 * or an imported Checkpoint (ADR 0002); or `ended`, an ID naming a Run that had already finished,
 * returned as it is with nothing run.
 */
export type StartedAs = "new" | "resumed" | "ended" | (string & {});

/** Where a Run is (ADR 0029). A Run that ends `failed` makes `finish()` and `stop()` throw its error. */
export type Lifecycle = "running" | "finished" | "stopped" | "failed" | (string & {});

/** Whether a Run's results cover its scope (ADRs 0005, 0029). A Collie Run its program finishes is `complete`. */
export type Completeness = "complete" | "incomplete" | "unverified" | (string & {});

/** Why a Run stopped: ADR 0029's causes, and its own timeout (ADR 0080). */
export type RunStopReason =
  | { readonly kind: "limit"; readonly limit: string }
  | { readonly kind: "timeout" }
  /** `program`: its own `stop()` or `close()`; `person`: from the Console; `schedule`: ADR 0036's scheduler. */
  | { readonly kind: "abort"; readonly by: "program" | "person" | "schedule" | (string & {}) }
  | { readonly kind: "engine_lost" }
  | { readonly kind: "resources" }
  | { readonly kind: "unknown"; readonly raw: string };

/**
 * A Human pause waiting for a person (ADR 0092). It begins and ends as a change of `progress()`; the
 * person learns of it from ADR 0027's Notification and the Console's "Needs you". Its time counts
 * toward no timeout, the Run's included, while its Usage counts toward Limits.
 */
export interface WaitingPause {
  /** The Session whose work waits. */
  readonly session: string;
  /** The page it holds, by the page's ID, when it holds one. */
  readonly page: string | null;
  /** The program's `reason`, or the engine's own, such as a login on a domain. */
  readonly reason: string | null;
  /** The `until` condition as text, the Locator's or the URL matcher's description; `null`: a person's "Done" ends it. */
  readonly until: string | null;
  /** `program`: `page.pause()`; `engine`: a pause the engine raised. */
  readonly raisedBy: "program" | "engine" | (string & {});
  /** For an engine's pause, the step that raised it: ADR 0006's login recovery or ADR 0035's Challenge ladder. */
  readonly step: "login_recovery" | "challenge" | (string & {}) | null;
  readonly since: Date;
  /** When it runs out: the first of `timeout.human_pause` and, in a scheduled run, ADR 0036's pause timeout; `null` when it waits without limit. */
  readonly deadline: Date | null;
}

/** A Run's progress: a telemetry value, coalesced under load (ADR 0024). */
export interface RunProgress {
  readonly lifecycle: Lifecycle;
  /** `none` when nothing works the Run, as after a crash, until a start with its ID resumes it. */
  readonly writer: "live" | "none" | (string & {});
  readonly pauses: readonly WaitingPause[];
  /** The program's own line, set with `setProgressMessage` (ADR 0037's RT-15). */
  readonly message: string | null;
  /** When the engine took this value. */
  readonly at: Date;
}

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

/**
 * A setting that weakened a safeguard for the Run (ADR 0028), with its key, value, layer and origin:
 * a costly control switched off (ADR 0023), the Run trace's "no trace" mark (ADR 0031) included, or a
 * value pressing a site harder than its built-in default (ADR 0086).
 */
export type WeakenedSetting = LimitSource;

/**
 * A Checkpoint, as ADR 0002 reports it, written where the engine keeps the Run's: its own folder
 * (ADR 0031), or wherever Goodfetch Cloud stores them. Copy it to this machine as an Export is copied (ADR 0103).
 */
export interface Checkpoint {
  /** Where the engine wrote it, on the engine's side. */
  readonly location: string;
  /** In bytes. */
  readonly size: number;
  readonly digest: string;
  readonly epoch: number;
  readonly journalPosition: number;
  readonly takenAt: Date;
  /** Copies it to a file on this machine, wherever the engine runs. */
  saveAs(path: string, options?: CallOptions): Promise<void>;
  /** Its bytes, streamed to this machine. */
  stream(): AsyncIterable<Uint8Array>;
}

/** A Plugin that ran in the Run (ADRs 0016, 0033). */
export interface PluginRun {
  readonly name: string;
  readonly version: string;
  /** The network level it ran at (ADR 0033). */
  readonly networkLevel: string;
  /** Where Operator policy allowed reduced confinement for a first-party Plugin, the parts the host could not provide. */
  readonly missingConfinement: readonly string[];
}

/** A warning the Run carries: a `stealth` choice (ADR 0035), stale Clean-up rules (ADR 0047), throttled CPU (ADR 0049), a degraded Politeness store (ADR 0053), a Plugin's reported figures (ADR 0033). A newer engine's code is kept (G3). */
export interface RunWarning {
  readonly code: string;
  readonly message: string;
}

/** A URL rule the Frontier learned, with its evidence (ADR 0005). */
export interface LearnedRule {
  readonly kind: "no_effect_parameter" | "trap_path" | "cursor_pagination" | (string & {});
  readonly domain: string;
  readonly rule: string;
  readonly evidence: string;
  readonly urlsMerged: number;
}

/**
 * The Run trace (ADR 0031), copied to this machine as an Export is (ADR 0103): what was recorded so
 * far, in the format ADR 0031 exports it in. A person replays it in the Console at `consoleUrl`.
 */
export interface RunTrace {
  /** In bytes. */
  readonly size: number;
  /** Copies it to a file on this machine, wherever the engine runs. */
  saveAs(path: string, options?: CallOptions): Promise<void>;
  /** Its bytes, streamed to this machine. */
  stream(): AsyncIterable<Uint8Array>;
}

/** Why a Run is not `verified`, naming the AI task concerned, an Agent's included, by its `callId` (ADR 0100). */
export interface VerdictReason {
  /** Such as `success_check_failed`, `no_success_check`, `check_switched_off` or `effect_uncertain`; a newer engine's is kept (G3). */
  readonly code: string;
  readonly message: string;
  readonly callId: string | null;
}

/**
 * Why a Run did nothing (G1):
 * - `already_ended`: the start found the Run ended;
 * - `no_seed_handled`: no seed reached a handler, each refused when added or skipped when fetched, counted by reason;
 * - `nothing_pending`: a resume or a retry found nothing left.
 */
export type NothingDone =
  | { readonly reason: "already_ended"; readonly endedAt: Date }
  | { readonly reason: "no_seed_handled"; readonly byReason: Readonly<Partial<Record<SkipReason, number>>> }
  | { readonly reason: "nothing_pending" }
  | { readonly reason: "unknown"; readonly raw: string };

/** A Run report: what a Run states about itself when it ends. */
export interface RunReport {
  readonly id: string;
  /** `finished` or `stopped`: a `failed` Run throws instead. */
  readonly lifecycle: Lifecycle;
  /** Set when `lifecycle` is `stopped`. */
  readonly stopReason: RunStopReason | null;
  /** The Run's Verdict (ADR 0029), reading every check it relies on, each AI task's Success check included. */
  readonly verdict: Verdict;
  /** Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects (ADR 0079). */
  readonly verdictReasons: readonly VerdictReason[];
  /** Each Effect left uncertain, waiting for a person to reconcile it in the Console (ADRs 0029, 0106). */
  readonly uncertainEffects: readonly UncertainEffect[];
  readonly completeness: Completeness;
  /** Why it is not `complete`: stopped early, a bound reached, offset pagination, pages left at `payment_required` (ADR 0005). */
  readonly completenessReasons: readonly string[];
  /** The final figures (ADR 0101), the uncapped mark included (ADR 0041). */
  readonly usage: RunUsage;
  /** What became of the Run's Plan (ADR 0091): every Run has one, its file `null` when it named none. Each Agent task is one entry (ADR 0100). */
  readonly plan: PlanReport;
  /** The Operator policy version it ran under (ADR 0031). */
  readonly policyVersion: string | null;
  /** Each effective value, with the layer that set it and the one that locked it (ADR 0028). */
  readonly settings: readonly ExplainedSetting[];
  readonly plugins: readonly PluginRun[];
  readonly warnings: readonly RunWarning[];
  readonly weakened: readonly WeakenedSetting[];
  /** The last Checkpoint written. */
  readonly checkpoint: Checkpoint | null;
  readonly nothingDone: NothingDone | null;
  /** The Run's page in the Console, where its Run trace replays, when the engine knows a Console address (ADR 0031). */
  readonly consoleUrl: string | null;
  /** What the Run did with its Response recording (ADR 0105): `null` when it named none. A replayed Run says so here. */
  readonly recording: RecordingReport | null;
  readonly startedAt: Date;
  readonly endedAt: Date;
}

/** A crawl's requests when its Run ended. Records per Dataset are #479's to add. */
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
  readonly status: "complete" | "complete_with_rejections" | "behind" | "failed" | (string & {});
  /** ADR 0026's Guarantee class. */
  readonly guaranteeClass: string;
  readonly deadLetters: number;
  /** Deliveries that fell outside the destination's deduplication window, so were at-least-once. */
  readonly atLeastOnce: number;
}

/** A crawl's Run report. */
export interface CrawlRunReport extends RunReport {
  readonly requests: CrawlRequestTotals;
  /** Per Dataset name. */
  readonly records: Readonly<Record<string, DatasetTotals>>;
  /** Per Sink name. */
  readonly sinks: Readonly<Record<string, SinkDeliveryStatus>>;
  readonly learnedRules: readonly LearnedRule[];
  /** Per sitemap seed's URL, as the progress has them. */
  readonly sitemaps: Readonly<Record<string, SitemapTotals>>;
}

/** What every Run does besides its pages: end, report, show progress and write Checkpoints. */
export interface RunControl {
  /** How the start opened this Run. On `ended`, a call that would give it new work throws `RunEndedError`. */
  readonly startedAs: StartedAs;
  /**
   * Lets the Run finish, ends it `finished`, closes its Sessions and pages, and returns its report: at
   * once for a Collie Run, whose program says it is done; once its Frontier is empty and nothing is in
   * flight for a crawl; when the engine ends it for a detached Run. If something stops the Run first
   * (a Limit, its timeout, a person, the engine), the report says `stopped`. Its wait has no timeout
   * of its own: the Run's own bounds it, and a `timeout` or `signal` given here ends only the wait.
   */
  finish(options?: CallOptions): Promise<RunReport>;
  /** Stops the Run at its next safe point with a Checkpoint, `stopped` and `incomplete` as an abort by its program, and returns its report. Resumable by its ID. */
  stop(options?: CallOptions): Promise<RunReport>;
  /** The Run's progress: its first value at once, then the latest after each change, until the Run ends. Leave the loop to read one. */
  progress(): EventIterator<RunProgress>;
  /** The program's own status line, shown in `progress()` and the Console. */
  setProgressMessage(message: string, options?: CallOptions): Promise<void>;
  /** Writes a Checkpoint now (ADR 0002), where the engine keeps the Run's. */
  checkpoint(options?: CallOptions): Promise<Checkpoint>;
  /** The Run trace recorded so far; `null` when the Run keeps none, which the report's `weakened` shows. */
  trace(options?: CallOptions): Promise<RunTrace | null>;
}

/**
 * A Run read by a program that does not drive it, opened by its ID with `openRun` (ADR 0102): a
 * detached Run, another program's, or one that has ended. It never gives the Run work and takes no
 * writer lock (ADR 0031); closing it frees only the view.
 */
export interface RunView extends Closeable {
  readonly id: string;
  /** Ends after what is committed when the Run has no live writer. */
  progress(): EventIterator<RunProgress>;
  /**
   * The Run's report: at once for an ended Run, at its end for a live one, which it never ends. On a
   * Run with no live writer that has not ended, throws `RunInterruptedError`.
   */
  report(options?: CallOptions): Promise<RunReport>;
  /** The Run trace recorded so far; `null` when the Run keeps none. */
  trace(options?: CallOptions): Promise<RunTrace | null>;
  /** Its Key-value store's reads, on every Run (ADR 0103). */
  readonly keyValueStore: Pick<KeyValueStore, "get" | "getBytes" | "keys">;
}

// The Run's Error codes (ADR 0102).

/** A view's `report()` on a Run with no live writer that has not ended: a start with its ID resumes it. */
export declare class RunInterruptedError extends UnavailableError {
  readonly code: "run_interrupted";
  readonly details: { readonly id: string };
}

/** The ID names no Run in the Run index. */
export declare class RunNotFoundError extends ProgramError {
  readonly code: "run_not_found";
  readonly details: { readonly id: string };
}

/** An ID the engine refuses, such as one with characters a folder name cannot hold. */
export declare class RunIdInvalidError extends ProgramError {
  readonly code: "run_id_invalid";
  readonly details: { readonly id: string };
}

/** An input of a resumed Run that differs from what the Run began with. */
export interface RunInputDifference {
  /** `product`, `kind` (a crawl or a Collie Run), `seeds`, `plan` (the Plan file's path or identity, never its version) or `caller` (who the connection acts for). */
  readonly input: "product" | "kind" | "seeds" | "plan" | "caller" | (string & {});
  readonly recorded: string;
  readonly given: string;
}
/** The ID names a Run started with other inputs, or by another caller: give another ID. */
export declare class RunConflictError extends ProgramError {
  readonly code: "run_conflict";
  readonly details: { readonly id: string; readonly differences: readonly RunInputDifference[] };
}
/** A call that would give an ended Run new work, such as `newPage()` or `addSeed()` on a Run that `startedAs` `ended`. */
export declare class RunEndedError extends ProgramError {
  readonly code: "run_ended";
  readonly details: { readonly id: string; readonly lifecycle: Lifecycle; readonly endedAt: Date };
}
/** The Run is live in another program, or already open in this one: its writer's lock is held (ADR 0031). Retryable: that program may end. */
export declare class RunInUseError extends RefusedError {
  readonly code: "run_in_use";
  readonly details: { readonly id: string; readonly owner: string };
}
/** A resume gave a Limit above the Limit as it stands: only a person raises one (ADR 0041). */
export declare class LimitRaiseRefusedError extends RefusedError {
  readonly code: "limit_raise_refused";
  readonly details: { readonly limit: string };
}
/** The Run index lists the Run, but its state is missing or unreadable: nothing starts afresh under its ID. */
export declare class RunStateUnavailableError extends UnavailableError {
  readonly code: "run_state_unavailable";
  readonly details: { readonly id: string; readonly state: "missing" | "damaged" | (string & {}); readonly path: string | null };
}
