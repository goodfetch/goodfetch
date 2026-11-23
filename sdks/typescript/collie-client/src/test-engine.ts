// The testing kit's shared part (ADRs 0069, 0095 and 0105): what a test
// engine starts with, `TestEngineOptions`, and what every Product's test
// engine is, `TestEngine`, which `@goodfetch/collie-client/testing`
// exports; and what a Product's kit starts its engine with, which
// `@goodfetch/collie-client/runtime` exports to the Product packages. A
// Product's kit starts an Attached engine in a temporary Goodfetch home,
// which closing it removes. Importing it reads, patches and logs nothing
// (ADR 0078). Rust's `testing` module of `goodfetch-collie-client` and
// Python's `goodfetch_collie_client.testing` are its twins.
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { HOME_VARIABLE } from "./engine-choice.ts";
import type { Settings } from "./generated/settings.ts";
import { processEnvironment } from "./product-connection.ts";

/** What a test engine starts with, each left out for the kit's isolations alone. */
export interface TestEngineOptions {
  /** The connection's settings. */
  readonly settings?: Settings | undefined;
  /**
   * The environment map the kit reads in place of the process's own, read
   * at `start()` and never at import.
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
  /** The engine's environment map. */
  readonly env: Readonly<Record<string, string>>;
  /** The connection's settings. */
  readonly settings: Settings;
  /** The project file the test named, else `null`, which reads none. */
  readonly project: string | null;
}

/**
 * What a Product's kit connects with in the temporary Goodfetch home
 * `home`: the caller's variables come from `options`' `env`, else the
 * process's environment, read here.
 */
export function partsOf(options: TestEngineOptions, home: string): TestEngineParts {
  const caller = options.env ?? processEnvironment();
  return {
    env: { ...caller, [HOME_VARIABLE]: home },
    settings: options.settings ?? {},
    project: options.project ?? null,
  };
}

/** A new temporary Goodfetch home, which `removeHome` removes. */
export async function makeHome(): Promise<string> {
  return await mkdtemp(join(tmpdir(), "goodfetch-test-engine-"));
}

/** Removes the temporary Goodfetch home `home` and all it holds. */
export async function removeHome(home: string): Promise<void> {
  await rm(home, { recursive: true, force: true });
}
