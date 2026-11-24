// The competitor adapter protocol, as the runner speaks it: one JSON line a
// message on the adapter's stdin and stdout, each request `{version: 1, id,
// method, params}` answered by `{version: 1, id, result}` or `{version: 1,
// id, error}`. The methods are `hello` (the task kinds it claims and its
// concurrency), `prepare`, `run` (one task) and `close`. Carried over from
// the lab's worker client and worker slot.
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { once } from "node:events";
import { BenchError } from "./errors.ts";
import { parserOf } from "./schema.ts";

/** How the runner starts one adapter. */
export interface AdapterCommand {
  /** The program and its arguments. */
  readonly command: readonly [string, ...string[]];
  readonly cwd: string;
  /** How long a call may take before the adapter is stopped. */
  readonly timeoutMs: number;
}

type Response =
  | { readonly version: 1; readonly id: number; readonly result: unknown }
  | { readonly version: 1; readonly id: number; readonly error: string };

const response = parserOf<Response>({
  type: "object",
  required: ["version", "id"],
  properties: {
    version: { const: 1 },
    id: { type: "integer" },
    result: true,
    error: { type: "string" },
  },
  oneOf: [{ required: ["result"] }, { required: ["error"] }],
});

/** What an adapter says of itself in answer to `hello`. */
export interface Hello {
  /** The task kinds it claims, such as `extract`. */
  readonly capabilities: readonly string[];
  /** The most calls it takes at once. */
  readonly max_concurrency?: number;
}

const hello = parserOf<Hello>({
  type: "object",
  required: ["capabilities"],
  properties: {
    capabilities: { type: "array", items: { type: "string" } },
    max_concurrency: { type: "integer", minimum: 1 },
  },
});

/** Most bytes an adapter may write before it is stopped. */
const outputLimit = 8_000_000;
/** Most calls pending at once on one adapter. */
const pendingLimit = 128;

interface Pending {
  readonly resolve: (value: unknown) => void;
  readonly reject: (error: Error) => void;
  readonly timer: NodeJS.Timeout;
}

/** One running adapter process and its pending calls. */
export class AdapterClient {
  readonly pid: number;
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly timeoutMs: number;
  private readonly pending = new Map<number, Pending>();
  private sequence = 0;
  private stopped = false;
  private buffer = "";

  constructor(spec: AdapterCommand) {
    this.timeoutMs = spec.timeoutMs;
    const [program, ...args] = spec.command;
    this.child = spawn(program, args, {
      cwd: spec.cwd,
      env: process.env,
      stdio: ["pipe", "pipe", "pipe"],
      detached: true,
    });
    this.pid = this.child.pid ?? 0;
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (data: string) => {
      this.read(data);
    });
    // Its stderr is drained so a chatty adapter never blocks on a full pipe.
    this.child.stderr.resume();
    this.child.on("error", (error) => {
      this.fail(new BenchError("adapter_exit", error.message, { cause: error }));
    });
    this.child.on("exit", (code, signal) => {
      this.fail(new BenchError("adapter_exit", `adapter exited (${code ?? signal})`));
    });
    this.child.stdin.on("error", (error) => {
      this.fail(new BenchError("adapter_exit", error.message, { cause: error }));
    });
  }

  /** Whether the process was stopped: no call is taken any more. */
  get closed(): boolean {
    return this.stopped;
  }

  /** Calls `method`; a call past its time stops the adapter and fails as `adapter_timeout`. */
  call(method: string, params: unknown, timeoutMs: number = this.timeoutMs): Promise<unknown> {
    if (this.stopped) {
      return Promise.reject(new BenchError("adapter_closed", "adapter closed"));
    }
    if (this.pending.size >= pendingLimit) {
      return Promise.reject(new BenchError("capacity", "adapter outstanding request limit"));
    }
    this.sequence += 1;
    const id = this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.fail(new BenchError("adapter_timeout", `adapter timeout (${method})`), id);
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.child.stdin.write(`${JSON.stringify({ version: 1, id, method, params })}\n`);
    });
  }

  /** Asks the adapter to close, then stops its process group. */
  async close(): Promise<void> {
    if (!this.stopped) {
      try {
        await this.call("close", {}, 1000);
      } catch {
        // It is stopped below either way.
      }
    }
    this.fail(new BenchError("adapter_closed", "adapter closed"));
    if (this.child.exitCode === null && this.child.signalCode === null) {
      const deadline = AbortSignal.timeout(400);
      await once(this.child, "exit", { signal: deadline }).catch(() => undefined);
    }
    this.signal("SIGKILL");
    this.child.stdin.destroy();
    this.child.stdout.destroy();
  }

  private read(data: string): void {
    this.buffer += data;
    if (this.buffer.length > outputLimit) {
      this.fail(new BenchError("output_limit", "adapter output exceeds the limit"));
      return;
    }
    let end = this.buffer.indexOf("\n");
    while (end >= 0) {
      const line = this.buffer.slice(0, end);
      this.buffer = this.buffer.slice(end + 1);
      let message: Response;
      try {
        message = response.parse(JSON.parse(line), "adapter response");
      } catch (error) {
        this.fail(new BenchError("adapter_protocol", "invalid adapter response", { cause: error }));
        return;
      }
      const wait = this.pending.get(message.id);
      if (wait !== undefined) {
        clearTimeout(wait.timer);
        this.pending.delete(message.id);
        if ("error" in message) {
          wait.reject(new BenchError("adapter_error", message.error));
        } else {
          wait.resolve(message.result);
        }
      }
      end = this.buffer.indexOf("\n");
    }
  }

  private signal(signal: NodeJS.Signals): void {
    if (this.pid === 0) {
      return;
    }
    try {
      process.kill(-this.pid, signal);
    } catch {
      // The group has already exited.
    }
  }

  /**
   * Stops the adapter. Only the call that ran past its time (`rootId`)
   * carries the timeout: the calls in flight beside it fail as
   * `sibling_abort`, so a failure rate never counts them.
   */
  private fail(error: BenchError, rootId?: number): void {
    if (this.stopped) {
      return;
    }
    this.stopped = true;
    for (const [id, wait] of this.pending) {
      clearTimeout(wait.timer);
      wait.reject(
        rootId === undefined || id === rootId
          ? error
          : new BenchError("sibling_abort", `aborted by ${error.message}`, { cause: error }),
      );
    }
    this.pending.clear();
    this.signal("SIGTERM");
    setTimeout(() => {
      this.signal("SIGKILL");
    }, 300).unref();
  }
}

/** What an adapter slot starts its adapter with, and how often it may restart it. */
export interface SlotSpec {
  readonly adapter: AdapterCommand;
  /** The parameters of `prepare`. */
  readonly prepare: unknown;
  readonly maxRestarts: number;
  /** The campaign's concurrency, which the adapter must take. */
  readonly concurrency: number;
  /** Called with each new process's ID, before `hello`. */
  readonly started: (pid: number) => void;
}

/**
 * One adapter for one block: started on first use, and restarted with a new
 * generation after a crash or a timeout, at most `maxRestarts` times.
 */
export class AdapterSlot {
  /** The current process's generation, from 0; -1 before the first. */
  generation = -1;
  capabilities: readonly string[] = [];
  /** Each start's time to its answer to `prepare`, in milliseconds. */
  readonly startupMs: number[] = [];
  private readonly spec: SlotSpec;
  private client: AdapterClient | undefined;
  private starting: Promise<AdapterClient> | undefined;
  private restarts = 0;

  constructor(spec: SlotSpec) {
    this.spec = spec;
  }

  /** The running adapter, started or restarted when it is not running. */
  async ensure(): Promise<AdapterClient> {
    // A process still starting is not ready: its callers wait for `prepare`.
    if (this.starting !== undefined) {
      return await this.starting;
    }
    if (this.client !== undefined && !this.client.closed) {
      return this.client;
    }
    this.starting ??= this.start().finally(() => {
      this.starting = undefined;
    });
    return await this.starting;
  }

  /** Stops the adapter. */
  async close(): Promise<void> {
    await this.client?.close();
  }

  private async start(): Promise<AdapterClient> {
    if (this.generation >= 0) {
      if (this.restarts >= this.spec.maxRestarts) {
        throw new BenchError(
          "restart_limit",
          `adapter restart limit ${this.spec.maxRestarts} reached`,
        );
      }
      this.restarts += 1;
    }
    this.generation += 1;
    const began = performance.now();
    const client = new AdapterClient(this.spec.adapter);
    this.client = client;
    this.spec.started(client.pid);
    try {
      const info = hello.parse(await client.call("hello", {}), "adapter hello");
      if (this.spec.concurrency > (info.max_concurrency ?? pendingLimit)) {
        throw new BenchError("capability", "the campaign exceeds the adapter's concurrency");
      }
      this.capabilities = info.capabilities;
      await client.call(
        "prepare",
        this.spec.prepare,
        Math.max(this.spec.adapter.timeoutMs, 45_000),
      );
    } catch (error) {
      await client.close();
      throw new BenchError(
        "startup_failed",
        error instanceof Error ? error.message : String(error),
        { cause: error },
      );
    } finally {
      this.startupMs.push(performance.now() - began);
    }
    return client;
  }
}
