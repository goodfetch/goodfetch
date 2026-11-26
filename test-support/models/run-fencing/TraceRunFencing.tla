---- MODULE TraceRunFencing ----
\* Validates a `goodfetch::lease` trace against RunFencing (ADR 0060), from
\* the TraceValidation template. Each line names its copy by the run epoch
\* it writes under: the copy whose `run_epoch` is the line's. Its work
\* items are the trace's, each known from the start, and its lease epochs
\* and run epochs go as high as the trace's. A `take_epoch` line is a start
\* or a resume of a copy, which takes the line's run epoch; a trace with no
\* `take_epoch` line, an in-process test's leases on one writer, starts its
\* copy with a step that consumes no line. `grant` is `Grant`, its lease
\* epoch the line's; `renew` is `Renew`; `expire` and `release` are
\* `Expire`; `commit` is `Commit`; `refuse_commit` is `RefuseCommit`.
\*
\* The model numbers a copy's leases by work item: its lease epoch and
\* whether it is leased are the work item's latest lease. A line about an
\* older lease of the work item, a late answer's `expire` or
\* `refuse_commit` once a newer lease was granted, changes nothing the
\* model holds, and is a step that changes no variable. So is a
\* `refuse_commit` for `run_epoch`, the run file's own fence of a copy a
\* newer run epoch replaced: the model fences between copies at the
\* destinations only. Checkpoints, threshold batches, seeding calls and
\* Effects carry no `goodfetch::lease` event, and no trace holds them.
EXTENDS RunFencing, TraceValidation

MaxOf(S) == CHOOSE m \in S : \A n \in S : n <= m

\* The constants the trace gives the model.
TraceCopies == ValuesOf("run_epoch")
TraceItems == ValuesOf("work_item")
TraceMaxEpoch == MaxOf(ValuesOf("run_epoch"))
TraceMaxLease == MaxOf(ValuesOf("lease_epoch") \cup {0})

TraceInit == Init /\ l = 1

\* The copy writing under the line's run epoch.
Writer(c) == run_epoch[c] = Line.run_epoch

\* The line is about an older lease of its work item than copy `c` holds.
Older(c) == lease_epoch[c][Line.work_item] > Line.lease_epoch

\* The line is about the latest lease of its work item that `c` holds.
Latest(c) == lease_epoch[c][Line.work_item] = Line.lease_epoch

TakeEpochLine ==
    /\ IsEvent("take_epoch")
    /\ \E c \in Copies :
        /\ \/ Start(c)
           \/ ResumeAfterStop(c)
           \/ ResumeAfterCrash(c)
           \/ \E from \in Copies : ResumeFrom(c, from)
        /\ run_epoch'[c] = Line.run_epoch
    /\ Consume

GrantLine ==
    /\ IsEvent("grant")
    /\ \E c \in Copies :
        /\ Writer(c)
        /\ Grant(c, Line.work_item)
        /\ lease_epoch'[c][Line.work_item] = Line.lease_epoch
    /\ Consume

\* A line of the latest lease, which `Action` takes; of an older one, a
\* step that changes nothing.
LeaseLine(transition, Action(_, _)) ==
    /\ IsEvent(transition)
    /\ \E c \in Copies :
        /\ Writer(c)
        /\ \/ Latest(c) /\ Action(c, Line.work_item)
           \/ Older(c) /\ UNCHANGED vars
    /\ Consume

RefuseCommitLine ==
    /\ IsEvent("refuse_commit")
    /\ \E c \in Copies :
        /\ Writer(c)
        /\ CASE Line.reason = "lease_lost" ->
                   \/ Latest(c) /\ RefuseCommit(c, Line.work_item)
                   \/ Older(c) /\ UNCHANGED vars
             [] Line.reason = "run_epoch" ->
                   /\ \E other \in Copies : run_epoch[other] > run_epoch[c]
                   /\ UNCHANGED vars
             [] OTHER -> FALSE
    /\ Consume

\* A trace of one writer's leases, with no `take_epoch` line, starts its
\* copy at the trace's run epoch.
UntracedStart ==
    /\ LinesOf("take_epoch") = {}
    /\ \E c \in Copies : Start(c) /\ run_epoch'[c] = Trace[1].run_epoch
    /\ UNCHANGED l

TraceNext ==
    \/ TakeEpochLine
    \/ GrantLine
    \/ LeaseLine("renew", Renew)
    \/ LeaseLine("expire", Expire)
    \/ LeaseLine("release", Expire)
    \/ LeaseLine("commit", Commit)
    \/ RefuseCommitLine
    \/ UntracedStart
====
