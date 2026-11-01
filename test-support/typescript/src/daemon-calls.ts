// What a served test or Journey variant asks of its test's
// Daemon outside the page: its executable's commands, in the test's home,
// and a call over its socket as an SDK makes one, after `hello`.
import { execFile } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";
import { promisify } from "node:util";
import type { ServedDaemon } from "./served-daemon.ts";

const execute = promisify(execFile);

/** Runs the executable of the test's Daemon with `args` in its home, answering what it prints. */
export async function command(daemon: ServedDaemon, args: readonly string[]): Promise<string> {
  const { stdout } = await execute(daemon.executable, [...args], { env: { ...daemon.env } });
  return stdout;
}

/** Calls `method` with `params` on the Daemon over its socket, as an SDK does, after `hello`. */
export async function callDaemon(
  daemon: ServedDaemon,
  method: string,
  params: unknown,
): Promise<unknown> {
  const info: unknown = JSON.parse(await command(daemon, ["engine", "info", "--json"]));
  const version =
    typeof info === "object" && info !== null && "product_version" in info
      ? String(info.product_version)
      : "";
  const socket = createConnection(daemon.socket);
  const lines = createInterface({ input: socket })[Symbol.asyncIterator]();
  const send = (id: number, name: string, values: unknown): void => {
    socket.write(`${JSON.stringify({ jsonrpc: "2.0", id, method: name, params: values })}\n`);
  };
  const answer = async (id: number): Promise<unknown> => {
    for (;;) {
      const line = await lines.next();
      if (line.done === true) {
        throw new Error(`the Daemon closed before answering ${method}`);
      }
      const message: unknown = JSON.parse(line.value);
      if (typeof message === "object" && message !== null && "id" in message && message.id === id) {
        if ("error" in message) {
          throw new Error(`${method} failed: ${JSON.stringify(message.error)}`);
        }
        return "result" in message ? message.result : undefined;
      }
    }
  };
  try {
    send(0, "hello", {
      protocol_version: "0.1",
      surface: "sdk_typescript",
      token: daemon.token,
      needs: "collie",
      product_version: version,
    });
    await answer(0);
    send(1, method, params);
    return await answer(1);
  } finally {
    socket.end();
  }
}

/**
 * A failure a request handler throws to answer the engine's request with an
 * Error code, as an SDK answers a handler's own exception: `handler_failed`
 * with the exception's type, message and stack in its `details`, say.
 */
export class EngineRequestFailure extends Error {
  readonly code: string;
  readonly retryable: boolean;
  readonly details: Readonly<Record<string, unknown>>;

  constructor(
    code: string,
    message: string,
    details: Readonly<Record<string, unknown>> = {},
    retryable = false,
  ) {
    super(message);
    this.code = code;
    this.retryable = retryable;
    this.details = details;
  }
}

/** How a Connection answers a request the engine sends it, such as a crawl's `crawl_request.handle`. */
export type EngineRequestHandler = (params: unknown) => unknown;

/** A Connection to the test's Daemon held open, as a program serving a crawl holds one. */
export interface DaemonConnection {
  /** Calls `method` with `params`, answering its result; a refusal throws. */
  call(method: string, params: unknown): Promise<unknown>;
  /** Closes the Connection. */
  close(): void;
}

/**
 * Opens a Connection to the Daemon over its socket, as an SDK of `needs`
 * does, after `hello`, answering each request the engine sends with the
 * result of `handlers`' function for its method, or with the Error code of
 * the `EngineRequestFailure` it throws; one with no function is refused
 * with `method_not_found`. It stays open until `close`.
 */
export async function connectDaemon(
  daemon: ServedDaemon,
  needs: "collie" | "foxhound",
  handlers: Readonly<Record<string, EngineRequestHandler>> = {},
): Promise<DaemonConnection> {
  const info: unknown = JSON.parse(await command(daemon, ["engine", "info", "--json"]));
  const version =
    typeof info === "object" && info !== null && "product_version" in info
      ? String(info.product_version)
      : "";
  const socket = createConnection(daemon.socket);
  const pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: Error) => void }
  >();
  let next = 0;
  const write = (message: unknown): void => {
    socket.write(`${JSON.stringify(message)}\n`);
  };
  const lines = createInterface({ input: socket });
  lines.on("line", (line) => {
    const message: unknown = JSON.parse(line);
    if (typeof message !== "object" || message === null) {
      return;
    }
    if ("method" in message && typeof message.method === "string") {
      if (!("id" in message)) {
        return;
      }
      const handler = handlers[message.method];
      const params = "params" in message ? message.params : undefined;
      if (handler === undefined) {
        write({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32601, message: `the test serves no ${message.method}` },
        });
        return;
      }
      const { id } = message;
      const answer = async (): Promise<void> => {
        try {
          write({ jsonrpc: "2.0", id, result: await handler(params) });
        } catch (error) {
          if (!(error instanceof EngineRequestFailure)) {
            throw error;
          }
          write({
            jsonrpc: "2.0",
            id,
            error: {
              code: -32000,
              message: error.message,
              // The protocol flattens an Error code's details into `data`, beside its code.
              data: { ...error.details, code: error.code, retryable: error.retryable },
            },
          });
        }
      };
      void answer();
      return;
    }
    if ("id" in message && typeof message.id === "number") {
      const waiting = pending.get(message.id);
      pending.delete(message.id);
      if ("error" in message) {
        waiting?.reject(
          new Error(`call ${String(message.id)} failed: ${JSON.stringify(message.error)}`),
        );
      } else {
        waiting?.resolve("result" in message ? message.result : undefined);
      }
    }
  });
  socket.on("close", () => {
    for (const waiting of pending.values()) {
      waiting.reject(new Error("the Daemon closed the Connection before answering"));
    }
    pending.clear();
  });
  const call = async (method: string, params: unknown): Promise<unknown> => {
    const id = next;
    next += 1;
    const answered = new Promise<unknown>((resolve, reject) => {
      pending.set(id, { resolve, reject });
    });
    write({ jsonrpc: "2.0", id, method, params });
    return answered;
  };
  await call("hello", {
    protocol_version: "0.1",
    surface: "sdk_typescript",
    token: daemon.token,
    needs,
    product_version: version,
  });
  return { call, close: () => socket.end() };
}
