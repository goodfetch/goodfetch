---- MODULE TraceValidation ----
\* The trace-validation spec template (ADR 0060; TESTING.md, Model
\* checking), in etcd's style. A model that validates traces holds a
\* `Trace<Model>.tla` that EXTENDS the model and this module, and a
\* `trace.cfg`; its `model.toml` names that spec as `trace_spec`. The spec
\* reads the NDJSON trace the `TRACE` environment variable names, one line
\* for each `tracing` event the model's target wrote (the model trace layer
\* of `goodfetch-test-support`), and its next-state relation maps each
\* line's `transition` to the model's action, its fields constraining the
\* action's parameters and the state it leaves; an action the code does not
\* trace is a step that consumes no line.
\*
\* `trace.cfg` checks `TraceUnmatched` as an invariant, beside the model's
\* own: a state that matched every line breaks it, and that is the trace
\* validating. A trace no behaviour of the model matches leaves it unbroken,
\* and fails its validation (`xtask tlc validate`). An invariant rather
\* than etcd's `TLCGet("queue")` property, because the steps that consume no
\* line branch, and a branching search keeps more than one state queued.
EXTENDS Naturals, Sequences, TLC, Json, IOUtils

VARIABLE l  \* the trace's next line, Len(Trace) + 1 once every line matched

Trace == ndJsonDeserialize(IOEnv.TRACE)

Line == Trace[l]

\* Whether the next line is a `transition` event.
IsEvent(transition) == l <= Len(Trace) /\ Line.transition = transition

\* The step consumes the next line.
Consume == l' = l + 1

\* The lines whose `transition` is `transition`.
LinesOf(transition) == {i \in 1..Len(Trace) : Trace[i].transition = transition}

\* The values of `field` in the lines that carry it.
ValuesOf(field) ==
    {Trace[i][field] : i \in {j \in 1..Len(Trace) : field \in DOMAIN Trace[j]}}

\* Broken once every line matched: the trace validates.
TraceUnmatched == l <= Len(Trace)
====
