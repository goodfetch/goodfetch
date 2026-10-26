// Editing settings and the Operator policy in Engine & settings, served by
// Collie's executable (spec #592; ADR 0031): a user-level value edited in the
// Console is written to user configuration, every other byte kept, and shown
// set by it; the
// Operator policy is written against the digest the Console read, every
// other byte of the file kept, and after a hand edit the next write answers
// `policy_conflict`, writes nothing, and the policy read again shows the
// hand edit.
import { execFile } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { promisify } from "node:util";
import type { SettingsExplanation } from "@goodfetch/collie-client/generated";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";
import { settingRow } from "#tests/effective-settings.ts";

const run = promisify(execFile);

/** The path of `layer`'s file, as `config explain --json` names it in the test's home. */
async function fileOf(daemon: ServedDaemon, layer: string): Promise<string> {
  const { stdout } = await run(daemon.executable, ["config", "explain", "--json"], {
    env: { ...daemon.env },
  });
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `config explain --json` prints `goodfetch-protocol`'s type, which the executable's own tests check against the bundle.
  const { files } = JSON.parse(stdout) as SettingsExplanation;
  const file = files.find((candidate) => candidate.layer === layer);
  if (file === undefined) {
    throw new Error(`config explain reads no ${layer} file`);
  }
  return file.path;
}

/** Writes one change to the Operator policy through the section's form. */
async function writePolicy(page: Page, table: string, key: string, value: string): Promise<void> {
  const form = page.getByRole("form", { name: "Change the Operator policy" });
  await form.getByLabel("Table").selectOption({ label: table });
  await form.getByLabel("Setting").fill(key);
  await form.getByLabel("Value").fill(value);
  await form.getByRole("button", { name: "Write" }).click();
}

test(
  "a user-level value edited in the Console is written to user configuration and takes effect",
  {
    annotation: [
      { type: "covers", description: "story:12-95" },
      { type: "covers", description: "operation:config.user_write" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    const user = await fileOf(daemon, "user_configuration");
    const before = await readFile(user, "utf8");

    await signIn.withLink(page);
    await page.goto(new URL("/settings", daemon.url).href);
    const row = settingRow(page, "timeout.navigation");
    await expect(row.getByRole("gridcell").nth(1)).toHaveText("Built-in default");
    await row.getByRole("button", { name: "Edit user value of timeout.navigation" }).click();
    await row.getByLabel("The user-level value of timeout.navigation, as JSON").fill("45000");
    await row.getByRole("button", { name: "Save" }).click();

    await expect(row.getByRole("gridcell").nth(1)).toHaveText("User configuration");
    await expect(row.getByRole("gridcell").first()).toHaveText("45000");
    // Every other byte of the test home's user configuration is kept.
    expect(await readFile(user, "utf8")).toBe(
      `${before}\n[settings.timeout]\nnavigation = 45000\n`,
    );
  },
);

test(
  "the Operator policy is written against the digest read, and a hand edit since is a policy_conflict that writes nothing",
  {
    annotation: [
      { type: "covers", description: "story:12-96" },
      { type: "covers", description: "operation:config.policy_read" },
      { type: "covers", description: "operation:config.policy_write" },
      { type: "covers", description: "error:policy_conflict" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    const policy = await fileOf(daemon, "operator_policy");
    await mkdir(dirname(policy), { recursive: true });
    await writeFile(policy, '# Kept by hand.\nversion = "0.1"\n');

    await signIn.withLink(page);
    await page.goto(new URL("/settings", daemon.url).href);
    await expect(page.getByText("The Operator policy sets, locks and caps nothing.")).toBeVisible();
    await writePolicy(page, "Caps", "timeout.run", "600000");

    await expect(page.getByRole("status").filter({ hasText: "Written." })).toBeVisible();
    await expect(
      page.getByRole("table", { name: "Operator policy entries" }).getByRole("row").last(),
    ).toHaveText("timeout.runCaps600000");
    expect(await readFile(policy, "utf8")).toBe(
      '# Kept by hand.\nversion = "0.1"\n\n[cap.timeout]\nrun = 600000\n',
    );

    const handEdit = '# Kept by hand.\nversion = "0.1"\n\n[cap.timeout]\nrun = 300000\n';
    await writeFile(policy, handEdit);
    await writePolicy(page, "Caps", "timeout.run", "900000");

    const alert = page.getByRole("alert");
    await expect(alert).toContainText(
      "The Operator policy changed since it was read, so nothing was written.",
    );
    await expect(alert.getByText("policy_conflict", { exact: true })).toBeVisible();
    expect(await readFile(policy, "utf8")).toBe(handEdit);
    await alert.getByRole("button", { name: "Read the policy again" }).click();
    await expect(
      page.getByRole("table", { name: "Operator policy entries" }).getByRole("row").last(),
    ).toHaveText("timeout.runCaps300000");
  },
);
