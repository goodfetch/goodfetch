// An error state served by Collie's executable (spec #592; ADRs 0073 and
// 0079): Engine & settings explaining with a run value no Product knows
// shows the engine's refusal with the catalog's message for its Error code,
// the code itself and the engine's fields as data under `translate="no"`.
// The Release suite's elsewhere and translation projects run it again, in
// a Turkish browser in Nepal's time zone and under emulated page
// translation (ADR 0073).
import { expect, test } from "@goodfetch/test-support/playwright";
import { effectiveSettings } from "#tests/effective-settings.ts";

test(
  "a refused explanation shows the catalog's message for its Error code, its fields as data",
  {
    annotation: [
      { type: "covers", description: "story:12-133" },
      { type: "covers", description: "screen:/settings" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await signIn.withLink(page);
    await page.goto(new URL("/settings", daemon.url).href);
    await expect(effectiveSettings(page)).toBeVisible();

    await page.getByLabel("Run values").fill("timeout.navigation_ = 1000");
    await page.getByRole("button", { name: "Explain" }).click();

    const alert = page.getByRole("alert");
    await expect(alert).toContainText("The engine could not explain the settings:");
    await expect(alert).toContainText("No Product knows this setting.");
    const code = alert.getByText("setting_unknown", { exact: true });
    await expect(code).toBeVisible();
    expect(await code.evaluate((element) => element.closest('[translate="no"]') !== null)).toBe(
      true,
    );
  },
);
