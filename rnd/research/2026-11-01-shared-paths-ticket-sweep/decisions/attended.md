# Decisions: attended (from #1696)

Read-only: nothing was built or run. Code read at `989050b0a`.

## Reversible choices

1. "Automation call" and `acting: automation`, not "agent call" and `agent`. The glossary says a single AI extraction is not an Agent, and a plain click is neither. Alternative: the brief's word, which would be wrong for most calls.
2. `release` is `hand_back`. It is the lock's existing event. Alternative: a new verb for one event.
3. Automation beside automation is allowed; only a person's hold refuses it. This narrows the brief's "only from `waiting`", which would break `Promise.all` and ADR 0085's side-by-side pages. Alternative: one call at a time per Session, which #1707 can still add as a rule.
4. The turn is a new ADR 0128 object in `sessions` that holds no person and reads the lock. Alternative: extending the lock with automation states, but the lock lives in `live_view` and `free` is what ADR 0031 means; one holder is kept either way.
5. The gate sits in `PageLife`'s `call`, for `Gesture` and `Goto`, and again after the gesture order is taken. Alternative: only at admission, which misses retries.
6. `when_held` is a setting only. A per-call argument would change every Gesture's parameters, and it stays additive later. Least certain: two Clients on one Session share one answer.
7. A waiting call runs only after an explicit hand back. Idle and lost-Connection releases fail it. ADR 0031's "never to automation" would otherwise break through the queue. Alternative: keep it waiting after an idle release, which leaves it hanging until its timeout with no signal.
8. A call made after that fall-back runs: its caller asked afresh, while nobody holds control.
9. A wait is bounded by the call's own timeout (ADR 0085 counts time behind another Gesture). Alternative: a new timeout setting, which adds nothing.
10. `session.control` as both a coalescing read and a `telemetry` event, as `run.progress` is. A program clicking in series would otherwise send two lossless events per Gesture. Alternative: a lossless event; the Audit log and Run trace keep every change by a person anyway.
11. No Audit entry for automation starting or ending: it is no person's act (ADR 0113's reasoning).
12. Control operations have SDK reach, plus a dispatch refusal on `mcp` and `worker`. Alternative: `Reach::PersonOnly`, which would require a passkey on every turn through #1643.
13. Step-up on the first take per Connection and Session, with `every` as a setting. This overrides #1646's brief ("a fresh step-up" to take control), because taking turns is routine and each call is signed by then. Least certain.
14. Holder is a Connection together with its principal, not the principal alone: otherwise two windows of one OS user would both be `take(h)` and both type.
15. `hand_over` completes at once, with no acceptance step. Idle release covers an absent receiver. Alternative: an accept step, which adds a state to the lock.
16. ADR 0031's request timeout stays, though the issue says "explicitly". It was decided, and asking is still a person's explicit act.
17. A take during a Human pause does not stop the paused call. The brief's "cancels at the next safe point" read literally would break ADR 0092.
18. `pause_resolved` under a take made before the pause keeps the lock. Lock table cell changed for attended Sessions only. Alternative: as #592's table has it, which pulls control from a person because of a program's condition.
19. Model reads wait, by #659's rule widened, rather than failing typed. Same mechanism, same canary. They stay bounded by their timeouts.
20. `page.lend()` refused on an attended Session. Alternative: treat a lend as a long automation call, which cannot be stopped at a Safe point.
21. `evaluate` keeps ADR 0085's class and runs during control. Splitting `PageEvent::Read` would touch every read. Documented as a limit. Least certain: a program can still change the page that way.
22. A Run with an attended Session and no handler outlives its Connection until `unwatched_stop` (default 1 hour). ADR 0078 would otherwise stop it on every network blip, and `detached` exists only for Recipe and catalog runs. Alternative: extend `detached` to `collie.start`, which ADR 0102 rejects by type. Least certain: it amends ADR 0078, and the 1 hour is a judgement.
23. Resume is asked by a Client, not automatic: nothing should reopen a browser for nobody.
24. "From its Checkpoint" narrowed to committed state: on the same machine ADR 0102 needs none.
25. Sealed cookies, storage and tab URLs captured at control changes, AI ends and Checkpoints, default on. Reading the code found that a reopened Session today has a fresh context, so the brief's acceptance criterion alone would give an empty window. Alternative: `keep_state` off by default, which makes "resume" hollow. Least certain: a login at rest by default, though sealed as ADR 0015 seals Session secrets.
26. Captures are never per Gesture, so a crash can lose recent changes. Alternative: per navigation, which adds work on C4 and F2.
27. #1696 becomes the tracer ticket: it already carries #959's native link, and #1697 and #1707 point at it.
28. #978 blocks the pause ticket, not the tracer: the tracer needs only the lock, and #978's own chain is long.
29. Milestones: M3 for what rests on #974, #959 and #659; M4 for AI verbs (#957, #834, #829 are M4) and for the relay ticket, whose blockers have no milestone yet.
30. No prefactoring ticket: nothing to move, as the lock isn't built and the gate tickets carry the benchmarks.
31. Control has catalog operations beside the channel's messages, both into one lock. A Client may take control with no channel open, and the issue asks for reach through a relay. Alternative: channel only, as the drafted #974 has it.
32. `sdks` is blocked by every M3 ticket and not by `ai` or `relay`: the Journey needs neither.

## OWNER

None new. No money, legal or one-way public item: nothing is published (ADR 0066), and tests use the Scripted model and local stand-ins (ADR 0076). #1689 stays the owner's; the spec holds on Yes, on Yes without live merging, and on No, since it names no working copy. If it is No, #1697 and #1707 change, not this spec.

## CLOSE

None. #1696 is absorbed as the tracer, not closed.

## For the reviewing session

- **Person-facing spec:** I read its drafts of #974 and #978. Three things need you:
  1. **Change one sentence in its `t-974.md`:** "A holder is a principal of the Connection" should read "a Connection together with its principal". I did not edit it, as it is outside my directory.
  2. **Add a blocker on publish:** `relay` also needs that spec's `accounts-store` ticket for its table per role. It has no number yet, so `tickets.json` can't name it.
  3. **#978 is written around a waiting pause:** its Consent runner and governor rules must hold for any held control, which this spec's `pause` ticket tests.
- **Relay spec:** `relay` blocks on #1641, #1642 and #1645 by number. If those are replaced by new tickets, repoint it to the ticket that gives a Connection through the relay and a checked person.
- **If #1697 or #1707 go ahead:** they should be blocked by `sdks`' dependencies, not only #1696 (the tracer): at least `stop` and `closed`.
- **#1696's title** still says "agent". I kept it, since it is an existing issue; a rename to "An Attended Session: ..." is optional.

## ADRs and glossary edits needed (not written)

- **New ADR:** an Attended Session lets a person take control at any time, and an Automation call runs only while nobody holds it.
  - Amends ADR 0031: take control beyond a Human pause; the lock's `pause_resolved` cell.
  - Amends ADR 0078: a Run with an attended Session and no handler outlives its Connection until `unwatched_stop`.
  - Amends ADR 0085: Gestures on a held attended Session fail or wait.
  - Amends ADR 0092: the cell above; #659's rule widened.
  - Amends ADR 0100: `abort` by a take.
  - Amends ADR 0102: a resume restores kept state.
  - Amends ADR 0113: `by: person` from any Client, with no `closed`.
  - Notes on ADRs 0079 (`session_control_held`), 0015 (a new sealed field) and 0089 (`page.lend()` refused).
- **`GLOSSARY.md`:** add **Attended Session** and **Automation call**. Change **Live view**: "which that person can take control of during a Human pause, or at any time in an Attended Session".
- **`docs/agents/rollout-order.md`:** nothing.
