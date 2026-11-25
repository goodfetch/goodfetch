// The program's own Plugins of Foxhound's kinds, registered by name on
// `connect()` without a process Plugin (ADRs 0033 and 0104): a Fetch backend
// a setting such as `fetch.backend` chooses, and a Sink that serves the
// `sinks.<name>` entry of its name. Their types live here; the engine's
// calls to them are each kind's own ticket's work.
import type { CallOptions } from "@goodfetch/collie-client";
import type { DatasetRecord } from "./generated/index.ts";

/** A request the engine hands the program's Fetch backend, with no credential in it (ADRs 0092, 0104). */
export interface BackendRequest {
  /** The URL to fetch. */
  readonly url: string;
  /** Its method, such as `GET`. */
  readonly method: string;
  /** Its headers, by name. */
  readonly headers: Readonly<Record<string, string>>;
  /** Its body, or `null` for none. */
  readonly body: Uint8Array | null;
  /** The proxy to send it through, with no password, or `null` for none. */
  readonly proxy: string | null;
}

/** What the program's Fetch backend answers a request with. */
export interface BackendResponse {
  /** Its HTTP status. */
  readonly status: number;
  /** Its headers, by name. */
  readonly headers: Readonly<Record<string, string>>;
  /** Its body. */
  readonly body: Uint8Array;
}

/**
 * A Fetch backend of the program's own, registered under the name settings
 * choose it by (ADR 0104). It runs with the program's full rights, never
 * confined.
 */
export type FetchBackendHandler = (
  request: BackendRequest,
  options: CallOptions,
) => Promise<BackendResponse>;

/**
 * A delivery of committed Records to the program's own Sink: its delivery
 * ID and Run epoch make it safe to retry (ADR 0008).
 */
export interface SinkDelivery {
  /** Its ID, the same on each retry of it. */
  readonly deliveryId: string;
  /** The Run's epoch it was made in. */
  readonly runEpoch: number;
  /** The Dataset's name. */
  readonly dataset: string;
  /** The Records it delivers, in commit order. */
  readonly records: readonly DatasetRecord[];
}

/**
 * A Sink of the program's own, registered under the name of the
 * `sinks.<name>` entry it serves (ADR 0104). It runs with the program's
 * full rights, never confined.
 */
export type SinkHandler = (delivery: SinkDelivery, options: CallOptions) => Promise<void>;
