// Snippet for Mapping the catalog to each language and surface (#471, ADR 0099): the shared Connection
// type, the raw catalog call on a connection and on a value, and names spelled by TypeScript's rule.
import { Goodfetch, type Connection, type JsonValue, type Page, type Run } from "goodfetch";

// A library written for both Products takes a Connection: `Collie` and `Foxhound` both implement it.
export async function engineVersion(connection: Connection): Promise<string> {
  return connection.engine.version; // fixed when the connection opened: a property
}

// The way down (G8): an operation this SDK does not generate yet, sent as the wire's snake_case JSON.
// `run.set_status_message` is area 3's (story RT-15); the Run fills its own `run` parameter.
export async function reportProgress(run: Run, done: number): Promise<void> {
  await run.call("run.set_status_message", { message: `${done} pages done` });
}

// On the connection, `params` names every value itself, and the same timeout and signal apply.
export async function newerOperation(connection: Connection, run: Run, signal: AbortSignal): Promise<JsonValue> {
  return connection.call("run.checkpoint", { run: run.id }, { timeout: 120_000, signal });
}

// A value's `call` takes only its own namespace: anything else fails before it is sent.
export async function wrongNamespace(page: Page): Promise<JsonValue> {
  return page.call("run.checkpoint"); // throws a ProgramError naming `page.*`
}

// Names follow one rule: initialisms in capitals (`waitForURL`, `newCDPSession`), reserved words unescaped.
export async function names(page: Page): Promise<boolean> {
  using _route = page.route("**/api/**", (route) => route.continue({ headers: { "x-trace": "1" } }));
  await page.waitForURL("**/orders");
  await using cdp = await page.newCDPSession();
  void cdp;
  const response = await page.goto("https://books.toscrape.com/");
  return response?.ok() ?? false; // a Playwright class keeps Playwright's accessor methods
}

export async function bothProducts(): Promise<void> {
  await using gf = await Goodfetch.connect(); // placeholder for `Collie.connect()` until #483
  const connection: Connection = gf;
  await engineVersion(connection);
}
