// The read-only banner against the in-memory adapter of the protocol (spec
// #592, seam 1; ADR 0031): while the Console has no Connection to its
// engine, a banner says the page is read-only until it reconnects and every
// action is disabled, so nobody acts on a stale view; once it reconnects by
// itself the banner goes and the actions work again.
import type { SettingsExplanation } from "@goodfetch/collie-client/generated";
import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import {
  createConsoleClient,
  type ConsoleOperations,
  type PageVisibility,
} from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page whose visibility the test sets. */
class Visibility implements PageVisibility {
  visible = true;
  readonly #listeners = new Set<() => void>();

  watch(changed: () => void): () => void {
    this.#listeners.add(changed);
    return () => this.#listeners.delete(changed);
  }

  set(visible: boolean): void {
    this.visible = visible;
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

const EXPLANATION: SettingsExplanation = { files: [], settings: [] };

const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

test(
  "while disconnected the page is read-only under a banner, and acts again once reconnected",
  { tags: ["story:12-125", "screen:/settings"] },
  async () => {
    // The adapter over every operation the Console calls, the person-only `config.policy_read` included.
    const adapter = createProtocolAdapter<ConsoleOperations>();
    adapter.answer("needs_you.list", { items: [] });
    adapter.answer("config.explain", EXPLANATION);
    adapter.answer("usage.list", { limits: [] });
    adapter.answer("config.policy_read", {
      path: "/home/1/operator-policy.toml",
      found: false,
      digest: "sha256:e3b0c44298fc1c14",
      entries: [],
    });
    const visibility = new Visibility();
    const client = createConsoleClient({
      transport: adapter.transport,
      bodyFetcher: adapter.bodyFetcher,
      visibility,
      product: "collie",
    });
    await render(
      <ConsoleI18n>
        <Shell client={client} route="/settings" />
      </ConsoleI18n>,
    );
    const explain = page.getByRole("button", { name: "Explain" });
    await expect.element(explain).toBeEnabled();
    await expect.element(page.getByRole("alert")).not.toBeInTheDocument();

    visibility.set(false);
    adapter.drop();

    await expect.element(page.getByRole("alert")).toHaveTextContent(BANNER);
    await expect.element(explain).toBeDisabled();

    visibility.set(true);

    await expect.element(page.getByRole("alert")).not.toBeInTheDocument();
    await expect.element(explain).toBeEnabled();
    client.close();
  },
);
