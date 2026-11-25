// A Connection's Schedules (spec 589, ADR 0099): `connection.schedules`, a
// sub-object reached without I/O whose calls act on no Schedule, and the
// `Schedule` handle `create` and `get` answer, whose members the catalog
// generates. A Schedule handle holds its ID and has no `close()`: it names a
// Schedule its store keeps, so dropping it frees nothing in the engine.

import type { CallOptions, Caller } from "./caller.ts";
import { Schedule as ScheduleMembers } from "./generated/handles.ts";
import type {
  CollieOperationTypes,
  ScheduleDefinition,
  ScheduleId,
  ScheduleInfo,
  ScheduleWarning,
} from "./generated/index.ts";
import { defined, timeoutOf, type HandleHost } from "./handle.ts";
import { warnExperimental } from "./markers.ts";
import type { RawSend } from "./raw-call.ts";

/** The options of a Schedule write: its own `timeout`. A write is one store transaction, so it takes no `signal` (ADR 0038). */
export interface ScheduleWriteOptions {
  /** How long the call may take, in milliseconds, at least 1: left out for `timeout.wait`, `null` for none (ADR 0080). */
  readonly timeout?: number | null | undefined;
}

/** The options of a Schedule read: its own `timeout`, and a `signal` that cancels it. */
export interface ScheduleCallOptions extends ScheduleWriteOptions {
  /** Cancels the call in flight when it aborts. */
  readonly signal?: AbortSignal | undefined;
}

/** What `schedules.create` answers: the created Schedule, and what its program should know. */
export interface CreatedSchedule {
  /** The Schedule, under the ID given or made. */
  readonly schedule: Schedule;
  /** What did not stop its creation but its program should know. */
  readonly warnings: readonly ScheduleWarning[];
}

/** What a Connection's Schedules reach the engine through. */
export interface SchedulesHost {
  /** The Connection's calls, typed by Collie's operations. */
  readonly calls: Caller<CollieOperationTypes>;
  /** Sends a raw call, its params and result unconverted. */
  readonly raw: RawSend;
  /** Fails with `closed` once the Connection is closed. */
  ensureOpen(operation: string): void;
}

/** A Schedule a program holds, by the ID its store keeps it under. */
export class Schedule extends ScheduleMembers {
  readonly #id: ScheduleId;

  /** @internal `schedules.create` and `schedules.get` answer one; a program never constructs one. */
  constructor(host: HandleHost<ScheduleId, CollieOperationTypes>) {
    super(host);
    this.#id = host.id;
  }

  /** The Schedule's ID, which names it for good. Read without a round trip. */
  id(): ScheduleId {
    return this.#id;
  }
}

/**
 * A Connection's Schedules: `connection.schedules`. Each call acts on the
 * Schedule store of the engine's major, and none on one Schedule: a
 * `Schedule`'s own members do.
 */
export class Schedules {
  readonly #host: SchedulesHost;

  /** @internal A Connection opens its Schedules; a program never constructs one. */
  constructor(host: SchedulesHost) {
    this.#host = host;
  }

  /**
   * Records a Schedule in the Schedule store, which the Daemon keeps, and answers its ID, as given or made (`s` and a lowercase ULID), with warnings. Nothing fires yet. A taken ID fails with `schedule_exists`, a trigger that does not parse with `schedule_trigger_invalid`. Its time zone is the Daemon host's when the definition names none. One store transaction: a cancel stops it only before it commits.
   *
   * Answers the `Schedule` under that ID, which holds it.
   *
   * @experimental
   */
  async create(
    definition: ScheduleDefinition,
    options: ScheduleWriteOptions = {},
  ): Promise<CreatedSchedule> {
    const operation = "schedule.create";
    warnExperimental(operation);
    this.#host.ensureOpen(operation);
    const created = await this.#host.calls.call(
      operation,
      defined({ definition, timeout: timeoutOf(options.timeout, "timeout.wait") }),
      callOptions(options),
    );
    return { schedule: this.#scheduleOf(created.scheduleId), warnings: created.warnings };
  }

  /**
   * Lists every Schedule of the Schedule store, by ID.
   *
   * @experimental
   */
  async list(options: ScheduleCallOptions = {}): Promise<ScheduleInfo[]> {
    const operation = "schedule.list";
    warnExperimental(operation);
    this.#host.ensureOpen(operation);
    const listed = await this.#host.calls.call(
      operation,
      defined({ timeout: timeoutOf(options.timeout, "timeout.wait") }),
      callOptions(options),
    );
    return listed.schedules;
  }

  /**
   * Reads the Schedule of an ID. An ID the store does not hold fails with `schedule_not_found`.
   *
   * Answers the `Schedule` of that ID once the store said it holds one; its `info()` reads it again.
   *
   * @experimental
   */
  async get(id: ScheduleId, options: ScheduleCallOptions = {}): Promise<Schedule> {
    const operation = "schedule.get";
    warnExperimental(operation);
    this.#host.ensureOpen(operation);
    const info = await this.#host.calls.call(
      operation,
      defined({ id, timeout: timeoutOf(options.timeout, "timeout.wait") }),
      callOptions(options),
    );
    return this.#scheduleOf(info.scheduleId);
  }

  /** The handle of the Schedule of ID `id`, whose calls go through this Connection. */
  #scheduleOf(id: ScheduleId): Schedule {
    return new Schedule(scheduleHost(this.#host, id));
  }
}

/** What a Schedule call's options give the Connection's call: its signal and its deadline. */
function callOptions(options: ScheduleCallOptions): CallOptions {
  return {
    signal: options.signal,
    deadline: { key: "timeout.wait", value: options.timeout },
  };
}

/** The host of the Schedule of ID `id`: its calls go through `host`, and it opens, closes and listens to nothing. */
function scheduleHost(
  host: SchedulesHost,
  id: ScheduleId,
): HandleHost<ScheduleId, CollieOperationTypes> {
  const nothingHeld = (what: string): never => {
    throw new TypeError(`invariant: a Schedule ${what}`);
  };
  return {
    id,
    call(operation, params, options) {
      host.ensureOpen(operation);
      return host.calls.call(operation, params, options);
    },
    raw(operation, params, options) {
      host.ensureOpen(operation);
      return host.raw(operation, params, options);
    },
    child: () => nothingHeld("opens no value"),
    close: () => nothingHeld("has no close()"),
    listen: () => nothingHeld("has no listener"),
    listenPublished: () => nothingHeld("has no listener"),
  };
}
