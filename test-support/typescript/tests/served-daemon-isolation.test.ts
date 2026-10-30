// Seam B (spec #326): two served tests on test support's Playwright `daemon`
// fixture each get a Daemon, a home and a Console port of their own, and
// nothing of either is left once their teardown ends, so served tests can
// run in any order. Playwright runs the repository's own
// `playwright.config.ts` on a spec file of two tests in one worker, through
// `repository-playwright.ts`.
// Each writes down what its fixture gave it, the Console being the one the
// Daemon serves itself, and signs in to that Console; once both have ended,
// while the worker still runs, a worker fixture counts the distinct homes,
// sockets and Console ports they were given and looks for what is left of
// each, the Daemon's process among it. The Daemon binds its Console on port
// 0, so another process may bind the port it freed (#1777): its listener is
// left only if it still answers its own sign-in (`answersAs`), not when
// anything accepts a connect there.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, onTestFinished } from "vitest";
import { test } from "@goodfetch/test-support";
import { runRepositoryPlaywright } from "./repository-playwright.ts";

const FIXTURES = pathToFileURL(join(import.meta.dirname, "..", "src", "playwright.ts")).href;
const IDENTITY = pathToFileURL(join(import.meta.dirname, "..", "src", "console-identity.ts")).href;

const SPEC = `import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from ${JSON.stringify(FIXTURES)};
import { answersAs, consoleIdentity } from ${JSON.stringify(IDENTITY)};

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

const served = test.extend({
  given: [
    async ({}, use) => {
      const given = [];
      await use(given);
      const left = [];
      for (const { home, socket, identity, answeredWhileServing, pid } of given) {
        left.push({
          homeLeft: existsSync(home),
          socketLeft: process.platform !== "win32" && existsSync(socket),
          answeredWhileServing,
          consoleAnswers: await answersAs(identity),
          daemonAlive: alive(pid),
        });
      }
      const distinct = (key) => new Set(given.map((one) => one[key])).size;
      const record = { homes: distinct("home"), sockets: distinct("socket"), ports: distinct("port"), left };
      writeFileSync(process.env.SERVED_DAEMON_RECORD, JSON.stringify(record));
    },
    { scope: "worker" },
  ],
});

// The Console the Daemon serves itself, so the port is the Daemon's own.
// The check answers while the Daemon serves, so one that could never see
// the Daemon's listener fails here rather than passing after teardown.
for (const name of ["first", "second"]) {
  served(name, async ({ given, daemon }) => {
    const { pid } = JSON.parse(readFileSync(join(daemon.home, "0.0", "daemon.json"), "utf8"));
    const identity = await consoleIdentity(daemon);
    const answeredWhileServing = await answersAs(identity);
    given.push({ home: daemon.home, socket: daemon.socket, port: daemon.port, identity, answeredWhileServing, pid });
  });
}
`;

describe("the daemon fixture", () => {
  test("gives two tests two Daemons, homes and ports, and leaves nothing of them", async ({
    processes,
  }) => {
    const records = mkdtempSync(join(tmpdir(), "served-daemon-record-"));
    onTestFinished(() => rmSync(records, { recursive: true, force: true }));
    const record = join(records, "left.json");

    const { code } = await runRepositoryPlaywright(processes, {
      folder: "served-daemon-test",
      spec: { name: "served-daemon.spec.ts", text: SPEC },
      env: { SERVED_DAEMON_RECORD: record },
      workers: 1,
    });

    expect(code).toBe(0);
    const left: unknown = JSON.parse(readFileSync(record, "utf8"));
    const eachLeftNothing = {
      homeLeft: false,
      socketLeft: false,
      answeredWhileServing: true,
      consoleAnswers: false,
      daemonAlive: false,
    };
    expect(left).toEqual({
      homes: 2,
      sockets: 2,
      ports: 2,
      left: [eachLeftNothing, eachLeftNothing],
    });
  });
});
