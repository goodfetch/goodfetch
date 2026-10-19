---- MODULE Sample ----
\* The TLC job's self-test (ADR 0060): hosts take and release one lease, each
\* take in a new epoch. `small.cfg` passes; `broken.cfg` checks an invariant
\* the first take breaks, so the job shows TLC failing on it.
EXTENDS Naturals

CONSTANTS Hosts, None, MaxEpoch

VARIABLES holder, epoch

vars == <<holder, epoch>>

Init == holder = None /\ epoch = 0

Take(host) ==
    /\ holder = None
    /\ epoch < MaxEpoch
    /\ holder' = host
    /\ epoch' = epoch + 1

Release(host) ==
    /\ holder = host
    /\ holder' = None
    /\ UNCHANGED epoch

\* Once every epoch is spent, the lease stays released.
Done == holder = None /\ epoch = MaxEpoch /\ UNCHANGED vars

Next == (\E host \in Hosts : Take(host) \/ Release(host)) \/ Done

Spec == Init /\ [][Next]_vars

TypeOK == holder \in Hosts \cup {None} /\ epoch \in 0..MaxEpoch

\* The lease is held only within an epoch that was taken.
HeldInAnEpoch == holder # None => epoch > 0

\* broken.cfg's invariant: no host ever holds the lease, which the first take
\* breaks.
NeverHeld == holder = None
====
