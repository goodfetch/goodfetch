// A value's realm (ADR 0081): a Connection and every value it hands out
// belong to the realm that opened it, so passing one to a `worker_threads`
// worker or through `structuredClone` is refused with a `DataCloneError`
// rather than arriving as a dead copy. A worker connects on its own.
import { markAsUncloneable } from "node:worker_threads";

/** Refuses `value` to every structured clone, `postMessage` included. */
export function holdInRealm(value: object): void {
  markAsUncloneable(value);
}
