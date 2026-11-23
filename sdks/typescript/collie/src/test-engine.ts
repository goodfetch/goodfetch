// Collie's test engine (ADRs 0069, 0090 and 0105): a real Collie engine for
// a test, an Attached engine of Collie's executable in a temporary Goodfetch
// home, which closing it removes. Rust's `goodfetch_collie::testing` and
// Python's `goodfetch_collie.testing` are its twins.
import type {
  TestEngine as ProductTestEngine,
  TestEngineOptions,
} from "@goodfetch/collie-client/testing";
import {
  COLLIE_ENGINE_PATH_VARIABLE,
  makeHome,
  partsOf,
  removeHome,
} from "@goodfetch/collie-client/runtime";
import { Collie } from "./collie.ts";

/**
 * A real Collie engine for a test: an Attached engine in a temporary
 * Goodfetch home, removed when it closes. Started by `start()`; `await
 * using` closes it.
 */
export class TestEngine implements ProductTestEngine<Collie> {
  readonly #connection: Collie;
  readonly #home: string;

  private constructor(connection: Collie, home: string) {
    this.#connection = connection;
    this.#home = home;
  }

  /**
   * Starts a test engine with `options`. It rejects with what
   * `Collie.connect()` rejects with, such as `setting_invalid` for a
   * project file that is not there, leaving no temporary home behind.
   */
  static async start(options: TestEngineOptions = {}): Promise<TestEngine> {
    const home = await makeHome();
    try {
      const parts = partsOf(options, home, COLLIE_ENGINE_PATH_VARIABLE);
      const connection = await Collie.connect({
        env: parts.env,
        engine: "attached",
        settings: parts.settings,
        project: parts.project,
      });
      return new TestEngine(connection, home);
    } catch (error) {
      await removeHome(home);
      throw error;
    }
  }

  /** The connection to the engine, on which the code under test runs unchanged. */
  get connection(): Collie {
    return this.#connection;
  }

  /**
   * Closes the connection, then removes the temporary Goodfetch home, even
   * when closing the connection rejects.
   */
  async close(): Promise<void> {
    try {
      await this.#connection.close();
    } finally {
      await removeHome(this.#home);
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}
