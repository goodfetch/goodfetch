// A Connection's Saved identities (ADRs 0015, 0035, 0056 and 0099; ticket
// #736): `connection.identities`, a sub-object reached without I/O exposing
// `list`, `import`, `delete` and `export` (encrypted `age` bundle only — no
// SDK writes clear-text `storageState`).

import type { CallOptions, Caller } from "./caller.ts";
import type {
  CollieOperationTypes,
  IdentityDeleted,
  IdentityExportParams,
  IdentityExported,
  IdentityImportParams,
  SavedIdentityInfo,
} from "./generated/index.ts";
import { warnExperimental } from "./markers.ts";

/** The options of a Saved identity write: its own `timeout`. */
export interface IdentityWriteOptions {
  /** How long the call may take, in milliseconds, at least 1: left out for `timeout.wait`, `null` for none (ADR 0080). */
  readonly timeout?: number | null | undefined;
}

/** The options of a Saved identity read: its own `timeout`, and a `signal` that cancels it. */
export interface IdentityCallOptions extends IdentityWriteOptions {
  /** Cancels the call in flight when it aborts. */
  readonly signal?: AbortSignal | undefined;
}

/** What a Connection's Saved identities reach the engine through. */
export interface IdentitiesHost {
  /** The Connection's calls, typed by Collie's operations. */
  readonly calls: Caller<CollieOperationTypes>;
  /** Fails with `closed` once the Connection is closed. */
  ensureOpen(operation: string): void;
}

/**
 * A Connection's Saved identities: `connection.identities`. Each call acts on
 * this Product major's encrypted Saved identity store.
 */
export class Identities {
  readonly #host: IdentitiesHost;

  /** @internal A Connection opens its Identities; a program never constructs one. */
  constructor(host: IdentitiesHost) {
    this.#host = host;
  }

  /**
   * Lists every non-expired Saved identity in this Product major's store, with its metadata and domains, never secret values (ADR 0035).
   *
   * @experimental
   */
  async list(options: IdentityCallOptions = {}): Promise<SavedIdentityInfo[]> {
    const operation = "identity.list";
    warnExperimental(operation);
    this.#host.ensureOpen(operation);
    const listed = await this.#host.calls.call(operation, {}, callOptions(options));
    return listed.identities;
  }

  /**
   * Imports cookies and storage from a Session source (`state_file`, `cookie_header`, etc.) into the encrypted per-major Saved identity store under `name`, replacing any existing identity of that name under the install-wide lock and recording the import in the Audit log (ADRs 0035 and 0056).
   *
   * @experimental
   */
  async import(
    params: IdentityImportParams,
    options: IdentityWriteOptions = {},
  ): Promise<SavedIdentityInfo> {
    const operation = "identity.import";
    warnExperimental(operation);
    this.#host.ensureOpen(operation);
    return await this.#host.calls.call(operation, params, callOptions(options));
  }

  /**
   * Deletes the named Saved identity from this Product major's encrypted store, wiping its secret fields and recording the deletion in the Audit log (ADR 0035).
   *
   * @experimental
   */
  async delete(name: string, options: IdentityWriteOptions = {}): Promise<IdentityDeleted> {
    const operation = "identity.delete";
    warnExperimental(operation);
    this.#host.ensureOpen(operation);
    return await this.#host.calls.call(operation, { name }, callOptions(options));
  }

  /**
   * Exports the named Saved identity as an `age`-encrypted `session-export` bundle to an `age` public key or a passphrase Secret reference, preserving its Access mode, Identity profile seed, proxy binding and machine binding, and recording the export in the Audit log (ADRs 0015, 0035 and 0056).
   *
   * No SDK writes clear-text `storageState` (ADR 0056).
   *
   * @experimental
   */
  async export(
    params: IdentityExportParams,
    options: IdentityWriteOptions = {},
  ): Promise<IdentityExported> {
    const operation = "identity.export";
    warnExperimental(operation);
    this.#host.ensureOpen(operation);
    return await this.#host.calls.call(operation, params, callOptions(options));
  }
}

function callOptions(options: IdentityCallOptions): CallOptions {
  return {
    signal: options.signal,
    deadline: { key: "timeout.wait", value: options.timeout },
  };
}
