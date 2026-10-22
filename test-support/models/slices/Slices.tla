---- MODULE Slices ----
\* Slices and reclaim (ADRs 0041 and 0060): runs of one user's engines take
\* Slices of one Limit, each in one ledger transaction, sized as the smaller
\* of a share of the Limit and what is left over twice the live runs holding
\* Slices, never below a minimum and never more than is left, and at least
\* the worst case a reservation needs, in the same transaction, or none. A
\* run journals what it uses of a Slice under its run epoch before it uses
\* it, a reservation's worst case included, journals again what a settled
\* reservation did not cost, and gives back what is left when it ends. An engine dies with its runs' Slices held;
\* the next engine to open the ledger reclaims each Slice whose run's engine
\* is gone, or whose run a later epoch resumed, charging what the run
\* journaled, or the whole Slice, for review, when its state cannot be read.
\* Each action is one transition `ledger` traces under `goodfetch::ledger`:
\* take_slice, charge (or reserve, a worst case charged unit by unit before
\* it is paid), settle, give_back, reclaim and mark_for_review.
EXTENDS Naturals, FiniteSets

CONSTANTS
    Runs,       \* the runs, each the Run ID of one engine's run
    SliceIds,   \* the Slice rows the ledger may hold
    Amount,     \* the Limit
    Share,      \* the largest Slice: 1 % of the Limit
    Minimum,    \* the smallest Slice: 0.1 % of the Limit
    Need,       \* the largest worst case a reservation needs at once
    MaxEpoch    \* how often a run may be resumed under a new run epoch

VARIABLES
    state,      \* each Slice row: "free" until taken, then "held",
                \* "returned", "reclaimed" or "review"
    holder,     \* the run that took each Slice
    taken,      \* the run epoch it took it under
    size,       \* its size
    journal,    \* what its run journaled of it, in the run's own state
    charged,    \* what the ledger charged for it once it is no longer held
    epoch,      \* each run's current run epoch
    alive,      \* whether each run's engine is alive
    used        \* what the runs used, in all

vars == <<state, holder, taken, size, journal, charged, epoch, alive, used>>

Min(a, b) == IF a < b THEN a ELSE b
Max(a, b) == IF a > b THEN a ELSE b

RECURSIVE SumOf(_, _)
SumOf(f, S) ==
    IF S = {} THEN 0
    ELSE LET s == CHOOSE s \in S : TRUE IN f[s] + SumOf(f, S \ {s})

Held == {s \in SliceIds : state[s] = "held"}
Closed == {s \in SliceIds : state[s] \in {"returned", "reclaimed", "review"}}

\* What the Limit counts: what closed Slices charged, and what held ones hold.
Counted == SumOf(charged, Closed) + SumOf(size, Held)
Left == Amount - Counted

\* The live runs holding Slices, other than r.
Others(r) == Cardinality({holder[s] : s \in Held} \ {r})

SliceSize(r) ==
    Min(Max(Min(Share, Left \div (2 * (Others(r) + 1))), Minimum), Left)

Init ==
    /\ state = [s \in SliceIds |-> "free"]
    /\ holder = [s \in SliceIds |-> CHOOSE r \in Runs : TRUE]
    /\ taken = [s \in SliceIds |-> 0]
    /\ size = [s \in SliceIds |-> 0]
    /\ journal = [s \in SliceIds |-> 0]
    /\ charged = [s \in SliceIds |-> 0]
    /\ epoch = [r \in Runs |-> 1]
    /\ alive = [r \in Runs |-> TRUE]
    /\ used = 0

\* take_slice: one transaction takes a free row, sized from what is left,
\* and at least the n units a reservation needs, or nothing.
TakeSlice(r, s, n) ==
    /\ alive[r]
    /\ state[s] = "free"
    /\ Max(SliceSize(r), n) > 0
    /\ Max(SliceSize(r), n) <= Left
    /\ state' = [state EXCEPT ![s] = "held"]
    /\ holder' = [holder EXCEPT ![s] = r]
    /\ taken' = [taken EXCEPT ![s] = epoch[r]]
    /\ size' = [size EXCEPT ![s] = Max(SliceSize(r), n)]
    /\ UNCHANGED <<journal, charged, epoch, alive, used>>

\* charge: the run journals one more unit of a Slice it holds under its
\* epoch, then uses it.
Charge(r, s) ==
    /\ alive[r]
    /\ state[s] = "held" /\ holder[s] = r /\ taken[s] = epoch[r]
    /\ journal[s] < size[s]
    /\ journal' = [journal EXCEPT ![s] = @ + 1]
    /\ used' = used + 1
    /\ UNCHANGED <<state, holder, taken, size, charged, epoch, alive>>

\* settle: a settled reservation cost one unit less than its worst case;
\* the run journals it, and may use that unit again.
Settle(r, s) ==
    /\ alive[r]
    /\ state[s] = "held" /\ holder[s] = r /\ taken[s] = epoch[r]
    /\ journal[s] > 0
    /\ journal' = [journal EXCEPT ![s] = @ - 1]
    /\ used' = used - 1
    /\ UNCHANGED <<state, holder, taken, size, charged, epoch, alive>>

Mine(r) == {s \in Held : holder[s] = r /\ taken[s] = epoch[r]}

\* give_back: the run ends and gives back each Slice it still holds,
\* charging what it used; it may start again under a later epoch.
GiveBack(r) ==
    /\ alive[r]
    /\ epoch[r] < MaxEpoch
    /\ state' = [s \in SliceIds |-> IF s \in Mine(r) THEN "returned" ELSE state[s]]
    /\ charged' = [s \in SliceIds |-> IF s \in Mine(r) THEN journal[s] ELSE charged[s]]
    /\ epoch' = [epoch EXCEPT ![r] = @ + 1]
    /\ UNCHANGED <<holder, taken, size, journal, alive, used>>

\* The run's engine dies; its Slices stay held.
Crash(r) ==
    /\ alive[r]
    /\ alive' = [alive EXCEPT ![r] = FALSE]
    /\ UNCHANGED <<state, holder, taken, size, journal, charged, epoch, used>>

\* Another engine resumes the run under the next run epoch.
Resume(r) ==
    /\ ~alive[r]
    /\ epoch[r] < MaxEpoch
    /\ alive' = [alive EXCEPT ![r] = TRUE]
    /\ epoch' = [epoch EXCEPT ![r] = @ + 1]
    /\ UNCHANGED <<state, holder, taken, size, journal, charged, used>>

\* Whose writer lock is free, or held at a later epoch.
Gone(s) == ~alive[holder[s]] \/ epoch[holder[s]] > taken[s]

\* reclaim: what the run journaled under the Slice's epoch.
Reclaim(s) ==
    /\ state[s] = "held" /\ Gone(s)
    /\ state' = [state EXCEPT ![s] = "reclaimed"]
    /\ charged' = [charged EXCEPT ![s] = journal[s]]
    /\ UNCHANGED <<holder, taken, size, journal, epoch, alive, used>>

\* mark_for_review: its run state cannot be read, so the whole Slice.
MarkForReview(s) ==
    /\ state[s] = "held" /\ Gone(s)
    /\ state' = [state EXCEPT ![s] = "review"]
    /\ charged' = [charged EXCEPT ![s] = size[s]]
    /\ UNCHANGED <<holder, taken, size, journal, epoch, alive, used>>

Next ==
    \/ \E r \in Runs, s \in SliceIds, n \in 0..Need : TakeSlice(r, s, n)
    \/ \E r \in Runs, s \in SliceIds : Charge(r, s) \/ Settle(r, s)
    \/ \E r \in Runs : GiveBack(r) \/ Crash(r) \/ Resume(r)
    \/ \E s \in SliceIds : Reclaim(s) \/ MarkForReview(s)
    \/ UNCHANGED vars

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ state \in [SliceIds -> {"free", "held", "returned", "reclaimed", "review"}]
    /\ holder \in [SliceIds -> Runs]
    /\ taken \in [SliceIds -> 0..MaxEpoch]
    /\ size \in [SliceIds -> 0..Amount]
    /\ journal \in [SliceIds -> 0..Amount]
    /\ charged \in [SliceIds -> 0..Amount]
    /\ epoch \in [Runs -> 1..MaxEpoch]
    /\ alive \in [Runs -> BOOLEAN]
    /\ used \in 0..Amount

\* What is charged and held never passes the Limit.
WithinLimit == Counted <= Amount

\* No run uses more than the Slices it took.
UsedWithinSlices == \A s \in SliceIds : journal[s] <= size[s]

\* Every unit used is charged once a Slice closes: what a run journaled
\* never escapes the Limit, and a Slice for review is charged in full.
NothingEscapes ==
    \A s \in Closed :
        IF state[s] = "review" THEN charged[s] = size[s] ELSE charged[s] = journal[s]

\* What the runs used never passes the Limit.
UsedWithinLimit == used <= Amount
====
