// The Runs screen's filter lives in the page's address (ADRs 0036 and
// 0117): every field `run.list` takes reads back from the query it writes,
// so a shared link opens the same view, and a value the engine would refuse
// is left out rather than sent.
import { expect, test } from "vitest";
import type { RunListParams } from "@goodfetch/collie-client/generated";
import { filterOfSearch, searchOfFilter } from "./filter.ts";

test("every field of the filter is written to the address and read back the same", () => {
  const filter: RunListParams = {
    tags: ["team:shop", "night run"],
    recipe: "shop-prices",
    schedule: "nightly",
    occurrence: "nightly.20261002t080000z",
    trigger: "time",
    product: "foxhound",
    lifecycle: "failed",
    verdict: "needs_review",
    completeness: "incomplete",
    started: { from: "2026-10-02T08:00:00.000Z", until: "2026-10-04T08:00:00.000Z" },
    ended: { from: "2026-10-03T10:00:00.000Z" },
    text: "shop & co",
    watch_runs: true,
    sort: "ended_at",
    order: "ascending",
  };

  const search = searchOfFilter(filter);

  expect(search).toBe(
    "?tag=team%3Ashop&tag=night+run&recipe=shop-prices&schedule=nightly&occurrence=nightly.20261002t080000z&trigger=time&product=foxhound&lifecycle=failed&verdict=needs_review&completeness=incomplete&started_from=2026-10-02T08%3A00%3A00.000Z&started_until=2026-10-04T08%3A00%3A00.000Z&ended_from=2026-10-03T10%3A00%3A00.000Z&text=shop+%26+co&watch_runs=true&sort=ended_at&order=ascending",
  );
  expect(filterOfSearch(search)).toEqual(filter);
});

test("an empty filter is an empty address, and an empty address no filter", () => {
  expect(searchOfFilter({})).toBe("");
  expect(filterOfSearch("")).toEqual({});
});

test("a value the engine does not take is left out of the filter", () => {
  expect(
    filterOfSearch(
      "?lifecycle=sleeping&verdict=maybe&trigger=cron&product=other&sort=size&order=up&started_from=yesterday&watch_runs=yes&text=",
    ),
  ).toEqual({});
});
