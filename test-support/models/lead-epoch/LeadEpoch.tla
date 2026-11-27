---- MODULE LeadEpoch ----
\* Lead Daemon confirmation, fencing and membership change (ADRs 0055 and 0060):
\* each Group member durably records the highest lead epoch it has seen and
\* refuses any message carrying a lower one. A Daemon configured as Lead Daemon
\* at a lead epoch acts as the single writer only after confirming that epoch
\* with a strict majority of its last-known Group members (itself included), or
\* after an operator confirms it (`group lead --confirm`). Hearing a higher
\* lead epoch fences a Lead Daemon off permanently for that epoch.
\*
\* Each traced action matches a transition `group` emits under
\* `goodfetch::group`: `raise`, `refuse_stale`, `ack`, `confirm_majority`,
\* `confirm_operator`, `fence`, `add_member` and `remove_member`.
EXTENDS Naturals, FiniteSets

CONSTANTS
    Members,    \* the Group members' names
    MaxEpoch    \* the highest lead epoch the model checks

VARIABLES
    highest,        \* each Group member's durably recorded highest lead epoch
    assigned,       \* at most one configured Lead Daemon per lead epoch
    role,           \* each member's role: "follower", "unconfirmed", "confirmed", "fenced"
    leadEpoch,      \* the lead epoch a member is configured to lead at (0 if follower)
    known,          \* each Lead Daemon's last-known Group members
    acked,          \* last-known Group members that acknowledged leadEpoch[m]
    confirmedOver,  \* known[m] at the moment m confirmed, for majority checking
    confirmedAt,    \* members that ever confirmed as Lead Daemon at each epoch
    byOperator,     \* whether each epoch was confirmed by `group lead --confirm`
    acted           \* members that ever acted as Lead Daemon at each epoch

vars == <<
    highest, assigned, role, leadEpoch, known, acked,
    confirmedOver, confirmedAt, byOperator, acted
>>

Max(a, b) == IF a > b THEN a ELSE b

Init ==
    /\ highest = [m \in Members |-> 1]
    /\ assigned = [e \in 1..MaxEpoch |-> {}]
    /\ role = [m \in Members |-> "follower"]
    /\ leadEpoch = [m \in Members |-> 0]
    /\ known = [m \in Members |-> {}]
    /\ acked = [m \in Members |-> {}]
    /\ confirmedOver = [m \in Members |-> {}]
    /\ confirmedAt = [e \in 1..MaxEpoch |-> {}]
    /\ byOperator = [e \in 1..MaxEpoch |-> FALSE]
    /\ acted = [e \in 1..MaxEpoch |-> {}]

\* A Daemon starts or restarts configured as Lead Daemon at epoch `e` with
\* last-known Group members `S` (itself included). Each lead epoch belongs to
\* at most one configured Lead Daemon (operator configuration invariant, ADR 0055).
StartLead(m, e, S) ==
    /\ e \in 1..MaxEpoch
    /\ S \subseteq Members
    /\ m \in S
    /\ assigned[e] \subseteq {m}
    /\ highest[m] <= e
    /\ highest' = [highest EXCEPT ![m] = e]
    /\ assigned' = [assigned EXCEPT ![e] = {m}]
    /\ role' = [role EXCEPT ![m] = "unconfirmed"]
    /\ leadEpoch' = [leadEpoch EXCEPT ![m] = e]
    /\ known' = [known EXCEPT ![m] = S]
    /\ acked' = [acked EXCEPT ![m] = {m}]
    /\ confirmedOver' = [confirmedOver EXCEPT ![m] = {}]
    /\ UNCHANGED <<confirmedAt, byOperator, acted>>

\* raise: a Group member receives a peer message carrying a higher lead epoch
\* from a configured Lead Daemon and durably records it.
Raise(p, e) ==
    /\ e \in 1..MaxEpoch
    /\ assigned[e] /= {}
    /\ e > highest[p]
    /\ highest' = [highest EXCEPT ![p] = e]
    /\ UNCHANGED <<
        assigned, role, leadEpoch, known, acked,
        confirmedOver, confirmedAt, byOperator, acted
       >>

\* refuse_stale: a Group member refuses a peer message whose lead epoch is lower
\* than its durably recorded highest.
RefuseStale(p, e) ==
    /\ e \in 1..MaxEpoch
    /\ e < highest[p]
    /\ UNCHANGED vars

\* ack: an unconfirmed Lead Daemon receives confirmation of its lead epoch from
\* a last-known Group member whose durable highest equals that epoch.
Ack(m, peer) ==
    /\ role[m] = "unconfirmed"
    /\ peer \in known[m] \ acked[m]
    /\ highest[peer] = leadEpoch[m]
    /\ acked' = [acked EXCEPT ![m] = @ \union {peer}]
    /\ UNCHANGED <<
        highest, assigned, role, leadEpoch, known,
        confirmedOver, confirmedAt, byOperator, acted
       >>

\* confirm_majority: an unconfirmed Lead Daemon reaches a strict majority of its
\* last-known Group members and may act.
ConfirmMajority(m) ==
    /\ role[m] = "unconfirmed"
    /\ 2 * Cardinality(acked[m]) > Cardinality(known[m])
    /\ role' = [role EXCEPT ![m] = "confirmed"]
    /\ confirmedOver' = [confirmedOver EXCEPT ![m] = known[m]]
    /\ confirmedAt' = [confirmedAt EXCEPT ![leadEpoch[m]] = @ \union {m}]
    /\ UNCHANGED <<highest, assigned, leadEpoch, known, acked, byOperator, acted>>

\* confirm_operator: an operator confirms an unconfirmed Lead Daemon by hand
\* (`group lead --confirm`) when a majority cannot be reached.
ConfirmOperator(m) ==
    /\ role[m] = "unconfirmed"
    /\ role' = [role EXCEPT ![m] = "confirmed"]
    /\ confirmedOver' = [confirmedOver EXCEPT ![m] = known[m]]
    /\ confirmedAt' = [confirmedAt EXCEPT ![leadEpoch[m]] = @ \union {m}]
    /\ byOperator' = [byOperator EXCEPT ![leadEpoch[m]] = TRUE]
    /\ UNCHANGED <<highest, assigned, leadEpoch, known, acked, acted>>

\* fence: a Lead Daemon hears a higher lead epoch from a peer and fences itself
\* permanently for its epoch.
Fence(m, h) ==
    /\ role[m] \in {"unconfirmed", "confirmed"}
    /\ h \in 1..MaxEpoch
    /\ h > leadEpoch[m]
    /\ \E p \in Members : highest[p] = h
    /\ role' = [role EXCEPT ![m] = "fenced"]
    /\ highest' = [highest EXCEPT ![m] = Max(highest[m], h)]
    /\ UNCHANGED <<assigned, leadEpoch, known, acked, confirmedOver, confirmedAt, byOperator, acted>>

\* add_member: a confirmed Lead Daemon admits a new Group member, which records
\* at least the Lead Daemon's epoch upon joining.
AddMember(m, target) ==
    /\ role[m] = "confirmed"
    /\ target \in Members \ known[m]
    /\ known' = [known EXCEPT ![m] = @ \union {target}]
    /\ highest' = [highest EXCEPT ![target] = Max(highest[target], leadEpoch[m])]
    /\ UNCHANGED <<assigned, role, leadEpoch, acked, confirmedOver, confirmedAt, byOperator, acted>>

\* remove_member: a confirmed Lead Daemon removes another Group member (never
\* itself).
RemoveMember(m, target) ==
    /\ role[m] = "confirmed"
    /\ target \in known[m] \ {m}
    /\ known' = [known EXCEPT ![m] = @ \ {target}]
    /\ UNCHANGED <<highest, assigned, role, leadEpoch, acked, confirmedOver, confirmedAt, byOperator, acted>>

\* A confirmed Lead Daemon performs a single-writer action at its lead epoch.
Act(m) ==
    /\ role[m] = "confirmed"
    /\ acted' = [acted EXCEPT ![leadEpoch[m]] = @ \union {m}]
    /\ UNCHANGED <<
        highest, assigned, role, leadEpoch, known, acked,
        confirmedOver, confirmedAt, byOperator
       >>

Next ==
    \/ \E m \in Members, e \in 1..MaxEpoch, S \in SUBSET Members : StartLead(m, e, S)
    \/ \E p \in Members, e \in 1..MaxEpoch : Raise(p, e) \/ RefuseStale(p, e)
    \/ \E m \in Members, peer \in Members : Ack(m, peer)
    \/ \E m \in Members : ConfirmMajority(m) \/ ConfirmOperator(m) \/ Act(m)
    \/ \E m \in Members, h \in 1..MaxEpoch : Fence(m, h)
    \/ \E m \in Members, target \in Members : AddMember(m, target) \/ RemoveMember(m, target)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ highest \in [Members -> 1..MaxEpoch]
    /\ assigned \in [1..MaxEpoch -> SUBSET Members]
    /\ role \in [Members -> {"follower", "unconfirmed", "confirmed", "fenced"}]
    /\ leadEpoch \in [Members -> 0..MaxEpoch]
    /\ known \in [Members -> SUBSET Members]
    /\ acked \in [Members -> SUBSET Members]
    /\ confirmedOver \in [Members -> SUBSET Members]
    /\ confirmedAt \in [1..MaxEpoch -> SUBSET Members]
    /\ byOperator \in [1..MaxEpoch -> BOOLEAN]
    /\ acted \in [1..MaxEpoch -> SUBSET Members]

\* At most one Lead Daemon ever confirms or acts at any given lead epoch.
OneLeadPerEpoch ==
    \A e \in 1..MaxEpoch :
        /\ Cardinality(confirmedAt[e]) <= 1
        /\ Cardinality(acted[e]) <= 1

\* A Lead Daemon is always in its own last-known member set until fenced.
LeadNeverRemoved ==
    \A m \in Members :
        role[m] \in {"unconfirmed", "confirmed"} => m \in known[m]

\* A fenced Lead Daemon has seen a strictly higher lead epoch.
FencedSawHigher ==
    \A m \in Members :
        role[m] = "fenced" => highest[m] > leadEpoch[m]

\* Every member that acknowledged a Lead Daemon durably recorded at least its
\* epoch, and every majority-confirmed Lead Daemon reached a strict majority of
\* its last-known Group members at confirmation time.
MajorityRecorded ==
    \A m \in Members :
        /\ \A p \in acked[m] : highest[p] >= leadEpoch[m]
        /\ (role[m] = "confirmed" /\ ~byOperator[leadEpoch[m]]) =>
            2 * Cardinality(acked[m]) > Cardinality(confirmedOver[m])
====
