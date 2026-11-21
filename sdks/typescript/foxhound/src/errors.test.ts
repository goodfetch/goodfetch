// `response_unavailable`'s `retry_after`, a point in time (ADRs 0092 and
// 0118): the time the site's `Retry-After` asked for, read as a `Date` as
// Collie's `BlockedError` reads its own. Python's
// `test_response_unavailable.py` is its twin.
import { describe, expect, test } from "vitest";
import { ResponseUnavailableError } from "./generated/errors.ts";

describe("ResponseUnavailableError", () => {
  test("reads retry_after as a point in time, null for a request held unsent", () => {
    const asked = new ResponseUnavailableError("unavailable", {
      details: { url: "https://a.test/", status: 503, retry_after: "2026-11-21T10:00:00+02:00" },
    });
    const held = new ResponseUnavailableError("unavailable", {
      details: { url: "https://a.test/", status: null },
    });

    expect([asked.retryAfter, held.retryAfter]).toEqual([new Date("2026-11-21T08:00:00Z"), null]);
  });
});
