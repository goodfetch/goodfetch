// Snippet for Entry points and lifecycle (#465): connect, open a Run, borrow the user's page, close in scope.
import { Goodfetch, type BorrowablePage, type Logger } from "goodfetch";

declare const playwrightPage: BorrowablePage; // the user's own Playwright page
declare const appLogger: Logger; // the program's own logger: console, pino or winston

export async function addStep(): Promise<void> {
  await using gf = await Goodfetch.connect({ logger: appLogger });
  await using run = await gf.collie.start();
  await using page = await run.borrow(playwrightPage);
  void page; // the AI step itself is #473's
} // closes page (ends the borrow), then run, then gf; the user's page stays open

export async function withoutAwaitUsing(): Promise<void> {
  const gf = await Goodfetch.connect({ engine: "daemon" });
  try {
    const run = await gf.collie.start();
    await run.newPage();
  } finally {
    await gf.close(); // closes the Run it opened; the Daemon keeps running
  }
}
