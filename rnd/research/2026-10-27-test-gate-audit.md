# Test gate audit

Audit of Goodfetch's test gates on 2026-10-27, for the owner's question: are they incremental, do they follow the `test-gating` skill, does a failed gate rerun everything or resume, and are they optimized without losing accuracy. Analysis only: no gate, script, ADR or setting was changed, and nothing heavy ran on the server (5 implementers and the spec gate share it). Every number comes from a log, the code or a cheap command, with its source; what could not be measured is marked.

This builds on the owner-facing assessment of 2026-10-21 (`.claude/handoff/m1-test-gates-assessment.md`), which judged the checks themselves "high-end" and their schedule the problem. That still holds. This audit looks at the gate's machinery: selection, reuse and the red-fix-rerun loop.

## 1. Verdict

| Question | Short answer |
|---|---|
| Incremental? | **By design, yes; in practice on the spec gate, no.** `xtask affected` selects per member and ADR 0135's gate records and reuses green results per step and per member. On #1583's last rerun the plan reused nothing: 20 of 20 steps reran in full. |
| Follows `test-gating`? | **Mostly.** Fail-safe selection, a fixed gate, red never reused, zero retries, a day's expiry and the honest plan log all match. Gaps: no T3 backstop yet (#1591, open), no step durations in the log, a red step marks every member red, results of the steps run beside are lost on an early stop, and the run went past the skill's "stop after two red rounds" without the candidate holding still. |
| After a failure: rerun or resume? | **Reruns everything the next time, in practice.** The design resumes (it reruns only red, missing or reached units), but four things defeat it on the spec gate (section 4). Inside a step, nextest runs every test with fail-fast off, which is right. |
| Optimized without losing accuracy? | **The test runner is; the loop around it is not.** Pre-merge runs 6,732 s of tests in 455 s (about 15 at once on 16 cores). The waste is in whole-gate reruns on a moving head, a stress run that keeps going 3 hours after its first red iteration, and lint failures the merge step could have caught. |

## 2. Question 1: is each tier incremental?

| Tier or command | What decides the scope | Incremental? | Evidence |
|---|---|---|---|
| Pre-push hook | `.githooks/pre-push` runs `just quick` | Yes: affected members, against the merge base with `origin/main` | `.githooks/pre-push`, `justfile:736` |
| `just test-affected <suite>` | `xtask affected --local` or `--base/--head` | Yes, per crate, pnpm and uv package; dependants included; unowned path → everything | `justfile:714`, `xtask/src/affected/selection.rs:256-280` |
| `xtask affected` `gate=quick` | No test, benchmark, model, container or Evaluation reads the change (docs, `rnd/`, top-level Markdown) | Yes: skips perf, Linux-only, visual, Pre-merge, stress, TLC, MSRV, Miri | ADR 0135 amendment of 2026-10-21; `ci/pre-merge.sh:59-72` |
| `just merge` → `just pre-merge all` | `xtask gate plan` over `target/gate-runs/` | Yes by design: per step (`Always`, `Job`, `Crates`) and per member (Quick, Pre-merge, stress) | `xtask/src/gate/plan.rs:106-128`, `ci/pre-merge.sh:127-247` |
| Whole-change checks (dco, commit messages, lint, audit, commit leaks, fuzz-check, proofs, Journey tasks, Feature and Client tests, notices, feature combinations) | `Reach::Always` | No, on purpose: every run reruns them (the skill's "quick checks always run") | `plan.rs:107-118` |
| Vitest, Playwright, pytest members | The head's order seed | No, on purpose: every member reruns in the head's order (owner's F8 option B, 2026-10-22) | `plan.rs:308-313`, ADR 0135 |
| Stress | New and changed tests against the merge base with `origin/main` | Yes by test; per member on a rerun; base move reruns it | `plan.rs:141-145`, TESTING.md "Before merge" |
| Perf, visual, TLC, MSRV, Miri | `xtask affected` jobs; MSRV and Miri on any crate | Yes, per job | `plan.rs:119-125` |
| Mutants | `m1/spec-mutants.sh` after a green spec gate | Changed lines only (ADR 0072) | `.claude/handoff/m1/spec-check.sh:80` |
| Merge queue (`m1/merge-queue.sh`, per ticket merge) | Fixed list | No tests: `cargo check`, commit messages and a growing list of lint pieces | `merge-queue.sh:54-82` |
| Smoke (`m1/smoke.sh`, per batch) | `--base smoke-base --head tip` | Yes: affected Quick, codegen, Feature, modules, compat, typos, typecheck when TS changed | `smoke.sh` |

Where it runs everything, and why:

- **A gate input changed.** `GATE_INPUTS` (`plan.rs:154-173`): the justfile, `ci/`, `.config/`, `.cargo/`, the root manifests and lockfiles, `rust-toolchain.toml`, `xtask`, the Proving ground's pages and fixtures, shared fixtures, `chromium/`, Collie's pins. Any of these reruns every step in full.
- **Vitest, Playwright and pytest** always rerun every member (above).
- **An unowned path** in `xtask affected` selects everything (`selection.rs:271`), as the skill's invariant 1 requires.

## 3. Question 2: mapping to the `test-gating` skill

| Skill rule | Goodfetch | Status |
|---|---|---|
| Inv. 1, fail-safe selection; unmatched → everything; base must be an ancestor | `selection.rs:271`; `merge-pr.sh:34` refuses a head without its base's tip | Met |
| Inv. 2, backstop: what a gate skips runs in T3 on a cadence, with an owner who reads it | `ci/gate-backstop.sh` (daily full run, compares reused results with it) is in #1591, open as a draft since 2026-10-23 | **Gap until #1591 merges** |
| Inv. 3, same key, same result; red always reruns; quick checks always run; tooling change reruns all; expiry; flaky steps every time | Key = step + member + host fingerprint (kernel, packages, runtimes, runners, toolchain, environment) + origin head's change; red never reused; day expiry; gate inputs rerun all | Met. One coarseness: a red step marks every member it ran red (`gate.rs:660-672`), where the skill's unit of record here is the member |
| Inv. 4, fail fast: cheap checks first | Seconds-checks, then Quick, then Pre-merge, then stress (`pre-merge.sh:195-240`) | Met between steps. **Not inside stress**: `--no-fail-fast` across 20 iterations (`xtask/src/repeat.rs:302`); rerun 10's stress went on for about 3 h after its first red iteration |
| Inv. 5, discover locally, confirm in hosted CI once per ready change | Hosted CI off except the Release suite (ADR 0135 amendment of 2026-10-21, money) | Recorded deviation (ADR 0076) |
| Inv. 6, honest scope: what ran, reused, skipped, and its durations; one line on what green proves | Plan log per step with reasons and origins; merge commit names the reused run; skipped-on-host list | **Partly**: no per-step start, end or duration in the gate log, so the audit could not split wall-clock by step (section 5) |
| Inv. 7, the gate holds still | Gate inputs rerun all; narrowing variables refused (`pre-merge.sh:97-104`, widened in #1591) | Met for the gate's own files; collected-test ID comparison against the base exists for stress listings only |
| Tiers: T1 ≤ 2 min local; T2 ≤ 30 min | Quick budget ≤ 60 s local; Pre-merge "all green" ≤ 20 min (TESTING.md "Time budgets") | **Over budget**: the spec gate reaches the end of Pre-merge in about 32 min and runs 75 to 210 min with stress |
| Batches: T1 per item, T2 once per batch, batches a week at most | T1 per ticket by implementers; T2 once per spec | **Gap**: spec #577's batch is 2,654 commits since `main` (`spec-gate-577.log`), not a week |
| before-pushing: after two red rounds on one change, stop and report | Spec #577 ran 11 rounds, each root-caused into fix items | **Partly**: each round was root-caused (ADR 0064), but each tested a new head with new tickets in it, so rounds kept finding new failures |
| before-pushing: rerun repeats what the fix can affect, reuses the rest | Designed; never fired on #1583 (section 4) | **Gap in practice** |
| Flaky policy: retries off; quarantine with owner and expiry | Zero retries, every lock on; quarantine list with issues; 200 runs to leave it (TESTING.md) | Met, stricter than the skill |
| Gotcha: timing failures on a loaded machine are environment faults; fix how the suite runs | Memory floor, `push-hold`, cargo shim holding implementer tests, renicing; FQ120, FQ123 rewrote disk-bound waits | Met; three reruns (7, 9, 11) still hit load-sensitive timings |
| Mutation of changed lines, advisory, when an agent wrote the tests | `spec-mutants.sh` after a green spec gate | Met |

## 4. Question 3: after a red gate, what reruns?

### What the design does

`xtask gate plan` (`plan.rs:296-401`) gives each step `run`, `part` (some members) or `reuse`. A unit reuses its green result only when it is green, from the same host fingerprint, under a day old, with the same base for perf and stress, no gate input changed since, and `xtask affected` plus the read evidence (Cargo dep-info, runner listings, literal file names) do not reach it. Everything else reruns. That is the skill's incremental gate, and finer than most: it is per workspace member.

Inside a step nothing resumes, and that is correct: nextest runs with `fail-fast = false` and `retries = 0` in every profile (`.config/nextest.toml:17-50`), so a red run lists every failure at once. nextest 0.9.149 on the server has `-R/--rerun`, which runs "only tests that didn't pass" in a recorded run, plus new tests (`cargo nextest run --help`). That is `pytest --lf`: right for the fix loop at T0, wrong for a gate verdict, because a test that passed on the old head is not rerun on the new one even when the fix changed what it runs.

### What happened on #1583

| Rerun | Head | Start → red (UTC) | Time to red | Red in | Kind |
|---|---|---|---|---|---|
| 2 | 200f0ce2 | 10-25 06:20 → 06:50 | 30 min | Pre-merge, 2 tests | product/test defects |
| 3 | b965bf02 | 09:14 → 09:46 | 32 min | Pre-merge, 3 tests | defects |
| 4 | 524f657a | 14:33 → 18:05 | 3 h 32 | stress 0/20 (LazyFS) and one-offs | environment + defects |
| 5 | 12ee3b5e | 10-26 10:12 → 10:14 | 2 min | lint (oxlint) | merge step missed it |
| 6 | 317bf5c2 | 11:33 → 12:16 | 43 min | Playwright `/settings` walk | defect |
| 7 | 180b460f | 12:34 → 13:17 | 43 min | Pre-merge, 5 tests at load5 60–75 | load |
| 8 | 48539947 | 18:40 → 18:48 | 8 min | lint (pyright) + visual | merge step missed it; new tickets |
| 9 | 0a7fc418 | 10-27 03:10 → before 07:40 | — | Quick, 1 timeout | load (FQ120) |
| 10 | ef362f55 | 09:20 → 12:51 | 3 h 31 | stress 20/20 iterations (LazyFS mount, AppArmor) | environment |
| 11 | cfb620b5 | 18:36 → 19:08 | 32 min | Pre-merge 8450/8451 | defect (FQ126) |

Source: `.claude/handoff/m1-core.md` lines 1292–1845; `reports/merge-1583.log` (rerun 11 only, since `run-merge.sh` overwrites it); `reports/spec-gate-577.log`.

Rerun 11's plan reused nothing (`merge-1583.log` lines 23–43): perf, linux, visual, TLC, MSRV and Miri had "no result in the run on ef362f55c3"; Quick and Pre-merge reran because "justfile changed since ef362f55c3" and Vitest, Playwright and pytest always rerun; stress reran as "red on ef362f55c3". Four causes, in order of effect:

1. **The head moves between reruns.** Between consecutive rerun heads, 2 to 25 ticket merges landed (`git log --first-parent`), and 8 of the 9 diffs touched a gate input:

   | Range | Ticket merges | Gate inputs touched |
   |---|---|---|
   | 200f0ce2..b965bf02 | 4 | Cargo.lock, Cargo.toml |
   | b965bf02..524f657a | 11 | Cargo.lock, Cargo.toml, ci ×6 |
   | 524f657a..12ee3b5e | 25 | .config, Cargo.*, ci, justfile, proving-ground ×3, test-support ×4, xtask ×6 |
   | 12ee3b5e..317bf5c2 | 4 | Cargo.*, proving-ground |
   | 317bf5c2..180b460f | 2 | ci ×2 |
   | 180b460f..48539947 | 15 | .cargo, .config, Cargo.*, ci ×3, justfile, pnpm ×3, proving-ground, xtask ×27 |
   | 48539947..0a7fc418 | 20 | Cargo.*, ci ×2, pnpm-lock, test-support ×4, xtask ×6 |
   | 0a7fc418..ef362f55 | 0 | none |
   | ef362f55..cfb620b5 | 8 | justfile, pnpm-lock, test-support ×2, xtask ×30 |

   So the reuse rules, correctly, reran everything. It also explains why the rounds did not converge: rerun 8's two failures came from tickets merged after rerun 7 (#994's generated Python, #930/#937's screens).
2. **Steps run beside are never recorded when a host step fails.** Perf, linux and visual start in the background and are recorded only in the final wait loop (`pre-merge.sh:249-259`). A red Quick or Pre-merge exits first (`pre-merge.sh:228`, `set -e`), and the EXIT trap kills them (`:91`), finished or not. Their results are lost, so the next run always reruns them, even when they had passed.
3. **A red step marks every member red.** `record_run` writes the one status for every member the step ran (`gate.rs:660-686`). Rerun 10's stress failed only in LazyFS tests; every other member's 20 green iterations were recorded red.
4. **Steps after the red one never run,** so they have no result (TLC, MSRV, Miri on rerun 11). That is the fail-fast order working, and costs nothing extra on the next run: they had to run once anyway.

On the one rerun with a still head (9 → 10, one file), there was nothing to reuse either: rerun 9 stopped in Quick, before Pre-merge, and its beside steps were killed by cause 2.

## 5. Question 4: where the wall-clock goes

Measured from logs; nothing was timed live.

| Piece | Time | Source |
|---|---|---|
| Rerun 11, start to red at the end of Pre-merge | 32 min | `spec-gate-577.log` 18:36:06, `m1-core.md:1845` 19:08 |
| Pre-merge nextest, 8,451 tests | 455 s wall, 6,732 s of test time (≈ 15 at once); 40 tests over 30 s, 13 over 60 s; longest 140 s | `merge-1583.log:36735` and its PASS lines |
| Quick nextest, 5,163 tests | 15 s | `merge-1583.log` |
| Host builds before and between suites (dev, release, test profiles) | ≈ 10 min summed from Cargo's `Finished` lines | `merge-1583.log` |
| Perf's bench builds, beside | ≈ 25 min summed (6 m 00, 8 m 13, 3 m 25, 2 m 55…), competing for the same cores | `merge-1583.log` `[perf]` lines |
| Stress, #1591's gate (3,943 tests × 20) | 228–256 s an iteration, ≈ 80 min | `reports/gate-1591-f90593f8.log` |
| Stress, #1589's gate (3,215 tests × 20) | 89–146 s an iteration, ≈ 35 min | `reports/merge-1589.log` |
| A green full gate (#1589) | 4,429 s (74 min) | `reports/` `GATE GREEN for #1589` |
| Red stress runs on #1583 (4 and 10) | ≈ 3 h 30 each; all 20 iterations ran though iteration 1 was already red | `m1-core.md:1406, 1805` |
| Spec #577's gate rounds, summed | ≈ 11 h of gate time, plus a smoke and a Merge Danger rerun before each | table in section 4 |

Not measured, and why: the duration of each step (the gate log has no timestamps per step; inferring them would be guesswork), whether perf's 462 "Compiling" lines were sccache hits (Cargo prints "Compiling" either way), and Vitest, Playwright and pytest wall time (their output carries no summary line the log keeps). Timing them needs a gate run, which this audit was told not to start.

Reading it: the runner is not the bottleneck. On a red round the cost is the whole gate again, usually on a new head; on a red stress round it is up to 3 hours of iterations after the verdict was known; on a green round it is the stress run of a 2,654-commit batch, every test of which is "new or changed" against `main`.

## 6. Question 5: how leading projects gate

Retrieved 2026-10-27. Each line names the source that states it.

### What a gate tests, and what happens when it fails

- **rust-lang/rust (bors).** An approved pull request enters the merge queue, where bors "will run *all* the tests on *every* platform we support" ([rustc-dev-guide, PR lifecycle](https://github.com/rust-lang/rustc-dev-guide/blob/main/src/pr-lifecycle.md)). It tests the merge with the latest base, one build at a time: "Only one auto build runs at a time to ensure that each PR is tested against the same branch state it will be merged into" ([bors design](https://github.com/rust-lang/bors/blob/main/docs/design.md)). Small changes go in rollups. When a rollup fails, the release team bisects it "to a specific PR and unapprove[s] it with `@bors r-`", then recreates the rollup without it ([Forge, rollups](https://github.com/rust-lang/rust-forge/blob/master/src/release/rollups.md)). `@bors retry` is for a failure that "was spurious (e.g. due to a network problem or a timeout)" (same page). What reruns: the whole build of a new candidate; no partial reuse.
- **GitHub merge queue.** Each entry is tested in a `merge_group` with the base and the entries ahead of it. On a failing status, "the merge queue automatically removes pull request #1 from the merge queue", and the next group merges "without pull request #1 included" ([GitHub docs, managing a merge queue](https://github.com/github/docs/blob/main/content/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue.md)).
- **Chromium CQ (LUCI CV).** It compiles and runs "all test suites that might be affected by the CL", builders chosen by location filters. A failing shard is retried "with the exact same configuration", then the failing suites are rerun without the CL; the build fails only on tests that failed twice with the patch and passed without it ([Chromium CQ docs](https://chromium.googlesource.com/chromium/src/+/main/docs/infra/cq.md)). New tests are checked for flakiness in the CQ ("`test new tests for flakiness`" steps, same page). Build reuse: passing builds of "the same patchset (or an equivalent patchset across trivial rebases or commit message edits)" are reused within a commit-distance window (for example 800 commits on `main`) or 24 hours (same page).
- **Google TAP.** Postsubmit batches commits into "milestones", "typically cut every 45 minutes during peak development time", running test targets "selected using reverse dependencies on changed source files since the previous milestone" ([Memon et al., ICSE-SEIP 2017](https://huang.isis.vanderbilt.edu/cs8395-f23/paper/google-testing-icse-seip-17.pdf)). Google reported "about 1.5% of all test runs reporting a 'flaky' result" and "almost 16% of our tests have some level of flakiness" ([Google Testing Blog, 2016](https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html)).

Common pattern: **the candidate holds still.** A failure removes the culprit (bors `r-`, the queue's removal) or tests a fixed patchset again (Chromium); new changes wait their turn. None of them lets new work join a candidate that is being fixed.

### Result reuse and selection

- **Bazel.** `--cache_test_results=auto` (the default) reruns a test "if and only if: 1. Bazel detects changes in the test or its dependencies, 2. The test is marked as `external`, 3. Multiple test runs were requested with `--runs_per_test`, or 4. The test previously failed" ([Bazel source, TestConfiguration.java](https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/analysis/test/TestConfiguration.java)). That is the skill's "same key, same result", with stress runs (rule 3) never cached. Local `--disk_cache` and a self-hosted remote cache are free; hosted remote caches (BuildBuddy, EngFlow) bill. Moving Goodfetch to Bazel is out of scope.
- **guppy's determinator** (by nextest's author) maps changed files to Cargo packages, simulates builds with no, default and all features, and marks a package changed when "any of its dependencies (including feature sets) have changed"; for a third-party crate, "its version or feature set". A file outside any package means "everything needs to be rebuilt" unless a custom rule says otherwise ([docs.rs, determinator](https://docs.rs/determinator/latest/determinator/)). That is the lockfile narrowing the skill describes, which Goodfetch's gate does not do (a Cargo.lock change reruns everything).
- **Meta's predictive test selection** learned from history which tests to run; it "roughly halved" testing cost while reporting over 95% of individual failures and over 99.9% of faulty changes ([Machalica et al., arXiv 1810.05286](https://arxiv.org/abs/1810.05286)). It accepts misses by design, so it does not fit a gate that keeps the same results.
- **nextest.** `--rerun` reruns "tests that failed or didn't complete in a previous recorded run", plus new tests (`cargo nextest run --help`, 0.9.149 on the server); recording is "Experimental" ([nextest, record and rerun](https://nexte.st/docs/features/record-replay-rerun/)). `--max-fail` stops after N failures, with `wait` (default) letting running tests finish ([nextest, running](https://nexte.st/docs/running/)). The stress page documents `--stress-count` and iterations but not how a failure ends a stress run ([nextest, stress tests](https://nexte.st/docs/features/stress-tests/)); that must be checked on a small run before relying on it.

### Retries and flakes

Goodfetch is stricter than every project above: bors retries spurious failures, Chromium retries shards, Bazel retries tests marked `flaky=1` three times by default (`--flaky_test_attempts=default`, [ExecutionOptions.java](https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/exec/ExecutionOptions.java)). ADR 0064 allows none and root-causes every failure. That choice is why each of #1583's load-sensitive timeouts became a fix item (FQ107, FQ108, FQ120, FQ123) instead of a hidden retry. It also means a failure costs a whole round, which makes the round's cost the thing to cut.

## 7. Options, ranked

Each option keeps the same steps and the same results unless it says otherwise. "Reversible" means a script or code change that can be undone; "owner" means it changes a recorded decision (an ADR, TESTING.md, an owner rule) or the run's process.

Savings are estimates from the #1583 history, not measurements.

| # | Option | Expected saving | Why the results stay the same | Risk | Needs |
|---|---|---|---|---|---|
| 1 | **Hold the candidate still while it is red.** After a red spec gate, the next run gates the same head plus its fix commits only; ticket merges wait on the milestone branch for the green, as bors and GitHub's queue keep new entries out of a candidate. | Rounds stop finding failures that new tickets brought in (rerun 8's two failures came from tickets merged after rerun 7). The fix-only diff lets reuse fire where the fix does not reach, which the moving head made impossible (8 of 9 diffs touched a gate input). Size depends on where the fix lands: a fix in `goodfetch-test-support` still reaches everything. | Same steps, same reuse rules, and the merged verdict still covers the exact head merged. Held tickets get the next gate. | Tickets queue up while a gate is red; the next batch is bigger. | Owner (run process: `merge-queue.sh`, `spec-check.sh`); reversible |
| 2 | **End a stress run at the end of its first red iteration.** | Red stress rounds took about 3 h 30 (reruns 4 and 10) where the verdict was known after the first iteration (≈ 4 to 6 min of stress plus builds): about 6 h saved on spec #577 alone. | A green still needs 20 green iterations of every selected test (TESTING.md: "one failure fails it", "never lowered"). Only a run already red stops sooner. | The run reports fewer distinct failures, so a rare second flake shows on the next round instead of this one. nextest's stress behaviour with `--max-fail` must be checked on a small run first. | Owner (gate behaviour); reversible |
| 3 | **Record each step run beside when it ends.** Perf, linux and visual record their result at once, not in the final wait loop the early exit skips. | On a rerun the change does not reach: perf's builds and measurement (≈ 25 min of bench builds summed on rerun 11) and the visual run. | Records only what fully ran and finished, as now; a step still running at the stop records nothing and reruns. | Low; a recording bug could store a wrong status, which #1591's backstop catches. | Owner (gate change, `ci/pre-merge.sh`); reversible |
| 4 | **Record red per member.** Read the run's JUnit and record red only the members with a failing test; the members that passed in full record green. | On a rerun after a red Pre-merge or stress, every member the fix does not reach reuses (rerun 10: every member but the LazyFS ones). | The member is the unit of record ADR 0135 already uses; a member reuses only when all its tests passed on that run. A missing or unreadable JUnit file marks the step's members red, as today. | An unattributable failure (a crash before JUnit, the exercise audit) must fall back to all red. | Owner; reversible; ADR 0135 text unchanged |
| 5 | **Run the whole `just lint` at each ticket merge,** instead of growing `merge-queue.sh`'s list of lint pieces one red gate at a time (oxlint after rerun 5, pyright after rerun 8, received-requests). | Rounds lost to lint (reruns 5 and 8) move to the merge that brought the error. | Adds a check; removes none. | Merge time grows by lint's warm duration (not measured). | Owner (run process); reversible |
| 6 | **Make the gate log say when each step started and ended,** and keep each run's log (`run-merge.sh` overwrites `merge-<pr>.log`, so reruns 2 to 10 left no log). | None directly; it is the measurement every other option needs (skill: "Measure before changing anything"). | Output only. | None. | Reversible |
| 7 | **Merge #1591's backstop** (daily full run on `main`, compared with the incremental runs). | None; it is what makes more reuse safe (skill invariant 2). | It adds the T3 run. | Server time once a day. | Owner (#1591 is open) |
| 8 | **Keep spec pull requests one spec wide** once #1583 merges. | The stress run of a spec selects that spec's new tests, not 2,654 commits' (≈ 35 to 80 min of stress today). | Same rule ("new or changed against the base"); a smaller diff. | None. | Already the plan (owner, 2026-10-22) |
| 9 | **Narrow the gate inputs** (a Cargo.lock change reruns only the crates whose resolved dependencies changed, determinator-style; a `ci/` or justfile change reruns the steps that use the file). | Reuse on more reruns. | Only with a proof per input; otherwise a stale pass. | An undeclared input gives a silently wrong reuse. | Owner, ADR 0135; last, after a replay shows it pays (audit.md) |

Declined:

- **Retries in any form** (`@bors retry`, shard retries, `--flaky_test_attempts`): ADR 0064.
- **nextest `--rerun` or `pytest --lf` as the gate:** a test that passed on the old head is not rerun on the new one, even when the fix reaches it. Fine for the fix loop at T0.
- **Paid remote caches or runners** (BuildBuddy, EngFlow, hosted CI minutes): ADR 0076.
- **Learned test selection** (Meta): accepts missed failures by design.
- **Moving to Bazel or Buck2:** cost far beyond the saving; Cargo, nextest and sccache already give per-crate builds and caching.

Order of work if the owner agrees: 6 first (it measures the rest), then 1 and 2 (the largest savings, process and one flag), then 3 and 4 (gate code), 5, and 7 when #1591 is ready. 9 only after a replay.

## 8. Decisions

### Option 1: adopted (owner, 2026-10-28)

The owner's words: "While a spec gate is red, the next run gates the same head plus its fix commits only, and ticket merges wait for the green."

The first draft (commit `11fe61b58`) patched `merge-queue.sh`, `run-merge.sh` and `merger-brief.md`. The orchestrator then reported the transition of 2026-10-28: spec gates, smokes and the queue scripts are retired, and gate rerun 12 for #577 will never run. Two gates remain where the rule applies:
- the single final `just merge 1583`;
- every later spec PR's `just merge`, on `spec/<n>` (ADR 0136, in #1720).

This version replaces the first draft.

**The rule.** After a red `just merge <pr>`, the pull request's head changes only by:
1. commits that fix that red, each with its root cause (ADR 0064);
2. a merge of `main`, only when `main` has moved since the red run.

Nothing else lands on the head until a gate is green: no ticket, no review fix, and no other branch.

**Why `main` is the one exception.** `ci/merge-pr.sh:34` refuses a head that does not contain its base's tip, so the gate's verdict holds for the tree that lands on `main`. When `main` moves, merging it in is required, not optional. Leading practice agrees: bors keeps unrelated work out of a candidate, but lets the base move and retests on the new one (section 6). So "don't merge `main` again unless the fix needs it" becomes "merge `main` only when the gate requires it." What keeps `main` still is merge order:
- **Final gate:** land #1721 and #1720 on `main` first, then merge `main` into `milestone/m1-core`, then start the first final gate. While #1583 is red, merge nothing else into `main` unless the red's fix needs it, such as a new live advisory. #1721 alone changes `Cargo.lock` and `pnpm-lock.yaml`, both gate inputs, so landing it between two reruns would force a full rerun.
- **Spec PRs:** under the skill, every ticket is merged before the PR is marked ready, so only fixes and `main` can move the head. The rule needs no new mechanism. It needs one line in ADR 0136 or ADR 0135, so it outlives the M1 run.

**No change to `ci/merge-pr.sh`.** The script already guarantees the property that matters: the verdict covers exactly the head that merges. Holding the head still is a matter of what gets committed, and a script can't tell a fix commit from other work without a list someone maintains. For the final gate, the M1 run can still enforce the rule with the first draft's `run-merge.sh` guard alone (`11fe61b58`). That guard refuses a rerun whose range holds a merge not listed as `fix <item>` in `m1/red-hold`. It is optional, since the orchestrator controls every merge into the milestone.

**Text for ADR 0136, proposed:**

> After a red `just merge`, the pull request's head changes only by commits that fix that red and, when `main` has moved, a merge of `main` (`ci/merge-pr.sh` requires its tip). Nothing else lands on it until a gate is green, so each rerun tests the red head plus its fixes and `xtask gate plan` can reuse what the fixes do not reach.
