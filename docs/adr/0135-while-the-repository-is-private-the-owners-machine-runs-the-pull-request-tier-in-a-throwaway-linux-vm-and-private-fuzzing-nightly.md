# While the repository is private, the owner's machine runs the pull-request tier in a throwaway Linux VM, and private fuzzing nightly

While the repository is private, ADR 0108's self-hosted runner on the owner's machine (Apple M3 Max) also runs the pull-request tier: every job of the tier runs there, and each job's build and tests run inside a Linux VM that Lima creates for that job and destroys after it. Private fuzzing runs there too, nightly on `main`'s head, in the same kind of VM. Nothing falls back to GitHub-hosted runners. The hosted jobs left (the cache-warming job, the ingest of test results and the checks run by hand) share the 400 included minutes a month that ADRs 0064 and 0108 leave, each under a budget of its own.

ADR 0108 holds 600 of the account's 2,000 included minutes a month for the manual-dispatch Release runs, beside ADR 0064's 1,000 for test health, and leaves at most 400 for every other hosted job of the account. Two consumers had no budget. The pull-request tier (ADR 0061: `just quick all`, then the Pre-merge suite on Linux) runs on every push of every pull request, and agents open many. Private fuzzing (ADRs 0067 and 0076) needs about 1,800 minutes a month for its nightly `batch` of 3,600 s alone, before 600 s of `code-change` after every push. The cache-warming job on every push to `main` (ADR 0061), the ingest workflow after every finished run (ADR 0064) and the checks run by hand, such as ADR 0065's Live checks and ADR 0043's online conformance, draw on the same remainder. Without a payment method, an account that runs out is blocked, not billed, so pull requests would stop getting checks.

Facts this rests on, checked on 2026-10-18:

- **Pull-request volume:** 183 pull requests were opened in the 30 days before 2026-10-18 (`gh pr list`), before any code exists. At 10 hosted minutes a pull request, that is about 1,800 minutes a month, more than four times what is left.
- **Minutes are rounded per job** ([Actions runner pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing)): each job's minutes and partial minutes are rounded up to the nearest whole minute, so a hosted job that runs for seconds after every pull-request run costs a minute each time.
- **Self-hosted runners are free today** ([GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)): "GitHub Actions usage is free for self-hosted runners". The $0.002-a-minute charge for self-hosted runners in private repositories was postponed, not cancelled, with no new date (ADR 0108's facts).
- **Pre-job and post-job scripts** ([run scripts](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/run-scripts)): the scripts see the job's default variables and the full event payload at `GITHUB_EVENT_PATH`, run as the runner's service user, block the job while they run and have no timeout setting. The docs do not say whether the post-job script runs when a job is cancelled.
- **A queued job fails after 24 hours** on a self-hosted runner (ADR 0108's facts).
- **Lima** ([multi-architecture](https://lima-vm.io/docs/config/multi-arch/)) is free and open source. On an Apple silicon Mac its VMs run Linux arm64 natively. "Running a VM with a foreign architecture is extremely slow"; x86-64 programs run faster in an arm64 VM through "Rosetta for Linux", which is "available for VZ instances on ARM hosts".
- **Actions run where the runner runs.** `actions/checkout`, `Swatinem/rust-cache` and `actions/upload-artifact` run on the macOS host, not in the VM, and rust-cache's key includes the runner's OS and architecture ([rust-cache](https://github.com/Swatinem/rust-cache)).

Decided by the owner on 2026-10-17 in [Fit the pull-request tier, private fuzzing and other hosted jobs in the included minutes left](https://github.com/goodfetch/goodfetch/issues/545): the owner's machine runs the pull-request tier, in a throwaway Linux VM, and private fuzzing runs nightly on merged `main` there. On 2026-10-18 the owner settled four more points: the VM tool is Lima; the machine never sleeps while on power; a check that failed because the runner was offline is re-run with `gh run rerun` once it is back; and nothing falls back to GitHub-hosted runners. This ADR records those, settles what they and earlier ADRs imply, and lists what is left for the owner. It builds on ADRs 0032, 0061, 0064, 0067, 0071, 0076 and 0108.

## Decision

### Where each job runs, and its included minutes

| Job | Where it runs | Budget, Linux-equivalent minutes a month |
|---|---|---|
| Every job of the pull-request tier: the Quick job, the Pre-merge jobs, "all green", the Chromium build gate's check, the lab job | the owner's machine; whatever builds or runs the repository's code runs in a Linux VM recreated for the job | none |
| Private fuzzing: nightly `batch` on `main`'s head, then `prune` and `coverage` | the owner's machine, in a Linux VM recreated for the job | none |
| The ingest of each finished run that had a hosted job | GitHub-hosted Linux | 90 |
| The ingest of runs that ran only on the owner's machine | open (question 5) | 60 at most |
| The cache-warming job | open (question 2) | 160 at most |
| Checks run by hand: ADR 0065's Live checks, ADR 0043's online conformance | GitHub-hosted | 90 |
| **Total** | | **400 at most** |

Whichever way questions 2 and 5 are answered, the total stays within 400: a line that moves to the owner's machine draws nothing. The other hosted jobs keep their own ceilings: the Release runs' hosted cells under ADR 0108's 600, and test health, the coverage job and ADR 0072's in-diff mutation job under ADR 0064's 1,000 (question 4 asks whether the mutation job moves too). The three ceilings add up to the account's 2,000.

### The pull-request tier on the owner's machine

- **Every job of the pull-request tier runs on the owner's machine,** through ADR 0108's runner: the Quick job, the Pre-merge jobs, "all green", the Chromium build gate's check and the lab job. ADR 0072's in-diff mutation job, outside the suites and "all green", stays hosted unless question 4 moves it; the test V8 build already runs on the machine (ADR 0067). A step that builds or runs the repository's code runs inside a Linux VM that Lima starts for the job and destroys when the job ends. A job that builds and runs nothing of the repository's, such as the Chromium build gate's placeholder check, needs no VM.
- **The VM is recreated for every job,** from a base image pinned by digest. It holds no secret and has only the job's `contents: read` token. No host folder it can write survives the job, except the job's own workspace, which the runner cleans at start and end. The pre-job script destroys any VM an earlier job left behind before it admits the next job, since a cancelled job may skip the post-job script.
- **The allowlist admits pull-request jobs only from branches of this repository.** Its pull-request entry is the pull-request tier's jobs, on the `pull_request` event. The pre-job script reads the event payload and fails the job before its first step unless the pull request's head repository is this repository. A pull request from a fork never runs a step on the machine (ADR 0076), and the list still admits no other job.
- **Write access is still the boundary.** A pull request runs its workflow file from its own branch, so its steps outside the VM run whatever the branch asks, as the runner's standard user. Only those with write access can push a branch: the owner, and agents acting for the owner. They can already push to `main`, which has no branch protection while private (ADR 0108). The VM keeps the build and the tests, and anything they leave, from outliving the job.
- **This squares the entry with ADRs 0032 and 0071,** which keep pull-request code off the machine that times the benchmark and the Performance series. The machine runs one job at a time, so no pull-request job runs during a timing. Nothing a pull-request job builds or leaves survives its VM, and timings run on the macOS host, outside any VM.
- **Once the repository is public,** standard hosted minutes are free, and the pull-request tier goes back to GitHub-hosted Linux runners. Its allowlist entry is removed, and the cache-warming job and the ingest go back to ADR 0061's and ADR 0064's rules.

### Private fuzzing on the owner's machine

- **A second runner instance on the machine serves the private fuzzing repository.** A runner is registered on one repository, never on the organization (ADR 0108), and ADR 0067 keeps fuzzing's logs and results in that repository, so the main repository's runner cannot run it. The instance runs as its own standard macOS user, as a launchd service, with the label `owner-machine` and the same pre-job script. Its allowlist copy admits only that repository's nightly `schedule` on its default branch.
- **The machine still runs one job at a time.** The two instances' pre-job scripts share one lock on the machine, so a job on one instance waits until the other instance's job ends. No job overlaps a timing, as ADR 0108's single instance ensured.
- **Fuzzing runs nightly on `main`'s head, in a VM recreated for the job.** It builds the fuzz targets, runs `batch` for 3,600 s, then `prune` and `coverage`. Push fuzzing (600 s of `code-change` after every push) and the continuous builds it needs do not run; the next night's `batch` covers each day's pushes.
- **ADR 0067's containment holds.** The job that builds and runs the targets holds no secret and a token with no permissions. A later job stores the results with the fuzzing repository's own token, on the same machine, after the VM is gone. Crashes, corpora, coverage and logs stay in that repository.
- **It stays on the owner's machine after the main repository is public,** since the fuzzing repository stays private. OSS-Fuzz replaces its `batch` run once Goodfetch qualifies (ADR 0067).

### Availability, and how a queued or failed check is noticed

- **A job waits in GitHub's queue while the runner is offline, and fails after 24 hours there.** The machine is set never to sleep while on power ([#1562](https://github.com/goodfetch/goodfetch/issues/1562)), so the runner is offline only when the machine is off or loses its network.
- **A check that failed because the runner was offline is re-run once the runner is back,** with `gh run rerun <run-id> --failed`. Such a job never reached a runner: it has no runner name in `gh run view <run-id> --json jobs`. A job that reached the runner and failed, whether the pre-job script refused it or a test failed, is an ordinary failure and is never re-run (ADR 0064).
- **No job falls back to GitHub-hosted runners.**
- **The merging agent reads the checks before every merge** with `gh pr checks <number>`. It never merges while "all green" is pending or failed. When a check is pending and `gh api repos/goodfetch/goodfetch/actions/runners` shows the `owner-machine` runner offline, it tells the owner that the check is waiting for the machine. When a check failed because the runner was offline, it re-runs it once that call shows the runner online.
- **The owner is notified by GitHub's own Actions notifications,** on by default, which report a failed run to the account that triggered it; agents push and dispatch under the owner's account. A queued check shows as pending on its pull request, and the merging agent reports it as above. Nothing else is set up, and nothing costs money.
- **A nightly fuzzing run that failed because the runner was offline is re-run the same way,** or left to the next night's run, which covers the same `main`.

### The hosted jobs that still draw on the included minutes

Every line is counted in ADR 0108's Linux-equivalent minutes, from the job durations the ingest records, as ADRs 0064 and 0108 count theirs.

- **The ingest of each finished run that had a hosted job** stays as ADR 0064 has it, so the ceilings' counts stay current. A workflow whose jobs all run on the owner's machine, the pull-request tier first among them, is left out of its trigger: one hosted ingest job after each pull-request run would cost a minute each time, 183 minutes or more a month on its own. How those runs reach the results store is question 5.
- **The cache-warming job, if it stays hosted** (question 2), runs one run at a time with a 30-minute timeout, and starts its build only while the month's count of its line, plus that timeout, stays within 160. Otherwise it writes the skip to its job summary, and the pull-request jobs restore an older cache or build without one.
- **The checks run by hand** are dispatched through their recipes, which read this line's count for the month from the results store before dispatching, locally and with no runner. A dispatch whose estimate would take the line past 90 is refused with the minutes needed and the date the month turns.
- **A line past its budget opens an issue.** The daily test-health job, which already reads the month's counts, opens or updates one `needs-triage` issue per line that has passed its budget, as a budget breach does (ADR 0061). The fix is to shorten its jobs or move work to the owner's machine. Nothing is bought.

### When things change

- **If GitHub announces a date for the self-hosted charge in private repositories,** this design is revisited before that date: the agent that finds the announcement opens a `needs-triage` issue that links this ADR. The pull-request tier would not fit the included minutes even at one third of a Linux minute, and the charge is never paid (ADR 0076).
- **Once the repository is public,** the pull-request tier leaves the owner's machine, as above. Fuzzing stays.

### Open questions for the owner

No ADR or owner decision settles these; each is answered before the ticket named.

1. **The VM's architecture**, before [#132](https://github.com/goodfetch/goodfetch/issues/132) builds the pull-request workflow. Lima runs Linux arm64 natively; x86-64 runs only through Rosetta for Linux or through emulation. Several Pre-merge checks are pinned to Linux x64: visual tests in the pinned Playwright image (ADR 0073), the count snapshots, `xtask perf` and benchmark admission (ADR 0071), the size estimate (ADR 0057) and `cargo fuzz check` (ADR 0067). Whether ClusterFuzzLite's images run on Linux arm64 was not confirmed. The options are an x86-64 userland under Rosetta for Linux, those checks re-pinned to Linux arm64 while private, or those checks on hosted Linux x64 runners.
2. **How the VM gets a build cache**, before #132. The options are the hosted cache-warming job on the VM's architecture, restored on the host and handed to the VM; a cache folder on the host written only by a `main` job on the owner's machine and given read-only to pull-request VMs; or no cache.
3. **What machine the pull-request tier's time budgets refer to**, before #132. ADR 0061 measures them on GitHub-hosted runs, and none exist while this holds, so they would measure nothing. The options are measuring the same budgets on the owner's machine, from each job's start; suspending them while the tier runs there, keeping the hard caps; or new budgets for the owner's machine.
4. **Whether ADR 0072's in-diff mutation job moves to the owner's machine**, before the "Test infrastructure" spec builds it. It is a pull-request job, hosted on Linux x64 under ADR 0064's ceiling, at most four shards a run and stopping at 550 minutes a month.
5. **How runs that ran only on the owner's machine reach the results store**, before [#318](https://github.com/goodfetch/goodfetch/issues/318) builds the ingest. The options are one daily hosted ingest job within this line's 60 minutes; the daily test-health job ingesting them, within ADR 0064's 1,000; or an ingest job on the owner's machine, which would hold the results store's token, against ADR 0108's rule that the runner holds no secret.

## Considered Options

- **The pull-request tier and fuzzing on GitHub-hosted runners, with per-run caps and a reduced fuzzing schedule:** rejected. With 183 pull requests a month, the pull-request tier alone passes the 400 minutes left, and nightly fuzzing alone needs about 1,800.
- **Paid minutes, or a payment method with a spending limit:** declined while there is no revenue (ADR 0076).
- **Making the repository public now:** declined by the owner for now (ADR 0108); reopening it is the owner's call alone.
- **The pull-request tier directly on the macOS host:** rejected by the owner's decision of a throwaway Linux VM. Nothing a job left behind would be cleared before the next job or a timing, and the Pre-merge suite's platform is Linux.
- **A fallback to GitHub-hosted runners while the machine is offline:** declined by the owner on 2026-10-18. A day of queued checks would spend the month's minutes, and a re-run once the runner is back costs nothing.
- **Fuzzing in the main repository's workflows, on its runner instance:** rejected. Their logs would become public with the repository, which ADR 0067's containment forbids.
- **One runner registered on the organization, serving both repositories:** rejected. ADR 0108 registers the runner on the repository only, so that no other repository of the organization reaches the machine.

## Consequences

- **Amends ADR 0108:** its allowlist gains the pull-request tier's entry, squared with ADRs 0032 and 0071 above, and its "one runner instance" becomes two instances that never run jobs at once, the second serving the private fuzzing repository. One-line notes sit at its allowlist rule and its single-instance rule.
- **Amends ADR 0061:** while the repository is private, the pull-request tier runs on the owner's machine in a Linux VM, never on GitHub-hosted runners, and the cache-warming job stays within 160 included minutes a month while hosted. One-line notes sit at its tier table and its cache-warming job.
- **Amends ADR 0064:** while the repository is private, the ingest after each run covers only workflows with a hosted job. A one-line note sits at its ingest workflow.
- **Amends ADR 0067:** fuzzing runs nightly only, on the owner's machine in a VM, through a runner instance of the private fuzzing repository; push fuzzing and continuous builds do not run. A one-line note sits at where it runs.
- **Amends ADR 0076:** the owner's machine also runs the pull-request tier while private, and private fuzzing, instead of fuzzing within the included minutes. One-line notes sit at its CI rule and at the fuzzing row of its table.
- **Amends ADR 0032's rule, which ADR 0071 restates:** the machine that stands in for the reference host runs pull-request code, in a VM recreated for each job and never during a timing. A one-line note sits at ADR 0032's release-candidate benchmark.
- **`TESTING.md`** states where the pull-request tier and fuzzing run, the VM's rules, the budget lines of the 400 and how a queued or failed check is noticed.
- **[Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)** builds the pull-request workflow on the owner's machine with its VM and allowlist entry, the pre-job script's fork check, VM clean-up and shared lock, the ingest's narrower trigger, the cache-warming job's budget, the hand-run recipes' count check and the breach issues. Its private-period lines say so.
- **[Owner step: register the Mac as the self-hosted runner](https://github.com/goodfetch/goodfetch/issues/1562)** covers Lima, never sleeping on power, and the fuzzing repository's runner instance. It blocks [#132](https://github.com/goodfetch/goodfetch/issues/132), whose pull-request workflow is the first to need the runner, and [Private fuzzing repository and the private gates App](https://github.com/goodfetch/goodfetch/issues/320).
