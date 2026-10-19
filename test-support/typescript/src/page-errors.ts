// What a served test's pages do wrong (ADR 0068): a `console` error, an
// uncaught page error, a CSP violation, seen both by the page's
// `securitypolicyviolation` event and as the report the browser sends to the
// CSP report endpoint, and a same-origin request that failed. The
// `failOnErrors` fixture fails a test that recorded any.
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

/** The errors of every page of one context. */
export interface PageErrorWatch {
  /** Every error recorded so far, in the order they came. */
  readonly errors: readonly PageError[];
  /** Fails with `page_errors` when any error was recorded, once each open page has delivered those it raised. */
  check(): Promise<void>;
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

/** Records the errors of every page `context` has or opens from now on. */
export async function watchPageErrors(context: BrowserContext): Promise<PageErrorWatch> {
  const errors: PageError[] = [];
  const record = (kind: PageErrorKind, text: string, url: string): void => {
    errors.push({ kind, text, url });
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
  return {
    errors,
    check: async () => {
      // A violation reaches the watch through the page's own event loop:
      // one round trip to each open page lets those already raised arrive.
      await Promise.all(
        context.pages().map(async (page) => {
          await page.evaluate(() => 0).catch(() => undefined);
        }),
      );
      if (errors.length > 0) {
        const lines = errors.map((error) => `- ${error.kind} at ${error.url}: ${error.text}`);
        throw new TestSupportError(
          "page_errors",
          `the test's pages went wrong:\n${lines.join("\n")}`,
        );
      }
    },
  };
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
