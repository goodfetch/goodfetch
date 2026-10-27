# Absolute ceilings and Performance targets join the per-change budget, and M1 ends with an optimization pass

ADR 0071's per-change **Performance budget** gains **absolute ceilings** on counted figures: per message, per URL in the Frontier and per crawled page. `xtask perf` checks them beside the +5% comparison, and no "Performance change" section passes a change over one. Wall time and resident memory get **Performance targets**, measured on the owner's machine and reported at each milestone's owner test. M1 ends with one **optimization pass**, which profiles the engine and cuts its top costs before the first ceilings are set from the tuned build. The resident-memory ceilings are then committed in M1 rather than M4.

ADR 0071 guards against regressions only. Every figure it gates is relative to the change's base, and an increase passes with a section the owner accepts. Its one absolute check, the drift check, compares `main` with the last release ([0071](0071-performance-budgets-are-counted-per-change-and-time-is-measured-on-the-reference-host.md#relation-to-the-benchmark-and-to-releases)), so before the first release nothing anchors the figures at all: accepted steps under 5% add up unseen. It sets no absolute figure, schedules no optimization work, sets its resident-memory ceilings from the first measured build, and leaves wall time against a competitor until M4. The owner wants an extremely memory-efficient, fast and correct core engine. The facts:
- **The R&D lab never measured the per-message or per-URL figures.** It recorded no allocations or bytes per message, no Frontier memory and no JSON-RPC overhead (`rnd/lab/reports/portal-2026-10-09.md:187-192`, `rnd/adr/0001-runtime-language.md:58,82`), and all its figures are relative ones from the owner's machine (`rnd/lab/hosts/local-controlled.json:3`). Its prototype's smallest Rust process peaked at about 18 MiB, its HTTP transport at 4.5–10 MiB, and its 16-concurrency workflow at 162 MiB (`rnd/lab/reports/integrated-2026-10-08.md:23`).
- **An idle Tokio process is small.** On the owner's machine (M3 Max, macOS 27.0.1, rustc 1.93.0, Tokio 1.53.3), a multi-thread runtime with a bound listener and a timer idles at 2.43–2.46 MB RSS and starts in 8.9 ± 2.5 ms; ripgrep 15.1 uses 3.0–5.5 MB, Bun 1.3 idles at 21.0 MB and Node 24 at 42.4 MB (local, unpublished, ADR 0076). Linkerd's Tokio proxy uses 5.7–26 MB under load ([Linkerd, 2021](https://linkerd.io/2021/11/29/linkerd-vs-istio-benchmarks-2021/)).
- **Crawlers at scale keep per-URL memory small or constant.** IRLbot stores 8 bytes per URL ([paper](https://archives.iw3c2.org/www2008/papers/pdf/p427-leeA.pdf)), Heritrix's default Bloom filter about 4 and Scrapy 20 in memory. BUbiNG keeps memory constant as URLs grow, with 64-bit hashes on disk, and rejects Bloom filters because a false positive silently drops a page ([paper](https://arxiv.org/pdf/1601.06919)).
- **Mature projects pair relative checks with absolute caps.** The Datadog Agent's quality gates fail when idle memory passes 181 MiB, beside Vector's and Datadog's relative regression detector ([experiment](https://raw.githubusercontent.com/DataDog/datadog-agent/main/test/regression/cases/quality_gate_idle/experiment.yaml)). TigerBeetle sketches memory, disk, network and CPU up front and bounds every loop and queue ([TIGER_STYLE](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md)). ADR 0057 already rejected both "ceilings alone" and "an increase check alone" for sizes.
- **Spider's published figures are its maker's own** ([README](https://raw.githubusercontent.com/spider-rs/spider-py/main/README.md)), and the lab's audit found its benchmark unfair: a warm in-process Spider against rivals started fresh, with no check that each did the same work (`rnd/research/2026-10-07-benchmark-audit-followup.md:17-38`).

Decided with the owner in a grilling session on 2026-10-18, before the engine tracer ([#214](https://github.com/goodfetch/goodfetch/issues/214)). It builds on ADRs 0040, 0057, 0058, 0060, 0061, 0064, 0071, 0076, 0108 and 0133.

## Decision

### Two kinds of absolute figures

- **Absolute ceilings are counted, and they gate.** `xtask perf` checks each one on every change, beside ADR 0071's +5% comparison with the base. A "Performance change" section cannot pass a change over a ceiling. Only the owner moves one, in either direction.
  > Amended on 2026-10-22 (owner decision of 2026-10-22): until the optimization pass below ends, performance is measured for information only. `xtask perf` still checks every ceiling and ADR 0071's +5% threshold and lists each figure over one with its change or value in the gate's log, as a note, but no merge gate fails on it, and no pull request needs a "Performance change" section or a "Performance change accepted" comment. From a nearly empty `main`, almost every ticket legitimately raises the startup, memory and instruction figures, so the per-change budget produced paperwork rather than signal; real waste is still found by the informational `just perf` the M1 run does every 10 merged tickets. A broken measurement still fails: a benchmark that fails its admission, or a ceiling whose figure the head lost. The switch is one constant, `GATING` in `xtask/src/perf/budget.rs`.
- **Performance targets are timed or resident, and they inform the owner test.** They are measured on the owner's machine, never on a hosted runner, and never fail a change. ADR 0071's rule that only counted figures gate a change stands.
- **A missed target is written up, and the owner decides at the owner test.** The milestone's report names each target, its measured figure and, for a miss, the gap and its cause. The owner signs the milestone off or keeps it open.
- **Targets move only by amending this ADR.**

### The figures

| Figure | Kind | Value | Measured by |
|---|---|---|---|
| Instructions per dispatched JSON-RPC message | ceiling | 1.10× the tuned value | gungraun, Callgrind, the dispatch benchmark |
| Allocations and bytes allocated per dispatched message | ceiling | 1.10× the tuned value | gungraun, DHAT |
| Heap bytes per URL in the Frontier, at 100,000 URLs | ceiling | **8 bytes** | gungraun, DHAT: bytes at the global maximum, less the same scenario with no URL, over the URL count |
| Bytes allocated per crawled page | ceiling | **2× the page's body bytes** | gungraun, DHAT, a crawl fed by an in-memory fixture |
| Startup to `hello`, each Product's Attached engine, median | target | **10 ms** | hyperfine on the owner's machine |
| Idle RSS of an Attached engine after `hello`, each Product | target | **8 MiB** | the resident-memory check's scenario |
| Idle RSS of a Daemon with no run, each Product | target | **16 MiB** | the resident-memory check's scenario |
| Peak RSS of the 1,000-page HTTP crawl at M1's concurrency | target | **32 MiB** | the resident-memory check's scenario, on Foxhound |
| Pages per second on that crawl | target | **at least Spider's**, in the same comparison run | the Spider comparison |

- **The 1.10× ceilings are set by the optimization pass,** at 1.10 times each figure of the tuned build, rounded up, with the owner's approval. About two accepted 5% steps then fit under a ceiling, and the third needs the owner.
- **The fixed ceilings start when their benchmark first runs.** The pass may propose a lower one, at 1.10 times a tuned figure below it, for the owner to approve.
- **The 8-byte Frontier ceiling needs an exact duplicate check outside memory.** The Frontier already lives in each run's SQLite file (ADR 0037, RT-10); what it keeps in memory is a bounded cache. A lossy filter (Bloom, cuckoo) is not used, because a false positive silently drops a page.
- **The 2× page ceiling holds the zero-copy rule:** a body is read once into `Bytes` and parsed in place.
  > Note (agent decision, pending owner review), 2026-10-27, [#1572](https://github.com/goodfetch/goodfetch/issues/1572): the crawl fed by an in-memory fixture is `goodfetch-foxhound`'s heap benchmark `crawl::in_memory_pages`, a code-free crawl (`follow: scope`, one field read from each page) of an index and twelve pages of about 1 kB to 1 MB, its bodies borrowed from a static fixture so serving them allocates nothing, counted over the whole process. It is fed through a new constructor of the fetch tiers, `FetchTiers::serving(transport)`, which sends every Run's HTTP hops with the given implementation of ADR 0129's `Transport` primitive in place of reqwest. That is one `pub` item added for a benchmark, against ADR 0071's note of 2026-10-24, because no pure module carries the crawl path: it crosses the engine, the Frontier, the fetch tier and the run file. Every Product composes `FetchTiers::new()`, so none sends through it.
- **Every target sits below or near a measured peer,** with room for what Goodfetch carries: mimalloc's arenas, `tracing`, settings and the transport, with V8, SQLite and the browser started only when used.

### The optimization pass

- **One M1 ticket,** blocked by every other M1 ticket of the engine, the Frontier and the fetch tiers, and blocking the M1 owner test ([#289](https://github.com/goodfetch/goodfetch/issues/289)). It is a ticket, not a standing process step (ADR 0133).
- **It profiles an idle Attached engine, an idle Daemon and the 1,000-page crawl** with the tools `TESTING.md` names, cuts the largest costs until each target is met or its gap is written up, and records before-and-after figures in its pull request's Evidence.
- **It then proposes the 1.10× ceilings** and the resident-memory ceilings below, for the owner to approve.
  > Amended on 2026-10-22 (owner decision of 2026-10-22): the pass's pull request, which sets the ceilings from the tuned build, also sets `GATING` in `xtask/src/perf/budget.rs` back to `Enforced`, with the tests that expect a note in its place. From then on the per-change budget and the ceilings gate again, and a figure past its threshold needs its "Performance change" section and the owner's acceptance.

### Resident-memory ceilings come from the tuned build, in M1

- **ADR 0071's rule is kept:** 1.5 times the highest peak of 20 runs, rounded up, with the owner's approval. It is measured on the build after the optimization pass, so the first build's waste is not locked in.
- **The ceilings are committed in M1** ([#440](https://github.com/goodfetch/goodfetch/issues/440) moves from M4 to M1, after the pass), for the OSes the private-period matrix runs: macOS arm64, Linux x64, Linux arm64 and Windows x64 (ADR 0108). Native macOS Intel and Windows arm64 get theirs in M4, once the repository is public.
- **The crawl's ceiling is tied to M1's concurrency.** If the default changes before 1.0 ([#178](https://github.com/goodfetch/goodfetch/issues/178)), that ticket re-measures it.
- **A ceiling may sit above its target.** The ceiling catches a blow-up on any change; the target is checked again at each owner test.

### Memory at scale

Both scenarios run in the Release suite on every OS, are checked by a ratio so the machine does not matter, as the Console's leak check is, and fit `TESTING.md`'s five-minute cap per test.

- **The Frontier at scale:** one process enqueues 100,000 synthetic URLs, then 1,000,000, with no network. Peak RSS at 1,000,000 is at most **1.5×** peak RSS at 100,000.
- **The Daemon soak:** one Daemon runs **20** consecutive 1,000-page Proving ground crawls. Its RSS after the 20th is within **10%** of its RSS after the 3rd; the first three are warm-up. If 20 crawls take more than about two minutes, the scenario lowers the pages per crawl, never the crawl count.
- **Both pin the resource governor's memory allowance** (ADR 0040) far above the expected peak, so neither measures load shedding.
  > Note (agent decision, pending owner review), 2026-10-25, [#1571](https://github.com/goodfetch/goodfetch/issues/1571): the Frontier at scale runs the Frontier's admission in the test's own process, against a run file on disk opened as a crawl's is, and reads that process's peak resident memory (`getrusage`, or the peak working set on Windows) after 100,000 URLs and after 1,000,000; no engine runs, so no governor sheds load and none needs pinning. An Attached engine fed through `run.add_seeds` would also start an attempt for each seed, whose fetches the check must not make. Outside the Release suite the same check runs on 10,000 and 100,000 URLs. The run file's writer bounds its SQLite page cache at 512 KiB (`run_state::PAGE_CACHE_KIB`): SQLite's default of 2,000 KiB alone came to 20 bytes per URL at 100,000 URLs, over the 8-byte ceiling. The heap benchmark counts the whole process (DHAT with no entry point), since an allocation of SQLite's page cache sits deeper than DHAT's 25 recorded callers below the benchmark function, where the default entry point would leave it out.

### The Spider comparison

- **`just compare-spider`, on the owner's machine, in M1.** It crawls the same 1,000-page Proving ground site with Foxhound and Spider, each in a fresh process each trial and at the same concurrency. It reports pages per second and peak RSS only after checking that both fetched the same pages and the same bytes. The site has no redirect to a private address, which Spider's own guard refuses.
- **Its report is labelled `local-controlled` and never published** (ADR 0076). It is linked under the M1 owner test's Reports, beside the feature, coverage and mutation reports (ADR 0072). It is not a Live check and gates nothing.
- **The release-candidate benchmark and the Performance series stay in M4** ([#147](https://github.com/goodfetch/goodfetch/issues/147), [#313](https://github.com/goodfetch/goodfetch/issues/313)); their competitor adapters may absorb the recipe.

### Tools and rules

- **`TESTING.md` names the profiling tools:** samply for CPU on macOS and Linux; gungraun's DHAT and heaptrack for the heap on Linux (on the owner's machine, in the Lima VM), with the gnu build and the system allocator `benches/heap.rs` already uses; Instruments on macOS when needed; and for async behaviour, the ops listener's metrics and Tokio's stable runtime metrics through `tracing`.
- **tokio-console is not adopted.** It needs the `tokio_unstable` cfg and `console-subscriber`, which starts its own thread and opens its own TCP listener outside `transport` (ADR 0060), behind a Cargo feature.
- **`STYLE.md` gains memory rules:** bodies and frames as `Bytes`, never copied on the per-request path; every channel and buffer between tasks bounded, with backpressure when full; no `String` clone per request; the Frontier's in-memory structures bounded and its duplicate check exact.
- **This ADR, its `STYLE.md` and `TESTING.md` edits and its glossary entries are accepted before #214 merges,** so the engine's first code follows them.

## Considered Options

- **Absolute figures as goals only, gating nothing:** rejected. Before the first release the ratchet has no anchor and the drift check compares nothing, so a goal no one checks drifts the same way.
- **Gating every figure per change, wall time included:** rejected, as in ADR 0071. Hosted runners vary by about 2.7% in wall time, so a duration gate gives false alarms; counts repeat exactly.
- **Fewer figures (dispatch, startup and idle RSS only):** rejected. Memory per URL in the Frontier and per page is where a crawler's memory goes as it grows, and without pages per second the memory targets could be met by crawling slowly.
- **Targets from the lab prototype (20 ms, 20 MiB idle, 64 MiB crawl):** rejected. The prototype did real work; a bare Tokio daemon idles at about 2.4 MB, so 20 MiB would allow Bun's footprint.
- **A ceiling at the tuned value exactly, or at 1.25×:** rejected. The first makes every accepted increase an ADR amendment; the second lets about five 5% steps pass unseen.
- **Resident-memory ceilings committed in M4, as planned:** rejected. M2 and M3 add the SDKs, MCP and the browser with no resident-memory gate, and the ceilings would lock in the first build's waste.
- **Crawlee as the comparison:** rejected. The lab measured pooled Crawlee at 245–512 MiB per task, so beating it says little; Spider is the Rust HTTP crawler nearest Foxhound's M1 crawl.
- **The comparison as a Live check:** rejected. A Live check's failure opens an issue in the current milestone (ADR 0064) and would hold the owner test on a competitor's result.
- **A Frontier ceiling in bytes per URL alone:** rejected. It allows memory to grow in step with the Frontier; the ratio makes bounded growth the design property.
- **A soak of several hours:** rejected for now. No suite holds it (five-minute cap per test, 600 hosted minutes a month while private), and the owner's machine is not always on. A soak counted in crawls finds a per-crawl leak in minutes. A timed Live check can follow if real Daemons leak over time.
- **tokio-console:** rejected, as above.

## Consequences

- **Amends ADR 0071,** with a note at each spot: what may fail a change (absolute ceilings), the threshold (ceilings beside +5%), the resident-memory ceilings (tuned build, M1), memory beyond the heap (the two scenarios) and the drift check (ceilings hold before the first release).
- **`STYLE.md`** gains its memory rules under Rust; **`TESTING.md`**'s Performance budgets section gains the ceilings, the targets, the scenarios at scale and the profiling tools.
- **`GLOSSARY.md`** widens **Performance budget** to the absolute ceilings and adds **Performance target**.
- **New tickets:**
  - in M1: the optimization pass ([#1577](https://github.com/goodfetch/goodfetch/issues/1577)); the absolute ceilings in `xtask perf` ([#1570](https://github.com/goodfetch/goodfetch/issues/1570)); the Frontier's heap benchmark and its scenario at scale ([#1571](https://github.com/goodfetch/goodfetch/issues/1571)); bytes per crawled page ([#1572](https://github.com/goodfetch/goodfetch/issues/1572)); the Daemon soak ([#1573](https://github.com/goodfetch/goodfetch/issues/1573)); `just compare-spider` ([#1574](https://github.com/goodfetch/goodfetch/issues/1574)); `just targets`, the Performance targets report linked from the owner test ([#1575](https://github.com/goodfetch/goodfetch/issues/1575));
  - in M4: the resident-memory ceilings for macOS Intel and Windows arm64 ([#1576](https://github.com/goodfetch/goodfetch/issues/1576)).
- **Changed tickets:** [#440](https://github.com/goodfetch/goodfetch/issues/440) moves to M1 and is blocked by the optimization pass; [#390](https://github.com/goodfetch/goodfetch/issues/390) also reports each idle and crawl figure against its target; [#289](https://github.com/goodfetch/goodfetch/issues/289)'s Reports line names the targets report and the Spider comparison.
- **Owner approvals:** every 1.10× ceiling and every resident-memory ceiling, after the pass.
- **A design constraint for the Frontier's tickets:** they must meet 8 bytes per URL in the Frontier and the 1.5× ratio, with an exact duplicate check.
