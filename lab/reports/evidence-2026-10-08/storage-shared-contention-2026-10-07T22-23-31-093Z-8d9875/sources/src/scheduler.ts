export type ScheduleOptions = { mode: 'closed' | 'open'; iterations: number; concurrency: number; ratePerSecond?: number; maxDurationMs: number };
export async function schedule(options: ScheduleOptions, task: (index: number, queueMs: number) => Promise<void>, reject: (index: number, queueMs: number) => void | Promise<void>) {
  const start = performance.now(); let next = 0; let active = 0;
  const pending = new Set<Promise<void>>();
  const dispatch = (index: number, intended: number) => {
    active++; let promise: Promise<void>;
    promise = task(index, Math.max(0, performance.now() - intended)).finally(() => { active--; pending.delete(promise); });
    pending.add(promise);
  };
  if (options.mode === 'closed') {
    while (next < options.iterations) {
      if (performance.now() - start >= options.maxDurationMs) { while (next < options.iterations) await reject(next++, performance.now() - start); break; }
      if (active < options.concurrency) dispatch(next++, performance.now()); else await Promise.race(pending);
    }
  } else {
    const interval = 1000 / (options.ratePerSecond ?? 1);
    while (next < options.iterations) {
      const intended = start + next * interval; const wait = intended - performance.now();
      if (wait > 0) await new Promise(resolve => setTimeout(resolve, Math.min(wait, Math.max(0, start + options.maxDurationMs - performance.now()))));
      if (performance.now() - start >= options.maxDurationMs || intended >= start + options.maxDurationMs) {
        while (next < options.iterations) { await reject(next, Math.max(0, performance.now() - (start + next * interval))); next++; }
        break;
      }
      if (active >= options.concurrency) await reject(next, Math.max(0, performance.now() - intended));
      else dispatch(next, intended);
      next++;
    }
  }
  await Promise.all(pending);
  return { durationMs: performance.now() - start, scheduled: options.iterations };
}
