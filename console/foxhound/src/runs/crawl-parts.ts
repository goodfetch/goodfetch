// What a crawl adds to its Run's progress (ADR 0102), as `run.progress`
// carries it beside every Run's: Foxhound's `CrawlRunProgress` in
// `goodfetch-protocol`. The catalog's `run.progress` answers Collie's
// `RunProgress`, which keeps a crawl's parts as members it does not name,
// so no generated type names them yet; these follow the Rust types field
// for field, and a part no area supplied is absent. Engine answers are not
// validated beyond the protocol version check (ADR 0038).
import type {
  AllowanceSource,
  GovernorLevel,
  PointInTime,
  RunProgress,
  SettingLimit,
} from "@goodfetch/collie-client/generated";
import type { CountOf, SitemapReport } from "@goodfetch/foxhound/generated";

/** A crawl's requests by state. */
export interface RequestProgress {
  readonly pending: number;
  readonly fetching: number;
  readonly handling: number;
  readonly in_human_pause: number;
  readonly succeeded: number;
  readonly failed: number;
  readonly skipped: number;
  readonly blocked: number;
  readonly by_skip_reason?: readonly CountOf[];
  readonly by_error_code?: readonly CountOf[];
  readonly by_block_cause?: readonly CountOf[];
}

/** A domain a reason holds, and when it is next admitted: none until a request in flight ends. */
export interface HeldDomain {
  readonly domain: string;
  readonly next_admitted_at?: PointInTime | null;
}

/** The due requests one reason holds; a reason a newer engine sends is kept as it came. */
export interface WaitGroup {
  readonly reason: string;
  readonly count: number;
  /** At most ten; the rest are counted in `more_domains`. */
  readonly domains: readonly HeldDomain[];
  readonly more_domains?: number;
  /** The setting behind the wait, none when no setting sets it, as for the governor. */
  readonly setting?: SettingLimit | null;
}

/** Where a request in flight is. */
export type InFlightPhase = "fetching" | "handling" | "waiting_for_permit";

/** A request being fetched or handled. */
export interface InFlightRequest {
  readonly request_id: string;
  readonly url: string;
  readonly phase: InFlightPhase;
  readonly tier?: string | null;
  /** From 1. */
  readonly attempt: number;
  readonly started_at: PointInTime;
}

/** A crawl's handlers running and the Resource governor's level. */
export interface CrawlConcurrency {
  readonly handlers: number;
  readonly handler_concurrency: number;
  readonly governor?: GovernorLevel | null;
  readonly open_pages?: number | null;
  readonly page_allowance?: number | null;
  readonly allowance_set_by?: AllowanceSource | null;
}

/** A crawl's parts of its Run's progress. */
export interface CrawlParts {
  readonly requests?: RequestProgress | null;
  readonly waiting?: readonly WaitGroup[] | null;
  readonly in_flight?: readonly InFlightRequest[] | null;
  readonly sitemaps?: readonly SitemapReport[] | null;
  readonly idle?: boolean | null;
  readonly concurrency?: CrawlConcurrency | null;
}

/** A crawl's progress: every Run's, with the crawl's parts beside it. */
export type CrawlRunProgress = RunProgress & CrawlParts;

/** `progress` read as a crawl's, its parts as the engine sent them. */
export function crawlProgressOf(progress: RunProgress): CrawlRunProgress {
  // A crawl's engine sends its parts beside every Run's, among the members
  // `RunProgress` keeps unnamed; the Console does not validate them again
  // (ADR 0038).
  return progress;
}
