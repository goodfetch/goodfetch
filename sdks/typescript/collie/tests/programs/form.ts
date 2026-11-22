// The form program (#2400): a Run started, a page opened in it and sent to
// a form page, a field filled, a box clicked, Enter pressed on the button,
// the page's Markdown read once the page shows its end state, then the page
// closed and the Run finished. The unit tests run it on the Scripted engine
// over the in-memory transport, the integration tests on an Attached engine
// of the executable of the same commit, on the Proving ground's form page
// (ADR 0069): the same code on both.
import type { Collie } from "@goodfetch/collie";

/** What the form program saw. */
export interface FormFilled {
  /** The status of the form page's document. */
  readonly status: number | undefined;
  /** The page's Markdown once its form was sent. */
  readonly markdown: string | undefined;
  /** How the Run ended. */
  readonly lifecycle: string;
}

/** Fills and sends the form at `url` on a page of a new Run of `collie`, then closes the Connection. */
export async function fillTheForm(collie: Collie, url: string): Promise<FormFilled> {
  const run = await collie.start();
  const page = await run.newPage();
  const navigated = await page.goto(url);
  await page.locator("#name").fill("Goodfetch");
  await page.locator("#consent").click();
  await page.locator("#submit").press("Enter");
  await page.locator("#done:not(:empty)").waitFor();
  const formats = await page.formats({ markdown: { content: "full" } });
  await page.close();
  const report = await run.finish();
  await collie.close();
  return {
    status: navigated?.status,
    markdown: formats.markdown,
    lifecycle: report.lifecycle,
  };
}
