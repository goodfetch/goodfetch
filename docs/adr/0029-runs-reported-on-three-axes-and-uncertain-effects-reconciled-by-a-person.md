# A run is reported on three separate axes, and a person reconciles an uncertain Effect

Every surface (the protocol, SDKs, CLI, MCP, the Console) reports a run on three separate axes instead of one status. **Lifecycle** says where the run is: `running`, `stopped` (ended early by a Limit or an abort, resumable by a continuation), `finished` or `failed`. **Verdict** says whether its committed results are proven, once it has stopped or finished: `verified` when every declared Success check passed and no Effect is left uncertain, `needs-review` otherwise (ADR 0013). **Completeness** says whether those results cover the scope: `complete`, `incomplete` or `unverified` (ADR 0005).

A single status would have to merge facts that vary independently. A run stopped by its Limit can hold verified Records and still be incomplete. A finished crawl can pass its checks with an `unverified` Completeness. A finished agent task without a Success check needs review. A Human pause does not change the Lifecycle, because only the affected Session waits.

When an Effect is left uncertain (ADRs 0002 and 0010), a person **reconciles** it with one of three answers:
- **It happened**: the Effect is committed in the Effect journal, and no continuation repeats it.
- **It did not happen**: a continuation may perform it once.
- **Leave uncertain**: the run stays `needs-review`.

Each answer is recorded in the Audit log with who gave it.

Decided in [Prototype: V1 Console screens and flows](https://github.com/Jbmanllr/automan/issues/80), where the prototype showed these states on real-looking runs.

## Considered Options

- **One status enum** (`succeeded`, `partial`, `needs_review`, `failed`): rejected. It cannot say "verified but incomplete" or "finished but needs review" without inventing combined values, and it mixes what happened with how far the result can be trusted.
- **Only "it happened" or "it did not happen"**: rejected. A person often cannot tell yet, for example while waiting for an order e-mail. They need to leave the Effect uncertain without blocking the rest of the review.

## Consequences

- `failed` covers an error or a declared Success check that fails; a `failed` run has no Verdict.
- A run stopped by a Limit or an abort is never `complete` (ADR 0005). Raising the Limit resumes it as a continuation (ADR 0012).
- Reconciling is an operation of the Operation catalog (ADR 0011), so the CLI, SDKs and MCP can do it as well as the Console.
- A retry after "it did not happen" is an ordinary Effect. ADR 0014's confirmations still apply, so an agent cannot use a reconciliation to repeat a payment silently.
- Notifications (ADR 0027) and the run report use these same three axes.
