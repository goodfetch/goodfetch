---- MODULE TraceDegradedShares ----
\* Validates a `goodfetch::politeness` trace against DegradedShares (ADRs 0053,
\* 0060 and 0142), from the TraceValidation template.
\* Logical time ticks (`Tick`) and reservation frees (`FreeReservation`) are
\* untraced steps TLC may take between traced transitions so timestamps advance
\* to match Delegation expiry, drain waits, and `politeness.degraded_max`.
EXTENDS DegradedShares, TraceValidation

TraceEngines == ValuesOf("engine")

TraceCap == 2
TraceMaxTime == 10
TraceHeartbeatTtl == 1
TraceDelegationTtl == 2
TraceDegradedMax == 4
TraceRequestTimeout == 1

TraceInit == Init /\ l = 1

AcquireDelegationLine ==
    /\ IsEvent("acquire_delegation")
    /\ AcquireDelegation(Line.engine)
    /\ Consume

RenewDelegationLine ==
    /\ IsEvent("renew_delegation")
    /\ RenewDelegation(Line.engine)
    /\ Consume

ExpireDelegationLine ==
    /\ IsEvent("expire_delegation")
    /\ ExpireDelegation(Line.engine)
    /\ Consume

HeartbeatLine ==
    /\ IsEvent("heartbeat")
    /\ IF phase[Line.engine] = "delegated"
       THEN UNCHANGED vars
       ELSE /\ Heartbeat(Line.engine)
            /\ grantedShare'[Line.engine] = Line.share
    /\ Consume

TakeSharedLine ==
    /\ IsEvent("take_shared")
    /\ TakeShared(Line.engine)
    /\ Consume

GiveBackLine ==
    /\ IsEvent("give_back")
    /\ GiveBack(Line.engine)
    /\ Consume

CutOffLine ==
    /\ IsEvent("cut_off")
    /\ CutOff(Line.engine)
    /\ Consume

EnterDegradedLine ==
    /\ IsEvent("enter_degraded")
    /\ EnterDegraded(Line.engine)
    /\ grantedShare[Line.engine] = Line.share
    /\ Consume

TakeDegradedLine ==
    /\ IsEvent("take_degraded")
    /\ TakeDegraded(Line.engine)
    /\ localPermits'[Line.engine] = Line.in_flight
    /\ Consume

StopDegradedLine ==
    /\ IsEvent("stop_degraded")
    /\ StopDegraded(Line.engine)
    /\ Consume

ReconnectLine ==
    /\ IsEvent("reconnect")
    /\ Reconnect(Line.engine)
    /\ storePermits'[Line.engine] = Line.registered
    /\ Consume

Untraced ==
    /\ \/ Tick
       \/ \E e \in Engines : TakeDelegated(e) \/ FreeReservation(e)
    /\ UNCHANGED l

TraceNext ==
    \/ AcquireDelegationLine
    \/ RenewDelegationLine
    \/ ExpireDelegationLine
    \/ HeartbeatLine
    \/ TakeSharedLine
    \/ GiveBackLine
    \/ CutOffLine
    \/ EnterDegradedLine
    \/ TakeDegradedLine
    \/ StopDegradedLine
    \/ ReconnectLine
    \/ Untraced
====
