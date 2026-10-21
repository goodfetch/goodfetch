---- MODULE RunFencing ----
\* Run fencing (ADRs 0002, 0003 and 0060; spec #213): work-item leases with a
\* lease epoch inside each copy's run file, the run epoch a writer takes from
\* the state it starts from, Checkpoints of that state, and the destinations
\* that writes leaving the engine reach, each by its Guarantee class. A
\* resume starts from a run file or a Checkpoint, on its own machine or
\* another, while the copy it replaced may go on as a zombie: inside a copy,
\* a commit needs a live lease; between copies, only the destinations'
\* compare-and-set on the run epoch fences the zombie's writes. The
\* variables and actions carry the names of the `goodfetch::lease` trace
\* events: `run_epoch`, `lease_epoch`, and the transitions `take_epoch`,
\* `grant`, `renew`, `expire`, `release`, `commit` and `refuse_commit`.
EXTENDS Naturals, FiniteSets

CONSTANTS Copies, Items, MaxEpoch, MaxLease

VARIABLES
    run_epoch,     \* each copy's run epoch, 0 before it writes the Run
    lease_epoch,   \* each copy's lease epoch of each work item
    leased,        \* the work items each copy holds a live lease on
    done,          \* the work items each copy's run file committed
    checkpoint,    \* each copy's last Checkpoint: its committed state and run epoch
    fenced,        \* the Fenced destination: the run epochs whose writes it accepted
    fenced_epoch,  \* the run epoch it compares and sets
    keyed,         \* the Keyed destination: each work item's Records, by run epoch
    keyed_epoch    \* the run epoch it compares and sets

vars == <<run_epoch, lease_epoch, leased, done, checkpoint, fenced, fenced_epoch,
          keyed, keyed_epoch>>

NoLeases == [w \in Items |-> 0]

Init ==
    /\ run_epoch = [c \in Copies |-> 0]
    /\ lease_epoch = [c \in Copies |-> NoLeases]
    /\ leased = [c \in Copies |-> {}]
    /\ done = [c \in Copies |-> {}]
    /\ checkpoint = [c \in Copies |-> [done |-> {}, lease_epoch |-> NoLeases, run_epoch |-> 0]]
    /\ fenced = {}
    /\ fenced_epoch = 0
    /\ keyed = [w \in Items |-> {}]
    /\ keyed_epoch = 0

Destinations == <<fenced, fenced_epoch, keyed, keyed_epoch>>

\* `take_epoch` at the start: one copy begins the Run at run epoch 1.
Start(c) ==
    /\ \A other \in Copies : run_epoch[other] = 0
    /\ run_epoch' = [run_epoch EXCEPT ![c] = 1]
    /\ UNCHANGED <<lease_epoch, leased, done, checkpoint>>
    /\ UNCHANGED Destinations

\* A Checkpoint of a writer's committed state, with its run epoch.
Checkpoint(c) ==
    /\ run_epoch[c] > 0
    /\ checkpoint' = [checkpoint EXCEPT ![c] =
           [done |-> done[c], lease_epoch |-> lease_epoch[c], run_epoch |-> run_epoch[c]]]
    /\ UNCHANGED <<run_epoch, lease_epoch, leased, done>>
    /\ UNCHANGED Destinations

\* `take_epoch` on a resume from the copy's own run file, after a crash or a
\* stop: the next run epoch of its file; its leases ended with its process.
Restart(c) ==
    /\ run_epoch[c] > 0
    /\ run_epoch[c] < MaxEpoch
    /\ run_epoch' = [run_epoch EXCEPT ![c] = @ + 1]
    /\ leased' = [leased EXCEPT ![c] = {}]
    /\ UNCHANGED <<lease_epoch, done, checkpoint>>
    /\ UNCHANGED Destinations

Max(a, b) == IF a >= b THEN a ELSE b

\* `take_epoch` on a resume of copy `c`, on any machine, from `from`'s last
\* Checkpoint, which becomes its run file: the next run epoch after both
\* that state's and the one `c`'s own run file held, so a copy's run epoch
\* never goes down. The copy `from` may live on.
ResumeFrom(c, from) ==
    /\ checkpoint[from].run_epoch > 0
    /\ Max(checkpoint[from].run_epoch, run_epoch[c]) < MaxEpoch
    /\ run_epoch' = [run_epoch EXCEPT ![c] = Max(checkpoint[from].run_epoch, @) + 1]
    /\ done' = [done EXCEPT ![c] = checkpoint[from].done]
    /\ lease_epoch' = [lease_epoch EXCEPT ![c] = checkpoint[from].lease_epoch]
    /\ leased' = [leased EXCEPT ![c] = {}]
    /\ UNCHANGED checkpoint
    /\ UNCHANGED Destinations

\* `grant`: a writer leases a work item its run file has not committed,
\* under the item's next lease epoch.
Grant(c, w) ==
    /\ run_epoch[c] > 0
    /\ w \notin done[c]
    /\ w \notin leased[c]
    /\ lease_epoch[c][w] < MaxLease
    /\ lease_epoch' = [lease_epoch EXCEPT ![c][w] = @ + 1]
    /\ leased' = [leased EXCEPT ![c] = @ \cup {w}]
    /\ UNCHANGED <<run_epoch, done, checkpoint>>
    /\ UNCHANGED Destinations

\* `renew`: only lengthens a live lease, so nothing a commit checks changes.
Renew(c, w) ==
    /\ w \in leased[c]
    /\ UNCHANGED vars

\* `expire` or `release`: the lease ends, and its attempt may still answer.
Expire(c, w) ==
    /\ w \in leased[c]
    /\ leased' = [leased EXCEPT ![c] = @ \ {w}]
    /\ UNCHANGED <<run_epoch, lease_epoch, done, checkpoint>>
    /\ UNCHANGED Destinations

\* The Fenced destination accepts a write of a run epoch no older than the
\* one it holds, and sets it.
ToFenced(epoch) ==
    IF epoch >= fenced_epoch
    THEN /\ fenced' = fenced \cup {epoch}
         /\ fenced_epoch' = epoch
    ELSE UNCHANGED <<fenced, fenced_epoch>>

\* The Keyed destination does too, and keys each Record by its work item, so
\* a newer run epoch's Record replaces the item's older one.
ToKeyed(w, epoch) ==
    IF epoch >= keyed_epoch
    THEN /\ keyed' = [keyed EXCEPT ![w] = {epoch}]
         /\ keyed_epoch' = epoch
    ELSE UNCHANGED <<keyed, keyed_epoch>>

\* `commit`: an attempt's outputs commit once, in the copy's run file, under
\* a live lease, and its Records leave the engine carrying the copy's run
\* epoch.
Commit(c, w) ==
    /\ w \in leased[c]
    /\ w \notin done[c]
    /\ done' = [done EXCEPT ![c] = @ \cup {w}]
    /\ leased' = [leased EXCEPT ![c] = @ \ {w}]
    /\ ToFenced(run_epoch[c])
    /\ ToKeyed(w, run_epoch[c])
    /\ UNCHANGED <<run_epoch, lease_epoch, checkpoint>>

\* `refuse_commit`: a late answer, its lease ended, commits nothing.
RefuseCommit(c, w) ==
    /\ run_epoch[c] > 0
    /\ w \notin leased[c]
    /\ UNCHANGED vars

Next ==
    \/ \E c \in Copies : Start(c) \/ Checkpoint(c) \/ Restart(c)
    \/ \E c, from \in Copies : ResumeFrom(c, from)
    \/ \E c \in Copies, w \in Items :
        Grant(c, w) \/ Renew(c, w) \/ Expire(c, w) \/ Commit(c, w)
        \/ RefuseCommit(c, w)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ run_epoch \in [Copies -> 0..MaxEpoch]
    /\ lease_epoch \in [Copies -> [Items -> 0..MaxLease]]
    /\ leased \in [Copies -> SUBSET Items]
    /\ done \in [Copies -> SUBSET Items]
    /\ fenced \in SUBSET (1..MaxEpoch)
    /\ fenced_epoch \in 0..MaxEpoch
    /\ keyed \in [Items -> SUBSET (1..MaxEpoch)]
    /\ keyed_epoch \in 0..MaxEpoch

\* A copy commits only work items it leased and never twice, so its leased
\* items are never committed ones.
LeasedNotDone == \A c \in Copies : leased[c] \cap done[c] = {}

\* A copy's run epoch never goes down, so its own last Checkpoint is never
\* of a newer run epoch than it writes under.
CheckpointBehind ==
    \A c \in Copies : checkpoint[c].run_epoch <= run_epoch[c]
                      \/ checkpoint[c].run_epoch = 0

\* No write of a stale run epoch lands at the Fenced destination after a
\* newer one did: the run epoch it holds is the newest it accepted.
NoStaleWriteLands == \A epoch \in fenced : epoch <= fenced_epoch

\* No work item's Record is twice at the Keyed destination, however many
\* copies committed it.
NoRecordTwice == \A w \in Items : Cardinality(keyed[w]) <= 1

\* `zombie.cfg`'s invariant: one copy at most ever writes the Run. A resume
\* while the replaced copy lives on breaks it, so the variant shows the model
\* reaches the zombie copy the destinations must fence.
OneWriter == Cardinality({c \in Copies : run_epoch[c] > 0}) <= 1
====
