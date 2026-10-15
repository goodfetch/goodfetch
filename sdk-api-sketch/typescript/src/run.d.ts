// The Run in code (ADR 0102, decided in #478).
// A Run starts with `collie.start()` or `foxhound.startCrawl(crawl, …)`, which return once it is started.
// Its ID, chosen by the program or generated, names it for good: a start whose ID names an existing
// Run resumes it, returns it ended, or refuses, and nothing is inferred from a storage folder.
// `finish()` lets the Run finish and returns its report, `stop()` ends it early with its report, and
// ADR 0078's `close()` is unchanged. Progress is a telemetry sequence (ADRs 0024, 0081); a crawl's
// outcomes are read from its committed state (crawl.d.ts).

import type { EventIterator } from "./concurrency.js";
import type { ReportedFetchTier } from "./crawl.js";
import type { Closeable } from "./lifecycle.js";
import type { RecordCounts } from "./output.js";
import type { RunUsage } from "./models.js";
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

/** A Human pause waiting for a person (ADR 0092). */
export interface WaitingPause {
  /** The Session whose work waits. */
  readonly session: string;
  /** The program's `reason`, or the engine's own, such as a login on a domain. */
  readonly reason: string | null;
  /** `program`: `page.pause()`; `engine`: a pause the engine raised, such as ADR 0006's login recovery. */
  readonly raisedBy: "program" | "engine" | (string & {});
  readonly since: Date;
}

/** A Run's progress: a telemetry value, coalesced under load (ADR 0024). */
export interface RunProgress {
  readonly lifecycle: Lifecycle;
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
  readonly requests: CrawlRequestCounts;
  readonly waiting: readonly WaitGroup[];
  readonly inFlight: readonly InFlightRequest[];
  readonly concurrency: CrawlConcurrency;
}

/**
 * A setting that weakened a safeguard for the Run (ADR 0028), with its key, value, layer and origin:
 * a costly control switched off (ADR 0023), the Run trace's "no trace" mark (ADR 0031) included, or a
 * value pressing a site harder than its built-in default (ADR 0086).
 */
export type WeakenedSetting = LimitSource;

/** A Checkpoint, as ADR 0002 reports it. */
export interface Checkpoint {
  readonly path: string;
  /** In bytes. */
  readonly size: number;
  readonly digest: string;
  readonly epoch: number;
  readonly journalPosition: number;
  readonly takenAt: Date;
}

/** Options of `checkpoint()`. */
export interface CheckpointOptions extends CallOptions {
  /** Where to write it. Left out: the Run's own folder (ADR 0031). */
  readonly path?: string;
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
  readonly verdict: Verdict;
  /** Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects (ADR 0079). */
  readonly verdictReasons: readonly string[];
  readonly completeness: Completeness;
  /** Why it is not `complete`: stopped early, a bound reached, offset pagination, pages left at `payment_required` (ADR 0005). */
  readonly completenessReasons: readonly string[];
  /** The final figures (ADR 0101), the uncapped mark included (ADR 0041). */
  readonly usage: RunUsage;
  readonly weakened: readonly WeakenedSetting[];
  /** The last Checkpoint written. */
  readonly checkpoint: Checkpoint | null;
  readonly nothingDone: NothingDone | null;
  /** The Run's page in the Console, where its Run trace replays, when the engine knows a Console address (ADR 0031). */
  readonly consoleUrl: string | null;
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
  readonly deadLetters: number;
}

/** A crawl's Run report. */
export interface CrawlRunReport extends RunReport {
  readonly requests: CrawlRequestTotals;
  /** Per Dataset name. */
  readonly records: Readonly<Record<string, DatasetTotals>>;
  /** Per Sink name. */
  readonly sinks: Readonly<Record<string, SinkDeliveryStatus>>;
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
  /** Writes a Checkpoint now (ADR 0002). */
  checkpoint(options?: CheckpointOptions): Promise<Checkpoint>;
}

/**
 * A Run read by a program that does not drive it, opened by its ID with `openRun` (ADR 0102): a
 * detached Run, another program's, or one that has ended. It never gives the Run work and takes no
 * writer lock (ADR 0031); closing it frees only the view.
 */
export interface RunView extends Closeable {
  readonly id: string;
  progress(): EventIterator<RunProgress>;
  /** The Run's report: at once for an ended Run, at its end for a live one, which it never ends. */
  report(options?: CallOptions): Promise<RunReport>;
}

// The Run's Error codes (ADR 0102).

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
  /** `product`, `kind` (a crawl or a Collie Run), `seeds`, `plan` or `caller`. */
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
/** The Run is live in another program, or already open in this one: its writer's lock is held (ADR 0031). */
export declare class RunInUseError extends RefusedError {
  readonly code: "run_in_use";
  readonly details: { readonly id: string; readonly owner: string };
}
/** A resume gave a Limit above the value the Run began with: only a person raises one (ADR 0041). */
export declare class LimitRaiseRefusedError extends RefusedError {
  readonly code: "limit_raise_refused";
  readonly details: { readonly limit: string };
}
/** The Run index lists the Run, but its state is missing or unreadable: nothing starts afresh under its ID. */
export declare class RunStateUnavailableError extends UnavailableError {
  readonly code: "run_state_unavailable";
  readonly details: { readonly id: string; readonly state: "missing" | "damaged" | (string & {}); readonly path: string | null };
}
