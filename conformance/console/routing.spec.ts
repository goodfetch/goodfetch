// Which engine a gateway runs each run on (ADR 0096; TESTING.md, The
// Console conformance suite): a run a Client starts gets the Client's
// Product, and a run the Console starts gets Foxhound's when its target
// names anything only Foxhound carries, by the generated list of
// Foxhound's names, Collie's otherwise; a Foxhound-only call on a run of
// Collie's engine answers `not_in_product` naming the run's Product. Which
// engine a run landed on shows in what it answers a Foxhound Connection's
// `run.seeds`: Foxhound's engine answers it, Collie's refuses it naming
// Collie. Only a gateway target runs these cases. The Recipe rows wait for
// a start that carries a Recipe's manifest (#1691).
import { expect, test } from "@goodfetch/test-support/conformance";
import type { ServedDaemon } from "@goodfetch/test-support/playwright";
import { FOXHOUND_OPERATIONS } from "@goodfetch/console/generated/foxhound";
import { asConsole, field, ProgramConnection, type Speaking } from "./target.ts";

/** The capability every case of this file proves. */
const ROUTING = { type: "capability", description: "routing" };
/** The Features every case of this file covers. */
const COVERS = [
  { type: "covers", description: "story:12-231" },
  { type: "covers", description: "story:12-232" },
];

/** A Collie Client program and a Foxhound one. */
const COLLIE_CLIENT: Speaking = { surface: "sdk_typescript", needs: "collie" };
const FOXHOUND_CLIENT: Speaking = { surface: "sdk_typescript", needs: "foxhound" };

/** A crawl with one seed it never fetches. */
const CRAWL = { seeds: [{ url: "ftp://example.invalid/", label: "start" }] };

/** A setting key only Foxhound carries. */
const CRAWL_SETTING = { crawl: { max_requests: 10 } };

/** The Connections a case opened, closed after it. */
const opened: ProgramConnection[] = [];

test.beforeEach(({ daemon }) => {
  test.skip(daemon.gateway === undefined, "only a gateway routes a run to an engine");
});

test.afterEach(() => {
  for (const connection of opened.splice(0)) {
    connection.close();
  }
});

/** A Connection of the test's own to the gateway, as `speaking`. */
async function as(daemon: ServedDaemon, speaking: Speaking): Promise<ProgramConnection> {
  const connection = await ProgramConnection.open(daemon, speaking);
  opened.push(connection);
  return connection;
}

/** The `not_in_product` error data of an answer, its details among it, or `undefined` for another one. */
function refusal(answer: unknown): unknown {
  const data = field(field(answer, "error"), "data");
  return field(data, "code") === "not_in_product" ? data : undefined;
}

/** The Product of the engine `runId` landed on, from its answer to a Foxhound Connection's `run.seeds`. */
async function landedOn(daemon: ServedDaemon, runId: string): Promise<string> {
  const foxhound = await as(daemon, FOXHOUND_CLIENT);
  const answer = await foxhound.call("run.seeds", { run_id: runId });
  if (field(answer, "result") !== undefined) {
    return "foxhound";
  }
  const refused = refusal(answer);
  expect(refused, JSON.stringify(answer)).toMatchObject({ product: "collie", needs: "foxhound" });
  return "collie";
}

test(
  "a-collie-clients-run-lands-on-collies-engine-and-its-crawl-start-answers-not-in-product",
  { annotation: [ROUTING, ...COVERS] },
  async ({ daemon }) => {
    const collie = await as(daemon, COLLIE_CLIENT);

    const runId = await collie.start("run.start", {});
    const crawl = await collie.call("crawl.start", CRAWL);

    expect(await landedOn(daemon, runId)).toBe("collie");
    expect(refusal(crawl), JSON.stringify(crawl)).toMatchObject({
      product: "collie",
      needs: "foxhound",
      requested: "crawl.start",
    });
  },
);

test(
  "a-foxhound-clients-run-lands-on-foxhounds-engine",
  { annotation: [ROUTING, ...COVERS] },
  async ({ daemon }) => {
    const foxhound = await as(daemon, FOXHOUND_CLIENT);

    const runId = await foxhound.start("run.start", {});

    expect(await landedOn(daemon, runId)).toBe("foxhound");
  },
);

test(
  "a-run-the-console-starts-lands-by-what-its-target-names",
  { annotation: [ROUTING, ...COVERS] },
  async ({ daemon }) => {
    const console = await as(daemon, asConsole(daemon));
    // The start's operation is one of the generated list's names.
    expect(FOXHOUND_OPERATIONS).toContain("crawl.start");
    expect(FOXHOUND_OPERATIONS).not.toContain("run.start");

    const plain = await console.start("run.start", {});
    const crawl = await console.start("crawl.start", CRAWL);
    const keyed = await console.call("run.start", { settings: CRAWL_SETTING });

    expect(await landedOn(daemon, plain)).toBe("collie");
    expect(await landedOn(daemon, crawl)).toBe("foxhound");
    // Foxhound's engine carries the key and refuses it on a run that is no
    // crawl; Collie's would answer `not_in_product`.
    expect(field(field(field(keyed, "error"), "data"), "code"), JSON.stringify(keyed)).toBe(
      "setting_not_applicable",
    );
  },
);

test(
  "a-foxhound-only-call-on-a-run-of-collies-engine-answers-not-in-product-naming-its-product",
  { annotation: [ROUTING, ...COVERS] },
  async ({ daemon }) => {
    const console = await as(daemon, asConsole(daemon));
    const runId = await console.start("run.start", {});

    const answer = await console.call("run.seeds", { run_id: runId });

    const refused = refusal(answer);
    expect(refused, JSON.stringify(answer)).toMatchObject({
      product: "collie",
      needs: "foxhound",
    });
    // Behind the gateway, the fix names no install command.
    expect(String(field(refused, "fix"))).not.toMatch(/install/i);
  },
);
