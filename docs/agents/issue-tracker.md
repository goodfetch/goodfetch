# Issue tracker: GitHub

Issues and specs for this repo live as GitHub issues. Use the `gh` CLI for all operations.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --json number,title,body,labels,comments`.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Make an issue a sub-issue of a parent**: `gh issue create --parent <parent> ...`, or `gh issue edit <parent> --add-sub-issue <child>` afterwards (`gh` 2.94+). Older `gh`: `gh api --method POST repos/<owner>/<repo>/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>` (database id, as in **Blocking** below). Without sub-issues, put `Part of #<parent>` at the top of the child body.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

Infer the repo from `git remote -v`; `gh` does this automatically when run inside a clone.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --comments` and `gh pr diff <number>` for the diff.
- **List external PRs for triage**: `gh api --paginate 'repos/{owner}/{repo}/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`.
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either: resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Read it as in **Read an issue** above.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (see **Make an issue a sub-issue of a parent**). Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's **native issue dependencies**, the canonical, UI-visible representation. Add an edge with `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/<owner>/<repo>/issues/<n> --jq .id`, _not_ the `#number` or `node_id`). GitHub reports `issue_dependencies_summary.blocked_by` (open blockers only, the live gate). Where dependencies aren't available, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children (`gh issue list --state open`, scoped to the map's sub-issues / task list), drop any with an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or an assignee; first in map order wins. Skip `ready-for-human` issues: a person takes those.
- **Claim**: `gh issue edit <n> --add-assignee @me`, the session's first write.
- **Resolve**: `gh issue comment <n> --body "<answer>"`, then `gh issue close <n>`, then append a context pointer (gist + link) to the map's Decisions-so-far.

## Delivery milestones

V1 is delivered in four GitHub milestones (ADR 0058): `M1 Core`, `M2 Client surfaces`, `M3 Browser automation` and `M4 Everything else`.

- **Create** (once): `gh api repos/<owner>/<repo>/milestones -f title="M1 Core"`, and the same for the other three. Then assign a milestone to every existing open ticket, applying **Order** below (wayfinder tickets that block a milestone's tickets join that milestone), and create the testing issue of M1, M2 and M3.
- **Assign**: every ticket carries one milestone. `/to-tickets` sets it at creation: `gh issue create --milestone "<name>" ...`, or later `gh issue edit <n> --milestone "<name>"`.
- **Order**: a ticket's milestone is never later than the earliest milestone of any ticket it blocks, directly or transitively. `/to-tickets` checks this when it assigns milestones. Within one spec, it moves a blocker earlier when it finds one later than what it blocks. Across specs, it moves the dependent later or splits it, unless pulling the blocker earlier changes the milestone of fewer tickets, counted transitively, a tie moving the dependent; it lists every such move for the owner in the quiz and comments on each moved ticket ([ADR 0131](../adr/0131-a-cross-spec-milestone-inversion-moves-the-dependent-later-unless-pulling-the-blocker-earlier-moves-fewer-tickets.md)).
- **Frontier**: the current milestone is the earliest one that still has an open ticket. When picking implementation work, take tickets from the current milestone, then apply the frontier query above (no open blocker, no assignee, not `ready-for-human`). Open `flaky-test` issues come first, then open `live-check` issues, before any other ticket of the milestone; once every V1 milestone is closed, they carry no milestone and still come first (ADR 0064). A milestone spans several specs: order them by ADR 0037's area order, then by each spec's ticket order.
- **Gate**: each of M1, M2 and M3 holds a `ready-for-human` testing issue, created with the milestone, with no blocking links (GitHub allows at most 50 per issue). It is ready for the owner once it is the only open issue of its milestone, and the frontier skips `ready-for-human` issues, so no agent takes it. Until the owner closes it, no ticket of the next milestone starts, even when it is unblocked.

## Implementation loop

The owner builds V1 with `/implement-spec` (owner, 2026-10-17). It is user-invoked, so the owner starts each run.

- **Scope**: one spec's tickets of the current milestone that are unblocked, unassigned and not `ready-for-human`. Tickets close only when the run's pull request merges, so within a run a ticket counts as unblocked when each of its open blockers is either built on this run's integration branch or already closed. Start it as `/implement-spec #<spec>, only tickets in milestone "<full milestone title>" that the Scope bullet of the Implementation loop in docs/agents/issue-tracker.md admits`.
- **Next spec**: the one holding the first unblocked ticket in the **Frontier** order above; within one area, the spec with the lower issue number comes first. A spec with none is skipped, not waited on. Before any run, open `flaky-test` and then `live-check` issues are worked one at a time with `/implement` (ADR 0064). A `live-check` issue attributed to an external party has nothing to commit: it closes as **Test health labels** below says.
- **Claim**: an implementer claims its ticket with the Wayfinding **Claim** command (`gh issue edit <n> --add-assignee @me`) before starting it, so another run skips it. Before it ends, a run unassigns each ticket it claimed but did not build (`gh issue edit <n> --remove-assignee @me`).
- **End**: a run ends when its spec has nothing left it can build. Most specs take several runs per milestone, because most of their tickets wait on other specs. When no spec has an unblocked ticket and tickets remain open, the owner decides.
- **Delivery**: a run's integration branch goes up as one pull request with `Closes #<n>` for each ticket it built, never for the spec. Review and handoff are the skill's own steps 7 and 8 (`code-review`, then ready for review); the merge closes those tickets. GitHub does not close a parent issue when its sub-issues close: once a merge closes a spec's last ticket, close the spec with `gh issue close <spec>`.

## Test health labels

Two labels, applied by CI's automation (ADR 0064), never by triage:

- **`flaky-test`**: one issue per flaky test, in the current milestone, with the runs that differed. It comes first on the frontier, and the test's quarantine-list entry links it.
- **`live-check`**: one issue per Live check whose last run failed, or was inconclusive three times in a row, owned by the Live check's area, in the current milestone. It comes right after `flaky-test` issues on the frontier, and a comment states its cause within two business days. It closes once that cause is fixed in Goodfetch, or attributed to an external party with a linked upstream report.
