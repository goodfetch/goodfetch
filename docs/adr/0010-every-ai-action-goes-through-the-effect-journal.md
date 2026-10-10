# Every AI-driven action goes through the Effect journal; an unverifiable Effect is never redone

Every action an AI decides (built-in agent, MCP, function tools, code mode, `act`, Repair) is recorded in the run's Effect journal, so a retried or corrected step skips an Effect the site already received instead of performing it again. When redone work after a machine loss reaches an Effect that has no way to verify whether it already happened, the engine skips it and reports the work item uncertain, unless the Effect declares itself repeatable. In the lab, raw code mode gave 17 false successes out of 45 with one model and none with the journal. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72), answering the question ADR 0003 left open.

## Considered Options

- **The journal for code mode only**, the R&D's scope: rejected. Every AI surface can repeat an action after a correction, and the journal costs nothing measurable.
- **Forcing it on the user's own code**: rejected. User code opts in by declaring Effects.
- **Performing an unverifiable Effect again**: rejected. A double order is worse than a missing one that is reported.

## Consequences

- An `act` that fails after its Effect happened is reported uncertain, never retried silently (Stagehand documents that `act()` can fail after acting).
- Model calls are journaled too, so a resumed run reuses a journaled answer instead of calling the model again.
