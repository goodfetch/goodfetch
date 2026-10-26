// Engine & settings against the in-memory adapter of the protocol (spec
// #592, seam 1; ADRs 0031 and 0087): the screen shows each setting
// `config.explain` answers with its value, the layer that set it and the
// layer that locked it, lists a key only Foxhound honours as Foxhound's,
// and explains again with the project configuration and run values a
// person gives it, as `config explain --project … --set …` does.
import type {
  ConfigExplainParams,
  ExplainedSetting,
  SettingsExplanation,
} from "@goodfetch/collie-client/generated";
import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import {
  createConsoleClient,
  type ConsoleOperations,
  type PageVisibility,
} from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

/** A setting as `config.explain` answers it, set by no layer, overridden where a test asserts. */
function setting(
  key: string,
  overrides: Partial<Omit<ExplainedSetting, "value" | "cap">> & {
    readonly value?: unknown;
    readonly cap?: unknown;
  } = {},
): ExplainedSetting {
  const { value = 30000, cap, ...rest } = overrides;
  return {
    key,
    products: ["collie", "foxhound"],
    applied: true,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
    value: value as ExplainedSetting["value"],
    layer: "default",
    weakens: false,
    ...rest,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a cap is any JSON value too.
    ...(cap === undefined ? {} : { cap: cap as NonNullable<ExplainedSetting["cap"]> }),
  };
}

/** The explanation the engine answers. */
function explanation(settings: ExplainedSetting[]): SettingsExplanation {
  return { files: [], settings };
}

/** Engine & settings, signed in, over the adapter, whose `config.explain` answers `answer`. */
async function engineSettings(
  answer: SettingsExplanation | ((params: ConfigExplainParams) => SettingsExplanation),
) {
  // The adapter over every operation the Console calls, the person-only `config.policy_*` included.
  const adapter = createProtocolAdapter<ConsoleOperations>();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("config.explain", answer);
  adapter.answer("usage.list", { limits: [] });
  adapter.answer("config.policy_read", {
    path: "/home/1/operator-policy.toml",
    found: false,
    digest: "sha256:e3b0c44298fc1c14",
    entries: [],
  });
  const client = createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
  await render(
    <ConsoleI18n>
      <Shell client={client} route="/settings" />
    </ConsoleI18n>,
  );
  return { adapter, client };
}

/** The text of each cell of the row of `key`. */
async function cellsOf(key: string): Promise<string[]> {
  const row = page.getByRole("row").filter({ hasText: key });
  await expect.element(row).toBeVisible();
  return row
    .getByRole("gridcell")
    .elements()
    .map((cell) => cell.textContent ?? "");
}

test(
  "Engine & settings shows each effective value with the layer that set it and the layer that locked it",
  { tags: ["story:12-94", "screen:/settings"] },
  async () => {
    const { client } = await engineSettings(
      explanation([
        setting("network.resolve", {
          value: { "staging.example": ["127.0.0.1"] },
          layer: "operator_policy",
          locked_by: "operator_policy",
        }),
        setting("timeout.action"),
        setting("timeout.first_byte", { value: 10000, layer: "project_configuration" }),
        setting("timeout.navigation", { value: 45000, layer: "user_configuration" }),
        setting("timeout.run", { value: 600000, layer: "run", origin: "settings", cap: 900000 }),
        setting("tls.accept_invalid_certs", {
          value: ["staging.example"],
          layer: "user_configuration",
          weakens: true,
        }),
      ]),
    );

    await expect
      .element(page.getByRole("heading", { level: 1 }))
      .toHaveTextContent("Engine & settings");
    expect(await cellsOf("network.resolve")).toEqual([
      '{"staging.example":["127.0.0.1"]}',
      "Operator policy",
      "Operator policy",
      "",
      "",
    ]);
    expect(await cellsOf("timeout.action")).toEqual([
      "30000",
      "Built-in default",
      "Not locked",
      "",
      "",
    ]);
    expect(await cellsOf("timeout.first_byte")).toEqual([
      "10000",
      "Project configuration",
      "Not locked",
      "",
      "Edit the project configuration file, under version control",
    ]);
    expect(await cellsOf("timeout.navigation")).toEqual([
      "45000",
      "User configuration",
      "Not locked",
      "",
      "",
    ]);
    expect(await cellsOf("timeout.run")).toEqual([
      "600000",
      "Run, from the run's settings",
      "Not locked",
      "Capped at 900000",
      "Edit run value",
    ]);
    expect(await cellsOf("tls.accept_invalid_certs")).toEqual([
      '["staging.example"]',
      "User configuration",
      "Not locked",
      "Weakens a guarantee",
      "",
    ]);
    client.close();
  },
);

test(
  "a key only Foxhound honours is listed as Foxhound's, not hidden, on Collie's engine",
  { tags: ["story:12-102"] },
  async () => {
    const { client } = await engineSettings(
      explanation([
        setting("crawl.retry.max", {
          value: 5,
          products: ["foxhound"],
          applied: false,
          layer: "user_configuration",
        }),
      ]),
    );

    expect(await cellsOf("crawl.retry.max")).toEqual([
      "5",
      "User configuration",
      "Not locked",
      "Foxhound's setting, not applied by this engine",
      "",
    ]);
    client.close();
  },
);

test(
  "Engine & settings explains again with the project configuration and run values a person gives",
  { tags: ["story:12-94"] },
  async () => {
    const { adapter, client } = await engineSettings((params) =>
      explanation(
        params.project === undefined || params.project === null
          ? [setting("timeout.navigation")]
          : [
              setting("timeout.navigation", { value: 60000, layer: "project_configuration" }),
              setting("timeout.run", { value: 600000, layer: "run", origin: "settings" }),
            ],
      ),
    );
    expect(await cellsOf("timeout.navigation")).toEqual([
      "30000",
      "Built-in default",
      "Not locked",
      "",
      "",
    ]);

    const project = new File(
      ['version = "0.1"\n\n[settings.timeout]\nnavigation = 60000\n'],
      "goodfetch.toml",
      { type: "application/toml" },
    );
    await userEvent.upload(page.getByLabelText("Project configuration"), project);
    await userEvent.fill(page.getByLabelText("Run values"), "timeout.run = 600000");
    await userEvent.click(page.getByRole("button", { name: "Explain" }));

    expect(await cellsOf("timeout.run")).toEqual([
      "600000",
      "Run, from the run's settings",
      "Not locked",
      "",
      "Edit run value",
    ]);
    expect(await cellsOf("timeout.navigation")).toEqual([
      "60000",
      "Project configuration",
      "Not locked",
      "",
      "Edit the project configuration file, under version control",
    ]);
    const explains = adapter.requests.filter((request) => request.method === "config.explain");
    expect(explains.at(-1)?.params).toEqual({
      project: { path: "goodfetch.toml", settings: { timeout: { navigation: 60000 } } },
      settings: { timeout: { run: 600000 } },
    });
    client.close();
  },
);

test("a project configuration that is no TOML is refused before any call, naming the file", async () => {
  const { adapter, client } = await engineSettings(explanation([setting("timeout.navigation")]));
  await expect.element(page.getByRole("grid", { name: "Effective settings" })).toBeVisible();

  await userEvent.upload(
    page.getByLabelText("Project configuration"),
    new File(["[settings\n"], "goodfetch.toml", { type: "application/toml" }),
  );
  await userEvent.click(page.getByRole("button", { name: "Explain" }));

  await expect
    .element(page.getByRole("alert"))
    .toMatchTextContent(/^goodfetch\.toml is no TOML file: Invalid TOML document/);
  expect(adapter.requests.filter((request) => request.method === "config.explain")).toHaveLength(1);
  client.close();
});
