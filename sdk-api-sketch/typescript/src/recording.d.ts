// Response recordings (ADR 0105, decided in #482).
// A Run's start may name a Response recording: a WARC 1.1 file on the program's machine, read and
// written by the SDK there, as a Plan file is (ADR 0091), so a Daemon or Goodfetch Cloud works the same.
// - `replay`: every request the Run would send to an address a page can reach (ADR 0017's first
//   class: crawl requests, navigations, browser subresources, `page.request`, robots.txt) is answered
//   from the file and never sent. One the file lacks is refused: a call the program made throws
//   `ResponseNotRecordedError`, a crawl request ends `failed` with it, and a subresource fails in its
//   page as a network error. Every miss is listed in the Run report. Hosts the Run allowlists for tests
//   (ADR 0017), such as a test's own server on loopback, are reached as usual.
// - `record`: the Run fetches as usual, and the file is replaced, atomically, by every response it
//   received when the Run ends. No credential is kept.
// The testing kit (testing.d.ts) replays by default. Model calls are never recorded: a Plan replayed
// with `plan.frozen` or the kit's Scripted model stands in for the model (ADRs 0065, 0091).

import type { ProgramError, RefusedError } from "./results.js";

/** A Response recording a Run's start names: `replay` it, or `record` into it. */
export interface Recording {
  readonly mode: "replay" | "record";
  /** A WARC file, relative to the working directory, gzipped per record when it ends in `.gz`. */
  readonly path: string;
}

/** A request a replaying Run did not find in its recording, and so never sent. */
export interface RecordingMiss {
  readonly method: string;
  readonly url: string;
}

/** What a Run did with its Response recording, in its report (ADR 0102's `RunReport`). */
export interface RecordingReport {
  readonly mode: "replay" | "record" | (string & {});
  readonly path: string;
  /** Responses answered from the file (`replay`) or written to it (`record`). */
  readonly responses: number;
  /**
   * When the responses used were first fetched: the recording's age, so a test can see how old the
   * site it checks against is. `null` when none was used.
   */
  readonly recordedFrom: Date | null;
  readonly recordedTo: Date | null;
  /** The engine that wrote the file. */
  readonly recordedBy: string | null;
  /** Every request not found, in order. Empty on `record`. */
  readonly misses: readonly RecordingMiss[];
}

/**
 * A replaying Run refused a request its recording lacks, before sending anything. Not retryable: a
 * retry would miss again. `fix` says to record again, or to answer the request with a route.
 */
export declare class ResponseNotRecordedError extends RefusedError {
  readonly code: "response_not_recorded";
  readonly details: {
    readonly method: string;
    readonly url: string;
    readonly recording: string;
    /** Recorded URLs closest to it (same host and path), for a query string that changed. */
    readonly nearest: readonly string[];
  };
}

/** The recording cannot be read, is not WARC, or names a WARC version this engine does not read (ADR 0030). */
export declare class RecordingInvalidError extends ProgramError {
  readonly code: "recording_invalid";
  readonly details: { readonly recording: string; readonly reason: string };
}
