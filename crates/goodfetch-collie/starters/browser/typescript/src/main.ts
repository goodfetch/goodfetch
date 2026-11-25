// Prints the title of the page its argument names, the Proving ground's
// when it has none: `npm start -- https://example.com/`.
import { Collie } from "@goodfetch/collie";

/** Opens `url` in a browser page of a new Run on `collie` and answers the page's title. */
export async function readTitle(collie: Collie, url: string): Promise<string> {
  const run = await collie.start();
  const page = await run.newPage();
  await page.goto(url);
  const title = await page.evaluate("() => document.title");
  await run.finish();
  return String(title);
}

// Started as the program, not imported by its test.
if (import.meta.main) {
  const url = process.argv[2] ?? "https://proving-ground.goodfetch.dev/";
  await using collie = await Collie.connect();
  console.log(await readTitle(collie, url));
}
