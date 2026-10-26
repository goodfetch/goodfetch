// Enter during an IME composition (ADR 0073): in every Console form and
// search field, the Enter that an input method sends while a composition is
// open submits nothing and runs no search; once the composition is
// committed, Enter submits as before. The composition goes to the page
// through CDP's `Input.imeSetComposition`, as an input method's does, and
// Enter through `Input.dispatchKeyEvent`, which Chromium treats as the
// committing key, so the browser itself would submit the form.
/// <reference types="@vitest/browser-playwright" />
import type {
  ConfigExplainParams,
  ExplainedSetting,
  SettingsExplanation,
} from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { cdp, page, userEvent, type Locator } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { Address } from "#address.ts";
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

/** The page's address. */
class TestAddress implements Address {
  search = "";

  replace(search: string): void {
    this.search = search;
  }
}

/** A client of `adapter`'s engine, its "Needs you" list empty. */
function clientOf(adapter: ProtocolAdapter<ConsoleOperations>): ConsoleClient {
  adapter.answer("needs_you.list", { items: [] });
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
}

/** The params of each call of `method` the Client sent. */
function paramsOf(adapter: ProtocolAdapter<ConsoleOperations>, method: string): unknown[] {
  return adapter.requests
    .filter((request) => request.method === method)
    .map((request) => request.params);
}

/** Empties `field`, presses Enter as Chromium's input method does while it composes `text`, then commits `text`. */
async function composeAndPressEnter(field: Locator, text: string): Promise<void> {
  await userEvent.fill(field, "");
  await cdp().send("Input.imeSetComposition", {
    text,
    selectionStart: text.length,
    selectionEnd: text.length,
  });
  await cdp().send("Input.dispatchKeyEvent", {
    type: "keyDown",
    key: "Enter",
    code: "Enter",
    windowsVirtualKeyCode: 13,
    text: "\r",
  });
  await cdp().send("Input.dispatchKeyEvent", {
    type: "keyUp",
    key: "Enter",
    code: "Enter",
    windowsVirtualKeyCode: 13,
  });
  await cdp().send("Input.insertText", { text });
}

test.for(["Search", "Tags"])(
  "Enter while the Runs filter's %s field composes runs no search, and Enter after it does",
  { tags: ["story:12-134", "screen:/runs"] },
  async (label) => {
    const adapter = createProtocolAdapter<ConsoleOperations>();
    adapter.answer("run.list", { runs: [] });
    const client = clientOf(adapter);
    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs" address={new TestAddress()} />
      </ConsoleI18n>,
    );
    await expect.poll(() => paramsOf(adapter, "run.list").length).toBe(1);

    await composeAndPressEnter(page.getByLabelText(label, { exact: true }), "にほん");
    await userEvent.keyboard("{Enter}");

    await expect.poll(() => paramsOf(adapter, "run.list").length).toBe(2);
    expect(JSON.stringify(paramsOf(adapter, "run.list").at(-1))).toContain("にほん");
    client.close();
  },
);

/** A setting at its built-in default, which user configuration may set. */
function setting(key: string): ExplainedSetting {
  return {
    key,
    products: ["collie", "foxhound"],
    applied: true,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
    value: 30000 as unknown as ExplainedSetting["value"],
    layer: "default",
    weakens: false,
    user_settable: true,
    fixed_for_run: false,
  };
}

test(
  "Enter while a setting's value composes saves nothing, and Enter after it saves",
  { tags: ["story:12-134", "screen:/settings"] },
  async () => {
    const adapter = createProtocolAdapter<ConsoleOperations>();
    adapter.answer("config.explain", (_params: ConfigExplainParams): SettingsExplanation => ({
      files: [],
      settings: [setting("timeout.navigation")],
    }));
    adapter.answer("usage.list", { limits: [] });
    adapter.answer("config.policy_read", {
      path: "/home/1/operator-policy.toml",
      found: false,
      digest: "sha256:e3b0c44298fc1c14",
      entries: [],
    });
    adapter.answer("config.user_write", { diff: [] });
    const client = clientOf(adapter);
    await render(
      <ConsoleI18n>
        <Shell client={client} route="/settings" />
      </ConsoleI18n>,
    );
    await userEvent.click(
      page.getByRole("button", { name: "Edit user value of timeout.navigation" }),
    );

    await composeAndPressEnter(
      page.getByLabelText("The user-level value of timeout.navigation, as JSON"),
      "45000",
    );
    expect(paramsOf(adapter, "config.user_write")).toEqual([]);
    await userEvent.keyboard("{Enter}");

    await expect
      .poll(() => paramsOf(adapter, "config.user_write"))
      .toEqual([{ changes: [{ key: "timeout.navigation", value: 45000 }] }]);
    client.close();
  },
);
