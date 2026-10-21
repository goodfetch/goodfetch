import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { ConsoleI18n } from "./i18n.tsx";
import { Shell } from "./shell.tsx";

test("the shell's first screen names the Product, then Collie", async () => {
  await render(
    <ConsoleI18n>
      <Shell />
    </ConsoleI18n>,
  );

  const main = page.getByRole("main");
  await expect
    .element(main.getByRole("heading", { level: 1 }))
    .toHaveTextContent("Goodfetch Collie");
  await expect.element(main.getByText("Collie's Console.", { exact: false })).toBeVisible();
});
