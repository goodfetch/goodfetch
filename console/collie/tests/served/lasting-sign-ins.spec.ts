// A Console sign-in that lasts (spec #592; ADRs 0031 and 0098), on Collie's
// executable built from the commit, under its production headers, each test
// with a Daemon and home of its own on port 0: the cookie the sign-in link
// sets lasts the 30 idle days of `console.sign_in.idle_expiry`, and the
// sign-in outlives a Daemon restart, so the Console reopened on the new
// Daemon's port shows its engine with no sign-in link.
import { spawn } from "node:child_process";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

/** Thirty days, in seconds. */
const THIRTY_DAYS = 30 * 24 * 60 * 60;

/** Runs the Daemon's executable with `args` in the test's home: its exit code and stdout. */
async function command(
  daemon: ServedDaemon,
  args: readonly string[],
): Promise<{ readonly code: number | null; readonly stdout: string }> {
  const child = spawn(daemon.executable, args, {
    env: { ...daemon.env },
    stdio: ["ignore", "pipe", "inherit"],
  });
  let stdout = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  const code = await new Promise<number | null>((resolve) => child.on("close", resolve));
  return { code, stdout };
}

/** The field `name` of `value`, if it is an object that has one. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** Stops the test's Daemon and starts it again: the URL its Console is served at then. */
async function restarted(daemon: ServedDaemon): Promise<string> {
  expect((await command(daemon, ["daemon", "stop"])).code).toBe(0);
  expect((await command(daemon, ["daemon", "start"])).code).toBe(0);
  const status: unknown = JSON.parse(
    (await command(daemon, ["daemon", "status", "--json"])).stdout,
  );
  const url = field(status, "console_url");
  if (typeof url !== "string") {
    throw new Error(`the restarted Daemon serves no Console: ${JSON.stringify(status)}`);
  }
  return url;
}

test(
  "a sign-in lasts its idle expiry and outlives a Daemon restart",
  { annotation: { type: "covers", description: "story:12-5" } },
  async ({ page, context, daemon, signIn }) => {
    await signIn.withLink(page);
    const signedIn = Date.now() / 1000;
    const [cookie] = await context.cookies(daemon.url);

    // Away from the Console first, so no page reaches the stopped Daemon.
    await page.goto("about:blank");
    const url = await restarted(daemon);
    await page.goto(url);

    await expect(page.getByRole("main").getByRole("status")).toHaveText(
      /^Connected to Collie \d+\.\d+\.\d+\S*\.$/,
    );
    expect(new URL(page.url()).port).not.toBe(String(daemon.port));
    expect(cookie).toEqual(expect.objectContaining({ httpOnly: true, sameSite: "Strict" }));
    expect(cookie?.expires).toBeGreaterThanOrEqual(Math.floor(signedIn) + THIRTY_DAYS - 60);
    expect(cookie?.expires).toBeLessThanOrEqual(Math.ceil(signedIn) + THIRTY_DAYS + 60);
  },
);
