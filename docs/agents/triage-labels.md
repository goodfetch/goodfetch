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

[#2242](https://github.com/goodfetch/goodfetch/issues/2242) builds the command of `xtask` that checks a list and the sweep that applies this. Until it lands, no sweep runs and nothing checks a list but its author: `to-spec` and `to-tickets` apply `ready-for-agent` themselves, and `triage`'s "Verify the claim" reaches an issue only when the maintainer runs `triage` on it.
