// The Connection every Product's Connection type is built on (ADRs 0078
// and 0095): it opens with `hello`, naming the Product the Client needs and
// its version, and holds the engine's answer; it starts Runs and closes them
// before itself; and once closed, every call fails with `closed`. The only
// check of the engine's answers is the protocol version's (STYLE.md).
import { Buffer } from "node:buffer";
import { AttachedEngine, type EngineExit } from "./attached-engine.ts";
import { errorOfReply, type ErrorClasses } from "./errors.ts";
import {
  CallCancelledError,
  ClosedError,
  CollieOperations,
  DeadlineExceededError,
  EngineExitedError,
  EngineOutputInvalidError,
  PROTOCOL_VERSION,
  ProtocolVersionUnsupportedError,
  type CallOptions,
  type Caller,
  type CollieOperationTypes,
  type GoodfetchError,
  type HelloParams,
  type HelloResult,
  type KeyValueStoreSetParams,
  type Product,
  type RunStartParams,
} from "./generated/index.ts";
import type { JsonValue } from "./key-value-store.ts";
import { holdInRealm } from "./realm.ts";
import { Run, type RunHost } from "./run.ts";
import { Channel, type Transport, type TransportEnd } from "./rpc.ts";
import { PRODUCT_VERSION } from "./version.ts";

/** How long `close()` waits for an Attached engine to stop: `timeout.close`'s default (ADR 0080). */
const CLOSE_TIMEOUT = 30_000;
/** How long an ended transport waits for its engine's exit status before reporting without it. */
const EXIT_GRACE = 2_000;

/** The engine a Connection reached, fixed when it opened (G6). */
export interface EngineInfo {
  /** How it is reached; an Attached engine is the one kind so far. */
  readonly kind: "attached" | "daemon" | "remote";
  /** The engine's Product: `foxhound` serves a Collie Connection too (ADR 0095). A newer one is kept. */
  readonly product: Product | (string & {});
  /** The engine's Product version, reported, never compared (ADR 0090). */
  readonly version: string;
  /** The protocol version `major.minor` it speaks. */
  readonly protocolVersion: string;
  /** The executable started, or `null` when none was. */
  readonly enginePath: string | null;
}

/** What starting a Run takes (ADR 0102). */
export interface StartOptions {
  /** The Run's ID; the engine generates one when it is left out. An existing ID returns that Run. */
  readonly runId?: string | undefined;
  /** Its tags: at most 16, each at most 64 characters (ADR 0036). */
  readonly tags?: readonly string[] | undefined;
}

/** What opening a Connection takes. */
export interface ClientConnectionOptions {
  /** The bytes to the engine. */
  readonly transport: Transport;
  /** The Attached engine whose stdio `transport` is, when the Connection started one. */
  readonly engine?: AttachedEngine | undefined;
  /** The Product the Client needs, which `hello` names. */
  readonly needs: Product;
  /** The error class of each code the Client's Product knows. */
  readonly errorClasses: ErrorClasses;
}

/** The common Connection under each Product's Connection type. */
export class ClientConnection implements AsyncDisposable {
  readonly #channel: Channel;
  readonly #engine: AttachedEngine | undefined;
  readonly #operations: CollieOperations;
  readonly #runs = new Set<Run>();
  readonly #runHost: RunHost;
  #info: EngineInfo | undefined;
  #ended = false;
  #closing: Promise<void> | undefined;

  private constructor(options: ClientConnectionOptions) {
    this.#engine = options.engine;
    const { errorClasses } = options;
    this.#channel = new Channel(options.transport, {
      ended: (end) => this.#endedWith(end),
      failure: (error) => errorOfReply(error, errorClasses),
    });
    this.#operations = new CollieOperations(this.#caller());
    this.#runHost = {
      finish: (runId, callOptions) => this.#operations.runFinish({ run_id: runId }, callOptions),
      abort: (runId) => this.#operations.runAbort({ run_id: runId }),
      reachable: () => !this.#ended,
      forget: (run) => this.#runs.delete(run),
      keyValueStore: {
        get: async (runId, key) => {
          const { value } = await this.#operations.keyValueStoreGet({ run_id: runId, key });
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): a stored value is JSON.
          return value as JsonValue;
        },
        set: async (runId, key, value) => {
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the generated layer types a schemaless JSON value as an object; any JSON value but null is the engine's to take.
          const stored = value as KeyValueStoreSetParams["value"];
          await this.#operations.keyValueStoreSet({ run_id: runId, key, value: stored });
        },
        getBytes: async (runId, key) => {
          const { bytes } = await this.#operations.keyValueStoreGetBytes({ run_id: runId, key });
          return bytes
            ? {
                content: new Uint8Array(Buffer.from(bytes.content, "base64")),
                contentType: bytes.content_type,
              }
            : null;
        },
        setBytes: async (runId, key, content, contentType) => {
          await this.#operations.keyValueStoreSetBytes({
            run_id: runId,
            key,
            content: Buffer.from(content.buffer, content.byteOffset, content.byteLength).toString(
              "base64",
            ),
            content_type: contentType,
          });
        },
        delete: async (runId, key) =>
          (await this.#operations.keyValueStoreDelete({ run_id: runId, key })).existed,
      },
    };
    holdInRealm(this);
  }

  /**
   * Opens a Connection over `transport` and finishes `hello` before it
   * returns: a refused Product or protocol version fails here, and the
   * transport and its engine are ended.
   */
  static async open(options: ClientConnectionOptions): Promise<ClientConnection> {
    const connection = new ClientConnection(options);
    try {
      await connection.#hello(options.needs);
    } catch (error) {
      await connection.#end();
      throw error;
    }
    return connection;
  }

  /** The engine reached. */
  engine(): EngineInfo {
    if (this.#info === undefined) {
      throw new TypeError("invariant: an open Connection has finished its hello");
    }
    return this.#info;
  }

  /** Starts a Run and returns once it is started (ADR 0102). */
  async start(options: StartOptions = {}): Promise<Run> {
    this.#openOrThrow("run.start");
    const params: RunStartParams = {
      ...(options.runId === undefined ? {} : { run_id: options.runId }),
      ...(options.tags === undefined ? {} : { tags: [...options.tags] }),
    };
    const started = await this.#operations.runStart(params);
    const run = new Run(this.#runHost, started);
    this.#runs.add(run);
    return run;
  }

  /**
   * Closes the Connection; idempotent. It closes each Run it opened first,
   * stopping one still running, cancels the calls in flight, then ends its
   * Attached engine and waits for it to stop.
   */
  close(): Promise<void> {
    this.#closing ??= this.#close();
    return this.#closing;
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  #openOrThrow(operation: string): void {
    if (this.#closing !== undefined) {
      throw new ClosedError(`${operation}: this Connection is closed; connect again`, {
        fix: "Connect again: a closed Connection never reopens.",
      });
    }
  }

  #caller(): Caller<CollieOperationTypes> {
    const channel = this.#channel;
    return {
      call<Name extends keyof CollieOperationTypes>(
        operation: Name,
        params: CollieOperationTypes[Name]["params"],
        options: CallOptions,
      ): Promise<CollieOperationTypes[Name]["result"]> {
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): the result is the operation's as the catalog types it.
        return channel.call(operation, params, options) as Promise<
          CollieOperationTypes[Name]["result"]
        >;
      },
    };
  }

  async #hello(needs: Product): Promise<void> {
    const params: HelloParams = {
      protocol_version: PROTOCOL_VERSION,
      surface: "sdk_typescript",
      needs,
      product_version: PRODUCT_VERSION,
    };
    const answer = await this.#channel.call("hello", params);
    const spoken = protocolVersionOf(answer);
    if (spoken === undefined || !speaks(spoken)) {
      throw new ProtocolVersionUnsupportedError(
        spoken === undefined
          ? `this Client speaks protocol ${PROTOCOL_VERSION}, and the engine's answer names no protocol version`
          : `this Client speaks protocol ${PROTOCOL_VERSION}, and the engine answered with ${spoken}`,
        {
          fix:
            spoken === undefined
              ? `Use the engine and the SDK of one Product version: this SDK ${PRODUCT_VERSION} speaks protocol ${PROTOCOL_VERSION}.`
              : `Use the engine and the SDK of one Product version: this SDK ${PRODUCT_VERSION} speaks protocol ${PROTOCOL_VERSION}, and the engine ${productVersionOf(answer)} speaks ${spoken}.`,
          details: {
            client_protocol_version: PROTOCOL_VERSION,
            engine_protocol_version: spoken ?? null,
          },
        },
      );
    }
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the one answer checked is the protocol version it speaks, above (STYLE.md).
    const result = answer as HelloResult;
    this.#info = {
      kind: "attached",
      product: result.product,
      version: result.product_version,
      protocolVersion: result.protocol_version,
      enginePath: this.#engine?.path ?? null,
    };
  }

  /**
   * Closes the Runs, cancels the calls in flight and ends the engine, all
   * within `timeout.close`. Past it, whatever is still waiting is cancelled,
   * an Attached engine is killed, and the close fails with
   * `deadline_exceeded`.
   */
  async #close(): Promise<void> {
    const startedAt = Date.now();
    const closed = await within(this.#closeInOrder(), CLOSE_TIMEOUT);
    if (closed === "late") {
      this.#channel.cancelAll(cancelledByClose());
      this.#engine?.kill();
      await this.#channel.close();
      await this.#engine?.exited;
      throw new DeadlineExceededError(
        `the Connection did not close within timeout.close (${CLOSE_TIMEOUT} ms), so its engine was stopped`,
        {
          code: "deadline_exceeded",
          group: "deadline_exceeded",
          retryable: true,
          fix: "Raise timeout.close, or find why the engine stopped answering in the engine's stderr, which the logger got.",
          details: {
            limit: { key: "timeout.close", value: CLOSE_TIMEOUT, layer: "default" },
            elapsed: Date.now() - startedAt,
            waited_for: "close",
          },
        },
      );
    }
    if (closed.failure !== undefined) {
      throw closed.failure;
    }
  }

  /** Closes each Run, then cancels the calls in flight and ends the engine; answers the first Run's failure. */
  async #closeInOrder(): Promise<{ readonly failure: unknown }> {
    let failure: unknown;
    for (const run of this.#runs) {
      try {
        await run.close();
      } catch (error) {
        failure ??= error;
      }
    }
    this.#channel.cancelAll(cancelledByClose());
    await this.#channel.close();
    // An Attached engine stops at the end of its input.
    await this.#engine?.exited;
    await this.#channel.ended;
    return { failure };
  }

  /** Ends the Client's side and waits for the engine to stop. */
  async #end(): Promise<void> {
    await this.#channel.close();
    if (this.#engine !== undefined) {
      await this.#engine.stop(CLOSE_TIMEOUT);
    }
    await this.#channel.ended;
  }

  async #endedWith(end: TransportEnd): Promise<GoodfetchError> {
    this.#ended = true;
    if (end.kind === "invalid") {
      this.#engine?.kill();
      return new EngineOutputInvalidError(
        "the engine wrote a line that is not one JSON-RPC message, so the Connection ended",
        { details: { line: end.line } },
      );
    }
    if (this.#closing !== undefined) {
      return new ClosedError("this Connection is closed; connect again", {
        fix: "Connect again: a closed Connection never reopens.",
      });
    }
    const exit = await this.#exitOfEngine();
    return new EngineExitedError(
      exit.exitCode === null && exit.signal === null
        ? "the engine ended before its Connection was closed"
        : `the engine ended before its Connection was closed, with ${exit.signal ?? `exit code ${exit.exitCode}`}`,
      {
        details: { exit_code: exit.exitCode, signal: exit.signal, stderr_tail: exit.stderrTail },
        cause: end.kind === "failed" ? end.cause : undefined,
      },
    );
  }

  async #exitOfEngine(): Promise<EngineExit> {
    const unknown: EngineExit = { exitCode: null, signal: null, stderrTail: "" };
    if (this.#engine === undefined) {
      return unknown;
    }
    const exit = await within(this.#engine.exited, EXIT_GRACE);
    return exit === "late" ? unknown : exit;
  }
}

/** What `promise` settles to, or `"late"` when it has not within `timeout` milliseconds. */
async function within<T>(promise: Promise<T>, timeout: number): Promise<T | "late"> {
  let timer: NodeJS.Timeout | undefined;
  const late = new Promise<"late">((resolve) => {
    timer = setTimeout(() => resolve("late"), timeout);
  });
  try {
    return await Promise.race([promise, late]);
  } finally {
    clearTimeout(timer);
  }
}

/** The error a call in flight fails with when its Connection closes: cancelled by its Client. */
function cancelledByClose(): CallCancelledError {
  return new CallCancelledError("the call's Connection was closed", {
    details: { by: "client" },
  });
}

/** The protocol version a `hello` answer names, when it names one. */
function protocolVersionOf(answer: unknown): string | undefined {
  const version: unknown =
    typeof answer === "object" && answer !== null && "protocol_version" in answer
      ? answer.protocol_version
      : undefined;
  return typeof version === "string" ? version : undefined;
}

/** The Product version a `hello` answer names, for a message. */
function productVersionOf(answer: unknown): string {
  const version: unknown =
    typeof answer === "object" && answer !== null && "product_version" in answer
      ? answer.product_version
      : undefined;
  return typeof version === "string" ? version : "of an unknown version";
}

/** Whether an engine of `version` takes this Client: the same major, the same or a newer minor (ADR 0030). */
function speaks(version: string): boolean {
  const [major, minor] = version.split(".").map(Number);
  const [ownMajor, ownMinor] = PROTOCOL_VERSION.split(".").map(Number);
  return (
    major !== undefined &&
    minor !== undefined &&
    major === ownMajor &&
    ownMinor !== undefined &&
    minor >= ownMinor
  );
}
