// What a served test's pages do wrong (ADR 0068): a `console` error, an
// uncaught page error, a CSP violation, seen both by the page's
// `securitypolicyviolation` event and as the report the browser sends to the
// CSP report endpoint, and a same-origin request that failed. The
// `failOnErrors` fixture fails a test that recorded any.
//
// A known issue of a test tool, such as a CSP violation the tool's own
// injected code raises, is tolerated only while the one action that raises
// it runs (`tolerating`), and only the errors it matches: a `csp_violation`
// it matches, and beside each such violation one `console` message and one
// `csp_report` it matches, which the browser sends for the same refusal.
// Tolerated errors never fail a test; they stay listed in `tolerated`.
import type { BrowserContext, Request } from "@playwright/test";
import { TestSupportError } from "./errors.ts";

/** Which way a page went wrong. */
export type PageErrorKind =
  /** A `console.error` message, or one the browser logged at the error level. */
  | "console"
  /** An error the page threw and nothing caught. */
  | "page"
  /** The CSP refused something, as the page's `securitypolicyviolation` event says. */
  | "csp_violation"
  /** The browser sent a CSP violation report. */
  | "csp_report"
  /** A same-origin request failed, or answered with an HTTP status of 400 or above. */
  | "request_failed";

/** One thing a page did wrong. */
export interface PageError {
  readonly kind: PageErrorKind;
  /** What happened, as the browser describes it. */
  readonly text: string;
  /** The URL of the page or request it happened on. */
  readonly url: string;
}

/** A fault of a test tool that makes a page error the page did not cause. */
export interface KnownIssue {
  /** A short name for it. */
  readonly id: string;
  /** Where it is tracked upstream, or what reporting it needs. */
  readonly upstream: string;
  /** Whether `error` is one this issue raises. */
  matches(error: PageError): boolean;
}

/** An error a known issue matched, which fails no test. */
export interface ToleratedError extends PageError {
  /** The `id` of the known issue that matched it. */
  readonly issue: string;
}

/** The errors of every page of one context. */
export interface PageErrorWatch {
  /** Every error recorded so far, in the order they came. */
  readonly errors: readonly PageError[];
  /** Every error a known issue matched while it was tolerated, in the order they came. */
  readonly tolerated: readonly ToleratedError[];
  /**
   * Runs `during`, tolerating the errors `issue` matches that the pages
   * raise meanwhile, once each open page has delivered those it raised.
   */
  tolerating<T>(issue: KnownIssue, during: () => Promise<T>): Promise<T>;
  /** Fails with `page_errors` when any error was recorded, once each open page has delivered those it raised. */
  check(): Promise<void>;
  /**
   * Answers and forgets every error recorded so far, once each open page
   * has delivered those it raised: a test that provokes errors on purpose,
   * such as the production CSP's refusals, asserts them, and `check` judges
   * only those that come after.
   */
  take(): Promise<readonly PageError[]>;
}

/** The page's function, exposed by the watch, that each CSP violation calls. */
const VIOLATION_BINDING = "__goodfetchSecurityPolicyViolation";

/** The content types of a CSP report: `report-uri`'s and the Reporting API's. */
const REPORT_TYPES = ["application/csp-report", "application/reports+json"];

/**
 * How each browser names a request it cancelled itself, which is no error:
 * the page navigated away or closed while the request was in flight.
 */
const CANCELLED = new Set(["net::ERR_ABORTED", "NS_BINDING_ABORTED", "cancelled"]);

/** The watch of each context, so a helper given a page finds its watch. */
const watches = new WeakMap<BrowserContext, PageErrorWatch>();

/** The watch recording the errors of `context`'s pages, if one does. */
export function watchOf(context: BrowserContext): PageErrorWatch | undefined {
  return watches.get(context);
}

/** Records the errors of every page `context` has or opens from now on. */
export async function watchPageErrors(context: BrowserContext): Promise<PageErrorWatch> {
  const errors: PageError[] = [];
  const tolerated: ToleratedError[] = [];
  // The issue tolerated now, and how many of each issue's CSP violations
  // still wait for the report the browser sends for each.
  let active: KnownIssue | undefined;
  const issues = new Map<string, KnownIssue>();
  const reportsDue = new Map<string, number>();
  const record = (kind: PageErrorKind, text: string, url: string): void => {
    const error = { kind, text, url };
    if (active?.matches(error) === true) {
      // Checked against the violations once the action ends.
      tolerated.push({ ...error, issue: active.id });
      return;
    }
    if (kind === "csp_report") {
      // A report may come after its action ended: it is tolerated as the
      // report of a violation an issue matched, whenever it comes.
      for (const [id, due] of reportsDue) {
        if (due > 0 && issues.get(id)?.matches(error) === true) {
          reportsDue.set(id, due - 1);
          tolerated.push({ ...error, issue: id });
          return;
        }
      }
    }
    errors.push(error);
  };
  await context.exposeBinding(VIOLATION_BINDING, ({ page }, violation: string) => {
    record("csp_violation", violation, page.url());
  });
  await context.addInitScript((binding) => {
    document.addEventListener(
      "securitypolicyviolation",
      (event) => {
        const report: unknown = Reflect.get(globalThis, binding);
        if (typeof report === "function") {
          report(
            `${event.effectiveDirective} refused ${event.blockedURI || "inline code"} at ${event.sourceFile}:${event.lineNumber}`,
          );
        }
      },
      true,
    );
  }, VIOLATION_BINDING);
  context.on("console", (message) => {
    if (message.type() === "error") {
      record("console", message.text(), message.page()?.url() ?? "");
    }
  });
  context.on("weberror", (error) => {
    record("page", String(error.error()), error.page()?.url() ?? "");
  });
  context.on("request", (request) => {
    const type = request.headers()["content-type"] ?? "";
    if (REPORT_TYPES.some((reportType) => type.startsWith(reportType))) {
      record("csp_report", request.postData() ?? "", request.url());
    }
  });
  context.on("requestfailed", (request) => {
    const failure = request.failure()?.errorText ?? "failed";
    if (!CANCELLED.has(failure) && isSameOrigin(request)) {
      record("request_failed", failure, request.url());
    }
  });
  context.on("response", (response) => {
    if (response.status() >= 400 && isSameOrigin(response.request())) {
      record("request_failed", `HTTP ${response.status()}`, response.url());
    }
  });
  // A violation reaches the watch through the page's own event loop: the
  // browser queues a task to fire its event (Chromium on its networking task
  // queue), and an evaluation can run before that task does. Each open page
  // answers from a task of its own, queued after those already raised.
  const delivered = async (): Promise<void> => {
    await Promise.all(
      context.pages().map(async (page) => {
        await page
          .evaluate(() => new Promise((resolve) => setTimeout(resolve, 0)))
          .catch(() => undefined);
      }),
    );
  };
  const watch: PageErrorWatch = {
    errors,
    tolerated,
    tolerating: async (issue, during) => {
      if (active !== undefined) {
        throw new TestSupportError(
          "known_issue_nested",
          `the known issue ${issue.id} is tolerated while ${active.id} is`,
        );
      }
      issues.set(issue.id, issue);
      const start = tolerated.length;
      active = issue;
      try {
        return await during();
      } finally {
        await delivered();
        active = undefined;
        const violations = tolerated
          .slice(start)
          .filter(({ kind }) => kind === "csp_violation").length;
        returnUnpaired("console", violations, tolerated, errors, start);
        const reports = returnUnpaired("csp_report", violations, tolerated, errors, start);
        reportsDue.set(issue.id, (reportsDue.get(issue.id) ?? 0) + violations - reports);
      }
    },
    take: async () => {
      await delivered();
      return errors.splice(0);
    },
    check: async () => {
      await delivered();
      if (errors.length > 0) {
        const lines = errors.map((error) => `- ${error.kind} at ${error.url}: ${error.text}`);
        throw new TestSupportError(
          "page_errors",
          `the test's pages went wrong:\n${lines.join("\n")}`,
        );
      }
    },
  };
  watches.set(context, watch);
  return watch;
}

/**
 * Moves back to `errors` each error of `kind` tolerated since `start` beyond
 * `violations`, one per CSP violation tolerated with it, and answers how
 * many it kept: a console message or a report, which may name no page or
 * file, is tolerated only beside the violation it reports.
 */
function returnUnpaired(
  kind: "console" | "csp_report",
  violations: number,
  tolerated: ToleratedError[],
  errors: PageError[],
  start: number,
): number {
  const count = tolerated.slice(start).filter((error) => error.kind === kind).length;
  let spare = count - violations;
  for (let index = tolerated.length - 1; index >= start && spare > 0; index -= 1) {
    const error = tolerated[index];
    if (error?.kind === kind) {
      tolerated.splice(index, 1);
      errors.push({ kind: error.kind, text: error.text, url: error.url });
      spare -= 1;
    }
  }
  return Math.min(count, violations);
}

/** Whether `request` went to the origin of the page or worker that made it. */
function isSameOrigin(request: Request): boolean {
  // A navigation's own URL is the origin its page takes.
  if (request.isNavigationRequest()) {
    return true;
  }
  const worker = request.serviceWorker();
  const from = worker === null ? request.frame().url() : worker.url();
  return URL.parse(request.url())?.origin === URL.parse(from)?.origin;
}
