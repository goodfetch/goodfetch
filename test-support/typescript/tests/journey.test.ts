// Seams B and C (spec #326): the Playwright Journey helper and the reporter
// of `journey` documents, used as a `console` variant uses them. Each case
// runs Playwright on the repository's own `playwright.config.ts`, copied
// unchanged into a folder of its own with the Feature inventory it reads,
// on a spec file running one variant of a fixture Journey outside the
// catalog (`tests/fixtures/journeys/`): J-961, a Collie Journey that starts
// no Proving ground, and J-962, a Foxhound Journey on seed 962 naming
// `catalog/2`. The Console is the stand-in of `console-stand-in.ts`, whose
// home page names the Product that answered a `hello` with the Daemon's
// token. The reporter writes the variant's document into a folder of the
// case's own, which `goodfetch-conformance read --journey` loads as every
// reader does: through `goodfetch-protocol`'s reader, then compared with
// its Journey.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect } from "vitest";
import {
  test,
  toolPath,
  type JourneyResultDocument,
  type ProcessRecord,
  type TestEnvDescription,
} from "@goodfetch/test-support";
import { runRepositoryPlaywright } from "./repository-playwright.ts";

const JOURNEYS = join(import.meta.dirname, "fixtures", "journeys");

/** An `import` of a module of this package, by its absolute URL. */
function module(...path: string[]): string {
  return JSON.stringify(pathToFileURL(join(import.meta.dirname, ...path)).href);
}

/** The spec file of a case: one variant of `journey`, whose body is `body`. */
function spec(journey: string, body: string, options = ""): string {
  return `import { journey } from ${module("..", "src", "journey.ts")};
import { expect, test } from ${module("..", "src", "playwright.ts")};
import { standInConsole } from ${module("console-stand-in.ts")};
import { catalogRecord } from ${module("catalog-product.ts")};

test.use({ consoleServer: standInConsole() });

journey(${JSON.stringify(journey)}, "console", async ({ step, check, expectTask, cli, daemon, page }) => {
${body}
}, { file: ${JSON.stringify(join(JOURNEYS, `${journey}.toml`))}${options} });
`;
}

/** J-961's steps and check, run as a person would, with `skip` left out. */
function j961(skip?: string): string {
  const steps = {
    version: `  await step("Ask the executable for its version", async () => {
    const version = await cli(["--version"]);
    expect(version.code).toBe(0);
  });`,
    open: `  await step("Open the local Console's address", () => page.goto(daemon.url));`,
  };
  return `${Object.entries(steps)
    .filter(([name]) => name !== skip)
    .map(([, text]) => text)
    .join("\n")}
  await check("The Console names the Daemon's Product", () =>
    expect(page.getByRole("heading", { level: 1 })).toHaveText(/'s Daemon$/),
  );`;
}

/** J-962's steps and checks, submitting the product read, its price changed when `price` is given. */
function j962(price?: string): string {
  return `  let record;
  await step("Open the local Console's address", () => page.goto(daemon.url));
  await step("Read the product page of the task", async () => {
    record = await catalogRecord(daemon.provingGround, "catalog/2");
  });
  await check("The Console names the Daemon's Product", () =>
    expect(page.getByRole("heading", { level: 1 })).toHaveText("Foxhound's Daemon"),
  );
  await expectTask("The oracle verifies the product's Record", {
    status: "success",
    records: [${price === undefined ? "record" : `{ ...record, price: ${JSON.stringify(price)} }`}],
  });`;
}

/** What one Playwright run of a case gave. */
interface Ran {
  /** Playwright's exit code. */
  readonly code: number | null;
  /** The document the reporter wrote for the variant. */
  readonly document: JourneyResultDocument;
  /** `goodfetch-conformance read --journey`'s exit code on it. */
  readonly read: number | null;
  /** What the reader wrote on stderr. */
  readonly refusal: string;
}

/**
 * A configuration that is the repository's with every project setting
 * `provingGroundSeed: 7`, as a Console conformance project does.
 */
const SEEDED_CONFIG = `import config from "./playwright.config.ts";

export default {
  ...config,
  projects: (config.projects ?? []).map((project) => ({
    ...project,
    use: { ...project.use, provingGroundSeed: 7 },
  })),
};
`;

/** Runs `spec` under the repository's configuration, or `config`'s text built on it, and reads the document of `journey`'s `console` variant. */
async function run(
  processes: ProcessRecord,
  testEnv: TestEnvDescription,
  journey: string,
  specText: string,
  env: Readonly<Record<string, string>> = {},
  config?: string,
): Promise<Ran> {
  const { documents, code } = await runRepositoryPlaywright(processes, {
    folder: "journey-test",
    spec: { name: "journey.spec.ts", text: specText },
    env,
    ...(config === undefined ? {} : { config }),
    workers: 1,
  });
  const file = join(documents, `${journey}@console.json`);
  const reader = processes.spawn(
    toolPath(testEnv.tools, "conformance"),
    ["read", file, "--journey", join(JOURNEYS, `${journey}.toml`)],
    { env: testEnv.env },
  );
  reader.child.stdin.end();
  reader.child.stdout.resume();
  let refusal = "";
  reader.child.stderr.setEncoding("utf8");
  reader.child.stderr.on("data", (chunk: string) => {
    refusal += chunk;
  });
  const read = (await reader.exit).code;
  return { code, document: JSON.parse(readFileSync(file, "utf8")), read, refusal };
}

describe("a console variant", () => {
  test("that runs every step and passes every check is proved and accepted", async ({
    processes,
    testEnv,
  }) => {
    const ran = await run(processes, testEnv, "J-962", spec("J-962", j962()));

    expect(ran.document.outcome).toBe("proved");
    expect(ran.read).toBe(0);
    expect(ran.code).toBe(0);
  });

  test("that skips a step writes a document the reader refuses", async ({ processes, testEnv }) => {
    const ran = await run(processes, testEnv, "J-961", spec("J-961", j961("version")));

    expect(ran.read).toBe(1);
    expect(ran.refusal).toContain("step");
    expect(ran.code).toBe(1);
  });

  test("whose page logs a console error after every step and check passed is failed", async ({
    processes,
    testEnv,
  }) => {
    const ran = await run(
      processes,
      testEnv,
      "J-961",
      spec(
        "J-961",
        `${j961()}
  await page.evaluate(() => console.error("an error once the checks passed"));`,
      ),
    );

    expect(ran.code).toBe(1);
    expect(ran.document.outcome).toBe("failed");
    expect(ran.document.test_errors).toEqual([expect.stringContaining("page_errors")]);
    expect(ran.read).toBe(0);
  });

  test("whose task the oracle answers false_success is failed", async ({ processes, testEnv }) => {
    const ran = await run(processes, testEnv, "J-962", spec("J-962", j962("0.01")));

    expect(ran.document.tasks).toEqual([
      { id: "catalog/2", answer: expect.objectContaining({ outcome: "false_success" }) },
    ]);
    expect(ran.document.outcome).toBe("failed");
    expect(ran.read).toBe(0);
  });

  test("of a Collie Journey runs on Collie's executable", async ({ processes, testEnv }) => {
    const ran = await run(processes, testEnv, "J-961", spec("J-961", j961()));

    expect(ran.document.executable.product).toBe("collie");
    expect(ran.document.outcome).toBe("proved");
  });

  test("of a Foxhound Journey runs on Foxhound's executable", async ({ processes, testEnv }) => {
    const ran = await run(processes, testEnv, "J-962", spec("J-962", j962()));

    expect(ran.document.executable.product).toBe("foxhound");
  });

  test("of a Journey with no Proving ground table starts none", async ({ processes, testEnv }) => {
    const ran = await run(
      processes,
      testEnv,
      "J-961",
      spec(
        "J-961",
        `${j961()}
  expect(daemon.provingGround).toBeUndefined();`,
      ),
    );

    expect(ran.document.proving_ground).toBeUndefined();
    expect(ran.code).toBe(0);
  });

  test("of a Journey with no Proving ground table starts none where the project sets a seed", async ({
    processes,
    testEnv,
  }) => {
    const ran = await run(
      processes,
      testEnv,
      "J-961",
      spec(
        "J-961",
        `${j961()}
  expect(daemon.provingGround).toBeUndefined();`,
      ),
      {},
      SEEDED_CONFIG,
    );

    expect(ran.document.proving_ground).toBeUndefined();
    expect(ran.code).toBe(0);
  });

  test("starts its Journey's Proving ground on the Journey's seed, whatever the test seed", async ({
    processes,
    testEnv,
  }) => {
    const ran = await run(processes, testEnv, "J-962", spec("J-962", j962()), {
      GOODFETCH_TEST_SEED: "4242",
    });

    expect(ran.document.proving_ground?.seed).toBe(962);
  });

  test("runs on the engine executable it is given", async ({ processes, testEnv }) => {
    const foxhound = toolPath(testEnv.tools, "foxhound");

    const ran = await run(
      processes,
      testEnv,
      "J-961",
      spec("J-961", j961(), `, engine: ${JSON.stringify(foxhound)}`),
    );

    expect(ran.document.executable.product).toBe("foxhound");
  });
});
