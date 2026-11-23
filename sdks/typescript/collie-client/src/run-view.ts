// A view of a Run another program started (ADR 0102), such as a detached
// one, opened by `openRun()` on a Connection with no round trip. It takes no
// writer role, and closing it frees only the view: the Run goes on.
import { holdInRealm } from "./realm.ts";

/** A Run named by its Run ID, opened as a view: its close never stops it. */
export class RunView implements AsyncDisposable {
  readonly #id: string;

  /** @internal A Connection's `openRun()` opens a view; a program never constructs one. */
  constructor(id: string) {
    this.#id = id;
    holdInRealm(this);
  }

  /** The Run's ID, which names it for good. Read without a round trip. */
  id(): string {
    return this.#id;
  }

  /** Closes the view; idempotent. Frees only the view: the Run goes on. */
  close(): Promise<void> {
    return Promise.resolve();
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}
