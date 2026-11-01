// A run's Usage, the part of Usage and Limits a run's page shows (spec
// #591, story 99; ADRs 0041, 0046 and 0105): what the run used of each
// Usage kind, the responses its Response recording answered counted apart
// from the requests it fetched, and the Limits that apply to it with what
// is left. Amounts are asserted in their machine form.
import type { RecordingReport, RunUsage } from "@goodfetch/collie-client/generated";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { ConsoleI18n } from "#i18n.tsx";
import { RunUsageView } from "./run-usage.tsx";

/** What a run used, nothing unless overridden. */
function usage(overrides: Partial<RunUsage> = {}): RunUsage {
  return {
    model: { calls: 0, tokens: 0 },
    browser_time: 0,
    proxy_bytes: 0,
    requests: 0,
    ...overrides,
  };
}

/** A replay of `tests/login.warc` that used `responses` recorded responses. */
function replayed(responses: number, mode = "replay"): RecordingReport {
  return { mode, file: "tests/login.warc", responses_used: responses };
}

/** The machine value of what the run used, in the row named `name`. */
async function used(name: string): Promise<string | null> {
  const row = page
    .getByRole("table", { name: "What this run used" })
    .getByRole("row")
    .filter({ has: page.getByRole("rowheader", { name, exact: true }) });
  await expect.element(row).toBeVisible();
  return row.getByRole("cell").element().querySelector("data")?.getAttribute("value") ?? null;
}

test(
  "a run's replayed responses are counted apart from the requests it fetched",
  { tags: ["story:12-99"] },
  async () => {
    await render(
      <ConsoleI18n>
        <RunUsageView usage={usage({ requests: 12 })} recording={replayed(30)} />
      </ConsoleI18n>,
    );

    expect(await used("Requests fetched")).toBe("12");
    expect(await used("Responses replayed")).toBe("30");
  },
);

test("a recording made while fetching replays nothing", async () => {
  await render(
    <ConsoleI18n>
      <RunUsageView usage={usage({ requests: 7 })} recording={replayed(7, "record")} />
    </ConsoleI18n>,
  );

  expect(await used("Requests fetched")).toBe("7");
  expect(await used("Responses replayed")).toBe("0");
});

test("a run's page lists the Limits that apply to it with what is left", async () => {
  await render(
    <ConsoleI18n>
      <RunUsageView
        usage={usage({
          requests: 40,
          model: { calls: 3, tokens: 1_200 },
          paid_solves: [{ currency: "EUR", amount: 20_000 }],
          limits: [
            {
              scope: { kind: "install", id: "" },
              usage_kind: "requests",
              amount: 1_000,
              left: 960,
              set_by: "person",
            },
          ],
        })}
        recording={undefined}
      />
    </ConsoleI18n>,
  );

  expect(await used("Model calls")).toBe("3");
  expect(await used("Model tokens")).toBe("1200");
  expect(await used("Paid solves in EUR")).toBe("20000");
  const limit = page
    .getByRole("table", { name: "Limits that apply to this run" })
    .getByRole("row")
    .filter({ hasText: "requests" });
  await expect.element(limit).toBeVisible();
  expect(
    limit
      .getByRole("cell")
      .elements()
      .map((cell) => cell.querySelector("data")?.getAttribute("value") ?? cell.textContent),
  ).toEqual(["requests", "No period", "1000", "960", "A person"]);
});
