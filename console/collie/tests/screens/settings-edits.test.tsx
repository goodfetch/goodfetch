// Editing settings and the Operator policy in Engine & settings, against the
// in-memory adapter of the protocol (spec #592, seam 1; ADR 0031): a
// user-level value is written with `config.user_write` and a run's own value
// joins the run values, each then explained again; a project configuration's
// and a Recipe manifest's value offer no edit, and a run's value fixed for
// its life is read-only with a pointer to a new run. The Operator policy is
// written against the digest read, and a `policy_conflict` offers to read it
// again.
import type {
  ConfigExplainParams,
  ExplainedSetting,
  OperatorPolicyRead,
  PolicyEntry,
  SettingsExplanation,
} from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
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

/** A JSON value as the generated types write a setting's value. */
function json(value: unknown): PolicyEntry["value"] {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
  return value as PolicyEntry["value"];
}

/** A setting as `config.explain` answers it, at the built-in default, which user configuration may set. */
function setting(
  key: string,
  overrides: Partial<Omit<ExplainedSetting, "value">> & { readonly value?: unknown } = {},
): ExplainedSetting {
  const { value = 30000, ...rest } = overrides;
  return {
    key,
    products: ["collie", "foxhound"],
    applied: true,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
    value: value as ExplainedSetting["value"],
    layer: "default",
    weakens: false,
    user_settable: true,
    fixed_for_run: false,
    ...rest,
  };
}

/** A policy `config.policy_read` answers, with `digest`. */
function policy(digest: string, entries: OperatorPolicyRead["entries"] = []): OperatorPolicyRead {
  return { path: "/home/1/operator-policy.toml", found: true, digest, entries };
}

/** Engine & settings, signed in, over the adapter, whose `config.explain` answers `answer`. */
async function engineSettings(
  answer: (params: ConfigExplainParams) => SettingsExplanation,
  read: OperatorPolicyRead = policy("sha256:aaaaaaaaaaaaaaaa"),
) {
  // The adapter over every operation the Console calls, the person-only `config.policy_*` included.
  const adapter = createProtocolAdapter<ConsoleOperations>();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("config.explain", answer);
  adapter.answer("usage.list", { limits: [] });
  adapter.answer("config.policy_read", read);
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

/** The effective settings' row of `key`. */
function rowOf(key: string) {
  return page
    .getByRole("grid", { name: "Effective settings" })
    .getByRole("row")
    .filter({ hasText: key });
}

/** The params of each call of `method` the Client sent. */
function paramsOf(adapter: ProtocolAdapter<ConsoleOperations>, method: string): unknown[] {
  return adapter.requests
    .filter((request) => request.method === method)
    .map((request) => request.params);
}

test(
  "a user-level value edited in Engine & settings is written to user configuration and takes effect",
  { tags: ["story:12-95", "operation:config.user_write", "screen:/settings"] },
  async () => {
    let written = false;
    const { adapter, client } = await engineSettings(() => ({
      files: [],
      settings: [
        written
          ? setting("timeout.navigation", { value: 45000, layer: "user_configuration" })
          : setting("timeout.navigation"),
      ],
    }));
    adapter.answer("config.user_write", () => {
      written = true;
      return { diff: [{ key: "timeout.navigation", before: json(null), after: json(45000) }] };
    });

    await userEvent.click(
      page.getByRole("button", { name: "Edit user value of timeout.navigation" }),
    );
    await userEvent.fill(
      page.getByLabelText("The user-level value of timeout.navigation, as JSON"),
      "45000",
    );
    await userEvent.click(page.getByRole("button", { name: "Save" }));

    await expect.element(rowOf("timeout.navigation")).toMatchTextContent(/45000User configuration/);
    await expect
      .element(page.getByRole("status").filter({ hasText: "user configuration" }))
      .toHaveTextContent("timeout.navigation is written to user configuration, for new runs.");
    expect(paramsOf(adapter, "config.user_write")).toEqual([
      { changes: [{ key: "timeout.navigation", value: 45000 }] },
    ]);
    client.close();
  },
);

test(
  "a run's own value edited in Engine & settings joins the run values and is explained again",
  { tags: ["story:12-95", "screen:/settings"] },
  async () => {
    const { adapter, client } = await engineSettings((params) => ({
      files: [],
      settings: [
        setting("timeout.run", {
          value: params.settings?.["timeout"] === undefined ? 600000 : 900000,
          layer: "run",
          origin: "settings",
        }),
      ],
    }));
    await userEvent.fill(page.getByLabelText("Run values"), "timeout.run = 600000");
    await userEvent.click(page.getByRole("button", { name: "Explain" }));

    await userEvent.click(page.getByRole("button", { name: "Edit run value of timeout.run" }));
    await userEvent.fill(page.getByLabelText("The run's value of timeout.run, as JSON"), "900000");
    await userEvent.click(page.getByRole("button", { name: "Save" }));

    await expect.element(rowOf("timeout.run")).toMatchTextContent(/^timeout\.run900000Run/);
    await expect
      .element(page.getByLabelText("Run values"))
      .toHaveValue("[timeout]\nrun = 900000\n");
    expect(paramsOf(adapter, "config.explain").at(-1)).toEqual({
      settings: { timeout: { run: 900000 } },
    });
    expect(paramsOf(adapter, "config.user_write")).toEqual([]);
    client.close();
  },
);

test(
  "a project configuration's and a Recipe manifest's value offer no edit, and a run's value fixed for its life points to a new run",
  { tags: ["story:12-95", "screen:/settings"] },
  async () => {
    const { client } = await engineSettings(() => ({
      files: [],
      settings: [
        setting("crawl.scope.strategy", {
          value: "same_domain",
          layer: "run",
          origin: "settings",
          products: ["foxhound"],
          fixed_for_run: true,
        }),
        setting("timeout.first_byte", { value: 10000, layer: "project_configuration" }),
        setting("timeout.navigation", { value: 45000, layer: "recipe" }),
      ],
    }));

    await expect
      .element(rowOf("crawl.scope.strategy").getByRole("gridcell").last())
      .toHaveTextContent("Fixed for a run's life: start a new run to change it");
    await expect
      .element(rowOf("timeout.first_byte").getByRole("gridcell").last())
      .toHaveTextContent("Edit the project configuration file, under version control");
    await expect
      .element(rowOf("timeout.navigation").getByRole("gridcell").last())
      .toHaveTextContent("Edit the Recipe's manifest, under version control");
    expect(
      page.getByRole("grid", { name: "Effective settings" }).getByRole("button").elements(),
    ).toEqual([]);
    client.close();
  },
);

test(
  "an Operator policy change is written against the digest read, and the policy read again",
  { tags: ["story:12-96", "operation:config.policy_write", "screen:/settings"] },
  async () => {
    const { adapter, client } = await engineSettings(
      () => ({ files: [], settings: [setting("timeout.run")] }),
      policy("sha256:aaaaaaaaaaaaaaaa", [
        { table: "cap", key: "timeout.run", value: json(900000) },
      ]),
    );
    adapter.answer("config.policy_write", () => {
      adapter.answer(
        "config.policy_read",
        policy("sha256:bbbbbbbbbbbbbbbb", [
          { table: "cap", key: "timeout.run", value: json(600000) },
        ]),
      );
      return {
        digest: "sha256:bbbbbbbbbbbbbbbb",
        diff: [{ table: "cap", key: "timeout.run", before: json(900000), after: json(600000) }],
      };
    });
    const form = page.getByRole("form", { name: "Change the Operator policy" });

    await userEvent.selectOptions(form.getByLabelText("Table"), "Caps");
    await userEvent.fill(form.getByLabelText("Setting"), "timeout.run");
    await userEvent.fill(form.getByLabelText("Value"), "600000");
    await userEvent.click(form.getByRole("button", { name: "Write" }));

    await expect
      .element(page.getByRole("status").filter({ hasText: "Written." }))
      .toMatchTextContent(/policy version sha256:bbbbbbbbbbbbbbbb/);
    await expect
      .element(page.getByRole("table", { name: "Operator policy entries" }))
      .toMatchTextContent(/timeout\.runCaps600000/);
    expect(paramsOf(adapter, "config.policy_write")).toEqual([
      {
        digest: "sha256:aaaaaaaaaaaaaaaa",
        changes: [{ table: "cap", key: "timeout.run", value: 600000 }],
      },
    ]);
    expect(paramsOf(adapter, "config.explain")).toHaveLength(2);
    client.close();
  },
);

test(
  "a policy_conflict shows the catalog's message and reads the policy again on request",
  { tags: ["story:12-96", "error:policy_conflict", "screen:/settings"] },
  async () => {
    const { adapter, client } = await engineSettings(() => ({
      files: [],
      settings: [setting("timeout.run")],
    }));
    adapter.answer(
      "config.policy_write",
      failure("policy_conflict", {
        digest: "sha256:aaaaaaaaaaaaaaaa",
        current: "sha256:cccccccccccccccc",
        fix: "read the Operator policy again with `config.policy_read`",
      }),
    );
    const form = page.getByRole("form", { name: "Change the Operator policy" });

    await userEvent.fill(form.getByLabelText("Setting"), "timeout.run");
    await userEvent.fill(form.getByLabelText("Value"), "1000");
    await userEvent.click(form.getByRole("button", { name: "Write" }));
    await expect
      .element(page.getByRole("alert"))
      .toMatchTextContent(
        /The Operator policy changed since it was read, so nothing was written\./,
      );
    adapter.answer("config.policy_read", policy("sha256:cccccccccccccccc"));
    await userEvent.click(page.getByRole("button", { name: "Read the policy again" }));

    await expect.element(page.getByText(/version sha256:cccccccccccccccc/)).toBeVisible();
    expect(paramsOf(adapter, "config.policy_read")).toHaveLength(2);
    client.close();
  },
);
