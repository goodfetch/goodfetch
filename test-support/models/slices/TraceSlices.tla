---- MODULE TraceSlices ----
\* Validates a `goodfetch::ledger` trace against Slices (ADR 0060), from
\* the TraceValidation template. The trace holds one Limit, set by its first
\* line, sized by the default Slice sizing (a share of 10 thousandths of the
\* Limit, a minimum of 1 thousandth rounded up and never below 1); its runs
\* and Slices are the model's. Each charge and reserve line is as many
\* model `Charge` steps as its units, on any of the run's Slices, and each
\* settle line as many `Settle` steps as it refunds; a run's give_back
\* lines, one for each Slice it held, are one `GiveBack`. The ledger traces
\* no crash nor resume, so each is a step that consumes no line.
EXTENDS Slices, TraceValidation

VARIABLE pending  \* the units of the current charge, reserve or settle line still to step

MaxOf(S) == CHOOSE m \in S : \A n \in S : n <= m

\* The constants the trace gives the model.
TraceAmount == Trace[1].amount
TraceRuns == ValuesOf("run_id")
TraceSliceIds == ValuesOf("slice")
TraceShare == (TraceAmount * 10) \div 1000
TraceMinimum == Max((TraceAmount + 999) \div 1000, 1)
TraceMaxEpoch == MaxOf(ValuesOf("epoch")) + 1

ASSUME LinesOf("set_limit") = {1}

TraceInit == Init /\ l = 1 /\ pending = 0

SetLimitLine ==
    /\ IsEvent("set_limit")
    /\ UNCHANGED vars
    /\ Consume
    /\ UNCHANGED pending

TakeSliceLine ==
    /\ IsEvent("take_slice")
    /\ pending = 0
    /\ epoch[Line.run_id] = Line.epoch
    /\ TakeSlice(Line.run_id, Line.slice, Line.size)
    /\ size'[Line.slice] = Line.size
    /\ Consume
    /\ UNCHANGED pending

RefuseSliceLine ==
    /\ IsEvent("refuse_slice")
    /\ pending = 0
    /\ alive[Line.run_id]
    /\ epoch[Line.run_id] = Line.epoch
    /\ UNCHANGED vars
    /\ Consume
    /\ UNCHANGED pending

\* A charge, reserve or settle line: one unit of `Unit` on one of the run's
\* Slices at each step, the line consumed with its last unit.
UnitLine(transition, Units, Unit(_, _)) ==
    /\ IsEvent(transition)
    /\ epoch[Line.run_id] = Line.epoch
    /\ LET todo == IF pending = 0 THEN Units ELSE pending
       IN IF todo = 0
          THEN UNCHANGED vars /\ Consume /\ UNCHANGED pending
          ELSE /\ \E s \in SliceIds : Unit(Line.run_id, s)
               /\ IF todo = 1
                  THEN Consume /\ pending' = 0
                  ELSE UNCHANGED l /\ pending' = todo - 1

\* A run's give_back lines, one for each Slice it holds, each charging what
\* the run journaled of it.
GiveBackLines ==
    /\ IsEvent("give_back")
    /\ pending = 0
    /\ LET r == holder[Line.slice]
           last == l + Cardinality(Mine(r)) - 1
       IN /\ Line.slice \in Mine(r)
          /\ last <= Len(Trace)
          /\ {Trace[i].slice : i \in l..last} = Mine(r)
          /\ \A i \in l..last :
                /\ Trace[i].transition = "give_back"
                /\ journal[Trace[i].slice] = Min(size[Trace[i].slice], Trace[i].used)
          /\ GiveBack(r)
          /\ l' = last + 1
    /\ UNCHANGED pending

\* A reclaim or mark_for_review line, whose `Close` charges the Slice.
CloseLine(transition, Close(_)) ==
    /\ IsEvent(transition)
    /\ pending = 0
    /\ holder[Line.slice] = Line.run_id
    /\ taken[Line.slice] = Line.epoch
    /\ Close(Line.slice)
    /\ charged'[Line.slice] = Line.charged
    /\ Consume
    /\ UNCHANGED pending

\* An engine dies, or another resumes its run: the ledger traces neither.
Untraced ==
    /\ pending = 0
    /\ \E r \in Runs : Crash(r) \/ Resume(r)
    /\ UNCHANGED <<l, pending>>

TraceNext ==
    \/ SetLimitLine
    \/ TakeSliceLine
    \/ RefuseSliceLine
    \/ UnitLine("charge", Line.amount, Charge)
    \/ UnitLine("reserve", Line.worst, Charge)
    \/ UnitLine("settle", Line.refund, Settle)
    \/ GiveBackLines
    \/ CloseLine("reclaim", Reclaim)
    \/ CloseLine("mark_for_review", MarkForReview)
    \/ Untraced
====
