# A run is reported on three separate axes, and a person reconciles an uncertain Effect

Every Client (SDKs, CLI, MCP server, Console) reports a run on three separate axes instead of one status:

- **Lifecycle** says where the run is:
  - `running`
  - `stopped`: ended early by a Limit, an abort or the loss of its engine
  - `finished`: reached its end
  - `failed`: ended by an error in the engine or the user's code
- **Verdict** says whether the run's committed results are proven. It is given once the run is no longer `running`, `failed` runs included, and reads the latest evaluation of each declared Success check:
  - `verified`: the run declares at least one Success check, every one passed, none was switched off, and no Effect is left uncertain
  - `rejected`: a declared Success check failed
  - `needs-review`: everything else, including a run with no Success check, a run whose checks were switched off (ADRs 0013 and 0023), and a run with an uncertain Effect
- **Completeness** says whether those results cover the scope: `complete`, `incomplete` or `unverified` (ADR 0005). Like the Verdict, it is given once the run is no longer `running`.

A single status would have to merge facts that vary independently:
- A run stopped by its Limit can hold Records whose Success checks passed and still be `incomplete`.
- A finished crawl can be `verified` with an `unverified` Completeness, when offset pagination left the engine unable to vouch for full coverage.
- A finished agent task without a Success check is `needs-review`.
- A Human pause does not change the Lifecycle, because only the affected Session waits.

When an Effect is left uncertain (ADRs 0002 and 0010), a person **reconciles** it with one of three answers:
- **It happened**: the Effect is marked in the Effect journal as done, asserted by that person, and is never repeated.
- **It did not happen**: the work item goes back in the queue and the run resumes (ADR 0002), so the Effect may be performed once more.
- **Leave uncertain**: the run stays `needs-review`.

The latest answer replaces earlier ones, and each answer is recorded in the Audit log with the person who gave it. Decided in [Prototype: V1 Console screens and flows](https://github.com/Jbmanllr/automan/issues/80), where the prototype showed these states on real-looking runs.

## Considered Options

- **One status enum** (`succeeded`, `partial`, `needs_review`, `failed`): rejected. It cannot say "verified but incomplete" or "finished but needs review" without inventing combined values, and it mixes what happened with how far the result can be trusted.
- **Only "it happened" or "it did not happen"**: rejected. A person often cannot tell yet, for example while waiting for an order e-mail. They need to leave the Effect uncertain without blocking the rest of the review.
- **A failing Success check as a `failed` Lifecycle**: rejected for the same reason as the single enum. A crawl whose final check fails has still committed its Records.

## Consequences

- **Only a person reconciles.**
  - Reconciling is an idempotent write in the Operation catalog (ADR 0011), so the CLI, SDKs and Console can send a person's answer.
  - An agent cannot answer, whether over MCP or code mode. It can only ask, which opens a Human pause of kind review.
  - A model's word therefore never makes a run `verified` (ADR 0013).
- **A retry after "it did not happen" is an ordinary Effect.** Within ADR 0014's scope it still asks for confirmation, unless the run allows that category.
- **A run stopped by a Limit is never `complete`** (ADR 0005); this ADR extends the rule to an abort and to the loss of the engine. Raising the Limit resumes the run, as ADR 0012 already allows for a currency Limit.
- **Changes ADR 0010.** An Effect that cannot be verified is still never redone automatically. A person's "it did not happen" is now the one way to redo it.
- **Changes ADR 0027.** Notification events use these axes:
  - run started, finished, `failed`, or `stopped` (whether by a Limit, an abort or the loss of the engine)
  - Verdict `rejected` or `needs-review`
  - Completeness `incomplete`
  - an Effect reported uncertain or reconciled
