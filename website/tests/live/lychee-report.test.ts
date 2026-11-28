import { describe, expect, it } from "vitest";
import { sortFailures } from "./lychee-report.ts";

/** A lychee JSON report with `errors` and `timeouts`, each a page's list. */
function report(errors: Record<string, unknown[]>, timeouts: Record<string, unknown[]> = {}) {
  return JSON.stringify({ total: 3, error_map: errors, timeout_map: timeouts });
}

function failure(url: string, text: string, code?: number) {
  return { url, status: code === undefined ? { text } : { text, code } };
}

describe("sortFailures", () => {
  it("counts a link the other side answers as missing as broken", () => {
    const sorted = sortFailures(
      report({
        "docs/collie.html": [
          failure("https://example.com/gone", "Rejected status code: 404 Not Found", 404),
          failure("https://example.com/removed", "Rejected status code: 410 Gone", 410),
        ],
      }),
    );

    expect(sorted).toEqual({
      broken: [
        "https://example.com/gone in docs/collie.html: Rejected status code: 404 Not Found",
        "https://example.com/removed in docs/collie.html: Rejected status code: 410 Gone",
      ],
      unavailable: [],
    });
  });

  it("counts an unanswered or unavailable other side as unavailable, never as broken", () => {
    const sorted = sortFailures(
      report(
        {
          "docs/index.html": [
            failure("https://a.example/", "Rejected status code: 503 Service Unavailable", 503),
            failure("https://b.example/", "Rejected status code: 429 Too Many Requests", 429),
            failure("https://c.example/", "Rejected status code: 408 Request Timeout", 408),
            failure("https://d.example/", "Network error: connection refused"),
          ],
        },
        { "docs/index.html": [failure("https://e.example/", "Timeout")] },
      ),
    );

    expect(sorted.broken).toEqual([]);
    expect(sorted.unavailable).toEqual([
      "https://a.example/ in docs/index.html: Rejected status code: 503 Service Unavailable",
      "https://b.example/ in docs/index.html: Rejected status code: 429 Too Many Requests",
      "https://c.example/ in docs/index.html: Rejected status code: 408 Request Timeout",
      "https://d.example/ in docs/index.html: Network error: connection refused",
      "https://e.example/ in docs/index.html: Timeout",
    ]);
  });

  it("refuses a report that is not lychee's JSON", () => {
    expect(() => sortFailures(JSON.stringify({ error_map: { "a.html": [{ url: 1 }] } }))).toThrow(
      /not lychee's JSON report/,
    );
  });
});
