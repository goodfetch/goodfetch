// The base of every error a Goodfetch SDK raises (ADR 0079), which each
// Error code's generated class extends through its group's. Hand-written:
// only the seven groups come from the catalog.
import type { ErrorGroup } from "./generated/errors.ts";

/** What an error carries beyond its code, group and retryable flag (ADR 0079). */
export interface ErrorFields {
  /** What to do about it, when the engine or the runtime knows. */
  readonly fix?: string | undefined;
  /** The code's details, as the engine sent them, in wire names. */
  readonly details?: Readonly<Record<string, unknown>> | undefined;
  /** The error that caused it, in this process. */
  readonly cause?: unknown;
}

/** The base of every error a Goodfetch SDK raises (ADR 0079). */
export class GoodfetchError extends Error {
  /** The Error code, as the engine sent it. */
  readonly code: string;
  /** The code's group, named for its remedy. */
  readonly group: ErrorGroup;
  /** Whether the same call may succeed when made again. */
  readonly retryable: boolean;
  /** What to do about it, when known. */
  readonly fix: string | undefined;
  /** The code's details, in wire names; empty when it has none. */
  readonly details: Readonly<Record<string, unknown>>;

  constructor(
    message: string,
    init: ErrorFields & {
      readonly code: string;
      readonly group: ErrorGroup;
      readonly retryable: boolean;
    },
  ) {
    super(message, init.cause === undefined ? undefined : { cause: init.cause });
    this.name = new.target.name;
    this.code = init.code;
    this.group = init.group;
    this.retryable = init.retryable;
    this.fix = init.fix;
    this.details = init.details ?? {};
  }
}

/** The class of one Error code: its message, then what it carries. */
export type ErrorClass = new (message: string, fields?: ErrorFields) => GoodfetchError;
