// Scenario runner for the Node SDKs. Usage: node scen.ts <napi|rpc-node> <scenario> '<json args>'
// Env: DATA (data dir), ENGINED (engined binary), SOCK (daemon socket; sidecar when unset).
import { appendFileSync } from 'node:fs';
import { NativeEngine } from '../sdk-ts/native.ts';
import { RpcEngine } from '../sdk-ts/rpc.ts';
import { nowUs, readBody, type Engine, type EngineEvent, type Page } from '../sdk-ts/types.ts';

const [runner, scen, argJson] = process.argv.slice(2);
const A = JSON.parse(argJson ?? '{}');
const DATA = process.env.DATA!;
const out = (o: object) => console.log(JSON.stringify({ runner, scen, ...o }));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const busy = (us: number) => { const end = nowUs() + us; while (nowUs() < end); };
const pct = (xs: ArrayLike<number>, p: number) => {
  const a = Float64Array.from(xs).sort();
  return a.length ? a[Math.min(a.length - 1, Math.floor(a.length * p))] : 0;
};
const maxRss = () => process.resourceUsage().maxRSS * 1024;
const base = () => ({ db_path: `${DATA}/${A.db ?? 'engine.db'}`, blob_dir: `${DATA}/blobs` });

async function open(): Promise<Engine> {
  if (runner === 'napi') return new NativeEngine();
  if (process.env.SOCK) return RpcEngine.connect(process.env.SOCK);
  return RpcEngine.sidecar(process.env.ENGINED!);
}

const scenarios: Record<string, (e: Engine) => Promise<void>> = {
  // S1: 100k small events; latency emit -> receive; optional slow (busy-wait) consumer.
  async s1(eng) {
    const n = A.n ?? 100_000, slow = A.slow_us ?? 0;
    const lats = new Float64Array(n);
    let k = 0, tLast = 0;
    const onEvent = (evs: EngineEvent[]) => {
      for (const e of evs) {
        if (e.kind !== 'tick') continue;
        const t = nowUs();
        lats[k++] = t - e.t_us;
        tLast = t;
        if (slow) busy(slow);
      }
    };
    const t0 = nowUs();
    const s = await eng.run({ ...base(), stream_events: n, event_bytes: 64, event_mode: A.mode ?? 'default', event_cap: 1024, event_batch: A.batch ?? 1 }, { handler: async () => null, onEvent });
    const tResolved = nowUs();
    const expected = s.events_emitted - s.events_dropped;
    while (k < expected && nowUs() - t0 < 120e6) await new Promise((r) => setImmediate(r));
    const got = lats.subarray(0, k);
    out({
      variant: A.variant, received: k, emitted: s.events_emitted, dropped: s.events_dropped,
      throughput_eps: k / ((tLast - s.t_first_emit_us) / 1e6), p50_us: pct(got, 0.5), p99_us: pct(got, 0.99),
      engine_emit_ms: (s.t_last_emit_us - s.t_first_emit_us) / 1000, consume_ms: (tLast - s.t_first_emit_us) / 1000,
      run_resolved_ms: (tResolved - s.t_first_emit_us) / 1000, sdk_peak_rss: maxRss(), engine_peak_rss: s.engine_peak_rss,
    });
  },

  // S2a: cancel a running run; time from cancel() to the run resolving (engine stopped).
  async s2a(eng) {
    const ms: number[] = [], pages: number[] = [];
    for (let r = 0; r < (A.reps ?? 5); r++) {
      const ac = new AbortController();
      const p = eng.run({ ...base(), run_id: `s2a-${runner}-${process.pid}-${r}`, n_pages: 1e6, page_bytes: 1024, concurrency: 8, item_events: false, progress_every: 0 },
        { handler: async () => { await sleep(1); return null; }, signal: ac.signal });
      await sleep(300);
      const t = nowUs();
      ac.abort();
      const s = await p;
      ms.push((nowUs() - t) / 1000);
      pages.push(s.pages_done);
      if (!s.cancelled) throw new Error('not cancelled');
    }
    out({ cancel_ms_p50: pct(ms, 0.5), cancel_ms: ms, pages_before_cancel: pages });
  },

  // S2b: handlers that never resolve; engine timeout must let the run finish.
  async s2b(eng) {
    let aborted = 0, hung = 0;
    const t0 = nowUs();
    const s = await eng.run({ ...base(), n_pages: 20, page_bytes: 1024, concurrency: 4, handler_timeout_ms: 500, item_events: false, progress_every: 0 }, {
      handler: async (p: Page, signal?: AbortSignal) => {
        if (p.idx % 5 === 0) {
          hung++;
          signal?.addEventListener('abort', () => aborted++);
          await new Promise(() => {});
        }
        return { ok: 1 };
      },
    });
    const elapsed = (nowUs() - t0) / 1000;
    await sleep(200);
    out({ done: s.pages_done, timeout: s.pages_timeout, elapsed_ms: elapsed, hung_handlers: hung, handler_cancellations_seen: aborted, main_done_us: nowUs() });
  },

  // S2c: one CPU-bound handler blocks the event loop for 3 s.
  async s2c(eng) {
    let maxLag = 0, nev = 0, bs = 0, be = 0;
    const s = await eng.run({ ...base(), n_pages: 40, page_bytes: 1024, concurrency: 4, handler_timeout_ms: 1000, item_events: true, progress_every: 0, timeline: true }, {
      handler: async (p: Page) => {
        if (p.idx === 3) { bs = nowUs(); busy(3e6); be = nowUs(); } else await sleep(10);
        return {};
      },
      onEvent: (evs) => { for (const e of evs) { maxLag = Math.max(maxLag, nowUs() - e.t_us); nev++; } },
    });
    const w0 = (bs - s.t_start_us) / 1000, w1 = (be - s.t_start_us) / 1000;
    const tl = s.timeline as [number, string, number][];
    const inBlock = tl.filter(([, , t]) => t >= w0 && t <= w1);
    out({
      done: s.pages_done, timeout: s.pages_timeout, timed_out_idx: tl.filter((x) => x[1] === 'timeout').map((x) => x[0]),
      elapsed_ms: (s.t_end_us - s.t_start_us) / 1000, block_ms: (be - bs) / 1000, committed_during_block: inBlock.length,
      timeouts_during_block: inBlock.filter((x) => x[1] === 'timeout').length, max_event_lag_ms: maxLag / 1000, events: nev,
    });
  },

  // S3: handler kills the process at page crash_at; acked pages are logged durably to ackfile.
  async s3(eng) {
    if ((eng as any).info?.pid && A.pidfile) appendFileSync(A.pidfile, `${(eng as any).info.pid}\n`);
    const s = await eng.run({ ...base(), run_id: A.run_id, n_pages: A.n ?? 200, page_bytes: 1024, concurrency: 4, leases: true, item_events: true, progress_every: 0 }, {
      handler: async (p: Page) => {
        if (p.idx === A.crash_at) {
          if (A.kind === 'exit') process.exit(1);
          if (A.kind === 'abort') process.abort();
        }
        await sleep(A.sleep_ms ?? 2);
        return {};
      },
      onEvent: (evs) => { for (const e of evs) if (e.kind === 'item_done' && e.data === 'done') appendFileSync(A.ackfile, `${e.idx}\n`); },
    });
    out({ completed: true, done: s.pages_done });
  },

  // S4: page delivery, inline vs by reference, by size.
  async s4(eng) {
    const variants = runner === 'napi' ? ['zero-copy', 'ref'] : ['inline', 'ref'];
    const iters: Record<number, number> = { 100000: 60, 1000000: 20, 10000000: 6 };
    for (const size of A.sizes ?? [100_000, 1_000_000, 10_000_000]) {
      for (const variant of variants) {
        const s = await eng.run({ ...base(), run_id: `s4-${runner}-${process.pid}-${variant}-${size}`, n_pages: iters[size] ?? 10, page_bytes: size, concurrency: 1, inline: variant !== 'ref', item_events: false, progress_every: 0 }, {
          handler: async (p: Page) => {
            const te = nowUs();
            const b = readBody(p);
            const tb = nowUs();
            return { t_enter_us: te, t_body_us: tb, n: b.length };
          },
        });
        out({ variant, size, n: s.pages_done, deliver_p50_us: s.deliver_p50_us, body_p50_us: s.body_p50_us, rt_p50_us: s.rt_p50_us });
      }
    }
  },

  // S5: one process' share of a shared-database run.
  async s5(eng) {
    const s = await eng.run({ ...base(), run_id: A.run_id, n_pages: A.n ?? 500, page_bytes: 1024, concurrency: 8, item_events: false, progress_every: 0 }, { handler: async () => ({}) });
    out({ rows: s.pages_done, t_start_us: s.t_start_us, t_end_us: s.t_end_us, commit_p50_us: s.commit_p50_us, commit_p99_us: s.commit_p99_us, writer_busy: s.writer_busy, writer_commits: s.writer_commits, writer_lock_wait_ms: s.writer_lock_wait_ms });
  },

  // Feel: what does a handler exception look like on the engine side?
  async throw(eng) {
    function parseListing(_html: unknown) { throw new Error('boom: selector .price not found'); }
    const s = await eng.run({ ...base(), n_pages: 1, page_bytes: 1024, item_events: false, progress_every: 0 }, { handler: async (p: Page) => parseListing(readBody(p)) });
    out({ error_seen_by_engine: s.errors[0] });
  },
  // Feel: a synchronous throw (non-async handler).
  async throwsync(eng) {
    const s = await eng.run({ ...base(), n_pages: 1, page_bytes: 1024, item_events: false, progress_every: 0 }, { handler: ((_p: Page) => { throw new Error('sync boom'); }) as any });
    out({ error_seen_by_engine: s.errors[0] });
  },
  async probe(eng) { out({ info: (eng as any).info ?? null }); },
};

if (scen === 'skew') {
  try { await RpcEngine.sidecar(process.env.ENGINED!, '2.0'); out({ error: null }); }
  catch (e: any) { out({ error: e.message, code: e.code }); process.exit(0); }
} else {
  const eng = await open();
  await scenarios[scen](eng);
  await eng.close();
}
