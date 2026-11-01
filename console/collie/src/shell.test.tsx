import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type PageVisibility } from "./client/client.ts";
import { ConsoleI18n } from "./i18n.tsx";
import { Shell } from "./shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

test("the shell's first screen names the Product, then Collie, and the engine it reached", async () => {
  const adapter = createProtocolAdapter();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("usage.list", { limits: [] });
  const client = createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });

  await render(
    <ConsoleI18n>
      <Shell client={client} route="/" />
    </ConsoleI18n>,
  );

  const main = page.getByRole("main");
  await expect
    .element(main.getByRole("heading", { level: 1 }))
    .toHaveTextContent("Goodfetch Collie");
  await expect.element(main.getByText("Collie's Console.", { exact: false })).toBeVisible();
  await expect.element(main.getByRole("status")).toHaveTextContent("Connected to Collie 0.1.0.");
  client.close();
});

test("the shell says when the page is not signed in", async () => {
  await render(
    <ConsoleI18n>
      <Shell client={undefined} route="/" />
    </ConsoleI18n>,
  );

  await expect
    .element(page.getByRole("status"))
    .toHaveTextContent(
      "Not signed in: run goodfetch-collie console to open the Console signed in.",
    );
});
