"""Render the report from audited campaign aggregates; no benchmark execution."""
import json, sys
from pathlib import Path
root=Path(__file__).resolve().parent.parent
analysis=json.loads((root/'reports/integrated-analysis.json').read_text())
current=Path(sys.argv[1]).resolve()
cs=json.loads((current/'summary.json').read_text())
campaigns=analysis['campaigns']
def by(prefix):return next(v for k,v in campaigns.items() if k.startswith(prefix))
def fmt(x,n=2):return '—' if x is None else f'{x:,.{n}f}'
def link(v):return f"[summary](../{v['directory']}/summary.json), [manifest](../{v['directory']}/manifest.json)"
def table(headers,rows):return '\n'.join(['| '+' | '.join(headers)+' |','| '+' | '.join(['---']*len(headers))+' |',*['| '+' | '.join(map(str,r))+' |' for r in rows]])
native=by('runtime-native-qualified-');recovery=by('runtime-recovery-qualified-');guard=by('browser-actionability-');readers=by('sqlite-readers-');
# The first SQLite series is diagnostic; select the current step explicitly.
reader_dir=next(s['directory'] for s in cs['steps'] if s['name']=='sqlite-readers-qualified')
readers=campaigns[Path(reader_dir).name]
spider=by('spider-http-transport-supported-')
sections=[]
def add(s):sections.append(s.strip())
add('''# Goodfetch cross-cutting qualification — October 8, 2026

**Rust/Tokio remains the candidate for the portable core.** Node/TypeScript is the experimental control and the compatibility path. The native Rust workflow reaches throughput close to Node with less measured CPU and footprint; none of these campaigns qualifies a Rust Chromium/CDP driver yet.

The prototype runs **acquisition → extraction → validation → bounded queue → batched writer → commit → ACK**, separately in Node and Rust. The writer is a dedicated process. Both ledgers load the same SQLite 3.53.4 library and the same generic C SQL shim, in WAL FULL; their job logic is independent. Extraction is specialized for the lab corpus.

The first suite covers the five requested areas, including transports and saturation. A second suite requalifies the fixes to Rust configuration, click stability and SQLite initialization. The campaigns are sequential; the initial data and diagnostics remain preserved.''')
add(f"[First suite](../{analysis['suite']}/summary.json), [requalification](../{current.relative_to(root)}/summary.json). Sources/binaries unchanged during the retained campaigns. [Reproducible analysis](integrated-analysis.json), [SHA-256 catalog](integrated-evidence-catalog.json), [source audit](integrated-source-audit.json), [contract audit](integrated-verification.json).")
add('## Complete runtime: Rust remains worthwhile')
add(f"**{native['statuses'].get('verified',0)}/30 verified cells**, three randomized blocks, **{native['jobs']:,} jobs / {native['records']:,} records**. C × 192 jobs per cell, 16 pages × 2 records/job, 4 ms/page server delay, C1/4/16, batch 1/4 with B ≤ C. Work windows of about fifteen seconds. Weighted throughput = exact jobs / work + finalization time. Complete throughput adds startup and the external oracle.")
nrows=[]
for c,b in [(1,1),(4,1),(4,4),(16,1),(16,4)]:
 n=next(g for g in native['groups'] if g['config']['runtime']=='node' and g['config']['concurrency']==c and g['config']['batchJobs']==b)
 r=next(g for g in native['groups'] if g['config']['runtime']=='rust' and g['config']['concurrency']==c and g['config']['batchJobs']==b)
 nrows.append([c,b,fmt(n['weightedJobsPerSecond']),fmt(r['weightedJobsPerSecond']),fmt(n['completeJobsPerSecond']),fmt(r['completeJobsPerSecond']),fmt(n['cpuMsPerJob']),fmt(r['cpuMsPerJob'])])
add(table(['C','Jobs/batch','Node jobs/s','Rust jobs/s','Node complete/s','Rust complete/s','Node CPU ms/job','Rust CPU ms/job'],nrows))
add('''At C16/B1: about 197 jobs/s for both implementations; Rust uses 1.42 ms CPU/job versus 2.78 for Node, about 49% less measured CPU. Maximum sampled footprint per cell: Rust 136–178 MiB, Node 314–325 MiB. This supports Rust for efficiency under comparable tested guarantees, without proving a universally faster scraper.

Copying the whole configuration corpus per Rust producer was replaced by immutable `Arc` sharing. The first C16/B1 series gave 205.55 Rust / 205.30 Node jobs/s and 204–248 / 306–327 MiB of footprint. The two series were run at different times on a shared Mac: the change is real in the code, but this memory and throughput difference is not a causal trial isolating `Arc` alone. The first series and its binary are preserved. B4 batching of the complete workflow does not systematically beat B1: acquisitions, actual batch size and drain also matter.''')
add(link(native)+'\n\n[First native series](../results/runtime-native-sustained-2026-10-07T23-21-59-298Z-bf6ecb/summary.json), [previous Rust binary](evidence-integrated/artifacts/automan-workflow-rust-before-arc).')
add('## HTTP acquisition: comparing lifecycles and saturation')
back=campaigns['runtime-http-backends']
add(f"**{back['statuses'].get('verified',0)}/24 cells**, {back['jobs']:,} jobs / {back['records']:,} records in the durable workflow. The runtime is Node when an external backend is used; resources then include its descendants. Native Rust is a separate complete implementation. A shared pool and a fresh crawler per task have different costs.")
add(table(['Runtime/backend','C','Weighted jobs/s','Min–max per block','CPU ms/job'],[[g['config']['runtime']+'/'+g['config'].get('backend','native'),g['config']['concurrency'],fmt(g['weightedJobsPerSecond']),fmt(g['blockJobsPerSecond']['min'])+'–'+fmt(g['blockJobsPerSecond']['max']),fmt(g['cpuMsPerJob'])] for g in sorted(back['groups'],key=lambda g:g['key'])]))
add('''Large differences between blocks appear, particularly for Crawlee pooled and some C16 cells. The host diagnostic shows other heavy activity; it documents a possible confounding factor and does not prove the cause of each slowdown. No general runtime penalty can be inferred from these variations alone.''')
add(link(back)+'\n\n[Load diagnostic](host-load-2026-10-08.json).')
add('''The open-loop campaigns offer 20/100/250 tasks/s, concurrency 4, three 12 s windows, eight pages with a 3 ms delay. They cover acquisition only, without a durable ledger. A rejected task counts in the denominator; admitted tasks may finish after the window. `queueMs` measures the generator's lag, with no hidden business queue.''')
openrows=[]
for rate in (20,100,250):
 v=campaigns[f'http-open-{rate}']
 for s in v['summary']['summaries']:
  m=s['measurement'];openrows.append([rate,s['backend'],m['verified'],m['attempts'],m['rejected'],fmt(s['throughput']['verifiedTasksPerSecond']),fmt(m['successLatencyMs']['p95'])])
add(table(['Offered/s','Backend','Exact','Offered','Rejected','Exact/s','p95 success ms'],openrows))
add('''Node fetch and Spider HTTP are good simple paths on this corpus. The pool avoids some costs of a fresh crawler per task, without erasing its guarantees and orchestration overhead. The rejections at 250/s expose a bounded capacity; low latency on successes alone does not mean that the whole load was served.''')
add('\n\n'.join(link(campaigns[f'http-open-{r}']) for r in (20,100,250)))
add(f"Transport: Node/fresh Crawlee/pooled Crawlee each verify 15 conformance checks and 45 measurements in the first series. Spider refuses the three loopback redirects: its native SSRF policy blocks hops to a private address, even when the local origin is intentional. This is not evidence that it cannot follow public redirects. The policy was not bypassed; the [preserved Spider 2.53.9 source](evidence-integrated/artifacts/spider-2.53.9-website.rs) contains `is_ssrf_redirect` and `ssrf_limited_policy`. The separate normal/gzip/chunked/large-response series gives **{spider['statuses'].get('verified',0)}/60 verified rows** ({spider['measureCount']} normal measurements; the variants are conformance checks).")
add(link(campaigns['http-transport'])+'\n\n'+link(spider))
add('## Browser: the raw Stagehand gain requires explicit guarantees')
browser=campaigns['browser-actionability']
add('''Six backends × nine variants × three blocks. The native policy only adds waiting for document initialization. The common guard enforces uniqueness, visibility, enablement, stability over two intervals between three RAF observations at least 16 ms apart, then a hit test. The oracle checks target, order, phase and trusted event. For the three negative variants, it requires refusal + zero effects; a verified cell therefore does not always imply a click.

The initial guard had four false successes on a moving target. A repeated public test reproduced them; the fix passed sixteen interactions, then the new matrix. The first attempt, limited to moving the observation into RAF, still failed: its diagnostic log remains preserved.''')
add(table(['Backend','Native: conforming/27','Corrected guard: conforming/27','Non-conforming native variants'],[[g['key'].split('/')[0],g['statuses'].get('verified',0),next(x['statuses'].get('verified',0) for x in guard['groups'] if x['key'].split('/')[0]==g['key'].split('/')[0]),', '.join(v['variant'] for v in g['variants'] if v['statuses'].get('failed',0)) or '—'] for g in browser['groups'] if g['key'].endswith('/native')]))
add(table(['Guarded backend','p50 normal task ms','p50 moving target ms','p50 normal startup ms'],[[g['key'].split('/')[0],fmt(next(v for v in g['variants'] if v['variant']=='normal')['taskMs']['p50']),fmt(next(v for v in g['variants'] if v['variant']=='moving')['taskMs']['p50']),fmt(next(v for v in g['variants'] if v['variant']=='normal')['startupMs']['p50'])] for g in guard['groups']]))
add('''The historical 50-click result (Stagehand 0.95 s / Playwright 2.71 s) used common readiness conditions. The new matrix shows why it should not be turned into an immediate replacement for Playwright: the tested native Stagehand primitives often fail on overlay, enablement, mutation and ambiguity. Playwright, Crawlee/Playwright and the Crawlee/Stagehand v3 extension pass this native matrix; the extension's handler uses a Playwright page attached over CDP.

The corrected guard provides another explicitly tested configuration; it does not prove all of Playwright's guarantees, nor the removal of the race between guard and click. Refusals and successful clicks have distinct latencies. Stagehand v4 does not expose the same locator timeout: guards/Playwright/Puppeteer budget 800 ms, Stagehand outer deadline 8 s. Startups are reported separately. No LLM call is made; observe/act/extract and self-heal remain outside the measurement.''')
add(link(browser)+'\n\n'+link(guard)+'\n\n[Browser contract](../docs/browser-actionability.md), [red regression](qualification-guard-red.log), [RAF attempt still failing](qualification-guard-green.log), [corrected regression](qualification-guard-calibrated-green.log).')
add('## SQLite: batches, writers, readers and checkpoints')
add(f"**{readers['statuses'].get('verified',0)}/108 verified cells**, **{readers['rows']:,} records**. Node N-API, synchronous Rust FFI and Go/cgo load the same C artifact: W1/4, B100/1000, no/short/long readers, three blocks. Each cell contains 32,000 rows of 512 bytes, WAL FULL, eight producers. The write windows are short; the oracle rereads every record and checks integrity after closing.")
add(table(['Binding','Writers','Batch rows','Rows/s write','Rows/s complete cycle','p95 RPC ACK ms'],[[*g['key'].split('/')[:2],g['key'].split('/')[3],fmt(g['weightedRowsPerSecond'],0),fmt(g['completeRowsPerSecond'],0),fmt(g['ackLatencyMs']['p95'])] for g in sorted(readers['groups'],key=lambda g:g['key']) if '/none/' in g['key']]))
add('''B1000 increases W1 throughput in all three bindings. W4 is not systematically slower: Node/B100 wins in this short series and Rust/B1000 is close to W1. Large W4 batches do, however, clearly increase p95 RPC latency, without a consistent throughput gain; the historical results and this series motivate the single writer as the baseline, not as a universal rule.

The 31 startup failures of the first Rust/Go W4 matrix occur before insertion. The schema and WAL mode were being created concurrently. Common initialization is now performed once before opening the actual concurrent writers; the W4 test with a long reader went from red to green. The writers remain independent during the measured window: the fix does not artificially serialize their writes.

A long reader must keep seeing zero rows during the inserts. PASSIVE then leaves non-checkpointed frames; after release, TRUNCATE succeeds with the WAL at zero. Short readers run 2 ms transactions every 10 ms, with a bounded increasing count. All these invariants are checked in the audit. A dedicated writer and batched transactions remain the starting point supported by these experiments; multiple writers do not remove SQLite's single write lock. [SQLite WAL](https://www.sqlite.org/wal.html).

The RPC p95 values come from the raw ACKs. The CPU/footprint measurements for this track include the already-running coordinator, whose initial counters are subtracted; its earlier allocations prevent ranking the languages' intrinsic memory. The historical JSON/JSONL, durable snapshot, NORMAL/FULL and SQLx results remain in the earlier reports, with their separate contracts and builds.''')
add(link(readers)+'\n\n[Readers/checkpoints contract](../docs/sqlite-readers.md), [diagnostic of the 31 failures](sqlite-initialization-diagnostics.json), [red regression](qualification-writers-red.log), [six fixed cases](qualification-writers-green.log), [historical writes](workflow-2026-10-08.md).')
add('## Durability: two runtimes, same observable invariants')
add(f"**{recovery['statuses'].get('verified',0)}/42 verified cells**, {recovery['jobs']:,} jobs / {recovery['records']:,} records, 42 writer restarts and 12 coordinator restarts. SIGKILL before commit, after commit, during ACK, after ACK; new coordinator after claim or commit; three repeated faults with a slowed writer. The initial series also had 42/42 verified cells.")
add(table(['Runtime/case','Cells','Writer restarts','Coord. restarts','Jobs/s complete cycle','p50 coord. recovery ms'],[[g['key'],g['statuses'].get('verified',0),g['restarts'],g['coordinatorRestarts'],fmt(g['completeJobsPerSecond']),fmt(g['coordinatorRecoveryMs']['p50'])] for g in sorted(recovery['groups'],key=lambda g:g['key'])]))
add('''The results and the receipt are committed before ACK, then ACK is recorded in a separate transaction. Replaying an already committed job reuses the receipt; old lease tokens are rejected. The external oracle reopens the database after shutdown and compares every record. The checkpoints at crash time attest a claim without results or a commit with results before recovery.

For coordinator recovery, throughput uses the outer complete time: the work time of the recovered process alone would omit part of the incident. The results queue stays bounded by C even with a slow writer. These trials prove the prototype's invariants under process crashes; they do not qualify power loss, exactly-once external effects, heartbeat or cloud cluster.''')
add(link(recovery)+'\n\n[Workflow and reproduction](../docs/runtime-workflow.md).')
add('## Verification, reproducibility and choices still open')
add('''The post-campaign audit passes **510 checks**, including trusted effects/refusals, bounds, restarts, checkpoints and frozen sources. It complements the record oracles run during the campaigns.

The **57 contract tests pass**, zero failures/skips, including false success, protocol/timeout, recovery of both runtimes, fencing, macOS CPU calibration, moving-target click and W4 startup. TypeScript and Rust formatting pass; the current Rust binary was rebuilt before requalification. [Test log](qualification-current-tests.log), [build](qualification-current-build.log).

The manifests pin versions, seeds, environment, and source and binary SHAs. Sources are copied before the trials and audited again afterwards. The catalog references the original evidence without exporting the oracle's private inputs; the complete databases and exports remain in the results folders. The analysis aggregates raw events for the quantiles, without averaging block p95s. Errors from the compared tools and the lab's own initial errors remain distinguished.

The macOS probe is calibrated against getrusage and converts Mach counters with `mach_timebase_info`. It measures the native coordinator and descendants, excludes fixture/oracle in the native track, samples every 100 ms and can miss spikes or process exits. RSS can count shared pages several times; footprint and I/O bytes are distinct indicators. The first series with wrong CPU units was interrupted and excluded. A 400 ms shutdown timer was fixed before the retained campaigns.

The lab currently supports: pursuing Rust for the durable HTTP core; keeping Node for reference and compatibility; preferring a simple HTTP path when rendering is not needed; keeping Playwright as the reference for robust interaction and evaluating Stagehand with its explicit guarantees; starting SQLite with one writer and batched transactions. These are prototype directions grounded in this corpus, not a frozen final architecture.

Still to qualify: a co-located Rust/CDP driver against Node, dedicated Linux, TLS/HTTP2/proxies/429 and retry, general/streaming parsing, holdout corpus, multi-tenant sessions, continuous admission/heartbeat, additional sinks and perception/LLM with identical budgets. The next experiments must resolve these specific uncertainties before any promise of universal performance.''')
add('[Coverage matrix](../docs/coverage-matrix.md), [gates and evidence](../docs/qualification-next.md), [index of all experiments](index.html).')
(root/'reports/integrated-2026-10-08.md').write_text('\n\n'.join(sections)+'\n')
