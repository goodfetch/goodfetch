# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

## `ready-for-agent` and statements about existing code

An issue that says what today's code does lists each such statement at its end, under a heading named exactly `Existing code`, at any level (ADR 0144):

- **Its first line** is "Read at `<commit>`.", the commit of `main` the code was read at, as at least 7 hexadecimal digits.
- **Each item** is one statement, followed by at least one citation in backticks: a path from the repository's root, with an optional line (`:493`) or range of lines (`:493-517`) and an optional symbol after `#`, as in `crates/goodfetch-page/src/page_events.rs:493#arm_runtime`. Cite a symbol wherever one names the thing, since it survives a moved line.
- **Which issues carry it:** every spec published on or after 2026-11-10, under its Further Notes, and a ticket only for statements its spec's list does not hold. A spec that touches no existing code says so in one item: "None: nothing this spec changes exists yet". An older spec gains its list before its run starts.
- **Read down to what decides.** A citation shows where the statement was read, not that it is true: follow the cited code into what it calls and the defaults it reads, and look for a counter-example before writing "only", "never", "always", "already" or "none".

An issue whose list no longer holds on `main` is `needs-triage`, not `ready-for-agent`, and no ticket of a spec that lacks `ready-for-agent` is started (the Frontier, in `issue-tracker.md`).

**Check one issue** with `just existing-code <issue>`, before applying `ready-for-agent` and in `triage`'s "Verify the claim". It checks the list against `origin/main`: a citation whose path, symbol or lines are gone or changed fails, naming the merge that moved them, and a line that only moved passes with where it went. It also fails a spec that must carry a list and has none: one published on or after 2026-11-10, or, until its run starts, one `rollout-order.md` lists, as a step or a `[Spec: …]` link. A failure says the evidence moved, not that the statement is false: read the code again, then fix the list or the statement.

**The sweep**, `just existing-code-sweep`, runs the same check over every open `ready-for-agent` issue but those of a spec whose run has started (a commit on `spec/<n>`). Alone it is a dry run; with `--apply` it takes `ready-for-agent` from each issue that fails, applies `needs-triage` and comments once with each item that fails. The merge gate's backstop runs it with `--apply` each day after its result (`ci/gate-backstop.sh`), so no sweep runs while `ci/gate-backstop.state` is `off`.
