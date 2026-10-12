# Scripts run in V8 without JIT, inside a confined worker per run

Code mode programs and Script Hooks run in the **Script sandbox**: V8, built from source with its own sandbox on and always without JIT, inside a worker process confined as ADR 0033 confines a Plugin at network level `none`, one set of workers per run. Both kinds of script are untrusted: a page can steer the model that writes a program through prompt injection, and a Recipe that carries a Script Hook may come from a stranger. A JavaScript engine is a large C or C++ program, so a bug in it hands the script whatever process it runs in; in the engine's own process that is every run, the resolved Secrets, Session cookies and the run key. The worker holds none of them, and the OS caps it where the engine's own limits fall short. Decided in [Code-mode JavaScript sandbox engine](https://github.com/Jbmanllr/automan/issues/125).

Evidence: [JavaScript engines for the code-mode sandbox and Script Hooks](https://github.com/Jbmanllr/automan/blob/research/js-sandbox-engines/rnd/research/2026-10-12-js-sandbox-engines.md) (measured on macOS arm64 and in aarch64 and x86_64 musl containers) and [Code-mode sandboxes in practice: engines, confinement and author contracts](https://github.com/Jbmanllr/automan/blob/research/code-mode-sandboxes/rnd/research/2026-10-12-code-mode-sandboxes-in-practice.md).

| | QuickJS-NG (`rquickjs` 0.14) | V8 without JIT (`v8` 152) | Boa 0.22 |
|---|---|---|---|
| Size added to the binary | +1.1 MB | +42.8 MB | +10.3 MB |
| One Script Hook call, JSON in and out | 4.8–5.5 µs | 1.4–2.3 µs | 11.7–11.9 µs |
| Memory limit | holds, but a script can catch the error | holds, with a near-heap-limit callback | none (grew to 1.15 GB) |
| Time limit | a 100 ms deadline overshot to 16.6 s on one long built-in | holds | loop and recursion counters only |
| test262 | 83.5%, no `Intl` or `Temporal` | 97.7% | 95.6% |

## Considered Options

- **Scripts in the engine's own process**: rejected. Fastest (one hook call in 1 to 5 µs instead of a pipe round trip), but one engine bug reaches every run and Secret. Both remaining engines also need a backstop the process cannot give itself: V8 aborts the whole process when it runs out of memory unless a callback catches it, and QuickJS's interrupt runs only every 10,000 bytecode steps. In-process JavaScript sandboxes keep being escaped: vm2 had about 60 advisories in 2026, n8n's `node:vm` runner was escaped twice in 2026, and Figma left its Realms shim for QuickJS inside WebAssembly.
- **QuickJS-NG through `rquickjs`**: rejected. It is 40 times smaller, but it had ten security advisories in 2026, several of them memory corruption on the out-of-memory path that a memory limit triggers, and on 2026-10-11 it stopped taking private security reports and issuing CVEs; it is not on OSS-Fuzz. It lacks `Intl` and `Temporal`, which models reach for when parsing prices and dates. `rquickjs` also prints failed jobs to stdout, which only the transport may write.
- **Boa**: rejected. It has no memory limit and nothing can stop a running script from outside; its interrupt pull request closed unmerged in August 2026.
- **`deno_core`**: rejected. 47 breaking releases in twelve months, it pins an older V8 than the `v8` crate, and it brings a runtime the sandbox does not need.
- **V8 with JIT**: rejected. On macOS the hardened runtime then needs the `allow-jit` entitlement, and an optimised loop is never stopped by `terminate_execution` ([rusty_v8#2088](https://github.com/denoland/rusty_v8/issues/2088), reproduced on 152.2.0). Without JIT it runs with no entitlement, every loop stops, and WebAssembly is gone, which narrows the attack surface.
- **The `v8` crate's prebuilt libraries**: rejected. They ship with V8's own sandbox off, their musl builds exist only since July 2026 and are not tested in the crate's CI, and the crate had not released since 2026-08-20 while Chrome fixed two V8 bugs exploited in the wild.
- **Type-checking programs before they run** (pctx, used by goose): rejected for performance. The TypeScript 6 compiler inside the sandbox adds about 9 MB and seconds per program without JIT; TypeScript 7's Go compiler would be a second binary per platform and has no stable programmatic API until 7.1. The engine already checks every catalog call's arguments against the schema of ADR 0038 before it acts, and the replay rule below makes a corrected program safe after earlier Effects.
- **TypeScript refused** (Cloudflare's prompt forbids it): rejected. Models given TypeScript declarations write annotations, and each refusal costs a turn.
- **State kept by a Script Hook between calls** (Cloudflare Workers keep globals without guaranteeing them): rejected. Results would depend on when a worker was last replaced, after a crash, a limit or a resume elsewhere (ADRs 0002 and 0003).
- **The Rust crate starting itself again as the worker** (Chromium's and Electron's pattern): rejected. It puts V8, its 43 MB and its musl link flag into every crate user's build, and a forgotten call in `main` fails only at run time.

## Consequences

**The engine.**
- `goodfetch` links V8 through the `v8` crate, built from source (`V8_FROM_SOURCE`) in release CI with V8's sandbox on, for every V1 target of ADR 0024. On musl the link needs `-lgcc` for `__clear_cache`. V8 always runs with `--jitless`.
- The engine's V8 is never more than one Chrome Stable major behind. When the `v8` crate lags, Goodfetch carries the update in a fork and sends it upstream; a release check fails when the rule is broken (adds to ADR 0030).
- The spec measures, on the reference host and with the bundled mimalloc, V8's cost with its sandbox on and the 3 to 5 times slowdown seen in a musl container, whose cause the research did not isolate.

**Confinement.**
- The worker is the `goodfetch` binary started in a worker mode, as `plugin-shim` is, confined as a Plugin at network level `none` with no file access, under ADR 0033's per-OS mechanisms. It reaches the engine only over its pipe and never holds a Secret: the engine inserts Secrets into the catalog calls it carries out, as it does for Plugins.
- Workers start lazily, at a run's first script, and are never shared between runs. The governor of ADR 0040 sizes each run's pool between one worker and the run's page concurrency, since an isolate runs one script at a time while it computes.
- Limits apply at two levels. V8's heap limit, its near-heap-limit callback and `terminate_execution` enforce them first; ADR 0033's resource limits are the backstop (cgroup v2 where delegated, else rlimits, on Linux; footprint sampling on macOS; a Job object on Windows), and the engine kills a worker past its hard deadline. The Linux rlimit fallback is `RLIMIT_DATA`, never `RLIMIT_AS`, since V8 reserves a 4 GB pointer-compression region and, with its sandbox on, far more address space than any memory limit.
- A worker killed for time or memory, or crashed, fails only the call it was running, with `script_time_exceeded`, `script_memory_exceeded` or `script_crashed`; a limit V8 itself enforces raises the same codes, and no script can catch them. A failed Script Hook fails its stage under ADR 0033's rules; a failed program returns the error and its captured `console` to the model. A new worker replaces the dead one under ADR 0033's crash rules.

**What a script is.**
- A program is JavaScript, or TypeScript whose types are stripped by `oxc` and never checked. A code-mode program is the body of an `async` function, with `await` at the top level, and returns a JSON value. A Script Hook is a single function expression (an arrow or a `function`), `async` or not, receiving its stage's context. There are no modules and no `import`.
- A script gets standard JavaScript including `Intl` and `Temporal`, and only the web APIs that touch nothing outside it: `URL`, `URLSearchParams`, `TextEncoder`, `TextDecoder`, `atob`, `btoa`, `structuredClone`, `crypto.getRandomValues` and `crypto.randomUUID`. `console` is captured into the Run trace and returned to the model. There is no `fetch`, no timers, no WebAssembly, no `require` or `process`, and no file or socket; waiting and fetching are catalog calls.
- Each catalog namespace is an async global (`await page.click(…)`, `await net.fetch(…)`), with the names and shapes of the TypeScript SDK (ADR 0038), from the same generator. Code mode's declarations are generated from the Operation catalog and trimmed to the MCP profile; a Script Hook's are those of its stage.
- A Script Hook has nowhere to keep state: its source must parse as one function expression, `globalThis` is frozen after setup, and one warm context per Script Hook serves the whole run. Lasting state goes in the run's Key-value store, whose operations Script Hooks gain (changes ADR 0033). Each code-mode program starts in a fresh context; what carries over travels through its return value or the Key-value store.

**Replay** (adds to ADR 0010).
- A corrected or resumed program is replayed against the Effect journal by position, the rule the lab measured at zero false successes in 135 runs: the same Effect at the same position is skipped and reported "already applied"; a different Effect while an Effect that cannot be repeated still lies ahead in the journal stops the program with `effect_diverged`, which it cannot catch; an Effect that only sets a value (`fill`, `select`) resumes live when nothing unrepeatable lies ahead.
- The engine carries out one program's Effects one at a time, so `Promise.all` cannot reorder them, as it does in Cloudflare's position-keyed runtime. Reads stay concurrent and are not journaled; model calls are, as ADR 0010 already says. `Date.now` and `Math.random` stay live, since only the order of Effects is compared.

**The Rust crate** (changes ADRs 0024 and 0033).
- The crate never links V8. For the Script sandbox and for `plugin-shim` it runs the `goodfetch` binary of its exact version, found through `GOODFETCH_ENGINE_PATH` or downloaded on first use into the shared cache, checked by digest and Platform signature (ADR 0039), as browsers are. Under `offline` (ADR 0043) a missing binary is a typed error.

**Other changes.**
- ADR 0011's code-mode sandbox and ADR 0033's sandbox for Script Hooks are the Script sandbox.
- ADR 0039: the `goodfetch` binary carries no `allow-jit` entitlement.
- Default limits (CPU time, wall time, memory, result and log sizes, number of catalog calls) are spec values, set through the layers of ADR 0028.

## Sources

- [rusty_v8](https://github.com/denoland/rusty_v8), [rquickjs](https://github.com/DelSkayn/rquickjs), [QuickJS-NG security policy](https://github.com/quickjs-ng/quickjs/blob/master/SECURITY.md), [Boa](https://github.com/boa-dev/boa), [deno_core](https://github.com/denoland/deno_core)
- [Cloudflare Code Mode](https://blog.cloudflare.com/code-mode/), [vm2 advisories](https://github.com/patriksimek/vm2/security/advisories), [n8n advisories](https://github.com/n8n-io/n8n/security/advisories), [Figma's plugin security update](https://www.figma.com/blog/an-update-on-plugin-security/), [oxc](https://github.com/oxc-project/oxc), [TypeScript 7 release](https://infoq.com/news/2026/08/typescript-7-released/)
- Lab: [code mode with an effect journal](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/code-mode-journal-2026-10-09.md)
