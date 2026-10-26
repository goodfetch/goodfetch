// Engine & settings served by Collie's executable (spec #592; ADRs 0028,
// 0031 and 0087): for settings seeded across the five layers `config
// explain` reads (the built-in defaults, an Operator policy that locks a
// key, user configuration, a project configuration and a run value), the
// screen shows what `goodfetch-collie config explain --json` prints for the
// same configuration: each value, the layer that set it and the layer that
// locked it, and the key only Foxhound honours as Foxhound's.
import { execFile } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import type { SettingsExplanation } from "@goodfetch/collie-client/generated";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";
import { effectiveSettings, settingRow } from "#tests/effective-settings.ts";

const run = promisify(execFile);

/** What `config explain --json` prints in the test's home, with `args`. */
async function explain(
  daemon: ServedDaemon,
  args: readonly string[],
): Promise<SettingsExplanation> {
  const { stdout } = await run(daemon.executable, ["config", "explain", "--json", ...args], {
    env: { ...daemon.env },
  });
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `config explain --json` prints `goodfetch-protocol`'s type, which the executable's own tests check against the bundle.
  return JSON.parse(stdout) as SettingsExplanation;
}

/** Writes the configuration of every layer but the run, and answers the project configuration's path. */
async function seed(daemon: ServedDaemon): Promise<string> {
  const { files } = await explain(daemon, []);
  const fileOf = (layer: string): string => {
    const file = files.find((candidate) => candidate.layer === layer);
    if (file === undefined) {
      throw new Error(`config explain reads no ${layer} file`);
    }
    return file.path;
  };
  const policy = fileOf("operator_policy");
  await mkdir(dirname(policy), { recursive: true });
  await writeFile(
    policy,
    'version = "0.1"\n\n[lock.network.resolve]\n"staging.example" = ["127.0.0.1"]\n',
  );
  await writeFile(
    fileOf("user_configuration"),
    'version = "0.1"\n\n[settings]\ntimeout.navigation = 45000\ncrawl.retry.max = 5\n',
  );
  const project = join(daemon.home, "project", "goodfetch.toml");
  await mkdir(dirname(project), { recursive: true });
  await writeFile(
    project,
    'version = "0.1"\n\n[settings.timeout]\nnavigation = 60000\nfirst_byte = 10000\n',
  );
  return project;
}

/** Each layer's name on the screen. */
const LAYERS: Readonly<Record<string, string>> = {
  default: "Built-in default",
  operator_policy: "Operator policy",
  user_configuration: "User configuration",
  project_configuration: "Project configuration",
  run: "Run",
};

/** What the screen's row of each key shows: its value, the layer that set it and the layer that locked it. */
async function shown(page: Page): Promise<Record<string, readonly string[]>> {
  const rows: Record<string, readonly string[]> = {};
  const settings = effectiveSettings(page)
    .getByRole("row")
    .filter({ has: page.getByRole("rowheader") });
  for (const row of await settings.all()) {
    const key = await row.getByRole("rowheader").textContent();
    const cells = await row.getByRole("gridcell").allTextContents();
    rows[key ?? ""] = cells.slice(0, 3);
  }
  return rows;
}

test(
  "Engine & settings shows the values, layers and locks config explain shows across the five layers",
  {
    annotation: [
      { type: "covers", description: "story:12-94" },
      { type: "covers", description: "story:12-102" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    const project = await seed(daemon);
    const cli = await explain(daemon, ["--project", project, "--set", "timeout.run=600000"]);

    await signIn.withLink(page);
    await page.goto(new URL("/settings", daemon.url).href);
    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Engine & settings");
    await expect(effectiveSettings(page)).toBeVisible();
    await page.getByLabel("Project configuration").setInputFiles(project);
    await page.getByLabel("Run values").fill("timeout.run = 600000");
    await page.getByRole("button", { name: "Explain" }).click();
    await expect(settingRow(page, "timeout.run").getByRole("gridcell").first()).toHaveText(
      "600000",
    );

    const expected = Object.fromEntries(
      cli.settings.map((setting) => [
        setting.key,
        [
          JSON.stringify(setting.value),
          LAYERS[setting.layer],
          setting.locked_by === undefined || setting.locked_by === null
            ? "Not locked"
            : LAYERS[setting.locked_by],
        ],
      ]),
    );
    const rows = await shown(page);
    // The run layer names its origin: the CLI's flag there, the run's settings here.
    expect({
      ...rows,
      "timeout.run": rows["timeout.run"]?.map((cell) => cell.replace(/, from .*$/, "")),
    }).toEqual(expected);
    expect(
      [
        "network.resolve",
        "timeout.action",
        "timeout.navigation",
        "timeout.first_byte",
        "timeout.run",
      ].map((key) => expected[key]),
    ).toEqual([
      ['{"staging.example":["127.0.0.1"]}', "Operator policy", "Operator policy"],
      ["30000", "Built-in default", "Not locked"],
      ["60000", "Project configuration", "Not locked"],
      ["10000", "Project configuration", "Not locked"],
      ["600000", "Run", "Not locked"],
    ]);
    await expect(settingRow(page, "crawl.retry.max")).toContainText(
      "Foxhound's setting, not applied by this engine",
    );
  },
);
