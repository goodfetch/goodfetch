// The in-memory adapter of the protocol (ADR 0068; spec #326): the engine a
// Console component or screen test passes to the Console's Client as its
// transport and body fetcher. It answers each operation with the value the
// test set, in whatever order the screen calls, answers `hello`,
// `events.subscribe` and, unless the test set an answer, `engine.info`
// itself, as the engine does, and refuses, failing the test, anything the
// engine could never send or take: every message the
// Client sends and every answer the test sets is checked against the
// committed JSON Schema bundle and catalog. It is not the Scripted engine
// (ADR 0069), which plays one ordered script.
//
// The `transport` and `bodyFetcher` are the shapes the Console's Client
// takes (spec #592): a transport opens Connections that carry JSON-RPC
// text, as the Console's same-origin WebSocket does, and a body fetcher
// reads a body's bytes by its handle.
import type {
  CollieEventPayloads,
  CollieOperationTypes,
  NeedsYouChange,
  NeedsYouItem,
  OperationTypes,
  Product,
} from "@goodfetch/collie-client/generated";
import { onTestFinished } from "vitest";
import { TestSupportError, type TestSupportErrorCode } from "./errors.ts";
import {
  productContract,
  type ProductContract,
  type Problems,
  type RequestId,
  type WireNotification,
  type WireRequest,
} from "./protocol-contract.ts";

export type { RequestId } from "./protocol-contract.ts";

/** What a Client hears from its open Connection. */
export interface ConnectionListener {
  /** A message the engine sent: the text of one JSON-RPC message, as a WebSocket carries it. */
  message(text: string): void;
  /** The Connection closed: the engine dropped it, or the Client closed it. */
  close(): void;
}

/** A Client's open link to the adapter. */
export interface Connection {
  /** Sends the text of one JSON-RPC message. Text sent once closed is discarded, as a closed WebSocket's is. */
  send(text: string): void;
  /** Closes the Connection; its listener then hears `close`. */
  close(): void;
}

/** What the Console's Client opens its Connections with. */
export interface Transport {
  /** Opens a Connection to the adapter, which delivers each message to `listener` after the current task. */
  open(listener: ConnectionListener): Connection;
}

/** What the Console's Client reads a page body, a screenshot or an Export with: its bytes, by handle. */
export type BodyFetcher = (handle: string) => Promise<Uint8Array>;

/** An answer that fails the call with an Error code of the catalog. */
export class Failure {
  readonly code: string;
  readonly details: Readonly<Record<string, unknown>>;

  constructor(code: string, details: Readonly<Record<string, unknown>>) {
    this.code = code;
    this.details = details;
  }
}

/**
 * An answer that fails the call with the catalog's Error code `code`, its
 * group's class and its `retryable`, and the code's own `details`.
 */
export function failure(code: string, details: Readonly<Record<string, unknown>> = {}): Failure {
  return new Failure(code, details);
}

/** What a call of an operation of `Types` answers: its result or a failure. */
export type Outcome<Types extends OperationTypes> = Types["result"] | Failure;

/** What a test sets an operation's calls to answer: a value, or a function of each call's params. */
export type Answer<Types extends OperationTypes> =
  | Outcome<Types>
  | ((params: Types["params"]) => Outcome<Types> | Promise<Outcome<Types>>);

/** A message the Client sent, as the adapter read it. */
export interface ClientMessage {
  readonly method: string;
  /** A request's id; a notification has none. */
  readonly id?: RequestId | undefined;
  readonly params?: unknown;
}

/** How a test sets up the adapter. */
export interface ProtocolAdapterOptions {
  /** The Product whose engine the adapter stands for: Collie's unless the screen is Foxhound's (ADR 0094). */
  readonly product?: Product | undefined;
}

/** The operation table of a Product: each operation's params and result. */
export type OperationTable<Operations> = { readonly [Name in keyof Operations]: OperationTypes };

/** The event the adapter sends itself, when a subscription resumes after a position it no longer holds or another engine instance gave. */
const RESYNC = "events.resync";

/** An event a test pushes: any of its Product's but `events.resync`, with its payload. */
export type PushedEvent<Events> = {
  readonly [Name in Exclude<keyof Events & string, typeof RESYNC>]: {
    readonly event: Name;
    readonly data: Events[Name];
  };
}[Exclude<keyof Events & string, typeof RESYNC>];

/** The Product version the adapter answers `hello` with. */
const PRODUCT_VERSION = "0.1.0";

/** The command that installs Foxhound, which `not_in_product`'s fix names: npm's, the channel of the TypeScript SDK (ADR 0090). */
const FOXHOUND_INSTALL = "npm install @goodfetch/foxhound";

/** The major of the Product version the adapter answers `hello` with, which `engine.info` names. */
const PRODUCT_MAJOR = 0;

/** One event of a `$/events` batch. */
interface EventEnvelope {
  readonly position: number;
  readonly event: string;
  readonly data: unknown;
}

/** A subscription in flight: the events it has not sent yet, and the credits its Client granted. */
interface Subscription {
  readonly id: RequestId;
  credits: number;
  readonly queue: EventEnvelope[];
}

/** One open Connection, as the adapter keeps it. */
interface OpenConnection {
  readonly listener: ConnectionListener;
  readonly subscriptions: Map<RequestId, Subscription>;
  open: boolean;
}

/** The `n`th engine instance the adapter stands for, as `hello` names it. */
function engineInstance(n: number): string {
  return n.toString(16).padStart(16, "0");
}

/** The engine instance and position of a subscription's `after`, if it names them. */
function lastRead(after: unknown): { engineInstance: string; position: number } | undefined {
  return typeof after === "object" &&
    after !== null &&
    "engine_instance" in after &&
    typeof after.engine_instance === "string" &&
    "position" in after &&
    typeof after.position === "number"
    ? { engineInstance: after.engine_instance, position: after.position }
    : undefined;
}

/** The Run ID of a `run.axes_changed` payload. */
function runIdOf(data: unknown): string | undefined {
  return typeof data === "object" &&
    data !== null &&
    "run_id" in data &&
    typeof data.run_id === "string"
    ? data.run_id
    : undefined;
}

/** The `lifecycle` an event's payload names, if it names one. */
function lifecycleOf(data: unknown): string | undefined {
  return typeof data === "object" &&
    data !== null &&
    "lifecycle" in data &&
    typeof data.lifecycle === "string"
    ? data.lifecycle
    : undefined;
}

/**
 * The in-memory adapter of the protocol. Create it inside a test (or a
 * fixture), through {@link createProtocolAdapter}.
 */
export class ProtocolAdapter<
  Operations extends OperationTable<Operations> = CollieOperationTypes,
  Events = CollieEventPayloads,
> {
  /** The Console Client's transport. */
  readonly transport: Transport;
  /** Every message the Client sent, in order, once the bundle accepted its envelope. */
  readonly requests: readonly ClientMessage[];
  readonly #requests: ClientMessage[] = [];
  readonly #contract: ProductContract;
  /** Foxhound's contract, read once a call names what Collie's lacks. */
  #foxhound: ProductContract | undefined;
  readonly #answers = new Map<string, Answer<OperationTypes>>();
  readonly #violations: TestSupportError[] = [];
  readonly #connections = new Set<OpenConnection>();
  /** The Console Client's body fetcher: the bytes `serve` gave each handle. */
  readonly bodyFetcher: BodyFetcher;
  readonly #bodies = new Map<string, Uint8Array>();
  /** The events pushed since positions were last forgotten, which a subscription resumes among. */
  #retained: EventEnvelope[] = [];
  /** The last position given. */
  #last = 0;
  /** The last position no longer kept: a subscription resumes after any position from it to `#last`. */
  #floor = 0;
  /** Each live Run's axes as `run.axes_changed` last reported them, which a resync gives. */
  readonly #runs = new Map<string, unknown>();
  /** Each "Needs you" item `needs_you.changed` left listed, by ID, which a resync gives. */
  readonly #needsYou = new Map<string, NeedsYouItem>();
  /** How many engine instances the adapter has stood for: `restart()` starts another. */
  #instances = 1;
  /** Whether the test that made it ended: nothing reaches a Client after that. */
  #ended = false;

  constructor(options: ProtocolAdapterOptions) {
    this.#contract = productContract(options.product ?? "collie");
    this.requests = this.#requests;
    this.transport = { open: (listener) => this.#open(listener) };
    this.bodyFetcher = (handle) => this.#fetch(handle);
    onTestFinished(() => this.#end());
  }

  /**
   * Ends the adapter with its test: closes every Connection without telling
   * its Client, so an answer or an event that comes later reaches no Client
   * of a later test, then fails the test with its violations. A function
   * answer still pending is dropped unchecked: its Client never saw it.
   */
  #end(): void {
    this.#ended = true;
    for (const connection of this.#connections) {
      connection.open = false;
    }
    this.#connections.clear();
    this.#verify();
  }

  /**
   * Fails the test with every violation, the first one's code leading: a
   * call the test did not answer, a message or an answer the bundle
   * refuses, or a body never served. The Client may have swallowed them.
   */
  #verify(): void {
    const [first] = this.#violations;
    if (first !== undefined) {
      throw new TestSupportError(
        first.code,
        this.#violations.map((violation) => `${violation.code}: ${violation.message}`).join("\n"),
      );
    }
  }

  /**
   * Pushes `event` at `position` to every subscription, as the engine
   * sends it once a Client's credits allow, and keeps it for a subscription
   * that resumes after an earlier position.
   *
   * @throws {TestSupportError} `event_invalid` for an event the catalog lacks, a payload breaking its schema, a position that does not go up past the last one, or `events.resync`, which the adapter sends itself.
   */
  push(event: PushedEvent<Events>, position: number): void {
    const { event: name, data } = event;
    const problems = this.#eventProblems(name, data, position);
    if (problems !== undefined) {
      throw new TestSupportError("event_invalid", problems);
    }
    const envelope: EventEnvelope = { position, event: name, data };
    this.#retained.push(envelope);
    this.#last = position;
    const runId = runIdOf(data);
    if (name === "run.axes_changed" && runId !== undefined) {
      // As the engine's: a Run leaves the resync once it ends (#1768).
      if (lifecycleOf(data) === "running") {
        this.#runs.set(runId, data);
      } else {
        this.#runs.delete(runId);
      }
    }
    if (name === "needs_you.changed") {
      this.#foldNeedsYou(data);
    }
    for (const connection of this.#connections) {
      for (const subscription of connection.subscriptions.values()) {
        subscription.queue.push(envelope);
        this.#flush(connection, subscription);
      }
    }
  }

  /** Folds a `needs_you.changed` payload, which its schema has checked, into the items a resync gives. */
  #foldNeedsYou(data: unknown): void {
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- push() checked the payload against the catalog's schema of `needs_you.changed` first.
    const change = data as NeedsYouChange;
    if (change.change === "listed") {
      this.#needsYou.set(change.item.id, change.item);
    } else {
      this.#needsYou.delete(change.id);
    }
  }

  /** Every item of "Needs you", as a resync gives them: the longest waiting first, then by ID. */
  #needsYouItems(): NeedsYouItem[] {
    return [...this.#needsYou.values()].toSorted((left, right) =>
      left.since === right.since
        ? left.id.localeCompare(right.id)
        : left.since.localeCompare(right.since),
    );
  }

  #eventProblems(name: string, data: unknown, position: number): Problems {
    if (name === RESYNC) {
      return `${RESYNC} is sent by the adapter itself, when a subscription resumes after a position it no longer holds`;
    }
    const payload = this.#contract.events.get(name);
    if (payload === undefined) {
      return `${name} is no event of ${this.#contract.product}'s catalog`;
    }
    if (!Number.isSafeInteger(position) || position <= this.#last) {
      return `${name} is pushed at position ${position}, which does not go up past ${this.#last}`;
    }
    const problems = payload.problems(data);
    return problems === undefined ? undefined : `${name}'s payload breaks its schema: ${problems}`;
  }

  /** Closes every open Connection, as a lost engine or network does; pushed events are kept. */
  drop(): void {
    for (const connection of this.#connections) {
      this.#close(connection);
    }
  }

  /**
   * Stands for a new engine instance, as a restarted Daemon: closes every
   * open Connection, names a new engine instance in `hello` and forgets
   * every position, Run and "Needs you" item pushed, so positions count from 1 again and a
   * subscription that resumes after one of the previous instance starts
   * with `events.resync`, then the stream.
   */
  restart(): void {
    this.drop();
    this.#instances += 1;
    this.#retained = [];
    this.#last = 0;
    this.#floor = 0;
    this.#runs.clear();
    this.#needsYou.clear();
  }

  /**
   * Forgets every position pushed so far, as an engine whose kept events
   * moved on: a subscription that resumes after one of them starts with
   * `events.resync`, the axes of every live Run pushed, then the stream.
   */
  forgetPositions(): void {
    this.#retained = [];
    this.#floor = this.#last;
  }

  /** Serves `bytes` as the body of `handle`: a page body, a screenshot or an Export. */
  serve(handle: string, bytes: Uint8Array): void {
    this.#bodies.set(handle, bytes.slice());
  }

  async #fetch(handle: string): Promise<Uint8Array> {
    const bytes = this.#bodies.get(handle);
    if (bytes === undefined) {
      throw this.#violation(
        "body_unserved",
        `the Client fetched ${handle}, a body the test never served`,
      );
    }
    return bytes.slice();
  }

  /**
   * Answers every call of `operation`, from now on, with `answer`: a value,
   * or a function of the call's params. A value is checked here; a
   * function's outcome when a call comes.
   *
   * @throws {TestSupportError} `answer_invalid` when the value is not one the engine could send.
   */
  answer<Name extends keyof Operations & string>(
    operation: Name,
    answer: Answer<Operations[Name]>,
  ): void {
    const problems = this.#answerProblems(operation, answer);
    if (problems !== undefined) {
      throw new TestSupportError("answer_invalid", problems);
    }
    this.#answers.set(operation, answer);
  }

  #answerProblems(operation: string, answer: unknown): Problems {
    const contract = this.#contract.operations.get(operation);
    if (contract === undefined) {
      return `${operation} is no operation of ${this.#contract.product}'s catalog`;
    }
    if (operation === "events.subscribe") {
      return "events.subscribe streams the events push() gives; the adapter answers it itself";
    }
    return typeof answer === "function" ? undefined : this.#outcomeProblems(operation, answer);
  }

  #outcomeProblems(operation: string, outcome: unknown): Problems {
    if (outcome instanceof Failure) {
      const code = this.#contract.errorCodes.get(outcome.code);
      return code === undefined
        ? `${operation}'s failure names ${outcome.code}, no Error code of ${this.#contract.product}'s catalog`
        : undefined;
    }
    const problems = this.#contract.operations.get(operation)?.result.problems(outcome);
    return problems === undefined
      ? undefined
      : `${operation}'s answer breaks its result's schema: ${problems}`;
  }

  #open(listener: ConnectionListener): Connection {
    const connection: OpenConnection = { listener, subscriptions: new Map(), open: true };
    this.#connections.add(connection);
    return {
      send: (text) => this.#receive(connection, text),
      close: () => this.#close(connection),
    };
  }

  #close(connection: OpenConnection): void {
    if (!connection.open) {
      return;
    }
    connection.open = false;
    this.#connections.delete(connection);
    queueMicrotask(() => {
      if (!this.#ended) {
        connection.listener.close();
      }
    });
  }

  /** Sends `message` to the Client after the current task, unless its Connection closed meanwhile. */
  #deliver(connection: OpenConnection, message: Record<string, unknown>): void {
    const text = JSON.stringify({ jsonrpc: "2.0", ...message });
    queueMicrotask(() => {
      if (connection.open) {
        connection.listener.message(text);
      }
    });
  }

  #violation(code: TestSupportErrorCode, message: string): TestSupportError {
    const violation = new TestSupportError(code, message);
    this.#violations.push(violation);
    return violation;
  }

  #receive(connection: OpenConnection, text: string): void {
    if (!connection.open) {
      return;
    }
    const message = this.#read(text);
    if (message === undefined) {
      return;
    }
    if ("id" in message) {
      this.#requests.push({ method: message.method, id: message.id, params: message.params });
      this.#request(connection, message);
    } else {
      this.#requests.push({ method: message.method, params: message.params });
      this.#notification(connection, message);
    }
  }

  /** The message `text` holds, once its envelope meets the bundle; a violation otherwise. */
  #read(text: string): WireRequest | WireNotification | undefined {
    let value: unknown;
    try {
      value = JSON.parse(text);
    } catch {
      this.#violation("request_invalid", `the Client sent text that is not JSON: ${text}`);
      return undefined;
    }
    const isRequest = typeof value === "object" && value !== null && "id" in value;
    const envelope = isRequest ? this.#contract.request : this.#contract.notification;
    const checked = envelope.check(value);
    if (!checked.holds) {
      this.#violation(
        "request_invalid",
        `the Client sent a message that is no JSON-RPC ${isRequest ? "request" : "notification"}: ${checked.problems}: ${text}`,
      );
      return undefined;
    }
    return checked.value;
  }

  #notification(connection: OpenConnection, message: WireNotification): void {
    const params = message.params ?? {};
    if (message.method === "$/credit") {
      const credit = this.#contract.creditParams.check(params);
      if (!credit.holds) {
        this.#paramsInvalid(message.method, credit.problems);
        return;
      }
      const subscription = connection.subscriptions.get(credit.value.id);
      if (subscription !== undefined) {
        subscription.credits += credit.value.credits;
        this.#flush(connection, subscription);
      }
      return;
    }
    if (message.method === "$/cancel") {
      const cancel = this.#contract.cancelParams.check(params);
      if (!cancel.holds) {
        this.#paramsInvalid(message.method, cancel.problems);
        return;
      }
      if (connection.subscriptions.delete(cancel.value.id)) {
        this.#deliver(connection, {
          id: cancel.value.id,
          error: this.#wireError(
            new Failure("call_cancelled", { by: "client" }),
            "The call was cancelled by its Client.",
          ),
        });
      }
      return;
    }
    this.#violation(
      "request_invalid",
      `the Client sent ${message.method}, no notification the engine takes`,
    );
  }

  /** Starts subscription `id`, resuming after `after` when it names a position of this engine instance. */
  #subscribe(connection: OpenConnection, id: RequestId, after: unknown): void {
    const queue: EventEnvelope[] = [];
    const read = lastRead(after);
    if (read !== undefined) {
      const { position } = read;
      if (
        read.engineInstance === engineInstance(this.#instances) &&
        position >= this.#floor &&
        position <= this.#last
      ) {
        queue.push(...this.#retained.filter((envelope) => envelope.position > position));
      } else {
        this.#last += 1;
        queue.push({
          position: this.#last,
          event: RESYNC,
          data: { runs: [...this.#runs.values()], needs_you: this.#needsYouItems() },
        });
      }
    }
    connection.subscriptions.set(id, { id, credits: 0, queue });
  }

  /** Sends subscription `subscription` as many of its unsent events as its credits allow, in one batch. */
  #flush(connection: OpenConnection, subscription: Subscription): void {
    const count = Math.min(subscription.credits, subscription.queue.length);
    if (count === 0) {
      return;
    }
    subscription.credits -= count;
    this.#deliver(connection, {
      method: "$/events",
      params: { id: subscription.id, events: subscription.queue.splice(0, count) },
    });
  }

  #request(connection: OpenConnection, message: WireRequest): void {
    const { id } = message;
    const params = message.params ?? {};
    if (message.method === "hello") {
      if (this.#paramsInvalid("hello", this.#contract.helloParams.problems(params))) {
        return;
      }
      const refusal = this.#helloRefusal(params);
      if (refusal !== undefined) {
        this.#deliver(connection, {
          id,
          error: this.#wireError(
            new Failure(refusal, {}),
            `The engine refuses this hello: ${refusal}.`,
          ),
        });
        return;
      }
      this.#deliver(connection, {
        id,
        result: {
          protocol_version: this.#contract.protocolVersion,
          product: this.#contract.product,
          product_version: PRODUCT_VERSION,
          engine_instance: engineInstance(this.#instances),
        },
      });
      return;
    }
    const operation = this.#contract.operations.get(message.method);
    if (operation === undefined && this.#onlyFoxhoundCarries(message.method)) {
      this.#deliver(connection, {
        id,
        error: this.#wireError(
          new Failure("not_in_product", {
            product: this.#contract.product,
            product_version: PRODUCT_VERSION,
            needs: "foxhound",
            requested: message.method,
            fix: `Run it with Foxhound, which contains everything Collie does (\`${FOXHOUND_INSTALL}\`), or connect to a Foxhound engine.`,
          }),
          `\`${message.method}\` is part of Foxhound, and this engine is Collie ${PRODUCT_VERSION}.`,
        ),
      });
      return;
    }
    if (operation === undefined) {
      this.#violation(
        "request_invalid",
        `the Client called ${message.method}, no operation of ${this.#contract.product}'s catalog`,
      );
      return;
    }
    if (this.#paramsInvalid(message.method, operation.params.problems(params))) {
      return;
    }
    if (message.method === "events.subscribe") {
      const after =
        typeof params === "object" && params !== null && "after" in params
          ? params.after
          : undefined;
      this.#subscribe(connection, id, after);
      return;
    }
    const answer = this.#answers.get(message.method) ?? this.#ownAnswer(message.method);
    if (answer === undefined) {
      this.#violation(
        "call_unanswered",
        `the Client called ${message.method}, which the test set no answer for`,
      );
      return;
    }
    void this.#settle(connection, id, message.method, answer, params);
  }

  /**
   * What the adapter answers `method` with when the test set no answer:
   * for `engine.info`, the engine `hello` names, as the engine answers it.
   * The Console Client's stall watchdog asks it of a Connection silent for
   * a stall timeout (`console/collie/src/client/client.ts`), so any test
   * that runs that long calls it (#1794).
   */
  #ownAnswer(method: string): CollieOperationTypes["engine.info"]["result"] | undefined {
    return method === "engine.info"
      ? {
          product: this.#contract.product,
          product_major: PRODUCT_MAJOR,
          product_version: PRODUCT_VERSION,
          protocol_version: this.#contract.protocolVersion,
        }
      : undefined;
  }

  /**
   * Whether `method` is an operation only Foxhound carries while the
   * adapter stands for Collie's engine, which refuses it with
   * `not_in_product` before reading its params, as the engine does.
   */
  #onlyFoxhoundCarries(method: string): boolean {
    if (this.#contract.product === "foxhound") {
      return false;
    }
    this.#foxhound ??= productContract("foxhound");
    return this.#foxhound.operations.has(method);
  }

  #paramsInvalid(method: string, problems: Problems): boolean {
    if (problems !== undefined) {
      this.#violation(
        "request_invalid",
        `the Client's params of ${method} break their schema: ${problems}`,
      );
    }
    return problems !== undefined;
  }

  /** Answers call `id` of `operation` with what `answer` gives for `params`, once checked. */
  async #settle(
    connection: OpenConnection,
    id: RequestId,
    operation: string,
    answer: Answer<OperationTypes>,
    params: unknown,
  ): Promise<void> {
    let outcome: unknown;
    try {
      outcome = typeof answer === "function" ? await answer(params) : answer;
    } catch (error) {
      if (!this.#ended) {
        this.#violation(
          "answer_invalid",
          `the test's answer of ${operation} threw: ${String(error)}`,
        );
      }
      return;
    }
    if (this.#ended) {
      return;
    }
    const problems = this.#outcomeProblems(operation, outcome);
    if (problems !== undefined) {
      this.#violation("answer_invalid", problems);
      return;
    }
    this.#deliver(
      connection,
      outcome instanceof Failure
        ? {
            id,
            error: this.#wireError(outcome, `The test answered ${operation} with ${outcome.code}.`),
          }
        : { id, result: outcome },
    );
  }

  /**
   * The Error code the engine refuses a schema-valid `hello` with, as the
   * generated refusal vectors give it: `not_in_product` for a Product this
   * engine does not carry, before `protocol_version_unsupported` for a
   * protocol version other than its own.
   */
  #helloRefusal(params: unknown): string | undefined {
    const fields = typeof params === "object" && params !== null ? params : {};
    const needs = "needs" in fields ? fields.needs : undefined;
    if (needs === "foxhound" && this.#contract.product !== "foxhound") {
      return "not_in_product";
    }
    const version = "protocol_version" in fields ? fields.protocol_version : undefined;
    return version === this.#contract.protocolVersion ? undefined : "protocol_version_unsupported";
  }

  /** The wire's error object of `failed`, whose code the catalog defines, told to people as `message`. */
  #wireError(failed: Failure, message: string): Record<string, unknown> {
    const code = this.#contract.errorCodes.get(failed.code);
    if (code === undefined) {
      throw new Error("invariant: a failure's code was checked against the catalog");
    }
    return {
      code: code.jsonRpcClass,
      message,
      data: { ...failed.details, code: failed.code, retryable: code.retryable },
    };
  }
}

/**
 * The in-memory adapter of the protocol for a Console test, standing for
 * `options.product`'s engine (Collie's by default). Call it inside a test
 * or a fixture.
 */
export function createProtocolAdapter<
  Operations extends OperationTable<Operations> = CollieOperationTypes,
  Events = CollieEventPayloads,
>(options: ProtocolAdapterOptions = {}): ProtocolAdapter<Operations, Events> {
  return new ProtocolAdapter<Operations, Events>(options);
}
