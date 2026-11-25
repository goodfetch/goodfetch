import type { Collie } from "@goodfetch/collie";

/** Opens `url` in a browser page of a new Run on `collie` and answers the page's title. */
export async function readTitle(collie: Collie, url: string): Promise<string> {
  const run = await collie.start();
  const page = await run.newPage();
  await page.goto(url);
  const title = await page.evaluate("() => document.title");
  await run.finish();
  return String(title);
}
