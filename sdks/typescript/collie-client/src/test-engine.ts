// The testing kit's shared part (ADRs 0069, 0095 and 0105): what a test
// engine starts with, `TestEngineOptions`, and what every Product's test
// engine is, `TestEngine`, which `@goodfetch/collie-client/testing`
// exports; and what a Product's kit starts its engine with, which
// `@goodfetch/collie-client/runtime` exports to the Product packages. A
// Product's kit starts an Attached engine in a temporary Goodfetch home,
// which closing it removes, with a cleared environment, `egress.offline`
// and `browser_cache.read_only` on, short durations and no project file
// unless the test names one; every host its `network.resolve` maps to
// loopback alone is the only one a Run's requests reach, live. Importing it
// reads, patches and logs nothing (ADR 0078). Rust's `testing` module of
// `goodfetch-collie-client` and Python's `goodfetch_collie_client.testing`
// are its twins.
import { mkdtemp, rm } from "node:fs/promises";
import { BlockList, isIP } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { HOME_VARIABLE, homeOf } from "./engine-choice.ts";
import type { Settings } from "./generated/settings.ts";
import { BROWSERS_PATH_VARIABLE, browserCache } from "./installed-executables.ts";
import { processEnvironment } from "./product-connection.ts";

/**
 * How long the test engine's calls wait where the catalog's defaults wait
 * 30 s, in milliseconds: a page's actions, navigations, waits and first
 * bytes, and a Run's close, so a test that went wrong fails in seconds. A
 * test's own settings take the place of each.
 */
const SHORT = 5_000;

/** The loopback addresses, IPv4-mapped IPv6 ones included, which `BlockList` matches. */
const LOOPBACK = new BlockList();
LOOPBACK.addSubnet("127.0.0.0", 8, "ipv4");
LOOPBACK.addAddress("::1", "ipv6");

/** What a test engine starts with, each left out for the kit's isolations alone. */
export interface TestEngineOptions {
  /**
   * The connection's settings, over the kit's: `network.resolve`'s hosts
   * mapped to loopback alone are the hosts its Runs reach, live, and a key
   * it gives, such as `timeout.navigation`, takes the place of the kit's.
   */
  readonly settings?: Settings | undefined;
  /**
   * The environment map the kit reads `GOODFETCH_HOME`,
   * `GOODFETCH_BROWSERS_PATH` and its Product's engine-path variable from,
   * and no other variable, in place of the process's own, read at
   * `start()` and never at import; the engine's own environment stays
   * cleared.
   */
  readonly env?: Readonly<Record<string, string>> | undefined;
  /**
   * The project file to read (ADR 0083), a relative one from the working
   * directory, in place of none.
   */
  readonly project?: string | undefined;
}

/**
 * A Product's test engine, whose connection `C` is the Product's own
 * Connection type: closing it closes the connection, then removes its
 * temporary Goodfetch home; `await using` closes it.
 */
export interface TestEngine<C> extends AsyncDisposable {
  /** The Product's connection, on which the code under test runs unchanged. */
  readonly connection: C;
  /**
   * Closes the connection, then removes the temporary Goodfetch home, even
   * when closing the connection rejects.
   */
  close(): Promise<void>;
}

/** What a Product's kit connects its test engine with, which `partsOf` answers. */
export interface TestEngineParts {
  /** The engine's environment map, cleared but for its home, browsers and engine. */
  readonly env: Readonly<Record<string, string>>;
  /** The connection's settings, the test's over the kit's. */
  readonly settings: Settings;
  /** The project file the test named, else `null`, which reads none. */
  readonly project: string | null;
}

/**
 * What a Product's kit connects with in the temporary Goodfetch home
 * `home`: the caller's variables come from `options`' `env`, else the
 * process's environment, read here; `enginePathVariable` is the Product's.
 */
export function partsOf(
  options: TestEngineOptions,
  home: string,
  enginePathVariable: string,
): TestEngineParts {
  const caller = options.env ?? processEnvironment();
  return {
    env: engineEnvironment(caller, home, enginePathVariable),
    settings: isolated(options.settings ?? {}),
    project: options.project ?? null,
  };
}

/**
 * The engine's environment map, every other variable of `caller` left
 * out: `GOODFETCH_HOME` names `home`, `GOODFETCH_BROWSERS_PATH` the
 * Browser cache of `caller`, its own variable's or its Goodfetch home's,
 * and `enginePathVariable` is as `caller` sets it.
 */
export function engineEnvironment(
  caller: Readonly<Record<string, string>>,
  home: string,
  enginePathVariable: string,
): Record<string, string> {
  const environment: Record<string, string> = { [HOME_VARIABLE]: home };
  const callerHome = homeOf(caller);
  const named = caller[BROWSERS_PATH_VARIABLE];
  if (callerHome !== null) {
    environment[BROWSERS_PATH_VARIABLE] = browserCache(callerHome, caller);
  } else if (named !== undefined && named !== "") {
    environment[BROWSERS_PATH_VARIABLE] = named;
  }
  const engine = caller[enginePathVariable];
  if (engine !== undefined && engine !== "") {
    environment[enginePathVariable] = engine;
  }
  return environment;
}

/**
 * `settings` over the kit's isolations, `settings` itself left unchanged:
 * `egress.offline` on, the hosts `network.resolve` maps to loopback alone
 * as the only hosts allowed and the private hosts a page may reach,
 * `browser_cache.read_only` on, so the caller's Browser cache is left as
 * the test found it (issue 3458), and the short durations.
 */
export function isolated(settings: Settings): Settings {
  const loopback = loopbackHosts(settings.network?.resolve);
  return {
    ...settings,
    browserCache: { readOnly: true, ...settings.browserCache },
    egress: {
      offline: true,
      allowedDomains: loopback,
      privateRanges: [...loopback],
      ...settings.egress,
    },
    timeout: {
      action: SHORT,
      navigation: SHORT,
      wait: SHORT,
      firstByte: SHORT,
      close: SHORT,
      ...settings.timeout,
    },
  };
}

/** The hosts `resolve` maps to addresses that are all loopback, sorted. */
function loopbackHosts(
  resolve: { readonly [host: string]: readonly string[] } | undefined,
): string[] {
  return Object.entries(resolve ?? {})
    .filter(([, addresses]) => addresses.length > 0 && addresses.every(isLoopback))
    .map(([host]) => host)
    .toSorted();
}

/** Whether `address` is a loopback IP address, an IPv4-mapped one included. */
function isLoopback(address: string): boolean {
  const version = isIP(address);
  return version !== 0 && LOOPBACK.check(address, version === 4 ? "ipv4" : "ipv6");
}

/** A new temporary Goodfetch home, which `removeHome` removes. */
export async function makeHome(): Promise<string> {
  return await mkdtemp(join(tmpdir(), "goodfetch-test-engine-"));
}

/** Removes the temporary Goodfetch home `home` and all it holds. */
export async function removeHome(home: string): Promise<void> {
  await rm(home, { recursive: true, force: true });
}
