# Click qualification

`pnpm actionability 3` runs 324 cells: six backends, nine variants,
two policies and three randomized blocks. Each cell starts a fresh
adapter, uses the same Chromium and viewport, then keeps its events,
effects and verdicts. Startup is separated from task time.
`pnpm actionability 3 guarded` makes it possible to requalify only the 162 cells
with the guard, while keeping the previous native matrix.

The backends are Playwright, Puppeteer, Stagehand v4, Stagehand v4 batch,
Crawlee/Playwright and `@crawlee/stagehand`. This last extension stays on
Stagehand v3 and its handler uses the primitives of the Playwright page.
No model is called; this campaign does not measure observe/act/extract.

| Variant | Observable contract |
| --- | --- |
| normal | One trusted click on the target, one exact effect |
| overlay | Wait for the covering element to disappear, without clicking the overlay |
| delayed | Wait for the button to be enabled |
| hidden | Wait for the button to be visible |
| moving | Wait for the movement to end |
| rerender | Find the button again after replacement, then enablement |
| never-enabled | Refuse the click; no effect |
| never-uncovered | Refuse the click; no effect, including on the overlay |
| ambiguous | Refuse a selector that matches two buttons |

The **native** policy only waits for the document to be initialized,
then calls the backend's own primitive. The **guarded** policy adds
the same guard: unique selector, visibility, enablement, rectangle stable
over two intervals between three RAF observations spaced at least 16 ms apart,
and hit-test at the center. High-frequency frames must not mask
a movement that occurs between two observations that are too close together.
The guard does not use the internal availability flag expected by the oracle.
It covers these fixtures, not all of Playwright's compatibility.

The first matrix exposed four premature clicks by the guard on moving
targets. The first observation was taken outside RAF and the guard could
accept two identical positions before the next movement. Moving
only this observation into RAF was not enough for the repeated public test.
The two stability intervals above fix this defect; the red/green
logs and the requalification matrix keep the evidence of the change.

The server oracle checks the target, the number of effects, `isTrusted` and the phase
in which the click took place. A premature click or a click on the overlay fails, even if the API
returns a success. For the three negative variants, the absence of effect is not
enough: the operation must also have been refused.

The budget of the guards and of the Playwright/Puppeteer primitives is 800 ms.
Stagehand v4 does not expose the same locator timeout in this version; the
harness keeps an outer deadline of 8 s and reports this difference.
The Crawlee handlers have an additional orchestration margin to let
the primitive refuse before the crawler kills the task. The times of refusals
must not be equated with the times of successful clicks.

The [documented Playwright guarantees](https://playwright.dev/docs/actionability)
provide the reference for uniqueness, visibility, stability, event
reception and enablement. The campaign tests these properties on its corpus,
without deducing a universal equivalence from a number of cells.

A backend that fails in native mode remains visible in the report. The times
of a variant are only interpreted for its verified effects, with the
sample sizes and the three blocks. The old 50-click results had common
availability conditions; this campaign extends their scope.
