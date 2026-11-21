// What a program can and cannot write in a scope's `settings` (ADR 0083):
// `tsc` fails on a wrong one, as the package's typecheck runs it.
import {
  Collie,
  type PageSettings,
  type Run,
  type RunSettings,
  type SessionSettings,
  type Settings,
} from "@goodfetch/collie";

declare const run: Run;

// Each segment in camelCase, nested as the wire nests the dotted key.
export const runSettings: RunSettings = {
  timeout: { navigation: 30_000 },
  filterLists: { firstPartyScripts: true },
};
export const sessionSettings: SessionSettings = { browser: { accessMode: "stealth" } };
export const pageSettings: PageSettings = { content: { extractor: "trafilatura" } };
// The connection's settings hold a Run's own timeout, as its Runs' default.
export const connectionSettings: Settings = { timeout: { run: 60_000 } };
// `null` clears a setting that can be cleared.
export const cleared: SessionSettings = { proxy: { server: null } };

// @ts-expect-error a Run's own timeout is its start's `timeout`, never its settings
export const runTimeout: RunSettings = { timeout: { run: 60_000 } };
// @ts-expect-error a Session's Access mode is chosen as it opens, never on its page
export const pageAccessMode: PageSettings = { browser: { accessMode: "stealth" } };
// @ts-expect-error a key the catalog does not have
export const unknownKey: RunSettings = { timeout: { navigationDelay: 1 } };
// @ts-expect-error the wire's snake_case is not the TypeScript spelling
export const snake: RunSettings = { filterLists: { first_party_scripts: true } };
// @ts-expect-error a value outside the setting's own
export const wrongValue: SessionSettings = { browser: { accessMode: "fast" } };
// @ts-expect-error `undefined` is never a value: leave the key out
export const notAValue: RunSettings = { timeout: { navigation: undefined } };
// @ts-expect-error `null` clears only a setting that can be cleared
export const notClearable: RunSettings = { timeout: { navigation: null } };
// @ts-expect-error configuration alone sets the proxy providers
export const configurationOnly: RunSettings = { proxy: { providers: {} } };

// Opening a Session or a page takes its own scope's settings.
export async function opened(): Promise<void> {
  const session = await run.newSession({ settings: { browser: { accessMode: "standard" } } });
  await session.newPage({ settings: { content: { extractor: "dom_smoothie" } } });
  // @ts-expect-error a Session's page takes no Access mode
  await session.newPage({ settings: { browser: { accessMode: "standard" } } });
}

// `connect()` takes the connection's settings, and refuses a key no
// program's code sets.
export async function connected(): Promise<void> {
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await Collie.connect({ settings: { timeout: { run: 60_000 } } });
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await Collie.connect({
    // @ts-expect-error configuration alone sets the proxy providers
    settings: { proxy: { providers: {} } },
  });
}

// The Client's own bounds are the connection's settings (issue 2518), never
// options of `connect()`, and never a Run's settings.
export const connectionBounds: Settings = {
  connection: { maxPendingCalls: 100, maxQueuedWriteBytes: 1_048_576, maxListenerQueue: 64 },
};
// @ts-expect-error a Client's bound is set in `connect()`'s settings alone
export const runBound: RunSettings = { connection: { maxPendingCalls: 100 } };
export async function bounded(): Promise<void> {
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await Collie.connect({
    // @ts-expect-error `maxPendingCalls` is `connection.maxPendingCalls` in `settings`
    maxPendingCalls: 100,
  });
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await Collie.connect({
    // @ts-expect-error `maxQueuedWriteBytes` is `connection.maxQueuedWriteBytes` in `settings`
    maxQueuedWriteBytes: 1_048_576,
  });
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await Collie.connect({
    // @ts-expect-error `maxListenerQueue` is `connection.maxListenerQueue` in `settings`
    maxListenerQueue: 64,
  });
}
