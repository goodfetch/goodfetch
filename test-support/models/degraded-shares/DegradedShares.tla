---- MODULE DegradedShares ----
\* Politeness store outages: granted shares, stale reservations, bounded
\* degraded admission, Delegation expiry under outage, and atomic reconnect
\* permit registration (ADRs 0053, 0060 and 0142).
\*
\* Each traced action matches a transition `goodfetch-engine`'s
\* `politeness::store_backend` emits under `goodfetch::politeness`:
\* `acquire_delegation`, `renew_delegation`, `expire_delegation`, `heartbeat`,
\* `take_shared`, `give_back`, `cut_off`, `enter_degraded`, `take_degraded`,
\* `stop_degraded`, `reconnect`.
EXTENDS Naturals, Integers, FiniteSets

CONSTANTS
    Engines,        \* the engine IDs sharing one Politeness key
    Cap,            \* the key's concurrency cap (>= 1)
    MaxTime,        \* highest logical tick explored
    HeartbeatTtl,   \* heartbeat TTL (<= Permit lease)
    DelegationTtl,  \* Delegation TTL
    DegradedMax,    \* politeness.degraded_max window
    RequestTimeout  \* longest request timeout reported by any engine

VARIABLES
    now,                \* discrete logical time (0..MaxTime)
    delegationHolder,   \* engine holding Delegation or "none"
    delegationExpires,  \* tick at which current Delegation expires
    connected,          \* [Engines -> BOOLEAN]: reachable from the store
    registered,         \* [Engines -> BOOLEAN]: present in :engines
    lastHb,             \* [Engines -> 0..MaxTime]: last acked heartbeat tick
    grantedShare,       \* [Engines -> 0..Cap]: last store-computed share
    ackedShare,         \* [Engines -> 0..Cap]: share counted while shrinking
    storePermits,       \* [Engines -> 0..Cap]: permits registered in store
    localPermits,       \* [Engines -> 0..Cap]: requests in flight on engine
    phase,              \* [Engines -> Phases]: engine lifecycle phase
    admitAt             \* [Engines -> Nat]: tick degraded admission may start

vars == <<
    now, delegationHolder, delegationExpires, connected, registered,
    lastHb, grantedShare, ackedShare, storePermits, localPermits,
    phase, admitAt
>>

Phases == {"normal", "delegated", "draining", "degraded", "stopped"}

Max(a, b) == IF a >= b THEN a ELSE b
Min(a, b) == IF a <= b THEN a ELSE b

RECURSIVE SumSet(_, _)
SumSet(S, f) ==
    IF S = {} THEN 0
    ELSE LET x == CHOOSE y \in S : TRUE
         IN f[x] + SumSet(S \ {x}, f)

IsLive(e) == registered[e] /\ now < lastHb[e] + HeartbeatTtl
IsStale(e) == registered[e] /\ now >= lastHb[e] + HeartbeatTtl
DelegationLive == delegationHolder /= "none" /\ now < delegationExpires

EffectiveShare(e) ==
    IF ~registered[e] THEN 0 ELSE Max(grantedShare[e], ackedShare[e])

StaleReserved(e) ==
    IF IsStale(e) THEN Max(EffectiveShare(e), storePermits[e]) ELSE storePermits[e]

StoreEffectiveInFlight ==
    SumSet(Engines, [e \in Engines |-> StaleReserved(e)])

TotalLocalInFlight ==
    SumSet(Engines, localPermits)

TotalStorePermits ==
    SumSet(Engines, storePermits)

OtherEffectiveShares(e) ==
    SumSet(Engines \ {e}, [o \in Engines \ {e} |-> EffectiveShare(o)])

LiveCountWith(e) ==
    Cardinality({o \in Engines : o = e \/ IsLive(o)})

ComputeShare(e) ==
    LET fair == Cap \div LiveCountWith(e)
        other == OtherEffectiveShares(e)
        left == IF Cap > other THEN Cap - other ELSE 0
    IN Max(1, Min(fair, left))

Init ==
    /\ now = 0
    /\ delegationHolder = "none"
    /\ delegationExpires = 0
    /\ connected = [e \in Engines |-> TRUE]
    /\ registered = [e \in Engines |-> FALSE]
    /\ lastHb = [e \in Engines |-> 0]
    /\ grantedShare = [e \in Engines |-> 0]
    /\ ackedShare = [e \in Engines |-> 0]
    /\ storePermits = [e \in Engines |-> 0]
    /\ localPermits = [e \in Engines |-> 0]
    /\ phase = [e \in Engines |-> "normal"]
    /\ admitAt = [e \in Engines |-> 0]

\* Connected engine acquires a Delegation when no other engine holds
\* Delegation, permits, or an active/stale heartbeat record (ADR 0142).
AcquireDelegation(e) ==
    /\ connected[e]
    /\ phase[e] = "normal"
    /\ ~DelegationLive
    /\ TotalStorePermits = 0
    /\ \A o \in Engines \ {e} : ~registered[o]
    /\ delegationHolder' = e
    /\ delegationExpires' = now + DelegationTtl
    /\ registered' = [registered EXCEPT ![e] = TRUE]
    /\ lastHb' = [lastHb EXCEPT ![e] = now]
    /\ grantedShare' = [grantedShare EXCEPT ![e] = Cap]
    /\ ackedShare' = [ackedShare EXCEPT ![e] = Cap]
    /\ phase' = [phase EXCEPT ![e] = "delegated"]
    /\ UNCHANGED <<now, connected, storePermits, localPermits, admitAt>>

\* Connected Delegation holder renews its Delegation and heartbeat.
RenewDelegation(e) ==
    /\ connected[e]
    /\ phase[e] = "delegated"
    /\ delegationHolder = e
    /\ now < delegationExpires
    /\ delegationExpires' = now + DelegationTtl
    /\ lastHb' = [lastHb EXCEPT ![e] = now]
    /\ UNCHANGED <<now, delegationHolder, connected, registered, grantedShare,
                   ackedShare, storePermits, localPermits, phase, admitAt>>

\* Delegation holder admits a request in memory (connected OR cut off while
\* the Delegation has not yet expired, ADR 0142 / #241).
TakeDelegated(e) ==
    /\ phase[e] = "delegated"
    /\ delegationHolder = e
    /\ now < delegationExpires
    /\ localPermits[e] < Cap
    /\ localPermits' = [localPermits EXCEPT ![e] = @ + 1]
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   registered, lastHb, grantedShare, ackedShare,
                   storePermits, phase, admitAt>>

\* Delegation expires: if cut off, transitions to ADR 0053's degraded-shares
\* drain/stop; if connected, promotes in-flight permits to shared leases.
ExpireDelegation(e) ==
    /\ phase[e] = "delegated"
    /\ delegationHolder = e
    /\ now >= delegationExpires
    /\ delegationHolder' = "none"
    /\ IF ~connected[e] THEN
         /\ IF now >= lastHb[e] + DegradedMax THEN
              /\ phase' = [phase EXCEPT ![e] = "stopped"]
              /\ UNCHANGED admitAt
            ELSE
              /\ phase' = [phase EXCEPT ![e] = "draining"]
              /\ admitAt' = [admitAt EXCEPT ![e] = delegationExpires + RequestTimeout]
         /\ UNCHANGED storePermits
       ELSE
         /\ phase' = [phase EXCEPT ![e] = "normal"]
         /\ storePermits' = [storePermits EXCEPT ![e] = localPermits[e]]
         /\ UNCHANGED admitAt
    /\ UNCHANGED <<now, delegationExpires, connected, registered, lastHb,
                   grantedShare, ackedShare, localPermits>>

\* Connected engine heartbeats: acknowledges its previous granted share,
\* receives store-computed new share (shrinking shares stay counted at old
\* acked size until the next heartbeat acknowledges them, ADR 0053).
\* Another engine's live Delegation must be recalled before a second engine
\* joins `:engines`.
Heartbeat(e) ==
    /\ connected[e]
    /\ phase[e] = "normal"
    /\ ~DelegationLive
    /\ LET newShare == ComputeShare(e)
           oldAck == IF registered[e] THEN grantedShare[e] ELSE 0
           storedAck == IF newShare < oldAck THEN oldAck ELSE newShare
       IN /\ registered' = [registered EXCEPT ![e] = TRUE]
          /\ lastHb' = [lastHb EXCEPT ![e] = now]
          /\ grantedShare' = [grantedShare EXCEPT ![e] = newShare]
          /\ ackedShare' = [ackedShare EXCEPT ![e] = storedAck]
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   storePermits, localPermits, phase, admitAt>>

\* Connected engine takes a shared Permit if effective in-flight (active
\* permits + stale engines' reserved shares) is below Cap.
TakeShared(e) ==
    /\ connected[e]
    /\ phase[e] = "normal"
    /\ IsLive(e)
    /\ ~DelegationLive
    /\ StoreEffectiveInFlight < Cap
    /\ storePermits' = [storePermits EXCEPT ![e] = @ + 1]
    /\ localPermits' = [localPermits EXCEPT ![e] = @ + 1]
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   registered, lastHb, grantedShare, ackedShare, phase, admitAt>>

\* Engine returns an in-flight Permit (in store if connected, locally if cut off).
GiveBack(e) ==
    /\ localPermits[e] > 0
    /\ localPermits' = [localPermits EXCEPT ![e] = @ - 1]
    /\ IF connected[e] /\ storePermits[e] > 0 THEN
         storePermits' = [storePermits EXCEPT ![e] = @ - 1]
       ELSE
         UNCHANGED storePermits
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   registered, lastHb, grantedShare, ackedShare, phase, admitAt>>

\* Engine loses connectivity to the store. A Delegation holder keeps serving
\* until its Delegation expires; a shared engine begins waiting for leases +
\* longest request timeout.
CutOff(e) ==
    /\ connected[e]
    /\ registered[e]
    /\ IsLive(e)
    /\ connected' = [connected EXCEPT ![e] = FALSE]
    /\ IF phase[e] = "normal" THEN
         /\ phase' = [phase EXCEPT ![e] = "draining"]
         /\ admitAt' = [admitAt EXCEPT ![e] = lastHb[e] + HeartbeatTtl + RequestTimeout]
       ELSE
         UNCHANGED <<phase, admitAt>>
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, registered,
                   lastHb, grantedShare, ackedShare, storePermits, localPermits>>

\* Cut-off engine finishes waiting for leases/delegation expiry + longest
\* request timeout while within politeness.degraded_max.
EnterDegraded(e) ==
    /\ ~connected[e]
    /\ phase[e] = "draining"
    /\ now >= admitAt[e]
    /\ now < lastHb[e] + DegradedMax
    /\ phase' = [phase EXCEPT ![e] = "degraded"]
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   registered, lastHb, grantedShare, ackedShare,
                   storePermits, localPermits, admitAt>>

\* Cut-off engine admits a request within its last granted share before
\* politeness.degraded_max elapses.
TakeDegraded(e) ==
    /\ ~connected[e]
    /\ phase[e] = "degraded"
    /\ now < lastHb[e] + DegradedMax
    /\ localPermits[e] < grantedShare[e]
    /\ localPermits' = [localPermits EXCEPT ![e] = @ + 1]
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   registered, lastHb, grantedShare, ackedShare,
                   storePermits, phase, admitAt>>

\* Cut-off engine reaches politeness.degraded_max and admits nothing further
\* to shared keys until it reconnects; its runs stay live and resumable.
StopDegraded(e) ==
    /\ ~connected[e]
    /\ phase[e] \in {"draining", "degraded"}
    /\ now >= lastHb[e] + DegradedMax
    /\ phase' = [phase EXCEPT ![e] = "stopped"]
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   registered, lastHb, grantedShare, ackedShare,
                   storePermits, localPermits, admitAt>>

\* Store frees a stale engine's reservation once politeness.degraded_max plus
\* the longest reported request timeout has passed.
FreeReservation(e) ==
    /\ IsStale(e)
    /\ now >= lastHb[e] + DegradedMax + RequestTimeout
    /\ storePermits[e] = 0
    /\ localPermits[e] = 0
    /\ registered' = [registered EXCEPT ![e] = FALSE]
    /\ grantedShare' = [grantedShare EXCEPT ![e] = 0]
    /\ ackedShare' = [ackedShare EXCEPT ![e] = 0]
    /\ UNCHANGED <<now, delegationHolder, delegationExpires, connected,
                   lastHb, storePermits, localPermits, phase, admitAt>>

\* Cut-off engine reconnects: atomically registers all local in-flight Permits
\* in the store BEFORE updating its heartbeat / freeing its stale reservation.
Reconnect(e) ==
    /\ ~connected[e]
    /\ connected' = [connected EXCEPT ![e] = TRUE]
    /\ IF phase[e] = "delegated" /\ delegationHolder = e /\ now < delegationExpires THEN
         /\ delegationExpires' = now + DelegationTtl
         /\ lastHb' = [lastHb EXCEPT ![e] = now]
         /\ UNCHANGED <<delegationHolder, registered, grantedShare, ackedShare,
                        storePermits, phase, admitAt>>
       ELSE
         LET newShare == ComputeShare(e)
             oldAck == IF registered[e] THEN grantedShare[e] ELSE 0
             storedAck == IF newShare < oldAck THEN oldAck ELSE newShare
         IN /\ storePermits' = [storePermits EXCEPT ![e] = localPermits[e]]
            /\ registered' = [registered EXCEPT ![e] = TRUE]
            /\ lastHb' = [lastHb EXCEPT ![e] = now]
            /\ grantedShare' = [grantedShare EXCEPT ![e] = newShare]
            /\ ackedShare' = [ackedShare EXCEPT ![e] = storedAck]
            /\ phase' = [phase EXCEPT ![e] = "normal"]
            /\ admitAt' = [admitAt EXCEPT ![e] = 0]
            /\ delegationHolder' = IF delegationHolder = e THEN "none" ELSE delegationHolder
            /\ UNCHANGED delegationExpires
    /\ UNCHANGED <<now, localPermits>>

\* Logical clock advances by 1 tick:
\* - Requests admitted before DegradedMax finish within RequestTimeout (so no
\*   stopped engine still has in-flight requests past DegradedMax + RequestTimeout).
\* - Requests admitted by connected engines before a cut-off engine went stale
\*   finish within RequestTimeout (so at admitAt, connected engines hold at most
\*   Cap - stale reservations).
\* - Expired leases of cut-off engines are pruned in the store once stale.
Tick ==
    /\ now < MaxTime
    /\ LET nextNow == now + 1
       IN /\ \A e \in Engines :
               (registered[e] /\ nextNow >= lastHb[e] + DegradedMax + RequestTimeout)
                 => localPermits[e] = 0
          /\ \A e \in Engines :
               (~connected[e] /\ phase[e] = "draining" /\ nextNow >= lastHb[e] + HeartbeatTtl)
                 => localPermits[e] = 0
          /\ \A e \in Engines :
               (~connected[e] /\ phase[e] = "draining" /\ nextNow >= admitAt[e])
                 => SumSet(Engines \ {e}, [o \in Engines \ {e} |-> IF connected[o] THEN localPermits[o] ELSE 0])
                    + EffectiveShare(e) <= Cap + Cardinality({x \in Engines : EffectiveShare(x) = 1})
          /\ now' = nextNow
          /\ storePermits' = [e \in Engines |->
               IF ~connected[e] /\ registered[e] /\ nextNow >= lastHb[e] + HeartbeatTtl
               THEN 0
               ELSE storePermits[e]]
    /\ UNCHANGED <<delegationHolder, delegationExpires, connected, registered,
                   lastHb, grantedShare, ackedShare, localPermits, phase, admitAt>>

Next ==
    \/ Tick
    \/ \E e \in Engines :
        \/ AcquireDelegation(e)
        \/ RenewDelegation(e)
        \/ TakeDelegated(e)
        \/ ExpireDelegation(e)
        \/ Heartbeat(e)
        \/ TakeShared(e)
        \/ GiveBack(e)
        \/ CutOff(e)
        \/ EnterDegraded(e)
        \/ TakeDegraded(e)
        \/ StopDegraded(e)
        \/ FreeReservation(e)
        \/ Reconnect(e)

Spec == Init /\ [][Next]_vars

\* --- Safety invariants (ADRs 0053 and 0142) ---

TypeOK ==
    /\ now \in 0..MaxTime
    /\ delegationHolder \in Engines \cup {"none"}
    /\ connected \in [Engines -> BOOLEAN]
    /\ registered \in [Engines -> BOOLEAN]
    /\ phase \in [Engines -> Phases]
    /\ \A e \in Engines :
        /\ grantedShare[e] \in 0..Cap
        /\ ackedShare[e] \in 0..Cap
        /\ storePermits[e] \in 0..Cap
        /\ localPermits[e] \in 0..Cap

\* Store-granted shares never sum to more than Cap except by the minimum of 1
\* per engine whose share would otherwise be 0.
SharesBounded ==
    SumSet(Engines, [e \in Engines |-> EffectiveShare(e)])
      <= Cap + Cardinality({e \in Engines : registered[e] /\ EffectiveShare(e) = 1})

\* Combined requests in flight across all engines never exceed Cap plus 1 per
\* engine whose share hit the minimum of 1 (ADR 0053).
InFlightBounded ==
    TotalLocalInFlight
      <= Cap + Cardinality({e \in Engines : registered[e] /\ EffectiveShare(e) = 1})

\* While a Delegation is live, no other engine holds permits in the store.
DelegationExclusive ==
    DelegationLive =>
      \A o \in Engines \ {delegationHolder} : storePermits[o] = 0

\* Every connected engine in normal phase has all its local in-flight requests
\* registered in the store (no unregistered permits after reconnect).
ReconnectRegistersFirst ==
    \A e \in Engines :
      (connected[e] /\ phase[e] = "normal") => storePermits[e] >= localPermits[e]

Safety ==
    /\ TypeOK
    /\ SharesBounded
    /\ InFlightBounded
    /\ DelegationExclusive
    /\ ReconnectRegistersFirst
====
