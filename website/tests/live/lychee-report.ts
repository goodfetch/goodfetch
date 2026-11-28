// lychee's JSON report of an online run, sorted for the Live check
// `docs-external-links` (ADR 0064): a link the other side answers as missing
// is broken, a fail; one it did not answer, answered as busy or as failing
// on its side is unavailable, which makes the run inconclusive, never a pass.

/** A run's failed links, each `<url> in <page>: <lychee's status>`. */
export interface SortedFailures {
  broken: string[];
  unavailable: string[];
}

interface Failure {
  url: string;
  status: { text: string; code?: number };
}

/** Status codes that say the other side is busy or slow, not that the link is wrong. */
const UNAVAILABLE_CODES = new Set([408, 429]);

function isFailure(value: unknown): value is Failure {
  if (typeof value !== "object" || value === null || !("url" in value) || !("status" in value)) {
    return false;
  }
  const { url, status } = value;
  if (typeof url !== "string" || typeof status !== "object" || status === null) {
    return false;
  }
  if (!("text" in status) || typeof status.text !== "string") {
    return false;
  }
  return !("code" in status) || typeof status.code === "number";
}

/** A report map's failures, each with its page; a map lychee left out has none. */
function failuresOf(report: object, map: string): [string, Failure][] {
  const pages: unknown = map in report ? Object.getOwnPropertyDescriptor(report, map)?.value : {};
  if (typeof pages !== "object" || pages === null) {
    throw new Error(`\`${map}\` is not lychee's JSON report's`);
  }
  return Object.entries(pages).flatMap(([page, failures]: [string, unknown]) => {
    if (!Array.isArray(failures) || !failures.every(isFailure)) {
      throw new Error(`\`${map}\` of ${page} is not lychee's JSON report's`);
    }
    return failures.map((failure): [string, Failure] => [page, failure]);
  });
}

function describe([page, failure]: [string, Failure]): string {
  return `${failure.url} in ${page}: ${failure.status.text}`;
}

/** Whether the other side answered that the link is wrong, as a 4xx but 408 and 429. */
function isBroken([, { status }]: [string, Failure]): boolean {
  return (
    status.code !== undefined &&
    status.code >= 400 &&
    status.code < 500 &&
    !UNAVAILABLE_CODES.has(status.code)
  );
}

/** Sorts the failed links of lychee's `--format json` report `json`. */
export function sortFailures(json: string): SortedFailures {
  const report: unknown = JSON.parse(json);
  if (typeof report !== "object" || report === null) {
    throw new Error("lychee printed what is not lychee's JSON report");
  }
  const errors = failuresOf(report, "error_map");
  return {
    broken: errors.filter(isBroken).map(describe),
    unavailable: [
      ...errors.filter((error) => !isBroken(error)),
      ...failuresOf(report, "timeout_map"),
    ].map(describe),
  };
}
