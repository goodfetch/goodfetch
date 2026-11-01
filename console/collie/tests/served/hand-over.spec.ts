// One Console for both Products (spec #591, stories 2 and 122; ADRs 0066
// and 0098), with Collie's and Foxhound's executables built from the
// commit, a Daemon and home of its own: the test's Collie Daemon serves the
// Console, on a named port, as a release's fixed port is. Foxhound's
// `console` records Foxhound as installed, so the
// Collie Daemon, idle but for the open Console, hands over to its major's
// new pick, Foxhound's Daemon, on the same port. The open Console
// reconnects by itself, and `console` of either executable opens that same
// port, the sign-in kept.
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

// The open Console keeps its address across the handover.
test.use({ namedConsolePort: true });

/** How long a handover may take: the old Daemon stops, then the pick's copy is placed and started. */
const HANDOVER = 60_000;

/** Foxhound's executable built from the commit, from the test tool manifest `just test` writes. */
async function foxhoundExecutable(): Promise<string> {
  const manifest = process.env["GOODFETCH_TEST_TOOLS"];
  if (manifest === undefined) {
    throw new Error("GOODFETCH_TEST_TOOLS is not set: run the test with `just test`");
  }
  const tools: unknown = JSON.parse(await readFile(manifest, "utf8"));
  const path =
    typeof tools === "object" && tools !== null && "foxhound" in tools ? tools.foxhound : undefined;
  if (typeof path !== "string") {
    throw new Error(`the test tool manifest ${manifest} lists no Foxhound executable`);
  }
  return path;
}

/** Runs `executable` with `args` in the test's home: its exit code, stdout and stderr. */
async function command(
  daemon: ServedDaemon,
  executable: string,
  args: readonly string[],
): Promise<{ readonly code: number | null; readonly stdout: string; readonly stderr: string }> {
  const child = spawn(executable, args, {
    env: { ...daemon.env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });
  const code = await new Promise<number | null>((resolve) => child.on("close", resolve));
  return { code, stdout, stderr };
}

/** The Console URL `console --json` of `executable` opens, its sign-in fragment taken off. */
async function consoleOf(daemon: ServedDaemon, executable: string): Promise<URL> {
  const output = await command(daemon, executable, ["console", "--json", "--no-browser"]);
  expect(output.code, output.stderr).toBe(0);
  const opened: unknown = JSON.parse(output.stdout);
  const url =
    typeof opened === "object" && opened !== null && "url" in opened ? opened.url : undefined;
  if (typeof url !== "string") {
    throw new Error(`\`console --json\` gave no link: ${output.stdout}`);
  }
  const link = new URL(url);
  link.hash = "";
  return link;
}

test(
  "the open Console reconnects after the Daemon hands over to Foxhound, and either executable's console opens the same port, signed in",
  {
    annotation: [
      { type: "covers", description: "story:12-2" },
      { type: "covers", description: "story:12-122" },
      { type: "covers", description: "command:console" },
    ],
  },
  async ({ page, context, daemon, signIn, failOnErrors }) => {
    test.setTimeout(4 * HANDOVER);
    const foxhound = await foxhoundExecutable();
    await signIn.withLink(page);
    await expect(page.getByRole("main").getByRole("status")).toHaveText(
      /^Connected to Collie \d+\.\d+\.\d+\S*\.$/,
    );
    const [cookie] = await context.cookies(daemon.url);

    // Foxhound's `console` records Foxhound as installed; once it leaves,
    // the Collie Daemon is idle but for the open Console and hands over.
    const fromFoxhound = await consoleOf(daemon, foxhound);

    await expect(page.getByRole("main").getByRole("status")).toHaveText(
      /^Connected to Foxhound \d+\.\d+\.\d+\S*\.$/,
      { timeout: HANDOVER },
    );
    expect(new URL(page.url()).port).toBe(String(daemon.port));
    // Each attempt while the Daemon handed over failed, as the browser says.
    expect(
      failOnErrors.errors.every(
        ({ text, url }) =>
          /\/protocol|ERR_CONNECTION_REFUSED|ERR_CONNECTION_RESET|ERR_FAILED/.test(text) ||
          url.includes("/protocol/events"),
      ),
    ).toBe(true);
    await failOnErrors.take();
    const fromCollie = await consoleOf(daemon, daemon.executable);
    const fromFoxhoundAgain = await consoleOf(daemon, foxhound);
    for (const url of [fromFoxhound, fromCollie, fromFoxhoundAgain]) {
      expect(url.origin).toBe(new URL(daemon.url).origin);
    }

    // A bookmark of the Console opens it signed in, with the cookie it had.
    await page.goto(fromCollie.href);
    await expect(page.getByRole("main").getByRole("status")).toHaveText(
      /^Connected to Foxhound \d+\.\d+\.\d+\S*\.$/,
    );
    expect(await context.cookies(daemon.url)).toEqual([
      expect.objectContaining({ name: cookie?.name, value: cookie?.value }),
    ]);
  },
);
