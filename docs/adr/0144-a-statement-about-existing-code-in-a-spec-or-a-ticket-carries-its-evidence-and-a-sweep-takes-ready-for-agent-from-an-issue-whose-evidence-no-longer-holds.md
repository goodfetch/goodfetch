# A statement about existing code in a spec or a ticket carries its evidence, and a sweep takes `ready-for-agent` from an issue whose evidence no longer holds

A spec or a ticket that says what today's code does lists each such statement with the file and symbol that show it and the commit they were read at. A command of `xtask` checks those citations against `main`, and an issue whose citation no longer holds loses `ready-for-agent` until someone reads the code again. Proposed by an agent and accepted by the owner on 2026-11-10.

On 2026-11-09 a read of the refactor's issues against `main` found statements about existing code that the code contradicts, and the repair of 2026-11-10 confirmed 40 of them: 14 in spec #2138 and its tickets, 18 in spec #2139 and its tickets, and 8 in six tickets beside them. The statements set what the tickets build:

- **A ticket's size.** #2167 was filed as a prefactor on the statement that URL keying and Scope "stay pure work on the page's task". They run inside the writer's jobs and read the run file, so #2167 was its spec's largest ticket, since split in five.
- **A ticket's test.** #2133 required a test that Payload counts reach the Run's counts, on the statement that they are lost. They are not lost, so the test passes before any change.
- **A design.** #2139 ended a handler's `page.on` subscriptions at "no command". A `page.on` that listens to `console` or `page_error` has sent `Runtime.enable` since 2026-10-23, and the page driver's module comment still said "`Runtime` is never enabled" two weeks later, when the spec was written.

Two causes produced them. `to-spec`'s step 1 explores the code, and nothing records what was read or at which commit, so a checked statement and an assumed one look the same. And a true statement goes stale without notice: 12 of the 19 places first found in #2139 were stale ones, three days after the spec was written.

## Decision

**The rule, for any issue.** A sentence that says what existing code does, holds or lacks is a statement of fact, and it carries its evidence where a tool can read it. A statement without evidence is an assumption, and an issue built on an assumption is not `ready-for-agent`.

**The `Existing code` list.** An issue that makes such statements ends with a list under a heading named exactly `Existing code`:

- its first line is "Read at `<commit>`.", the commit of `main` the code was read at;
- each item is one statement in the present tense, followed by at least one citation in backticks, a path from the repository's root that may carry lines and a symbol: `crates/goodfetch-page/src/page_events.rs:493#arm_runtime`;
- the symbol is the function, type or constant that names the thing. It is the anchor that survives a moved line, so a statement cites one wherever one exists;
- a spec that touches no existing code says so in one item, "None: nothing this spec changes exists yet", which is a statement like the others.

In a spec the list sits under `to-spec`'s Further Notes, so its Implementation Decisions stay free of paths, as the skill asks. A ticket carries a list only for statements its spec's list does not hold.

**The check.** A command of `xtask` reads an issue and fails when:

- a spec has no list;
- an item cites nothing, or cites a path or a symbol that does not exist on `main`;
- the lines an item cites changed between its commit and `main`. A line that only moved passes, and the command prints where it went.

It checks that the evidence still exists and is unchanged. Whether the evidence shows the statement is read by a person or an agent, once, when the item is written, and again only when the check fails.

**The sweep.** The same command runs over every open `ready-for-agent` issue with the merge gate's backstop, and on demand for one issue. For an issue that fails, it removes `ready-for-agent`, applies `needs-triage` and comments with the item, the citation and the merge that changed it, as `xtask issues` keeps the issues of the other checks. `triage`'s own step 3, "Verify the claim", is where the item is read again and the label returns.

- **A run's frontier takes `ready-for-agent` tickets only,** so a ticket the sweep set aside is not started, and neither is a ticket of a spec it set aside.
- **The sweep leaves a spec alone once its run has started,** from the first commit on `spec/<n>`: its tickets change the cited code on purpose, and its implementers read the code itself.
- **A spec must carry a list** when it was published on or after 2026-11-10, or when `rollout-order.md` lists it. Any other open spec gains its list before its run.

**How it applies to the specs at hand.** The repair of 2026-11-10 wrote the lists of 30 issues, each read at 95935d9b2 and checked by a prototype of the command: specs #2138, #2139 and #2212, and 27 tickets of theirs and of #1838. Specs #2164, #2165 and #2194 were not read in that pass and gain theirs before their runs.

## Considered Options

- **A rule that asks the author to check:** rejected. `to-spec` already says "explore the repo", and the three specs were written after such an exploration.
- **No paths in an issue,** as `to-spec` asks of Implementation Decisions, "they may end up being outdated very quickly": rejected for statements about existing code. The statements were in the specs anyway, without their evidence, and went stale just as fast with nothing to show it.
- **The lists as files in the repository, checked by `just lint`:** rejected. A spec would need a pull request before it is ready, a step `to-spec` does not have (ADR 0133), and a code change would fail its gate for a spec it never touched.
- **A check only when the label is applied,** by a hook of the agent's host: rejected as the only hold. It binds one host, and it misses every statement that goes stale after the label, which was the larger share.
- **Failing the merge that changes cited code:** rejected. The merge is right and the spec is stale. The gate's result would also depend on the tracker's state, so the same head could pass and then fail.
- **Removing the label whenever a cited file changes at all:** rejected. `delivery.rs` changes in most merges of the crawl; the cited lines and symbol say whether the statement's own code moved.

## Consequences

- **ADR 0133 stands.** The list is a standard that `to-spec` and `to-tickets` consult at their own steps, and the sweep is a check that keeps issues, as ADR 0064's are. No step, review or approval joins a skill's sequence.
- **`docs/agents/triage-labels.md`** says that `ready-for-agent` on an issue with statements about existing code means its list passes the check.
- **`docs/agents/issue-tracker.md`**'s Frontier takes `ready-for-agent` tickets only. It left out only `ready-for-human` before, so the four tickets of #1838 that carried `enhancement` alone were on a frontier.
- **One ticket builds the command and the sweep** in `xtask`, with tests over a scripted tracker: [#2242](https://github.com/goodfetch/goodfetch/issues/2242). Until it lands, a list is checked by whoever writes it and by `triage`'s "Verify the claim", and no sweep runs.
- **What the tool does not hold:** a statement written into an issue's prose and left out of its list. `code-review`'s Spec axis reads the spec, and a statement the diff contradicts is a finding there, after the fact.
- **The Quality register's Exceptions cite lines too.** Six of their citations had drifted by 2026-11-10, in X10, X27, X32 and X34, and X6, X12 and X22 also cite lines of 2026-11-03. The command could read `TESTING.md`'s Exceptions as it reads an issue's list. That is left to the ticket above to weigh, and is no part of this decision.
- **Glossary.** No term changes.
