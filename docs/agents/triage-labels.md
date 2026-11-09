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

An issue that says what today's code does lists each such statement at its end, under a heading named exactly `Existing code` (ADR 0144):

- **Its first line** is "Read at `<commit>`.", the commit of `main` the code was read at.
- **Each item** is one statement, followed by at least one citation in backticks: a path from the repository's root, with optional lines and a symbol after `#`, as in `crates/goodfetch-page/src/page_events.rs:493#arm_runtime`. Cite a symbol wherever one names the thing, since it survives a moved line.
- **A spec always carries the list,** under its Further Notes. A ticket carries one only for statements its spec's list does not hold.

An issue whose list no longer holds on `main` is `needs-triage`, not `ready-for-agent`. [#2242](https://github.com/goodfetch/goodfetch/issues/2242) builds the command of `xtask` that checks a list and the sweep that applies this; until it lands, `triage`'s "Verify the claim" is the only check.
