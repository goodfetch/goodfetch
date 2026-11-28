// Streams seeds to a running crawl (`run.add_seeds`, ADRs 0024 and 0102;
// issue 1494) under the engine's `$/credit` grants, yielding each seed's
// `Enqueued` answer in order without holding unbounded seeds in memory.
import {
  FOLLOWED_VALUE,
  Sequence,
  fromWire,
  toWire,
  type ClientConnection,
} from "@goodfetch/collie-client/runtime";
import { checkCrawlStartable, checkSeedPayloads, type Crawl } from "./crawl.ts";
import type { Enqueued, RunAddSeedsParams, Seed, SeedAttempt } from "./generated/index.ts";
import { foxhoundWire } from "./generated/wire.ts";

/** Maximum seeds sent in a single `$/seeds` batch, staying under `SEED_CREDITS` (256). */
const SEED_BATCH_MAX = 64;
/** Maximum unread `Enqueued` answers buffered before pausing further `$/seeds` batches. */
const MAX_QUEUED_ANSWERS = 256;

/** What opens a seed stream on a crawl's Run (optionally staged with an attempt). */
export interface SeedStreamInput {
  readonly connection: ClientConnection;
  readonly runId: string;
  readonly crawl: Crawl | undefined;
  readonly attempt: SeedAttempt | undefined;
  readonly source: Iterable<Seed> | AsyncIterable<Seed>;
  readonly signal: AbortSignal | undefined;
}

/** Pulls from either a sync `Iterable` or an `AsyncIterable` of seeds. */
function seedIterator(
  source: Iterable<Seed> | AsyncIterable<Seed>,
): Iterator<Seed> | AsyncIterator<Seed> {
  if (Symbol.asyncIterator in source) {
    return source[Symbol.asyncIterator]();
  }
  return source[Symbol.iterator]();
}

/** Closes a sync or async iterator on early cancellation or error. */
async function closeIterator(iterator: Iterator<Seed> | AsyncIterator<Seed>): Promise<void> {
  if (typeof iterator.return === "function") {
    try {
      await iterator.return();
    } catch {
      // Ignore cleanup errors when cancelling.
    }
  }
}

/** Opens `run.add_seeds` as a `Sequence<Enqueued>` driven by engine `$/credit` grants. */
export function openSeedStream(input: SeedStreamInput): Sequence<Enqueued> {
  return new Sequence<Enqueued>({
    open: (opened) => {
      const params: RunAddSeedsParams = {
        runId: input.runId,
        ...(input.attempt === undefined ? {} : { attempt: input.attempt }),
      };
      const wireParams = toWire(params, { type: "RunAddSeedsParams" }, foxhoundWire.types);

      let engineCredits = 0;
      let consumerRoom = 0;
      let pumping = false;
      let ended = false;
      let stopped = false;
      let failStream: ((error: unknown) => void) | undefined;
      const localAbort = new AbortController();
      const streamSignal =
        opened.signal === undefined
          ? localAbort.signal
          : AbortSignal.any([opened.signal, localAbort.signal]);
      const iterator = seedIterator(input.source);

      const isStopped = (): boolean => stopped;

      const finishOnAbort = (): void => {
        if (stopped) return;
        stopped = true;
        void closeIterator(iterator);
      };
      opened.signal?.addEventListener("abort", finishOnAbort, { once: true });

      const pump = async (): Promise<void> => {
        if (pumping || ended || isStopped()) return;
        pumping = true;
        try {
          while (engineCredits > 0 && consumerRoom > 0 && !ended && !isStopped()) {
            const limit = Math.min(engineCredits, consumerRoom, SEED_BATCH_MAX);
            const batch: Seed[] = [];
            while (batch.length < limit && !isStopped()) {
              const step = await iterator.next();
              if (step.done === true) {
                ended = true;
                break;
              }
              batch.push(step.value);
            }
            if (isStopped()) break;
            if (batch.length === 0 && !ended) break;
            if (input.crawl !== undefined && batch.length > 0) {
              checkCrawlStartable(input.crawl, batch);
              await checkSeedPayloads(input.crawl, batch);
            }
            engineCredits -= batch.length;
            consumerRoom -= batch.length;
            const wireSeeds = batch.map((item) =>
              toWire(item, { type: "Seed" }, foxhoundWire.types),
            );
            realStream.notify?.("$/seeds", {
              id: realStream.id,
              seeds: wireSeeds,
              ...(ended ? { end: true } : {}),
            });
          }
        } catch (error) {
          stopped = true;
          localAbort.abort(error);
          await closeIterator(iterator);
          failStream?.(error);
        } finally {
          pumping = false;
        }
      };
      const resume = (): void => {
        if (engineCredits > 0 && consumerRoom > 0 && !ended && !stopped) {
          void pump();
        }
      };

      const realStream = input.connection.stream("run.add_seeds", wireParams, {
        signal: streamSignal,
        notified: (method, raw) => {
          if (stopped) return;
          if (
            method === "$/credit" &&
            typeof raw === "object" &&
            raw !== null &&
            "credits" in raw
          ) {
            const granted = Reflect.get(raw, "credits");
            if (typeof granted === "number" && granted > 0) {
              engineCredits += granted;
              resume();
            }
            return;
          }
          if (
            method === "$/enqueued" &&
            typeof raw === "object" &&
            raw !== null &&
            "answers" in raw
          ) {
            const rawAnswers = Reflect.get(raw, "answers");
            if (Array.isArray(rawAnswers)) {
              for (const item of rawAnswers) {
                // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine `$/enqueued` carries `Enqueued` answers per catalog.
                const decoded = fromWire(
                  item,
                  { type: "Enqueued" },
                  foxhoundWire.types,
                ) as Enqueued;
                opened.notified?.(FOLLOWED_VALUE, decoded);
              }
            }
          }
        },
      });

      const localFailure = new Promise<never>((_resolve, reject) => {
        failStream = reject;
      });
      // Prevent unhandled rejection if `realStream.result` wins the race first.
      localFailure.catch(() => undefined);

      const result = Promise.race([realStream.result, localFailure]).finally(() => {
        stopped = true;
        void closeIterator(iterator);
      });

      return {
        result,
        // Sequence grants consumer room as answers are read; engine credits flow via `$/credit`.
        grant: (n) => {
          consumerRoom += n;
          resume();
        },
      };
    },
    itemOf: (method, params) => {
      if (method !== FOLLOWED_VALUE) return undefined;
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `FOLLOWED_VALUE` carries the decoded `Enqueued` from above.
      return params as Enqueued;
    },
    credits: MAX_QUEUED_ANSWERS,
    regrant: SEED_BATCH_MAX,
    signal: input.signal,
  });
}
