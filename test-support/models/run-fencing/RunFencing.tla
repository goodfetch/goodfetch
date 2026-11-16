---- MODULE RunFencing ----
\* Run fencing (ADRs 0002, 0003, 0060 and 0142; spec #213, spec #2138):
\* work-item leases, each under a lease epoch its copy keeps in memory and
\* writes to its run file (a crash can take one back, so only the run epoch
\* fences between copies), the run epoch a writer takes from the state it
\* starts from, Checkpoints of that
\* state, and the destinations that writes leaving the engine reach, each by
\* its Guarantee class. A resume starts from a run file or a Checkpoint, on
\* its own machine or another, while the copy it replaced may go on as a
\* zombie: inside a copy, a commit needs a live lease; between copies, only
\* the destinations' compare-and-set on the run epoch fences the zombie's
\* writes. The traced variables and actions carry the names of the
\* `goodfetch::lease` trace events: `run_epoch`, `lease_epoch`, and the
\* transitions `take_epoch` (`Start` and each resume), `grant`, `renew`,
\* `expire` and `release` (`Expire`), `commit` and `refuse_commit`.
\* `Checkpoint`, `WriteBatch`, `Seed` and `Effect` carry no `goodfetch::lease` event: no
\* trace holds them, and TLC's trace validation, which would read them,
\* joins with the Simulation in M4 (TESTING.md, Model checking).
\*
\* A copy's state lives in its memory, and its run file holds the last
\* Run-state batch it wrote (ADR 0142): the file moves only from one batch
\* boundary to the next, a crash returns the copy to it, and a clean stop's
\* last batch holds everything. A Record leaves the engine once its batch is
\* written. The `level` is `per_attempt`, where each lease and each outcome
\* is a batch of its own (`full`, and `fast` and `periodic` after a process
\* crash), or `batches`, where batches are sealed when a threshold says
\* (`batch`, the default). At every level an Effect's journal entry and the
\* change of a call no attempt will make again, a call that carries no
\* attempt (`run.add_seeds`), are barriers: their batch is written before
\* the Effect happens or the call is answered. A call an attempt makes
\* changes memory only, and reaches the file with its attempt's outcome:
\* `Commit` stands for both. Left out: `off`, whose crash discards the run
\* file and starts the Run again, and a power cut below `sync: each`, which
\* can take back a written batch.
EXTENDS Naturals, FiniteSets, TLC

CONSTANTS Copies, Items, Seeds, Levels, MaxEpoch, MaxLease

ASSUME Seeds \subseteq Items
ASSUME Levels \subseteq {"per_attempt", "batches"}

VARIABLES
    level,         \* the Run's level: "per_attempt" or "batches"
    run_epoch,     \* each copy's run epoch, 0 before it writes the Run
    lease_epoch,   \* each copy's lease epoch of each work item, in memory
    leased,        \* the work items each copy holds a live lease on
    done,          \* the work items each copy committed, in memory
    seeded,        \* the work items each copy knows, in memory
    journal,       \* the work items whose Effect each copy journaled, in memory
    file,          \* each copy's run file: the last batch it wrote
    checkpoint,    \* each copy's last Checkpoint: its written state and run epoch
    fenced,        \* the Fenced destination: the run epochs whose writes it accepted
    fenced_epoch,  \* the run epoch it compares and sets
    keyed,         \* the Keyed destination: each work item's Records, by run epoch
    keyed_epoch,   \* the run epoch it compares and sets
    ended,         \* the work items whose end each copy promised: a written batch
                   \* carries it, or a clean stop said so; none is fetched again
    answered,      \* the work items whose seeding call each copy answered
    restored,      \* the copies whose run file came from a Checkpoint
    replaced,      \* the copies another copy resumed from: each ended there
    performed      \* each work item's Effects that happened outside the engine

vars == <<level, run_epoch, lease_epoch, leased, done, seeded, journal, file,
          checkpoint, fenced, fenced_epoch, keyed, keyed_epoch, ended, answered,
          restored, replaced, performed>>

NoLeases == [w \in Items |-> 0]

\* A copy's state in memory, as a batch would write it.
Memory(c) ==
    [done |-> done[c], lease_epoch |-> lease_epoch[c], seeded |-> seeded[c],
     journal |-> journal[c]]

Empty == [done |-> {}, lease_epoch |-> NoLeases, seeded |-> Seeds, journal |-> {}]

Init ==
    /\ level \in Levels
    /\ run_epoch = [c \in Copies |-> 0]
    /\ lease_epoch = [c \in Copies |-> NoLeases]
    /\ leased = [c \in Copies |-> {}]
    /\ done = [c \in Copies |-> {}]
    /\ seeded = [c \in Copies |-> Seeds]
    /\ journal = [c \in Copies |-> {}]
    /\ file = [c \in Copies |-> Empty]
    /\ checkpoint = [c \in Copies |-> [done |-> {}, lease_epoch |-> NoLeases, seeded |-> Seeds,
                                        journal |-> {}, run_epoch |-> 0]]
    /\ fenced = {}
    /\ fenced_epoch = 0
    /\ keyed = [w \in Items |-> {}]
    /\ keyed_epoch = 0
    /\ ended = [c \in Copies |-> {}]
    /\ answered = [c \in Copies |-> {}]
    /\ restored = {}
    /\ replaced = {}
    /\ performed = [w \in Items |-> 0]

Destinations == <<fenced, fenced_epoch, keyed, keyed_epoch>>
Ghosts == <<answered, restored, replaced, performed>>

\* The Fenced destination accepts a write of a run epoch no older than the
\* one it holds, and sets it.
ToFenced(epoch) ==
    IF epoch >= fenced_epoch
    THEN /\ fenced' = fenced \cup {epoch}
         /\ fenced_epoch' = epoch
    ELSE UNCHANGED <<fenced, fenced_epoch>>

\* The Keyed destination does too, and keys each Record by its work item, so
\* a newer run epoch's Record replaces the item's older one.
ToKeyed(items, epoch) ==
    IF epoch >= keyed_epoch
    THEN /\ keyed' = [w \in Items |-> IF w \in items THEN {epoch} ELSE keyed[w]]
         /\ keyed_epoch' = epoch
    ELSE UNCHANGED <<keyed, keyed_epoch>>

\* Copy `c` writes the batch that moves its run file to `state`: the
\* Records of the work items it ends leave the engine carrying the copy's
\* run epoch.
WriteFile(c, state) ==
    LET new == state.done \ file[c].done IN
    /\ file' = [file EXCEPT ![c] = state]
    /\ IF new = {}
       THEN UNCHANGED Destinations
       ELSE ToFenced(run_epoch[c]) /\ ToKeyed(new, run_epoch[c])

\* A written batch promises the ends it carries.
Write(c, state) ==
    /\ WriteFile(c, state)
    /\ ended' = [ended EXCEPT ![c] = @ \cup state.done]

\* No batch is written: nothing leaves the engine and no end is promised.
NothingWritten == UNCHANGED <<file, ended>> /\ UNCHANGED Destinations

\* Copy `c`'s memory becomes `state`, a run file's, as a resume reads it.
ReadBack(c, state) ==
    /\ done' = [done EXCEPT ![c] = state.done]
    /\ lease_epoch' = [lease_epoch EXCEPT ![c] = state.lease_epoch]
    /\ seeded' = [seeded EXCEPT ![c] = state.seeded]
    /\ journal' = [journal EXCEPT ![c] = state.journal]

\* An attempt's write: a batch of its own at `per_attempt`'s level, in memory
\* until a batch holds it at `batches`'.
Attempt(c, state) ==
    IF level = "per_attempt"
    THEN Write(c, state)
    ELSE NothingWritten

\* `take_epoch` at the start: one copy begins the Run at run epoch 1.
Start(c) ==
    /\ \A other \in Copies : run_epoch[other] = 0
    /\ run_epoch' = [run_epoch EXCEPT ![c] = 1]
    /\ UNCHANGED <<level, lease_epoch, leased, done, seeded, journal, checkpoint>>
    /\ NothingWritten
    /\ UNCHANGED Ghosts

\* A Checkpoint of a writer's state: it writes the current batch first, so
\* the Checkpoint holds everything memory does, with its run epoch.
Checkpoint(c) ==
    /\ run_epoch[c] > 0
    /\ Write(c, Memory(c))
    /\ checkpoint' = [checkpoint EXCEPT ![c] =
           [done |-> done[c], lease_epoch |-> lease_epoch[c], seeded |-> seeded[c],
            journal |-> journal[c], run_epoch |-> run_epoch[c]]]
    /\ UNCHANGED <<level, run_epoch, lease_epoch, leased, done, seeded, journal>>
    /\ UNCHANGED Ghosts

\* A batch sealed by a threshold, at `batches`' level: every change memory
\* holds is written.
WriteBatch(c) ==
    /\ level = "batches"
    /\ run_epoch[c] > 0
    /\ file[c] # Memory(c)
    /\ Write(c, Memory(c))
    /\ UNCHANGED <<level, run_epoch, lease_epoch, leased, done, seeded, journal, checkpoint>>
    /\ UNCHANGED Ghosts

\* A clean stop, then `take_epoch` on a resume from the copy's own run
\* file: the stop drained its attempts, each ended or returned to pending,
\* and its last batch wrote everything memory held, so the resume reads
\* back every work item that had ended. The stop promises each end memory
\* held (ADR 0142), whatever its last batch carried.
ResumeAfterStop(c) ==
    /\ run_epoch[c] > 0
    /\ run_epoch[c] < MaxEpoch
    /\ run_epoch' = [run_epoch EXCEPT ![c] = @ + 1]
    /\ leased' = [leased EXCEPT ![c] = {}]
    /\ WriteFile(c, Memory(c))
    /\ ended' = [ended EXCEPT ![c] = @ \cup done[c]]
    /\ ReadBack(c, file'[c])
    /\ UNCHANGED <<level, checkpoint>>
    /\ UNCHANGED Ghosts

\* A crash, then `take_epoch` on a resume from the copy's own run file: its
\* leases ended with its process, and its memory is the last batch it
\* wrote. The work items whose end no written batch holds are fetched
\* again; their Records never left the engine.
ResumeAfterCrash(c) ==
    /\ run_epoch[c] > 0
    /\ run_epoch[c] < MaxEpoch
    /\ run_epoch' = [run_epoch EXCEPT ![c] = @ + 1]
    /\ leased' = [leased EXCEPT ![c] = {}]
    /\ ReadBack(c, file[c])
    /\ UNCHANGED <<level, checkpoint>>
    /\ NothingWritten
    /\ UNCHANGED Ghosts

Max(a, b) == IF a >= b THEN a ELSE b

\* `take_epoch` on a resume of copy `c`, on any machine, from `from`'s last
\* Checkpoint, which becomes its run file: the next run epoch after both
\* that state's and the one `c`'s own run file held, so a copy's run epoch
\* never goes down. The copy `from` may live on. The work since the
\* Checkpoint is redone, and every Effect the redone work reaches is checked
\* first (ADRs 0002 and 0010): the resume promises only the Checkpoint's
\* ends, and takes back the calls answered after it.
ResumeFrom(c, from) ==
    /\ checkpoint[from].run_epoch > 0
    /\ Max(checkpoint[from].run_epoch, run_epoch[c]) < MaxEpoch
    /\ LET state == [done |-> checkpoint[from].done,
                     lease_epoch |-> checkpoint[from].lease_epoch,
                     seeded |-> checkpoint[from].seeded,
                     journal |-> checkpoint[from].journal] IN
       /\ run_epoch' = [run_epoch EXCEPT ![c] = Max(checkpoint[from].run_epoch, @) + 1]
       /\ ReadBack(c, state)
       /\ file' = [file EXCEPT ![c] = state]
       /\ ended' = [ended EXCEPT ![c] = state.done]
    /\ leased' = [leased EXCEPT ![c] = {}]
    /\ answered' = [answered EXCEPT ![c] = {}]
    /\ restored' = restored \cup {c}
    /\ replaced' = IF from = c THEN replaced \ {c} ELSE (replaced \ {c}) \cup {from}
    /\ UNCHANGED <<level, checkpoint, performed>>
    /\ UNCHANGED Destinations

\* A call that carries no attempt (`run.add_seeds`), a barrier: it seeds a
\* work item, and is answered once the batch holding its change is written.
Seed(c, w) ==
    /\ run_epoch[c] > 0
    /\ w \notin seeded[c]
    /\ seeded' = [seeded EXCEPT ![c] = @ \cup {w}]
    /\ Write(c, [Memory(c) EXCEPT !.seeded = seeded'[c]])
    /\ answered' = [answered EXCEPT ![c] = @ \cup {w}]
    /\ UNCHANGED <<level, run_epoch, lease_epoch, leased, done, journal, checkpoint>>
    /\ UNCHANGED <<restored, replaced, performed>>

\* `grant`: a writer leases a work item it knows and has not committed,
\* under the item's next lease epoch, and the attempt begins.
Grant(c, w) ==
    /\ run_epoch[c] > 0
    /\ w \in seeded[c]
    /\ w \notin done[c]
    /\ w \notin leased[c]
    /\ lease_epoch[c][w] < MaxLease
    /\ lease_epoch' = [lease_epoch EXCEPT ![c][w] = @ + 1]
    /\ leased' = [leased EXCEPT ![c] = @ \cup {w}]
    /\ Attempt(c, [Memory(c) EXCEPT !.lease_epoch = lease_epoch'[c]])
    /\ UNCHANGED <<level, run_epoch, done, seeded, journal, checkpoint>>
    /\ UNCHANGED Ghosts

\* `renew`: only lengthens a live lease, so nothing a commit checks changes.
Renew(c, w) ==
    /\ w \in leased[c]
    /\ UNCHANGED vars

\* `expire` or `release`: the lease ends, and its attempt may still answer.
Expire(c, w) ==
    /\ w \in leased[c]
    /\ leased' = [leased EXCEPT ![c] = @ \ {w}]
    /\ UNCHANGED <<level, run_epoch, lease_epoch, done, seeded, journal, checkpoint>>
    /\ NothingWritten
    /\ UNCHANGED Ghosts

\* An Effect of an attempt, a barrier: the batch holding its journal entry
\* is written before the Effect happens, so a redone attempt finds it in the
\* journal. A copy whose run file came from a Checkpoint checks an Effect its
\* journal lacks first, and finds one that happened. A site's Effect has no
\* compare-and-set, and fencing cannot undo one (ADR 0003): the model
\* assumes, and does not check, that a resume elsewhere kills the copy it
\* resumed from first, so a replaced copy opens none. No Effect can be
\* opened in the code before #1550, so no trace holds one.
Effect(c, w) ==
    /\ w \in leased[c]
    /\ w \notin journal[c]
    /\ c \notin replaced
    /\ journal' = [journal EXCEPT ![c] = @ \cup {w}]
    /\ Write(c, [Memory(c) EXCEPT !.journal = journal'[c]])
    /\ performed' = IF c \in restored /\ performed[w] > 0
                    THEN performed
                    ELSE [performed EXCEPT ![w] = @ + 1]
    /\ UNCHANGED <<level, run_epoch, lease_epoch, leased, done, seeded, checkpoint>>
    /\ UNCHANGED <<answered, restored, replaced>>

\* `commit`: an attempt's outcome, with every change its calls made, commits
\* once in the copy's memory under a live lease, and its work item ended.
Commit(c, w) ==
    /\ w \in leased[c]
    /\ w \notin done[c]
    /\ done' = [done EXCEPT ![c] = @ \cup {w}]
    /\ leased' = [leased EXCEPT ![c] = @ \ {w}]
    /\ Attempt(c, [Memory(c) EXCEPT !.done = done'[c]])
    /\ UNCHANGED <<level, run_epoch, lease_epoch, seeded, journal, checkpoint>>
    /\ UNCHANGED Ghosts

\* `refuse_commit`: a late answer, its lease ended, commits nothing.
RefuseCommit(c, w) ==
    /\ run_epoch[c] > 0
    /\ w \notin leased[c]
    /\ UNCHANGED vars

Next ==
    \/ \E c \in Copies :
        Start(c) \/ Checkpoint(c) \/ WriteBatch(c) \/ ResumeAfterStop(c)
        \/ ResumeAfterCrash(c)
    \/ \E c, from \in Copies : ResumeFrom(c, from)
    \/ \E c \in Copies, w \in Items :
        Seed(c, w) \/ Grant(c, w) \/ Renew(c, w) \/ Expire(c, w) \/ Effect(c, w)
        \/ Commit(c, w) \/ RefuseCommit(c, w)

Spec == Init /\ [][Next]_vars

\* The copies are interchangeable, so a configuration can check one state
\* of each permutation of them.
CopySymmetry == Permutations(Copies)

State == [done : SUBSET Items, lease_epoch : [Items -> 0..MaxLease],
          seeded : SUBSET Items, journal : SUBSET Items]

TypeOK ==
    /\ level \in Levels
    /\ run_epoch \in [Copies -> 0..MaxEpoch]
    /\ lease_epoch \in [Copies -> [Items -> 0..MaxLease]]
    /\ leased \in [Copies -> SUBSET Items]
    /\ done \in [Copies -> SUBSET Items]
    /\ seeded \in [Copies -> SUBSET Items]
    /\ journal \in [Copies -> SUBSET Items]
    /\ file \in [Copies -> State]
    /\ fenced \in SUBSET (1..MaxEpoch)
    /\ fenced_epoch \in 0..MaxEpoch
    /\ keyed \in [Items -> SUBSET (1..MaxEpoch)]
    /\ keyed_epoch \in 0..MaxEpoch
    /\ ended \in [Copies -> SUBSET Items]
    /\ answered \in [Copies -> SUBSET Items]
    /\ restored \subseteq Copies
    /\ replaced \subseteq Copies
    /\ performed \in [Items -> Nat]

\* A copy commits only work items it leased and never twice, so its leased
\* items are never committed ones.
LeasedNotDone == \A c \in Copies : leased[c] \cap done[c] = {}

\* A copy's run file is a batch boundary its memory passed: it never holds
\* a change memory lacks, and holds every journaled Effect. At
\* `per_attempt`'s level it holds everything memory does.
FileBehindMemory ==
    \A c \in Copies :
        /\ file[c].done \subseteq done[c]
        /\ file[c].seeded \subseteq seeded[c]
        /\ \A w \in Items : file[c].lease_epoch[w] <= lease_epoch[c][w]
        /\ file[c].journal = journal[c]
        /\ level = "per_attempt" => file[c] = Memory(c)

\* No work item is fetched again once its copy promised its end: a crash
\* takes back only the ends no written batch carries, and a clean stop none.
NoFetchAfterEnd == \A c \in Copies : leased[c] \cap ended[c] = {}

\* No Effect happens twice, whatever the copies crash, stop or resume.
NoEffectTwice == \A w \in Items : performed[w] <= 1

\* A call that carries no attempt is answered only once its change is
\* written, so no crash takes it back.
AnsweredKept == \A c \in Copies : answered[c] \subseteq seeded[c]

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
