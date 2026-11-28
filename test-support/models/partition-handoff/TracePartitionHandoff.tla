---- MODULE TracePartitionHandoff ----
\* Validates a `goodfetch::partition` trace against PartitionHandoff (ADRs 0054
\* and 0060), from the TraceValidation template.
EXTENDS PartitionHandoff, TraceValidation

MaxOf(S) == CHOOSE m \in S : \A n \in S : n <= m

TraceMembers ==
    ValuesOf("member") \union ValuesOf("from") \union ValuesOf("to")

TraceDomains == ValuesOf("domain")

TraceEpochs ==
    ValuesOf("map_epoch") \union ValuesOf("old_epoch")

TraceMaxEpoch ==
    IF TraceEpochs = {} THEN 1 ELSE MaxOf(TraceEpochs)

TraceInit == Init /\ l = 1

AssignInitialLine ==
    /\ IsEvent("assign_initial")
    /\ AssignInitial(Line.domain, Line.member)
    /\ Consume

PublishMapLine ==
    /\ IsEvent("publish_map")
    /\ PublishMap(Line.map_epoch)
    /\ Consume

FenceDomainLine ==
    /\ IsEvent("fence_domain")
    /\ FenceDomain(Line.domain, Line.from, Line.to, Line.map_epoch)
    /\ Consume

DrainInflightLine ==
    /\ IsEvent("drain_inflight")
    /\ DrainInflight(Line.domain, Line.member, Line.map_epoch)
    /\ Consume

SendHandoffLine ==
    /\ IsEvent("send_handoff")
    /\ SendHandoff(Line.domain, Line.from, Line.to, Line.map_epoch)
    /\ Consume

AckHandoffLine ==
    /\ IsEvent("ack_handoff")
    /\ AckHandoff(Line.domain, Line.from, Line.to, Line.map_epoch)
    /\ Consume

AdoptHandoffLine ==
    /\ IsEvent("adopt_handoff")
    /\ AdoptHandoff(Line.domain, Line.from, Line.to, Line.map_epoch)
    /\ Consume

AdmitDomainLine ==
    /\ IsEvent("admit_domain")
    /\ AdmitDomain(Line.domain, Line.member, Line.map_epoch)
    /\ Consume

StartFetchLine ==
    /\ IsEvent("start_fetch")
    /\ StartFetch(Line.domain, Line.member)
    /\ Consume

FinishFetchLine ==
    /\ IsEvent("finish_fetch")
    /\ FinishFetch(Line.domain, Line.member)
    /\ Consume

RerouteForwardLine ==
    /\ IsEvent("reroute_forward")
    /\ RerouteForward(Line.domain, Line.from, Line.to, Line.old_epoch, Line.map_epoch)
    /\ Consume

TraceNext ==
    \/ AssignInitialLine
    \/ PublishMapLine
    \/ FenceDomainLine
    \/ DrainInflightLine
    \/ SendHandoffLine
    \/ AckHandoffLine
    \/ AdoptHandoffLine
    \/ AdmitDomainLine
    \/ StartFetchLine
    \/ FinishFetchLine
    \/ RerouteForwardLine
====
