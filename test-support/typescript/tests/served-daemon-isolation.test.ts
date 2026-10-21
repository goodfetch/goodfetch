// Seam B (spec #326): two served tests on test support's Playwright `daemon`
// fixture each get a Daemon, a home and a Console port of their own, and
// nothing of either is left once their teardown ends, so served tests can
// run in any order. Playwright runs the repository's own
// `playwright.config.ts`, copied unchanged into a folder of its own with the
// Feature inventory it reads and a spec file of two tests in one worker.
// Each writes down what its fixture gave it; once both have ended, while the
// worker still runs, a worker fixture counts the distinct homes, sockets
// and ports they were given and looks for what is left of each.
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, onTestFinished } from "vitest";
import { test } from "@goodfetch/test-support";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const FIXTURES = pathToFileURL(join(import.meta.dirname, "..", "src", "playwright.ts")).href;
const STAND_IN = pathToFileURL(join(import.meta.dirname, "console-stand-in.ts")).href;
const CLI = createRequire(import.meta.url).resolve("@playwright/test/cli");

const SPEC = `import { existsSync, writeFileSync } from "node:fs";
import { connect } from "node:net";
import { test } from ${JSON.stringify(FIXTURES)};
import { standInConsole } from ${JSON.stringify(STAND_IN)};

const answers = (port) =>
  new Promise((resolve) => {
    const socket = connect({ host: "127.0.0.1", port });
    socket.on("connect", () => { socket.destroy(); resolve(true); });
    socket.on("error", () => resolve(false));
  });

const served = test.extend({
  given: [
    async ({}, use) => {
      const given = [];
      await use(given);
      const left = [];
      for (const { home, socket, port } of given) {
        left.push({
          homeLeft: existsSync(home),
          socketLeft: process.platform !== "win32" && existsSync(socket),
          portAnswers: await answers(port),
        });
      }
      const distinct = (key) => new Set(given.map((one) => one[key])).size;
      const record = { homes: distinct("home"), sockets: distinct("socket"), ports: distinct("port"), left };
      writeFileSync(process.env.SERVED_DAEMON_RECORD, JSON.stringify(record));
    },
    { scope: "worker" },
  ],
});

served.use({ consoleServer: standInConsole() });

for (const name of ["first", "second"]) {
  served(name, ({ given, daemon }) => {
    given.push({ home: daemon.home, socket: daemon.socket, port: daemon.port });
  });
}
`;

describe("the daemon fixture", () => {
  test("gives two tests two Daemons, homes and ports, and leaves nothing of them", async ({
    processes,
  }) => {
    // Inside the repository, so the copied configuration finds `@playwright/test`.
    mkdirSync(join(ROOT, "target"), { recursive: true });
    const project = mkdtempSync(join(ROOT, "target", "served-daemon-test-"));
    onTestFinished(() => rmSync(project, { recursive: true, force: true }));
    copyFileSync(join(ROOT, "playwright.config.ts"), join(project, "playwright.config.ts"));
    mkdirSync(join(project, "features"));
    copyFileSync(
      join(ROOT, "features", "inventory.json"),
      join(project, "features", "inventory.json"),
    );
    writeFileSync(join(project, "served-daemon.spec.ts"), SPEC);
    const record = join(project, "left.json");
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      GOODFETCH_TEST_SUITE: "pre-merge",
      SERVED_DAEMON_RECORD: record,
    };
    delete env["PLAYWRIGHT_JUNIT_OUTPUT_FILE"];

    const run = processes.spawn(
      process.execPath,
      [CLI, "test", "--config", "playwright.config.ts", "--workers", "1"],
      { env, cwd: project },
    );

    const exit = await run.exit;
    expect(exit.code).toBe(0);
    const left: unknown = JSON.parse(readFileSync(record, "utf8"));
    const nothingLeft = { homeLeft: false, socketLeft: false, portAnswers: false };
    expect(left).toEqual({
      homes: 2,
      sockets: 2,
      ports: 2,
      left: [nothingLeft, nothingLeft],
    });
  });
});
