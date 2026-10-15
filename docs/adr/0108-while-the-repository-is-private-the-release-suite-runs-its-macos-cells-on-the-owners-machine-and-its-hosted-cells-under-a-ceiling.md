# While the repository is private, the Release suite runs its macOS cells on the owner's machine and its hosted cells under a ceiling of included minutes

While the repository is private, each manual-dispatch Release run puts its macOS cells on the owner's machine (Apple M3 Max) as a self-hosted runner: macOS arm64 natively, and macOS x64 under Rosetta 2, labelled as translated. Only Linux x64, Linux arm64 and Windows x64 run on GitHub-hosted runners, at least 270 Linux-equivalent minutes a run, under a ceiling of 600 included minutes a month. Native macOS Intel and Windows arm64 first run once the repository is public, when their minutes are free and before the first release. The Release suite reaches the runner only from a `workflow_dispatch` on `main`, so the machine runs only merged code. A pre-job hook installed on the machine refuses every other job before its first step. Making the repository public earlier is the owner's call and is not decided here.

ADR 0061 runs the Release suite on manual dispatch while the merge queue does not exist, before each milestone's owner test and before any release, and no ADR budgets its minutes. GitHub Free gives a private repository 2,000 included minutes a month. ADR 0064's ceiling holds 1,000 of them for test health, so about 1,000 remain for the pull-request tier and these runs. A run of the full matrix on hosted runners, at ADR 0061's 45-minute job budget, costs about 1,350 Linux-equivalent minutes, most of it the two macOS cells. That is more than what remains.

Facts this rests on, checked on 2026-10-15:

- **Rates** ([minute multipliers](https://docs.github.com/en/billing/reference/actions-minute-multipliers)): per minute, Linux 2-core x64 $0.006, Linux 2-core arm64 $0.005, Windows 2-core x64 and arm64 $0.010, macOS 3- or 4-core (M1 or Intel) $0.062. The page no longer states how Windows and macOS minutes draw on the included minutes, which GitHub's earlier multipliers did (Windows 2, macOS 10).
- **Included minutes** ([GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)): 2,000 a month on GitHub Free, reset at the start of each month. "GitHub Actions usage is free for self-hosted runners and for public repositories that use standard GitHub-hosted runners." "If your account does not have a valid payment method on file, usage is blocked once you use up your quota."
- **The self-hosted charge was postponed, not cancelled.** GitHub announced "a new $0.002 per minute GitHub Actions cloud platform charge" for self-hosted runners in private repositories from 2026-03-01, public repositories staying free. It then put it on hold: "We're postponing the announced billing change for self-hosted GitHub Actions to take time to re-evaluate our approach", with no new date ([changelog, 2025-12-16](https://github.blog/changelog/2025-12-16-coming-soon-simpler-pricing-and-a-better-experience-for-github-actions/)).
- **Hosted labels for private repositories** ([GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)): `ubuntu-24.04-arm`, `windows-11-arm`, `macos-15-intel` and the macOS arm64 labels are all listed.
- **No release is cut from the private repository.** ADR 0032 makes the repository public when the first version's release pull request merges, before that version is built.
- **Self-hosted runners** ([self-hosted runners](https://docs.github.com/en/actions/reference/runners/self-hosted-runners), [pre- and post-job scripts](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/run-scripts)): "If the job remains queued for more than 24 hours, the job will fail." A script named by `ACTIONS_RUNNER_HOOK_JOB_STARTED` runs before each job, and if it exits non-zero, "the job will not run and will be marked as failed".
- **Self-hosted runners and public repositories.** GitHub says self-hosted runners "should almost never be used for public repositories", and runner groups limited to selected repositories need the paid Team plan (ADR 0084).

Decided in [Fit the private-period Release suite runs in the free Actions minutes](https://github.com/goodfetch/goodfetch/issues/535), with the agent answering on the owner's delegation.

## Decision

### The matrix while the repository is private

| Cell | Where it runs | Linux-equivalent minutes, one 45-minute job per cell |
|---|---|---|
| Linux x64, glibc and musl | GitHub-hosted, `ubuntu-24.04` | 90 |
| Linux arm64, glibc and musl | GitHub-hosted, `ubuntu-24.04-arm` | 90 |
| Windows x64 | GitHub-hosted | 90 |
| macOS arm64 | the owner's machine, natively | none |
| macOS x64 | the owner's machine, under Rosetta 2 | none |
| macOS Intel, natively | not run while private | none |
| Windows arm64 | not run while private | none |

- **The owner's machine runs the macOS cells.** macOS arm64 runs natively. The x86-64 macOS build runs its macOS x64 cell under Rosetta 2, which catches the target's build, link and most of its failures. It is labelled `macos-x64 (Rosetta 2)` everywhere a cell is named. Its failures count like any other cell's, and its pass never stands in for native Intel.
- **Native macOS Intel and Windows arm64 wait for the repository to be public.** Their first run is the first Release run after the repository becomes public, on free hosted runners. That run comes before the first release, since ADR 0032 makes the repository public before the first version is built. They would cost about 495 and 90 minutes a run, and no release is cut while private.
- **The owner's-machine cells run only on `main`.** A dispatch on any other branch runs the hosted cells alone and lists the macOS cells as not run. The machine also times the Performance series and the release-candidate benchmark (ADR 0076), and ADRs 0032 and 0071 keep pull-request code off the machine that does.
- **A passing Release run while private is every cell above that runs, green on one commit of `main`.** It is what the milestone's owner test waits for (ADR 0058). Its cells may come from more than one dispatch of that commit: `just release-ci` takes the cells to run, all by default. A cell that was held back by the ceiling, timed out in the queue or failed with no failed test step may run again. A cell with a failed test step in any attempt leaves the commit red (ADR 0064).

### Counting and the ceiling

- **Minutes count in Linux-equivalent minutes.** Each hosted minute counts at its rate over Linux 2-core x64's, rounded up to a whole number: Linux x64 1, Linux arm64 1, Windows 2, macOS 11. GitHub no longer documents how Windows and macOS draw on the included minutes, so the count stays at or above both its earlier multipliers and today's rates. These are the "GitHub's minute multipliers" ADR 0064's ceiling counts with.
- **A ceiling of 600 included minutes a month covers every hosted job of manual-dispatch Release runs,** the ceiling check included. At 270 a run, that is two runs, which leaves at least 400 for the pull-request tier beside ADR 0064's 1,000. Jobs are counted from the durations the ingest records, as ADR 0064 counts its own.
- **270 is a floor.** ADR 0061 shards a cell over several jobs, and each shard has its own setup. So the ceiling check estimates a run from the recorded durations of the same cells' last run, and uses 270 only before any.
- **The run checks the ceiling before it starts a hosted cell.** The Release workflow's first job, a one-minute Linux job, reads the month's count and the estimate. If the run would pass 600, it starts no hosted cell. It writes the minutes it needs and the date the month turns to its job summary, and the run fails. The owner's-machine cells still run, for an early signal. A run held back by the ceiling is not a test failure, so no diagnostic run follows.
- **A run whose own estimate passes 600 still starts as the month's first Release run,** so a milestone's owner test is never blocked for good. It opens or updates a `needs-triage` issue, as a budget breach does (ADR 0061). The fix is to merge shards or speed up the tests.
- **Running out blocks, it never bills.** The account keeps no payment method for Actions, so GitHub blocks usage past the included minutes, as quoted above. That is an owner setting, recorded in [Owner confirmations and decisions](https://github.com/goodfetch/goodfetch/issues/392).

### The owner's machine as a runner

- **It is registered on the repository, never on the organization,** with the labels `self-hosted`, `macOS`, `ARM64` and `owner-machine`.
- **A committed allowlist names every job that may reach a self-hosted runner,** with its workflow, its events and its refs. The Release suite's entry is its macOS jobs, on `workflow_dispatch` with `refs/heads/main` only, and each job carries the matching `if:`. The other uses of the machine join the list with the jobs their own decisions create, on the triggers those decisions set: the Performance series (ADR 0071), the release-candidate benchmark (ADR 0032), the test V8 build (ADR 0067) and the Chromium build gate ([#445](https://github.com/goodfetch/goodfetch/issues/445)).
- **Two checks hold the list.**
  - A Quick-suite check of the workflow files refuses any job whose `runs-on` names `self-hosted` or any of the runner's labels and is not on the list with the same events, refs and `if:`.
  - A pre-job hook (`ACTIONS_RUNNER_HOOK_JOB_STARTED`) on the machine checks each job's repository, workflow, event and ref against its own copy of the list, which the owner installs, and fails any other job before its first step. A branch can change the committed list, but not the machine's copy. So a job pushed on a branch that names the runner never runs a step there, even before the Quick suite reports.
  - ADR 0076's rule that a self-hosted runner never runs a pull request from a fork holds in both.
- **It runs as its own standard macOS user,** not an administrator, as a launchd service. It cannot read the owner's home, keychain or `.env`. macOS puts every local user in the `staff` group, so the owner's setup checklist closes the home folder to that user (for example `chmod 700`) and checks it by trying to read it as that user. Its jobs hold no secret and a token with `contents: read` only, and clean their workspace at start and end. The Release suite needs no secret, since Live checks are in no suite (ADR 0061).
- **It runs only when the owner's machine is on.** Its cells wait in GitHub's queue, and fail after 24 hours there. A cell that timed out in the queue ran no test, so it may be dispatched again on the same commit. `just release-ci` says which cells are waiting for the owner's machine.
- **Its timings are labelled with the machine.** The results store records each cell's job with `owner-machine`, and flaky-test detection (ADR 0064) treats `macos-x64 (Rosetta 2)` as its own configuration. ADR 0061's time budgets are measured on GitHub-hosted runners, so they ignore these jobs, which keep the 90-minute job cap.
- **Diagnostic runs of its cells run there too.** ADR 0064's diagnostic run reruns a failed test on the same runner OS. For an owner's-machine cell that is the owner's machine, so these runs use no included minutes.

### When things change

- **If GitHub brings back the self-hosted charge for private repositories:**
  - if it draws on the included minutes, the owner's-machine cells count at one third of a Linux minute ($0.002 over $0.006);
  - if it would bill money, the owner's-machine cells stop (ADR 0076), and the macOS cells wait for the repository to be public, like native Intel. Nothing is bought to keep them.
- **Once the repository is public,** every cell runs on free hosted runners, native macOS Intel and Windows arm64 included. The Release workflow stops naming `owner-machine`, and this ceiling lapses with ADR 0064's. Whether the runner still serves builds on a public repository is decided in [Decide the free Chromium build gate](https://github.com/goodfetch/goodfetch/issues/445).

## Considered Options

- **The full matrix on hosted runners:** rejected. About 1,350 minutes a run is more than the roughly 1,000 left beside test health, before a single pull request.
- **The owner's machine for macOS arm64 alone, with every other cell hosted:** rejected. Native Intel at about 495 and Windows arm64 at 90 bring a run to about 855. That fits one run a month and leaves the pull-request tier about 145.
- **A smaller matrix alone, with macOS arm64 hosted:** rejected. About 765 a run for the same reason.
- **Native macOS Intel hosted before each owner test:** rejected. It would cost about 495 a run for what Rosetta 2 mostly catches for free, and native Intel runs free before the first release.
- **Linux arm64 in a Linux virtual machine on the owner's machine:** rejected for now. It would save 90 minutes a run at the cost of a second runner inside a VM to keep. Hosted arm64 is the cheapest rate GitHub sells.
- **Windows on the owner's machine:** rejected. It would need a paid hypervisor or a Windows licence.
- **Buying minutes, or a payment method with a spending budget:** declined while there is no revenue (ADR 0076).
- **Making the repository public earlier: not decided here; it is the owner's call,** which ADR 0076 also left open.
  - For it: standard hosted minutes become free, so every Release run is the full native matrix, and ADR 0064's ceiling and this one lapse. The nightly tier can turn on (ADR 0032), and the owner's machine leaves the Release suite.
  - Against it: the code is public before the first release, which ADR 0032 chose against. The merge queue still waits for the organization (ADR 0032). GitHub advises against self-hosted runners on public repositories, which matters for [Decide the free Chromium build gate](https://github.com/goodfetch/goodfetch/issues/445).
  - Either way, this ADR holds until the repository is public.

## Consequences

- **Amends ADR 0061:** while the repository is private, a manual-dispatch Release run runs the matrix above, not the full matrix of ADR 0024, and its hosted cells stay under the ceiling. `just release-ci` reaches the owner's machine, on `main` only, so "runs on GitHub-hosted runners only" no longer holds while private. One-line notes sit at its manual-dispatch rule and its reference-host rule.
- **Amends ADR 0064:** the minute multipliers its ceiling counts with are those above. The 1,000 included minutes not held for test health are split: 600 at most for manual-dispatch Release runs, and the rest for the pull-request tier. A diagnostic run of an owner's-machine cell runs on the owner's machine. A one-line note sits at its ceiling.
- **Amends ADR 0076:** a self-hosted runner costs nothing today, but GitHub has postponed, not cancelled, a $0.002-a-minute charge for private repositories. The owner's machine runs the private-period Release suite's macOS cells. One-line notes sit at its GitHub Actions fact and at its CI rule.
- **Adds to ADR 0032:** while the repository is private, the full matrix on manual dispatch is the matrix of this ADR. A one-line note sits at its private-period rule.
- **`TESTING.md`** states the private-period matrix, the ceiling and the runner's rules in its suites and free-tier sections, and its `just release-ci` row says so.
- **[Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)** builds the Release workflow's ceiling job, its self-hosted jobs and `just release-ci`'s cell selection, the self-hosted allowlist with its Quick-suite check and the pre-job hook, and the runner's setup as an owner checklist. Its private-period lines say so.
- **Not decided here:** how the pull-request tier itself fits the 400 minutes or more left to it.
