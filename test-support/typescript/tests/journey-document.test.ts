// The `journey` result documents the helper writes (ADR 0063): each matches
// the bundle's schema and `goodfetch-protocol`'s reader accepts it, through
// `goodfetch-conformance read` from the test tool manifest. A sample
// Journey-shaped test drives the Proving ground's `catalog/2` task and its
// oracle, and writes one `proved` and one `failed` document.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { Ajv2020 } from "ajv/dist/2020.js";
import { describe, expect } from "vitest";
import {
  journeyVectors,
  test,
  toolPath,
  writeJourneyDocument,
  type JourneyFacts,
  type ProcessRecord,
  type TestEnvDescription,
} from "@goodfetch/test-support";
import { exchange, product, type TaskList } from "./catalog-product.ts";

const root = new URL("../../../", import.meta.url);

const bundle: { $defs: object } = JSON.parse(
  readFileSync(new URL("crates/goodfetch-protocol/generated/bundle.json", root), "utf8"),
);
const ajv = new Ajv2020({ allErrors: true });
// schemars writes Rust's `u64` as this format.
ajv.addFormat("uint64", {
  type: "number",
  validate: (value: number) => Number.isSafeInteger(value) && value >= 0,
});
const isResultDocument = ajv.compile({ $ref: "#/$defs/ResultDocument", $defs: bundle.$defs });

const variant = {
  journey_id: "J-002",
  surface: "sdk-typescript",
  executable: { product: "collie", version: "0.0.0" },
  platform: { os: "linux", arch: "x86_64" },
  runtime: "node-24",
} as const;

const vectors = journeyVectors(variant);

/** Runs `goodfetch-conformance read <file>` and gives its exit code. */
async function read(
  processes: ProcessRecord,
  testEnv: TestEnvDescription,
  file: string,
): Promise<number | null> {
  const reader = processes.spawn(toolPath(testEnv.tools, "conformance"), ["read", file], {
    env: testEnv.env,
  });
  reader.child.stdin.end();
  return (await reader.exit).code;
}

/** What the bundle's schema finds wrong in the document at `file`; nothing when it matches. */
function schemaErrors(file: string): unknown[] {
  isResultDocument(written(file));
  return isResultDocument.errors ?? [];
}

/** The document `writeJourneyDocument` wrote, parsed. */
function written(file: string): unknown {
  return JSON.parse(readFileSync(file, "utf8"));
}

describe("writeJourneyDocument", () => {
  test.for(vectors)(
    "writes the vector $name as a document the reader accepts",
    async ({ facts }, { processes, testEnv }) => {
      const file = join(testEnv.folder, "journey.json");
      writeJourneyDocument(file, facts);

      const code = await read(processes, testEnv, file);

      expect(schemaErrors(file)).toEqual([]);
      expect(code).toBe(0);
    },
  );
});

/** A running local copy of the Proving ground, started on the test's seed. */
interface ProvingGround {
  readonly address: string;
  stop(): Promise<void>;
}

async function startProvingGround(
  processes: ProcessRecord,
  testEnv: TestEnvDescription,
): Promise<ProvingGround> {
  const serve = processes.spawn(
    toolPath(testEnv.tools, "proving_ground"),
    [
      "serve",
      "--seed",
      String(testEnv.seed),
      "--copy",
      "local",
      "--address",
      "127.0.0.1:0",
      "--network-profile",
      "none",
    ],
    { env: testEnv.env, cwd: testEnv.folder },
  );
  const lines = createInterface({ input: serve.child.stdout });
  const line = await new Promise<string>((resolve) => lines.once("line", resolve));
  lines.close();
  serve.child.stdout.resume();
  serve.child.stderr.resume();
  const reported: { address: string } = JSON.parse(line);
  return {
    address: reported.address,
    stop: async () => {
      serve.child.kill("SIGKILL");
      await serve.exit;
    },
  };
}

/**
 * A Journey-shaped run of `catalog/2`: read the product page, submit its
 * Record to the oracle (with a wrong price when `wrongPrice`), and order the
 * product only when the oracle verified it. Each step's outcome is recorded
 * as it happens.
 */
async function extractTheProduct(
  provingGround: ProvingGround,
  wrongPrice: boolean,
): Promise<JourneyFacts> {
  const list: TaskList = JSON.parse(await exchange(provingGround, "GET", "tasks", "/tasks"));
  const task = list.tasks.find(({ id }) => id === "catalog/2");
  if (task === undefined) {
    throw new Error("the task list has no catalog/2");
  }
  const url = new URL(task.start_url);
  const record = product(await exchange(provingGround, "GET", url.host, url.pathname));
  const submitted = wrongPrice ? { ...record, price: "0.01" } : record;
  const answer: { task: string; outcome: "verified" | "incomplete" | "false_success" | "failed" } =
    JSON.parse(
      await exchange(provingGround, "POST", "oracle", "/oracle", {
        task: task.id,
        status: "success",
        records: [submitted],
      }),
    );
  const verified = answer.outcome === "verified";
  return {
    ...variant,
    proving_ground: list.proving_ground,
    steps: [
      { text: "Read the product page", outcome: "passed" },
      { text: "Submit the product as the task's Record", outcome: "passed" },
      { text: "Order the product at its price", outcome: verified ? "passed" : "not_reached" },
    ],
    checks: [
      {
        text: "The oracle verifies the Record",
        task: task.id,
        outcome: verified ? "passed" : "failed",
      },
    ],
    tasks: [{ id: task.id, answer }],
  };
}

describe("a Journey-shaped test", () => {
  test("writes a proved document when the oracle verifies its Record", async ({
    processes,
    testEnv,
  }) => {
    const provingGround = await startProvingGround(processes, testEnv);
    const facts = await extractTheProduct(provingGround, false);
    await provingGround.stop();
    const file = join(testEnv.folder, "journey.json");

    const outcome = writeJourneyDocument(file, facts);

    expect(outcome).toBe("proved");
    expect(schemaErrors(file)).toEqual([]);
    expect(await read(processes, testEnv, file)).toBe(0);
  });

  test("writes a failed document when a step is not reached and the oracle answers false_success", async ({
    processes,
    testEnv,
  }) => {
    const provingGround = await startProvingGround(processes, testEnv);
    const facts = await extractTheProduct(provingGround, true);
    await provingGround.stop();
    const file = join(testEnv.folder, "journey.json");

    const outcome = writeJourneyDocument(file, facts);

    expect(outcome).toBe("failed");
    expect(written(file)).toMatchObject({
      outcome: "failed",
      steps: [{ outcome: "passed" }, { outcome: "passed" }, { outcome: "not_reached" }],
      tasks: [{ id: "catalog/2", answer: { outcome: "false_success" } }],
    });
    expect(await read(processes, testEnv, file)).toBe(0);
  });
});
