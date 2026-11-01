// J-009, "See where each setting's value comes from", on the `console`
// surface (ADRs 0063 and 0075): a person sets one setting in user
// configuration and the same one and a second in their project's
// configuration, then asks Collie's Console, in Engine & settings, where
// each effective value comes from, explaining with that project
// configuration as `config explain` run in the project's folder does.
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import type { SettingsExplanation } from "@goodfetch/collie-client/generated";
import { journey } from "@goodfetch/test-support/journey";
import { expect } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";
import { settingRow } from "#tests/effective-settings.ts";

/** The text of each cell of the row of `key`: its value, the layer that set it, the layer that locked it and its notes. */
async function rowOf(page: Page, key: string): Promise<string[]> {
  const row = settingRow(page, key);
  await expect(row).toBeVisible();
  return row.getByRole("gridcell").allTextContents();
}

journey("J-009", "console", async ({ step, check, cli, daemon, page, signIn }) => {
  const project = join(daemon.home, "project", "goodfetch.toml");

  await step(
    "Set one setting in the user configuration, and set it and a second setting in the project configuration",
    async () => {
      const explained = await cli(["config", "explain", "--json"]);
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `config explain --json` prints `goodfetch-protocol`'s type.
      const { files } = JSON.parse(explained.stdout) as SettingsExplanation;
      const user = files.find((file) => file.layer === "user_configuration");
      if (user === undefined) {
        throw new Error("config explain reads no user configuration");
      }
      await mkdir(dirname(user.path), { recursive: true });
      await writeFile(user.path, 'version = "0.1"\n\n[settings.timeout]\nnavigation = 45000\n');
      await mkdir(dirname(project), { recursive: true });
      await writeFile(
        project,
        'version = "0.1"\n\n[settings.timeout]\nnavigation = 60000\nfirst_byte = 10000\n',
      );
    },
  );

  await step("Ask where each effective value comes from", async () => {
    await signIn.withLink(page);
    await page.getByRole("link", { name: "Engine & settings" }).click();
    await page.getByLabel("Project configuration").setInputFiles(project);
    await page.getByRole("button", { name: "Explain" }).click();
    await expect(settingRow(page, "timeout.first_byte").getByRole("gridcell").first()).toHaveText(
      "10000",
    );
  });

  await check(
    "Each effective value is shown with the layer that set it and the layer that locked it, or none: the project configuration for the setting both configurations set, and the built-in default for a setting no layer set",
    async () => {
      expect([
        (await rowOf(page, "timeout.navigation")).slice(1, 3),
        (await rowOf(page, "timeout.first_byte")).slice(1, 3),
        (await rowOf(page, "timeout.run")).slice(1, 3),
      ]).toEqual([
        ["Project configuration", "Not locked"],
        ["Project configuration", "Not locked"],
        ["Built-in default", "Not locked"],
      ]);
    },
  );

  await check(
    "The setting both configurations set shows the project configuration's value, since no lower layer locked it",
    async () => {
      expect((await rowOf(page, "timeout.navigation"))[0]).toBe("60000");
    },
  );
});
