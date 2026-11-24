# A run is reported on three separate axes, and a person reconciles an uncertain Effect

Every Client reports a run on three separate axes instead of one status:

- **Lifecycle** says where the run is:
  - `running`
  - `stopped`: ended early by a Limit, an abort, the loss of its engine, or exhausted resources (ADR 0040)
    > Amended by [ADR 0080](0080-sdk-timeouts-nest-waits-arm-when-called-and-a-program-sets-limits-but-never-raises-them.md): a Run's own timeout is a fifth cause. A Run it stops is never `complete`, keeps a Checkpoint, and resumes with a fresh clock.
  - `finished`: reached its end
    > Amended by [ADR 0102](0102-a-program-names-its-run-by-an-id-that-a-start-resumes-ends-it-with-a-run-report-and-reads-what-waits-and-why.md): a finished crawl's Run returns to `running` when its program retries its failed requests, its Verdict and Completeness given anew when it ends, as "it did not happen" reopens a run.
  - `failed`: ended by an error in the engine or the user's code
- **Verdict** says whether the run's committed results are proven by the run's own checks. It is given once the run is no longer `running`, `failed` runs included, and reads the latest evaluation of each check:
  - `verified`: every check the run relies on passed (its Success checks, Page checks and Grounding), none of those checks nor the Target fingerprint check before the first Effect was switched off (ADR 0023), and no Effect is left uncertain. An AI task also needs a declared Success check that passed (ADR 0013).
    > Amended by [ADR 0091](0091-collies-ai-verbs-live-on-the-page-replay-from-a-plan-file-the-run-names-and-expect-runs-a-success-check.md): a Run whose program calls `act` or `observe` counts as an AI task, so it needs a Success check, which `expect` runs, evaluated after the last such call and passing; a Run whose only AI steps are `extract` is judged by Grounding.
    >
    > Note (owner's decision, 2026-11-10, grilling of decision G-06 of the consolidated review: Q3 answered "Yes, all fourteen", on the grilling's recommended option as put; the wording is the grilling session's, and the owner did not read it): of the controls ADR 0023's note of 2026-11-06 adds, only Block classification is a check a Run's results rely on, so `fetch.block_classification: false` gives the Verdict `needs_review`, with a reason naming the key. `frontier.learning`, `provenance.log` and `dataset.records` are listed in `weakened` and do not change the Verdict; nor do the security, durability and politeness switches. Built by [#2308](https://github.com/goodfetch/goodfetch/issues/2308).
    >
    > Note (the grilling session's own, 2026-11-11, from G-06's answer to Q3, awaiting the owner's reading): Block classification thereby joins the checks this line names.
  - `rejected`: a declared Success check failed.
  - `needs-review`: everything else, including an AI task with no Success check, a run whose checks were switched off (ADRs 0013 and 0023), and a run with an uncertain Effect.
- **Completeness** says whether those results cover the scope: `complete`, `incomplete` or `unverified` (ADR 0005). Like the Verdict, it is given once the run is no longer `running`.
  > Amended by [ADR 0102](0102-a-program-names-its-run-by-an-id-that-a-start-resumes-ends-it-with-a-run-report-and-reads-what-waits-and-why.md): a Collie Run its program finishes is `complete`, since the program defines its scope and says it reached its end.

A single status would have to merge facts that vary independently:
- A run stopped by its Limit can hold Records whose checks passed and still be `incomplete`.
  > Amended by [ADR 0107](0107-a-runs-model-allowance-refuses-its-model-calls-once-spent-while-the-run-goes-on.md): a Run that skipped a crawl request for its Model allowance is `incomplete` though its Lifecycle is `finished`, its reason naming the allowance and the count of requests it skipped.
- A finished crawl can be `verified` with an `unverified` Completeness, when offset pagination left the engine unable to vouch for full coverage.
- A finished agent task without a Success check is `needs-review`.
- A Human pause does not change the lifecycle, because only the affected Session waits.

The Verdict is the run's claim about itself. ADR 0025's oracle-verified result is a separate measurement, made by the benchmark against a known expected result.

When an Effect is left uncertain (ADRs 0002 and 0010), a person **reconciles** it with one of three answers:
- **It happened**: the Effect is marked in the Effect journal as done, asserted by that person, and is never repeated. The Verdict and Completeness are then given anew.
- **It did not happen**: the work item is scheduled again and the run resumes (ADR 0002), so the Effect may be performed once more. The run is `running` again, and its Verdict and Completeness are given anew when it ends.
- **Leave uncertain**: the run cannot become `verified`.

An answer applies to one uncertain occurrence of the Effect. A later answer to the same occurrence replaces the earlier one. Once a retry has run, its own journal entry governs. Each answer is recorded in the Audit log with the person who gave it.

Decided in [Prototype: V1 Console screens and flows](https://github.com/goodfetch/goodfetch/issues/80), where the prototype showed these states on real-looking runs.

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): a person reconciles an uncertain Effect, raises a Limit and answers a Human pause through the CLI's interactive prompts, or later Goodfetch Cloud, and logs in through a headful window locally or the Live view once it is built. The rule otherwise stands.

## Considered Options

- **One status enum** (`succeeded`, `partial`, `needs_review`, `failed`): rejected. It cannot say "verified but incomplete" or "finished but needs review" without inventing combined values, and it mixes what happened with how far the result can be trusted.
- **Only "it happened" or "it did not happen"**: rejected. A person often cannot tell yet, for example while waiting for an order e-mail. They need to leave the Effect uncertain without blocking the rest of the review.
- **A failing Success check as a `failed` lifecycle**: rejected for the same reason as the single enum. A crawl whose final check fails has still committed its Records.

## Consequences

- **Only a person reconciles.** Reconciling is an idempotent write in the Operation catalog (ADR 0011), answered only in the Console or an interactive CLI prompt.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - Changes ADR 0011: the answer is offered on no other surface generated from the catalog. That excludes MCP, code mode, the SDKs, agent skills, framework adapters, Integrations (ADR 0027), and the CLI when it is not interactive or runs with `--json`.
  - An agent can only ask for a reconciliation. Its request joins the items waiting for a person, and it is a Human pause only while the run is `running`.
  - This is a policy boundary, not a proof of humanity. A program that holds the user's own Daemon token (ADR 0024) can do whatever the user can. The Audit log records the surface that gave each answer.
    > Amended by [ADR 0149](0149-a-persons-passkey-is-checked-by-the-engine-over-the-connection-its-ceremony-is-the-clients-and-only-a-local-person-surface-answers-as-the-os-user.md) (spec [#1937](https://github.com/goodfetch/goodfetch/issues/1937), not read by the owner): the surface claim is believed only on a local carriage, an Attached engine's stdio or private endpoint or a machine's Daemon's own listener with its token. On any other carriage, an Exposed Daemon's address even at loopback and a Connection a gateway relays among them, it counts for nothing: the answer needs a Checked person, and Step-up for a write that approves, and with accounts off it is refused with `person_required`. The local rule stands.
  - No agent surface lets a model's word make a run `verified` (ADR 0013).
- **A retry after "it did not happen" is an ordinary Effect.** Within ADR 0014's scope it still asks for confirmation, unless the run allows that category.
- **A run stopped by a Limit is never `complete`** (ADR 0005); this ADR extends the rule to an abort, to the loss of the engine and to exhausted resources (ADR 0040). Raising the Limit resumes the run: this extends to every Limit what ADR 0012 allows for a currency Limit.
- **ADR 0009 keeps its meaning.** Its "verified run", after which a Repair is kept and a login Plan is promoted, is a `finished` run with the Verdict `verified`.
- **Changes ADR 0010.** An Effect that cannot be verified is still never redone automatically. A person's "it did not happen" is now the one way to redo it.
- **Changes ADR 0027.** These replace its "run started, finished, failed, or stopped by a Limit" and "an Effect reported uncertain" events; its other events, including "an approval requested", are unchanged:
  - run started, finished, `failed`, or `stopped` (whether by a Limit, an abort, the loss of the engine or exhausted resources)
  - Verdict `rejected` or `needs-review`
  - Completeness `incomplete`
  - an Effect reported uncertain, a reconciliation requested, or an Effect reconciled
