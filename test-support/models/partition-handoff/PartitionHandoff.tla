---- MODULE PartitionHandoff ----
\* Partitioned crawl domain handoff when Group members change (ADRs 0054, 0055
\* and 0060):
\* - A membership change publishes a new partition map under a higher
\*   partition-map epoch.
\* - A moving domain is handed off fenced: the old owner stops admitting it,
\*   waits until its requests in flight to it have ended (and seals its
\*   Run-state batch), sends its pending URLs, seen fingerprints and per-domain
\*   state, and the new owner starts admitting it only once the handoff is
\*   acknowledged.
\* - `runs adopt` completes a handoff from a removed Group member's kept volume
\*   when the drain grace period did not cover it.
\* - A forward routed under an old partition-map epoch is re-routed by a
\*   Partition that no longer owns the domain key.
\*
\* Each traced action matches a transition `goodfetch-partitioned-crawl` emits
\* under `goodfetch::partition`: `assign_initial`, `publish_map`,
\* `fence_domain`, `drain_inflight`, `send_handoff`, `ack_handoff`,
\* `adopt_handoff`, `admit_domain`, `start_fetch`, `finish_fetch` and
\* `reroute_forward`.
EXTENDS Naturals, FiniteSets

CONSTANTS
    Members,    \* the Group members' names
    Domains,    \* the domain keys
    MaxEpoch    \* the highest partition-map epoch the model checks

VARIABLES
    mapEpoch,       \* current published partition-map epoch (1..MaxEpoch)
    owner,          \* each domain's current target owner (Member or "none")
    admitting,      \* each domain's currently admitting members (subset of Members)
    inFlight,       \* each domain's members with fetches in flight (subset of Members)
    handoffState,   \* each domain's handoff phase
    handoffFrom,    \* old owner of an in-progress handoff (Member or "none")
    handoffTo,      \* new owner of an in-progress handoff (Member or "none")
    handoffEpoch    \* partition-map epoch of an in-progress handoff (0..MaxEpoch)

vars == <<
    mapEpoch, owner, admitting, inFlight,
    handoffState, handoffFrom, handoffTo, handoffEpoch
>>

HandoffStates == {"none", "fenced", "drained", "sent", "acked"}

Init ==
    /\ mapEpoch = 1
    /\ owner = [d \in Domains |-> "none"]
    /\ admitting = [d \in Domains |-> {}]
    /\ inFlight = [d \in Domains |-> {}]
    /\ handoffState = [d \in Domains |-> "none"]
    /\ handoffFrom = [d \in Domains |-> "none"]
    /\ handoffTo = [d \in Domains |-> "none"]
    /\ handoffEpoch = [d \in Domains |-> 0]

\* assign_initial: initial ownership of domain `d` to `m` at epoch 1 before any
\* membership change.
AssignInitial(d, m) ==
    /\ mapEpoch = 1
    /\ owner[d] = "none"
    /\ m \in Members
    /\ owner' = [owner EXCEPT ![d] = m]
    /\ admitting' = [admitting EXCEPT ![d] = {m}]
    /\ UNCHANGED <<mapEpoch, inFlight, handoffState, handoffFrom, handoffTo, handoffEpoch>>

\* publish_map: a membership change publishes a new partition map under a
\* strictly higher partition-map epoch once initial assignment is complete and
\* previous handoffs are settled.
PublishMap(e) ==
    /\ e = mapEpoch + 1
    /\ e <= MaxEpoch
    /\ \A d \in Domains : owner[d] /= "none" /\ handoffState[d] = "none"
    /\ mapEpoch' = e
    /\ UNCHANGED <<owner, admitting, inFlight, handoffState, handoffFrom, handoffTo, handoffEpoch>>

\* fence_domain: under `e = mapEpoch`, domain `d` moves from `from` to `to`.
\* `from` stops admitting `d` immediately; `to` does not admit `d` yet.
FenceDomain(d, from, to, e) ==
    /\ e = mapEpoch
    /\ e > 1
    /\ owner[d] = from
    /\ from \in Members
    /\ to \in Members
    /\ from /= to
    /\ handoffState[d] = "none"
    /\ owner' = [owner EXCEPT ![d] = to]
    /\ admitting' = [admitting EXCEPT ![d] = @ \ {from}]
    /\ handoffState' = [handoffState EXCEPT ![d] = "fenced"]
    /\ handoffFrom' = [handoffFrom EXCEPT ![d] = from]
    /\ handoffTo' = [handoffTo EXCEPT ![d] = to]
    /\ handoffEpoch' = [handoffEpoch EXCEPT ![d] = e]
    /\ UNCHANGED <<mapEpoch, inFlight>>

\* drain_inflight: old owner `from` has waited for all in-flight requests to `d`
\* to end and sealed its current Run-state batch.
DrainInflight(d, from, e) ==
    /\ handoffState[d] = "fenced"
    /\ handoffFrom[d] = from
    /\ handoffEpoch[d] = e
    /\ from \notin admitting[d]
    /\ inFlight[d] = {}
    /\ handoffState' = [handoffState EXCEPT ![d] = "drained"]
    /\ UNCHANGED <<mapEpoch, owner, admitting, inFlight, handoffFrom, handoffTo, handoffEpoch>>

\* send_handoff: old owner `from` sends `d`'s pending URLs, seen fingerprints
\* and per-domain state to `to`.
SendHandoff(d, from, to, e) ==
    /\ handoffState[d] = "drained"
    /\ handoffFrom[d] = from
    /\ handoffTo[d] = to
    /\ handoffEpoch[d] = e
    /\ inFlight[d] = {}
    /\ handoffState' = [handoffState EXCEPT ![d] = "sent"]
    /\ UNCHANGED <<mapEpoch, owner, admitting, inFlight, handoffFrom, handoffTo, handoffEpoch>>

\* ack_handoff: new owner `to` installs `d`'s pending URLs, seen fingerprints
\* and per-domain state and acknowledges the handoff.
AckHandoff(d, from, to, e) ==
    /\ handoffState[d] = "sent"
    /\ handoffFrom[d] = from
    /\ handoffTo[d] = to
    /\ handoffEpoch[d] = e
    /\ handoffState' = [handoffState EXCEPT ![d] = "acked"]
    /\ UNCHANGED <<mapEpoch, owner, admitting, inFlight, handoffFrom, handoffTo, handoffEpoch>>

\* adopt_handoff: `runs adopt` completes a handoff from a removed member's kept
\* volume directly to `to` once `from` has no in-flight fetches.
AdoptHandoff(d, from, to, e) ==
    /\ handoffState[d] \in {"fenced", "drained"}
    /\ handoffFrom[d] = from
    /\ handoffTo[d] = to
    /\ handoffEpoch[d] = e
    /\ from \notin admitting[d]
    /\ inFlight[d] = {}
    /\ handoffState' = [handoffState EXCEPT ![d] = "acked"]
    /\ UNCHANGED <<mapEpoch, owner, admitting, inFlight, handoffFrom, handoffTo, handoffEpoch>>

\* admit_domain: new owner `to` starts admitting `d` ONLY after the handoff has
\* been acknowledged.
AdmitDomain(d, to, e) ==
    /\ handoffState[d] = "acked"
    /\ handoffTo[d] = to
    /\ handoffEpoch[d] = e
    /\ owner[d] = to
    /\ inFlight[d] = {}
    /\ admitting' = [admitting EXCEPT ![d] = {to}]
    /\ handoffState' = [handoffState EXCEPT ![d] = "none"]
    /\ handoffFrom' = [handoffFrom EXCEPT ![d] = "none"]
    /\ handoffTo' = [handoffTo EXCEPT ![d] = "none"]
    /\ handoffEpoch' = [handoffEpoch EXCEPT ![d] = 0]
    /\ UNCHANGED <<mapEpoch, owner, inFlight>>

\* start_fetch: a Partition starts fetching a request for `d` only while it is
\* admitting `d`.
StartFetch(d, m) ==
    /\ m \in admitting[d]
    /\ m \notin inFlight[d]
    /\ inFlight' = [inFlight EXCEPT ![d] = @ \union {m}]
    /\ UNCHANGED <<mapEpoch, owner, admitting, handoffState, handoffFrom, handoffTo, handoffEpoch>>

\* finish_fetch: an in-flight request for `d` completes on `m`.
FinishFetch(d, m) ==
    /\ m \in inFlight[d]
    /\ inFlight' = [inFlight EXCEPT ![d] = @ \ {m}]
    /\ UNCHANGED <<mapEpoch, owner, admitting, handoffState, handoffFrom, handoffTo, handoffEpoch>>

\* reroute_forward: a forward routed under an older epoch to `from`, which no
\* longer owns `d`, is re-routed to `to = owner[d]` under `currentEpoch`.
RerouteForward(d, from, to, oldEpoch, currentEpoch) ==
    /\ oldEpoch \in 1..MaxEpoch
    /\ currentEpoch = mapEpoch
    /\ oldEpoch < currentEpoch
    /\ from \in Members
    /\ to \in Members
    /\ owner[d] = to
    /\ from /= to
    /\ UNCHANGED vars

Next ==
    \/ \E d \in Domains, m \in Members : AssignInitial(d, m)
    \/ \E e \in 2..MaxEpoch : PublishMap(e)
    \/ \E d \in Domains, from \in Members, to \in Members, e \in 2..MaxEpoch :
        \/ FenceDomain(d, from, to, e)
        \/ SendHandoff(d, from, to, e)
        \/ AckHandoff(d, from, to, e)
        \/ AdoptHandoff(d, from, to, e)
    \/ \E d \in Domains, m \in Members, e \in 2..MaxEpoch :
        \/ DrainInflight(d, m, e)
        \/ AdmitDomain(d, m, e)
    \/ \E d \in Domains, m \in Members :
        \/ StartFetch(d, m)
        \/ FinishFetch(d, m)
    \/ \E d \in Domains, from \in Members, to \in Members, oldE \in 1..MaxEpoch, curE \in 2..MaxEpoch :
        RerouteForward(d, from, to, oldE, curE)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ mapEpoch \in 1..MaxEpoch
    /\ owner \in [Domains -> Members \union {"none"}]
    /\ admitting \in [Domains -> SUBSET Members]
    /\ inFlight \in [Domains -> SUBSET Members]
    /\ handoffState \in [Domains -> HandoffStates]
    /\ handoffFrom \in [Domains -> Members \union {"none"}]
    /\ handoffTo \in [Domains -> Members \union {"none"}]
    /\ handoffEpoch \in [Domains -> 0..MaxEpoch]

\* At most one Partition admits any domain at any time.
SingleAdmitter ==
    \A d \in Domains : Cardinality(admitting[d]) <= 1

\* No domain is ever fetched by two Partitions at once (ADR 0054).
NoConcurrentFetch ==
    \A d \in Domains : Cardinality(inFlight[d]) <= 1

\* The new owner never admits a moving domain before its handoff is acknowledged.
NewOwnerAdmitsOnlyAfterAck ==
    \A d \in Domains :
        handoffState[d] \in {"fenced", "drained", "sent"} =>
            handoffTo[d] \notin admitting[d]

\* The old owner stops admitting and drains all in-flight fetches before
\* sending the handoff.
OldOwnerDrainedBeforeSend ==
    \A d \in Domains :
        handoffState[d] \in {"drained", "sent", "acked"} =>
            /\ handoffFrom[d] \notin admitting[d]
            /\ handoffFrom[d] \notin inFlight[d]
====
