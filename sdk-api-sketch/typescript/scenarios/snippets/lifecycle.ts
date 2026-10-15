// Snippet for Entry points and lifecycle (#465): connect, open a Run, borrow the user's page, close in scope.
import { Collie, type BorrowablePage, type Logger } from "@goodfetch/collie";

declare const playwrightPage: BorrowablePage; // the user's own Playwright page
declare const appLogger: Logger; // the program's own logger: console, pino or winston

export async function addStep(): Promise<void> {
  await using collie = await Collie.connect({ logger: appLogger });
  await using run = await collie.start();
  await using page = await run.borrow(playwrightPage);
  void page; // the AI step itself is #473's
} // closes page (ends the borrow), then run, then collie; the user's page stays open

export async function withoutAwaitUsing(): Promise<void> {
  const collie = await Collie.connect({ engine: "daemon" });
  try {
    const run = await collie.start();
    await run.newPage();
  } finally {
    await collie.close(); // closes the Run it opened; the Daemon keeps running
  }
}
