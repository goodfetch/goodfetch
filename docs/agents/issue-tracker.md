# Issue tracker: GitHub

Issues and specs for this repo live as GitHub issues. Use the `gh` CLI for all operations.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --json number,title,body,labels,comments`.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Make an issue a sub-issue of a parent**: `gh issue create --parent <parent> ...`, or `gh issue edit <parent> --add-sub-issue <child>` afterwards (`gh` 2.94+). Without sub-issues, put `Part of #<parent>` at the top of the child body.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue comment <number> --body "..."`, then `gh issue close <number>`.
- **Blocking**: `gh issue edit <child> --add-blocked-by <blocker>`.
- **Count open blockers**: `gh api repos/{owner}/{repo}/issues/<n> --jq .issue_dependencies_summary.blocked_by`.

Infer the repo from `git remote -v`; `gh` does this automatically when run inside a clone.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

GitHub shares one number space across issues and PRs, so a bare `#42` may be either: resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Read it as in **Read an issue** above.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (see **Make an issue a sub-issue of a parent**). Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's **native issue dependencies**, the canonical, UI-visible representation. Add an edge as in **Blocking** above; **Count open blockers** reads open blockers only, the live gate. Where dependencies aren't available, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children (`gh issue list --state open`, scoped to the map's sub-issues / task list), drop any with an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --add-assignee @me`, the session's first write.
- **Resolve**: `gh issue comment <n> --body "<answer>"`, then `gh issue close <n>`, then append a context pointer (gist + link) to the map's Decisions-so-far.

## Delivery milestones

V1 is delivered in four GitHub milestones (ADR 0058): `M1 Core`, `M2 Client surfaces`, `M3 Browser automation` and `M4 Everything else`. A milestone is ticket metadata and an owner checkpoint, never the scope of a run or a start gate (ADR 0136).

- **Assign**: every ticket carries one milestone: `gh issue create --milestone "<name>" ...`, or later `gh issue edit <n> --milestone "<name>"`.
- **Order**: a ticket's milestone is never later than the earliest milestone of any ticket it blocks, directly or transitively.
- **Frontier**: milestones neither pick nor order work. A run's frontier is its spec's open tickets with no open blocker and no assignee that carry `ready-for-agent`, with the `flaky-test` and `live-check` issues attached to the spec that carry no triage label or `ready-for-agent` (ADR 0144, amending ADR 0136). A `ready-for-human` or `needs-triage` ticket is never on it, and neither is any ticket of a spec that itself lacks `ready-for-agent`. A ticket blocked by another spec's open ticket waits until that ticket closes.
- **Gate**: each of M1, M2 and M3 holds a `ready-for-human` testing issue, created with the milestone, with no blocking links (GitHub allows at most 50 per issue). It is ready for the owner once it is the only open issue of its milestone, and the frontier skips `ready-for-human` issues, so no agent takes it. It is a checkpoint: no ticket waits for it to close.

## Implementation loop

Each `/implement-spec` run is one whole spec, every open ticket of that spec whatever its milestone (ADR 0136). The run follows the skill as written but for who builds its tickets (below), with the checks and the implementer limit of `TESTING.md`'s [Checks by role](../testing/checks-by-role.md).

- **To start** a run: `/implement-spec #<spec>`. Its integration branch is `spec/<n>`, where `<n>` is the spec's issue number, created by the skill's step 3.
- **Once the first ticket is pushed** to that branch, its draft pull request opens, marked as closing the spec and its tickets (`Closes #<n>` and one `Closes #<ticket>` line per ticket). It merges into `main` through `just merge` (`AGENTS.md`, Merging).
- **When a performance acceptance ticket misses its target,** the pull request has no `Closes` line for it: the ticket takes `ready-for-human`, and the owner closes it on reading the write-up (owner, 2026-11-10).
- **Before a run or relay takes a ticket,** `just load-proofs <n>` passes on it (`triage-labels.md`, what a fix's acceptance asks).
- **To pick the next spec:** while [the rollout order](rollout-order.md) lists specs, it picks the next one; otherwise the owner does, and ADR 0037's spec order mostly runs a providing spec first.
- **To fill the gate host's six slot worktrees,** several runs, and relays of `/implement`, go side by side (owner, 2026-11-11, « Oui, six », ADR 0136's note of that day) when the tickets they take share no file and none waits for another's in the tracker; Checks by role decides when each implementer is added.

One implementer builds the whole spec (ADR 0133's second note of 2026-11-11), in place of the skill's step 4, a fresh implementer for each ticket, and its step 5, a merger for each ticket. It works in one slot worktree on `spec/<n>`, takes the tickets one after another in dependency order and lets its context compact when it fills. It ends each ticket with a commit, Checks by role's implementer checks on that commit, a push and a note of about five lines in the commit or on the ticket, and starts the next by reading that ticket's text again. A second slot, with its own implementer, joins when a slot is free and the spec has 30 open tickets or more, two of them ready and sharing no file (ADR 0136's note of 2026-11-12). That implementer works on its own branch off `spec/<n>`, takes only tickets that share no file with the first's current ticket, and after each ticket brings `origin/spec/<n>` in, runs its checks on that tree and pushes it to `spec/<n>`. The skill's step 7, `code-review` then one fresh implementer for its fixes, is unchanged.

A recorded lot (owner, 2026-11-11, « Oui, envoie », ADR 0133's first note of that day) is two open tickets of one spec, recorded on the owner's approval by a `Lot` section in both tickets' bodies and a `Lots` list in the spec's body (above the `Existing code` list where the issue has one, which ADR 0144 keeps last). One implementer builds them back to back in the order given, with one final `just quick`. In a relay of `/implement` that is one worktree and one pull request that closes both, in place of a fresh implementer for each. In a run, where one implementer builds every ticket anyway, the one `just quick` for the two is all a lot changes.

## Automation labels

Three labels, applied by CI's automation (ADRs 0064 and 0035), never by triage. While the flaky-test scan (`TESTING.md`, After merge) and every workflow but the Release suite (`AGENTS.md`, Merging) are off, none opens on its own. Until #1719, `xtask issues` files a new one in the earliest open milestone with no parent: attach it to the spec that owns its test before that spec's run.

- **`flaky-test`**: one issue per flaky test, with the runs that differed; the test's quarantine-list entry links it. Its handling is `TESTING.md`'s [A flaky test found after merge](../testing/flaky-tests-and-live-checks.md#a-flaky-test-found-after-merge): the test is quarantined the same day, and the fix waits its turn behind Features' work ([the rollout order](rollout-order.md)).
- **`watchlist`**: one issue per bot-protection, challenge or CAPTCHA vendor webappanalyzer or Scrapfly's Antibot-Detector lists that no block-detection rule covers, opened by the weekly watchlist job (`.github/workflows/watchlist.yml`, story 61, which is disabled with every workflow but the Release suite) with `needs-triage` and no milestone. Keep the `watchlist` label on it: the job finds its issues, open or closed, by it. Close it once a rule covers the vendor, or as `wontfix` when the vendor refuses no automated client; the job never opens it again.
- **`live-check`**: one issue per Live check whose last run failed, or was inconclusive three times in a row. Its handling is `TESTING.md`'s [Flaky tests and Live checks](../testing/flaky-tests-and-live-checks.md).
