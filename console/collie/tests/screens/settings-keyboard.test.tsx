// The keyboard in Engine & settings' effective settings (ADR 0073; WAI-ARIA's
// data grid pattern): the table is a grid that takes one Tab stop, however
// many settings the engine has, and the arrow keys, Home, End, Page Up and
// Page Down move between its cells, onto a cell's Edit button where it has
// one. So a new setting never moves the screen's keyboard walk
// (`console/screens.toml`).
import type { ExplainedSetting, SettingsExplanation } from "@goodfetch/collie-client/generated";
import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { cleanup, render } from "vitest-browser-react";
import {
  createConsoleClient,
  type ConsoleClient,
  type ConsoleOperations,
  type PageVisibility,
} from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

/** A setting at its built-in default, which user configuration may set, or another layer's. */
function setting(key: string, layer: ExplainedSetting["layer"] = "default"): ExplainedSetting {
  return {
    key,
    products: ["collie", "foxhound"],
    applied: true,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
    value: 30000 as unknown as ExplainedSetting["value"],
    layer,
    weakens: false,
    user_settable: true,
    fixed_for_run: false,
  };
}

/** Engine & settings, signed in, over the in-memory adapter, explaining `settings`. */
async function engineSettings(settings: readonly ExplainedSetting[]): Promise<ConsoleClient> {
  const adapter = createProtocolAdapter<ConsoleOperations>();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("config.explain", (): SettingsExplanation => ({
    files: [],
    settings: [...settings],
  }));
  adapter.answer("usage.list", { limits: [] });
  adapter.answer("config.policy_read", {
    path: "/home/1/operator-policy.toml",
    found: true,
    digest: "sha256:aaaaaaaaaaaaaaaa",
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
  return client;
}

const grid = () => page.getByRole("grid", { name: "Effective settings" });
const settingHeader = () => page.getByRole("columnheader", { name: "Setting" }).element();
const rowheader = (key: string) => page.getByRole("rowheader", { name: key }).element();
const edit = (key: string) =>
  page.getByRole("button", { name: `Edit user value of ${key}` }).element();

/** Fails unless `element` has focus. */
function expectFocusOn(element: Element): void {
  expect(document.activeElement).toBe(element);
}

/** Each element Tab stops on inside the effective settings, from the Explain button to the stop after them. */
async function stopsInEffectiveSettings(): Promise<Element[]> {
  await expect.element(page.getByRole("rowheader", { name: "browser.headless" })).toBeVisible();
  const region = page.getByRole("region", { name: "Effective settings, scrollable" }).element();
  page.getByRole("button", { name: "Explain" }).element().focus();
  const stops: Element[] = [];
  // Far more presses than the settings have stops, so a stop per setting shows.
  for (let press = 0; press < 20; press += 1) {
    await userEvent.keyboard("{Tab}");
    const stop = document.activeElement;
    if (stop === null || !region.contains(stop)) {
      break;
    }
    stops.push(stop);
  }
  return stops;
}

test(
  "the effective settings take one Tab stop, and one more setting adds none",
  { tags: ["story:12-130"] },
  async () => {
    const three = ["browser.headless", "timeout.navigation", "timeout.run"].map((key) =>
      setting(key),
    );
    let client = await engineSettings(three);
    expect(await stopsInEffectiveSettings()).toEqual([settingHeader()]);
    client.close();
    await cleanup();

    client = await engineSettings([...three, setting("admission.max_requests_per_minute")]);
    expect(await stopsInEffectiveSettings()).toEqual([settingHeader()]);
    client.close();
  },
);

test(
  "the arrow keys, Home, End and Page keys move between the effective settings' cells",
  { tags: ["story:12-130"] },
  async () => {
    const client = await engineSettings([
      setting("browser.headless"),
      setting("timeout.navigation", "project_configuration"),
      setting("timeout.run"),
    ]);
    await expect.element(grid()).toBeVisible();
    settingHeader().focus();

    await userEvent.keyboard("{ArrowDown}");
    expectFocusOn(rowheader("browser.headless"));
    await userEvent.keyboard("{End}");
    expectFocusOn(edit("browser.headless"));
    await userEvent.keyboard("{ArrowDown}");
    // A cell with no widget takes focus itself.
    expectFocusOn(
      page
        .getByRole("gridcell", {
          name: "Edit the project configuration file, under version control",
        })
        .element(),
    );
    await userEvent.keyboard("{ArrowDown}");
    expectFocusOn(edit("timeout.run"));
    await userEvent.keyboard("{ArrowDown}");
    expectFocusOn(edit("timeout.run"));
    await userEvent.keyboard("{Home}");
    expectFocusOn(rowheader("timeout.run"));
    await userEvent.keyboard("{ArrowLeft}");
    expectFocusOn(rowheader("timeout.run"));
    await userEvent.keyboard("{PageUp}");
    expectFocusOn(settingHeader());
    await userEvent.keyboard("{Control>}{End}{/Control}");
    expectFocusOn(edit("timeout.run"));
    await userEvent.keyboard("{Control>}{Home}{/Control}");
    expectFocusOn(settingHeader());
    await userEvent.keyboard("{PageDown}");
    expectFocusOn(rowheader("timeout.run"));
    await userEvent.keyboard("{ArrowRight}");
    expectFocusOn(page.getByRole("gridcell", { name: "30000" }).last().element());

    // Tab leaves the grid, and Shift+Tab comes back to the cell it left.
    await userEvent.keyboard("{Tab}");
    expect(grid().element().contains(document.activeElement)).toBe(false);
    await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
    expectFocusOn(page.getByRole("gridcell", { name: "30000" }).last().element());
    client.close();
  },
);

test(
  "the arrow keys edit an open value's text instead of moving between cells",
  { tags: ["story:12-130"] },
  async () => {
    const client = await engineSettings([setting("browser.headless"), setting("timeout.run")]);
    await userEvent.click(
      page.getByRole("button", { name: "Edit user value of browser.headless" }),
    );
    const field = page.getByLabelText("The user-level value of browser.headless, as JSON");
    await userEvent.click(field);
    await userEvent.keyboard("{End}{ArrowLeft}{Backspace}");
    await expect.element(field).toHaveValue("3000");
    await userEvent.keyboard("{ArrowDown}{ArrowUp}{Home}{PageDown}");
    expectFocusOn(field.element());
    client.close();
  },
);
