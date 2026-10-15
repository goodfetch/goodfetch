// Snippet for Mapping the catalog to each language and surface (#471, ADR 0099): the shared Connection
// type, the raw catalog call on a connection and on a value, and names spelled by TypeScript's rule.
import { Goodfetch, type Connection, type JsonValue, type Page } from "goodfetch";

// A library written for both Products takes a Connection: `Collie` and `Foxhound` both implement it.
export async function engineVersion(connection: Connection): Promise<string> {
  return connection.engine().version; // a read on a handle is a method in TypeScript, as in Playwright
}

// The way down (G8): an operation this SDK does not generate, such as one a newer engine minor added
// (`page.emulate_vision_deficiency` is hypothetical here). The page fills its own `page` parameter, and
// the params are the wire's snake_case JSON: `type` needs no escape there.
export async function newerOperation(page: Page): Promise<void> {
  await page.call("page.emulate_vision_deficiency", { type: "deuteranopia" });
}

// On the connection, `params` names every value itself, and the same timeout and signal apply.
export async function onTheConnection(connection: Connection, runId: string, signal: AbortSignal): Promise<JsonValue> {
  return connection.call("run.migrate", { run: runId, to: "daemon" }, { timeout: 120_000, signal }); // hypothetical
}

// A value's `call` takes only its own namespace: anything else fails before it is sent.
export async function wrongNamespace(page: Page): Promise<JsonValue> {
  return page.call("run.migrate"); // throws a ProgramError naming `page.*`
}

// Names follow one rule: initialisms in capitals (`waitForURL`, `newCDPSession`), reserved words unescaped.
export async function names(page: Page): Promise<boolean> {
  using _route = page.route("**/api/**", (route) => route.continue({ headers: { "x-trace": "1" } }));
  await page.waitForURL("**/orders");
  await using cdp = await page.newCDPSession();
  void cdp;
  const response = await page.goto("https://books.toscrape.com/");
  return response?.ok() ?? false; // a read on a handle is a method, as all of Playwright's are
}

export async function bothProducts(): Promise<void> {
  await using gf = await Goodfetch.connect(); // placeholder for `Collie.connect()` until #483
  const connection: Connection = gf;
  await engineVersion(connection);
}
