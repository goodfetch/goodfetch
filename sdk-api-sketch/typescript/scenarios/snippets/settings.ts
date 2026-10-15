// Snippet for Settings in code (#470), on golden scenarios 01, 10 and 12; the last two are Foxhound's,
// shown on Collie until Crawl definition and handler context (#477) gives crawls their start verb.
import { Foxhound, SettingLockedError, type ExplainedSetting, type Logger } from "@goodfetch/foxhound";

declare const log: Logger; // the program's own logger

export async function searchQuotes(): Promise<readonly ExplainedSetting[]> {
  // One `settings` option per scope, inherited beneath it. `engine` is left at `auto`, so a
  // `GOODFETCH_URL` in the environment may fill it; a value written here would win.
  await using foxhound = await Foxhound.connect({
    settings: { timeout: { navigation: 60_000 } },
  });
  // A Session's identity is set where it opens, or above it, never on a page (ADR 0021).
  await using run = await foxhound.start({
    settings: { browser: { viewport: { width: 1280, height: 720 }, locale: "en-GB" } },
  });
  const page = await run.newPage({ settings: { timeout: { action: 10_000 } } });
  // A per-call argument applies to that call alone.
  await page.goto("https://quotes.toscrape.com/search.aspx", { timeout: 20_000 });

  // @ts-expect-error A wrong key fails tsc; built at runtime, the engine refuses it with `setting_unknown`, naming `navigation`.
  await run.newPage({ settings: { timeout: { navigate: 5_000 } } });
  // @ts-expect-error The Session's identity is not a page setting (`setting_not_applicable` at runtime).
  await run.newPage({ settings: { browser: { locale: "de-DE" } } });
  // @ts-expect-error A crawl setting on a Collie Run.
  await foxhound.start({ settings: { crawl: { payload: { maxSize: 1_024 } } } });
  // @ts-expect-error A Run's own timeout is its `timeout` argument, one spelling (S2).
  await foxhound.start({ settings: { timeout: { run: 60_000 } } });

  // What the page actually runs with, and which layer set each value (golden scenario 10 asserts on it).
  return page.explainSettings(["timeout.action", "timeout.navigation", "browser.locale"]);
}

// A service reads no project file and passes its own environment map (golden scenario 12).
export async function startInService(env: Readonly<Record<string, string>>): Promise<string> {
  // Every Run on this connection may last four hours unless it says otherwise.
  await using foxhound = await Foxhound.connect({ logger: log, project: null, env, settings: { timeout: { run: 4 * 60 * 60_000 } } });
  try {
    await using run = await foxhound.start({ timeout: null });
    return run.id();
  } catch (error) {
    // An Operator policy locked the Run's timeout: refused, never silently ignored; `limit` names the policy.
    if (error instanceof SettingLockedError) log.error(error.message, { key: error.details.key, lockedBy: error.details.lockedBy });
    throw error;
  }
}
