# Compiled plans are committed files, verified in full before the first Effect

AI work is compiled once into a Plan: concrete steps whose targets each carry a Fingerprint (tag, type, label, button text, plan marker; never the selector). The Plan is a readable file versioned next to the script or Recipe, like a lockfile, and later runs replay it with no model call. Before the first Effect, every step's target is checked against its Fingerprint; on Drift the engine makes one Repair (a fresh observation and one recompile, checked in full again), and a second Drift fails the run. In the lab, compiled replay cost 0.17 to 0.34 times an agent that replans each run, with no false success across three real models. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).

Evidence: [AI-in-the-loop browser frameworks](https://github.com/Jbmanllr/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md).

## Considered Options

- **Checking step by step**: rejected. In the lab it ticked a consent box before seeing the Drift, and the repair then reported a wrong booking as a success.
- **A cache held by a service**: rejected. Stagehand v4's cache works only with Browserbase-hosted browsers and receives secret values; users ask for a versioned file they can run in CI.
- **Automatic regeneration on every break** (Skyvern): rejected for Plans with Effects; an unreviewed new Plan could act on the wrong control.

## Consequences

- A Repair is used within its run once checked, but becomes a new Plan version only after the whole run finishes verified (Midscene never writes back a partly repaired plan; Momentic re-caches only after the rest of the test passes). Read-only Plans are promoted automatically; Plans with Effects produce a reviewable diff unless the Recipe opts into auto-approval.
- `observe` and `act` become Plan steps; `extract` stores its selectors but reads values live; `check` is never cached.
- The Console recorder and a successful AI agent login both write Plans in this format.
