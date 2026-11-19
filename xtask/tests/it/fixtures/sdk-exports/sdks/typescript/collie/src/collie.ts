import type { StartOptions } from "./options.ts";

export class Collie {
  #closed = false;

  private constructor() {}

  static connect(): Collie {
    return new Collie();
  }

  start(options?: StartOptions): string {
    return options?.runId ?? "";
  }

  close(): void {
    this.#closed = true;
  }

  async [Symbol.asyncDispose](): Promise<void> {
    this.close();
  }

  private hidden(): boolean {
    return this.#closed;
  }
}
