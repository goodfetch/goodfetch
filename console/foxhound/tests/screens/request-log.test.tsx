// Foxhound's request log of a crawl run (spec #593, story 219; ADRs 0071,
// 0079, 0104 and 0109), against the in-memory adapter of Foxhound's
// engine: it follows `run.outcomes` page by page until the sequence ends,
// one read per page and none per outcome, counted here; its filters run in
// the engine, one read per Apply; each request shows its attempts and the
// error of each failed attempt, with the program's exception (type,
// message and stack) of a TypeScript, a Python and a Rust handler, and a
// failed Hook's stage, Hook, Label and wrapped code; and a heavy log
// renders only the rows on screen.
import { createConsoleClient, type ConsoleClient } from "@goodfetch/console/client";
import type { RunView } from "@goodfetch/console/console-build";
import { ConsoleI18n } from "@goodfetch/console/i18n";
import type {
  AttemptError,
  CrawlRequestOutcome,
  CrawlRequestOutcomes,
  RunOutcomesParams,
} from "@goodfetch/foxhound/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";
import { FOXHOUND_RUN_VIEWS } from "#runs/run-views.tsx";

const RUN_ID = "crawl-run";
const ENDED_AT = "2026-10-26T09:00:00Z";

function logView(): RunView {
  const view = FOXHOUND_RUN_VIEWS.find((candidate) => candidate.id === "outcomes");
  if (view === undefined) {
    throw new Error("Foxhound's build has no request log");
  }
  return view;
}

function clientOf(adapter: ProtocolAdapter<FoxhoundConsoleOperations>): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: { visible: true, watch: () => () => {} },
    product: "foxhound",
  });
}

/** A request's outcome at `position`, of the Label `label`. */
function succeeded(position: number, label = "page"): CrawlRequestOutcome {
  return {
    position,
    request: {
      id: String(position),
      url: `https://shop.test/${String(position)}`,
      label,
      depth: 0,
    },
    attempts: 1,
    errors: [],
    ended_at: ENDED_AT,
    kind: "succeeded",
    records: [],
  };
}

/** A request at `position` that failed on each of `errors`, the last one its ending. */
function failed(position: number, errors: readonly AttemptError[]): CrawlRequestOutcome {
  const last = errors.at(-1);
  if (last === undefined) {
    throw new Error("a failed request has an error");
  }
  return {
    ...succeeded(position, "list"),
    attempts: errors.length,
    errors: [...errors],
    kind: "failed",
    error: last,
  };
}

/** The params of each `run.outcomes` the view sent. */
function outcomeReads(adapter: ProtocolAdapter<FoxhoundConsoleOperations>): unknown[] {
  return adapter.requests
    .filter((request) => request.method === "run.outcomes")
    .map((request) => request.params);
}

/** The URL of each row of the log. */
function loggedUrls(): string[] {
  return page
    .getByRole("table", { name: "Request log" })
    .getByRole("rowheader")
    .elements()
    .map((cell) => cell.textContent);
}

async function show(client: ConsoleClient): Promise<void> {
  await render(<ConsoleI18n>{logView().content({ client, runId: RUN_ID })}</ConsoleI18n>);
}

test(
  "the log follows the crawl's outcomes page by page, one read per page, until the sequence ends",
  { tags: ["story:12-219"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    // The engine answers a page at once while it holds outcomes, then the
    // last one once the crawl ended: two reads for three outcomes.
    adapter.answer("run.outcomes", (params: RunOutcomesParams): CrawlRequestOutcomes => {
      if (params.after === undefined || params.after === null) {
        return { outcomes: [succeeded(1), succeeded(2)], next: 2, ended: false };
      }
      return { outcomes: [succeeded(3)], next: 3, ended: true };
    });
    const client = clientOf(adapter);

    await show(client);

    await expect.element(page.getByText("3 outcomes The crawl ended.")).toBeVisible();
    expect(loggedUrls()).toEqual([
      "https://shop.test/1",
      "https://shop.test/2",
      "https://shop.test/3",
    ]);
    expect(outcomeReads(adapter)).toEqual([{ run_id: RUN_ID }, { run_id: RUN_ID, after: 2 }]);
    client.close();
  },
);

test(
  "a later outcome of a re-admitted request supersedes its earlier one",
  { tags: ["story:12-219"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    const retried = { ...succeeded(3), request: succeeded(1).request, attempts: 2 };
    adapter.answer("run.outcomes", {
      outcomes: [succeeded(1), succeeded(2), retried],
      next: 3,
      ended: true,
    });
    const client = clientOf(adapter);

    await show(client);

    await expect.element(page.getByText("2 outcomes The crawl ended.")).toBeVisible();
    expect(loggedUrls()).toEqual(["https://shop.test/2", "https://shop.test/1"]);
    client.close();
  },
);

test(
  "the outcome, Label and Error code filters run in the engine: one read per Apply, shown as answered",
  { tags: ["story:12-219"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    // The adapter answers every read alike: the view keeps what the engine
    // answers and filters nothing itself.
    adapter.answer("run.outcomes", {
      outcomes: [succeeded(1), succeeded(2, "list")],
      next: 2,
      ended: true,
    });
    const client = clientOf(adapter);
    await show(client);
    await expect.element(page.getByText("2 outcomes The crawl ended.")).toBeVisible();

    await userEvent.click(page.getByRole("checkbox", { name: "Failed" }));
    await userEvent.click(page.getByRole("checkbox", { name: "Blocked" }));
    await userEvent.fill(
      page.getByRole("textbox", { name: "Labels, separated by commas" }),
      "list",
    );
    await userEvent.fill(
      page.getByRole("textbox", { name: "Error codes, separated by commas" }),
      "handler_failed, hook_failed",
    );
    await userEvent.click(page.getByRole("button", { name: "Apply" }));

    await expect.poll(() => outcomeReads(adapter)).toHaveLength(2);
    expect(outcomeReads(adapter)[1]).toEqual({
      run_id: RUN_ID,
      kinds: ["failed", "blocked"],
      labels: ["list"],
      codes: ["handler_failed", "hook_failed"],
    });
    await expect.element(page.getByText("2 outcomes The crawl ended.")).toBeVisible();
    expect(loggedUrls()).toEqual(["https://shop.test/1", "https://shop.test/2"]);
    client.close();
  },
);

/** The text of each term and its description in the error list of `attempt`. */
function described(attempt: number): [string, string][] {
  const list = page.getByRole("definition").elements()[0]?.closest("table");
  if (list === null || list === undefined) {
    throw new Error("the log shows no error");
  }
  const errors = [...list.querySelectorAll(`dl[aria-label="Attempt ${String(attempt)}"]`)];
  const terms = errors.flatMap((error) => [...error.querySelectorAll("dt")]);
  return terms.map((term) => [term.textContent, term.nextElementSibling?.textContent ?? ""]);
}

const TYPESCRIPT_STACK =
  "TypeError: Cannot read properties of undefined (reading 'price')\n    at handle (/app/crawl.ts:12:18)";
const PYTHON_STACK =
  'Traceback (most recent call last):\n  File "/app/crawl.py", line 9, in handle\n    price = item["price"]\nKeyError: \'price\'';

test(
  "each failed attempt shows its error, a handler's exception its type, message and stack, a Rust one `BoxError` and no stack",
  { tags: ["story:12-219"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    adapter.answer("run.outcomes", {
      outcomes: [
        failed(1, [
          {
            attempt: 1,
            code: "handler_failed",
            message: "the handler threw TypeError: Cannot read properties of undefined",
            details: {
              exception_type: "TypeError",
              exception_message: "Cannot read properties of undefined (reading 'price')",
              exception_stack: TYPESCRIPT_STACK,
            },
          },
          {
            attempt: 2,
            code: "handler_failed",
            message: "the handler raised KeyError: 'price'",
            details: {
              exception_type: "KeyError",
              exception_message: "'price'",
              exception_stack: PYTHON_STACK,
            },
          },
          {
            attempt: 3,
            code: "handler_failed",
            message: "the handler returned BoxError: invalid digit found in string",
            details: {
              exception_type: "BoxError",
              exception_message: "invalid digit found in string",
              exception_stack: "",
            },
          },
        ]),
      ],
      next: 1,
      ended: true,
    });
    const client = clientOf(adapter);

    await show(client);
    await expect.element(page.getByText("1 outcome The crawl ended.")).toBeVisible();

    expect(described(1)).toEqual([
      ["Attempt", "1"],
      ["Error code", "handler_failed"],
      ["Message", "the handler threw TypeError: Cannot read properties of undefined"],
      ["Exception type", "TypeError"],
      ["Exception message", "Cannot read properties of undefined (reading 'price')"],
      ["Stack", TYPESCRIPT_STACK],
    ]);
    expect(described(2).slice(3)).toEqual([
      ["Exception type", "KeyError"],
      ["Exception message", "'price'"],
      ["Stack", PYTHON_STACK],
    ]);
    expect(described(3).slice(3)).toEqual([
      ["Exception type", "BoxError"],
      ["Exception message", "invalid digit found in string"],
      ["Stack", "No stack"],
    ]);
    const row = page.getByRole("row", { name: /shop\.test\/1/u });
    await expect.element(row.getByRole("cell", { name: "3", exact: true })).toBeVisible();
    client.close();
  },
);

test(
  "a failed Hook shows its stage, Hook, Label and wrapped code, and a panic its type",
  { tags: ["story:12-219"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    adapter.answer("run.outcomes", {
      outcomes: [
        failed(1, [
          {
            attempt: 1,
            code: "hook_failed",
            message: "the Hook add_token raised header_forbidden",
            details: {
              stage: "before_request",
              hook: "add_token",
              label: "list",
              wrapped_code: "header_forbidden",
              exception_type: "GoodfetchError",
              exception_message: "the header Cookie is the engine's own",
              exception_stack: "GoodfetchError: the header Cookie is the engine's own",
            },
          },
          {
            attempt: 2,
            code: "hook_failed",
            message: "the Hook on_start panicked",
            details: {
              stage: "on_start",
              hook: "on_start",
              label: null,
              wrapped_code: null,
              exception_type: "panic",
              exception_message: "index out of bounds",
              exception_stack: "",
            },
          },
        ]),
      ],
      next: 1,
      ended: true,
    });
    const client = clientOf(adapter);

    await show(client);
    await expect.element(page.getByText("1 outcome The crawl ended.")).toBeVisible();

    expect(described(1).slice(3, 7)).toEqual([
      ["Stage", "before_request"],
      ["Hook", "add_token"],
      ["Label", "list"],
      ["Wrapped code", "header_forbidden"],
    ]);
    expect(described(2).slice(3)).toEqual([
      ["Stage", "on_start"],
      ["Hook", "on_start"],
      ["Label", "None: a lifecycle stage has no request"],
      ["Wrapped code", "None: the program's own exception"],
      ["Exception type", "panic"],
      ["Exception message", "index out of bounds"],
      ["Stack", "No stack"],
    ]);
    client.close();
  },
);

test(
  "a heavy log of 10,000 outcomes renders only the rows on screen",
  { tags: ["story:12-219"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    const outcomes = Array.from({ length: 10_000 }, (_, index) => succeeded(index + 1));
    adapter.answer("run.outcomes", { outcomes, next: 10_000, ended: true });
    const client = clientOf(adapter);

    await show(client);

    await expect.element(page.getByText("10,000 outcomes The crawl ended.")).toBeVisible();
    const table = page.getByRole("table", { name: "Request log" });
    expect(loggedUrls().length).toBeLessThan(100);
    expect(table.element().getAttribute("aria-rowcount")).toBe("10001");
    client.close();
  },
);
