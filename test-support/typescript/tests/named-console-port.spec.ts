// The `namedConsolePort` option of test support's Playwright `daemon` fixture
// (#2035): a Daemon started again, after a stop or a crash, serves its Console
// where it did, on a port no program asking the OS for one can take in between.
import { readFileSync } from "node:fs";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

test.use({ namedConsolePort: true });

/** The first port this OS gives out for port 0 or as a connection's source. */
function firstEphemeralPort(): number {
  if (process.platform !== "linux") {
    return 49_152;
  }
  const [first] = readFileSync("/proc/sys/net/ipv4/ip_local_port_range", "utf8").split(/\s+/);
  return Number(first);
}

/** The field `name` of what `daemon status --json` says of the test's Daemon. */
async function status(daemon: ServedDaemon, name: string): Promise<unknown> {
  const output = await daemon.run(["daemon", "status", "--json"]);
  expect(output.code, output.stderr).toBe(0);
  const value: unknown = JSON.parse(output.stdout);
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

test("serves the Console on a port the OS gives no other program", ({ daemon }) => {
  expect(new URL(daemon.url).port).toBe(String(daemon.port));
  expect(daemon.port).toBeLessThan(firstEphemeralPort());
});

test("a Daemon started again after a crash serves its Console on that port", async ({ daemon }) => {
  process.kill(Number(await status(daemon, "pid")), "SIGKILL");
  await expect.poll(async () => status(daemon, "pid")).toBeNull();

  const started = await daemon.run(["daemon", "start"]);

  expect(started.code, started.stderr).toBe(0);
  expect(await status(daemon, "console_url")).toBe(daemon.url);
});

test("a Daemon started again after a stop serves its Console on that port", async ({ daemon }) => {
  const stopped = await daemon.run(["daemon", "stop"]);
  expect(stopped.code, stopped.stderr).toBe(0);

  const started = await daemon.run(["daemon", "start"]);

  expect(started.code, started.stderr).toBe(0);
  expect(await status(daemon, "console_url")).toBe(daemon.url);
});
