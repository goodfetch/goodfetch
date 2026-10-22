---- MODULE TraceProbe ----
\* The pinned tools' trace-validation probe (ADR 0060), in etcd's style: the
\* NDJSON trace the JSON environment variable names must be a behaviour of a
\* counter stepping by 1 or 2. `match.ndjson` is one, so TLC passes;
\* `diverge.ndjson` is not, so TLC fails on TraceMatched. It needs `Json`
\* from `tla2tools.jar` and `IOUtils` from CommunityModules, so a pair whose
\* CommunityModules fail to load fails it. Not a model (no `model.toml`):
\* `tools.toml` gives the commands that run it.
EXTENDS Naturals, Sequences, TLC, Json, IOUtils

VARIABLES x, l

TraceFile == IF "JSON" \in DOMAIN IOEnv THEN IOEnv.JSON ELSE "match.ndjson"

TraceLog == ndJsonDeserialize(TraceFile)

Init == x = 0 /\ l = 1

Step(d) ==
    /\ l <= Len(TraceLog)
    /\ x' = x + d
    /\ x' = TraceLog[l].x
    /\ l' = l + 1

TraceNext == Step(1) \/ Step(2)

\* Checked as a temporal property, after all successors of a state exist:
\* the trace is unmatched when the queue empties before its last line.
TraceMatched ==
    [](l <= Len(TraceLog) => [](TLCGet("queue") = 1 \/ l > Len(TraceLog)))
====
