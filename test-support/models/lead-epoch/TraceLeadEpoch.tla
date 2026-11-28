---- MODULE TraceLeadEpoch ----
\* Validates a `goodfetch::group` trace against LeadEpoch (ADR 0060), from the
\* TraceValidation template. Starting or restarting a Lead Daemon is untraced,
\* so `Untraced` supplies the matching `StartLead` step when the next event
\* requires it.
EXTENDS LeadEpoch, TraceValidation

MaxOf(S) == CHOOSE m \in S : \A n \in S : n <= m

TraceMembers ==
    ValuesOf("member") \union ValuesOf("peer") \union ValuesOf("target")

TraceEpochs ==
    ValuesOf("lead_epoch") \union ValuesOf("highest_lead_epoch")

TraceMaxEpoch == MaxOf(TraceEpochs)

LeadTransitions ==
    {"ack", "confirm_majority", "confirm_operator", "fence", "add_member", "remove_member"}

TraceLeadOf(e) ==
    {Trace[j].member :
        j \in {k \in 1..Len(Trace) :
            Trace[k].transition \in LeadTransitions /\ Trace[k].lead_epoch = e}}

TraceInit == Init /\ l = 1

RaiseLine ==
    /\ IsEvent("raise")
    /\ IF "member" \in DOMAIN Line
       THEN /\ highest[Line.member] = Line.highest_lead_epoch
            /\ Raise(Line.member, Line.lead_epoch)
       ELSE \E p \in Members :
            /\ highest[p] = Line.highest_lead_epoch
            /\ Raise(p, Line.lead_epoch)
    /\ Consume

RefuseStaleLine ==
    /\ IsEvent("refuse_stale")
    /\ IF "member" \in DOMAIN Line
       THEN /\ highest[Line.member] = Line.highest_lead_epoch
            /\ RefuseStale(Line.member, Line.lead_epoch)
       ELSE \E p \in Members :
            /\ highest[p] = Line.highest_lead_epoch
            /\ RefuseStale(p, Line.lead_epoch)
    /\ Consume

AckLine ==
    /\ IsEvent("ack")
    /\ leadEpoch[Line.member] = Line.lead_epoch
    /\ Ack(Line.member, Line.peer)
    /\ Consume

ConfirmMajorityLine ==
    /\ IsEvent("confirm_majority")
    /\ leadEpoch[Line.member] = Line.lead_epoch
    /\ ConfirmMajority(Line.member)
    /\ Consume

ConfirmOperatorLine ==
    /\ IsEvent("confirm_operator")
    /\ leadEpoch[Line.member] = Line.lead_epoch
    /\ ConfirmOperator(Line.member)
    /\ Consume

FenceLine ==
    /\ IsEvent("fence")
    /\ leadEpoch[Line.member] = Line.lead_epoch
    /\ Fence(Line.member, Line.highest_lead_epoch)
    /\ Consume

AddMemberLine ==
    /\ IsEvent("add_member")
    /\ leadEpoch[Line.member] = Line.lead_epoch
    /\ AddMember(Line.member, Line.target)
    /\ Consume

RemoveMemberLine ==
    /\ IsEvent("remove_member")
    /\ leadEpoch[Line.member] = Line.lead_epoch
    /\ RemoveMember(Line.member, Line.target)
    /\ Consume

\* Starting or restarting a Lead Daemon is untraced; supply it only when the
\* next trace line needs a Lead Daemon configured for that epoch (or for an
\* earlier epoch whose Lead Daemon appears later in the trace before a raise
\* advances its `highest`).
Untraced ==
    /\ l <= Len(Trace)
    /\ \/ /\ Line.transition = "raise"
          /\ \E e \in 1..Line.lead_epoch :
              /\ assigned[e] = {}
              /\ LET candidates ==
                     IF TraceLeadOf(e) /= {}
                     THEN TraceLeadOf(e)
                     ELSE IF e = Line.lead_epoch THEN Members ELSE {}
                 IN \E m \in candidates, S \in SUBSET Members :
                     StartLead(m, e, S)
       \/ /\ Line.transition = "refuse_stale"
          /\ \E e \in 1..Line.highest_lead_epoch :
              /\ assigned[e] = {}
              /\ LET candidates ==
                     IF TraceLeadOf(e) /= {}
                     THEN TraceLeadOf(e)
                     ELSE IF e = Line.highest_lead_epoch THEN Members ELSE {}
                 IN \E m \in candidates, S \in SUBSET Members :
                     StartLead(m, e, S)
       \/ /\ Line.transition = "confirm_majority"
          /\ (role[Line.member] /= "unconfirmed" \/ leadEpoch[Line.member] /= Line.lead_epoch)
          /\ StartLead(Line.member, Line.lead_epoch, {Line.member})
       \/ /\ Line.transition = "ack"
          /\ (role[Line.member] /= "unconfirmed" \/ leadEpoch[Line.member] /= Line.lead_epoch)
          /\ \E S \in SUBSET Members :
              /\ Line.peer \in S
              /\ StartLead(Line.member, Line.lead_epoch, S)
       \/ /\ Line.transition = "confirm_operator"
          /\ (role[Line.member] /= "unconfirmed" \/ leadEpoch[Line.member] /= Line.lead_epoch)
          /\ \E S \in SUBSET Members :
              StartLead(Line.member, Line.lead_epoch, S)
       \/ /\ Line.transition = "fence"
          /\ (role[Line.member] \notin {"unconfirmed", "confirmed"} \/ leadEpoch[Line.member] /= Line.lead_epoch)
          /\ \E S \in SUBSET Members :
              StartLead(Line.member, Line.lead_epoch, S)
    /\ UNCHANGED l

TraceNext ==
    \/ RaiseLine
    \/ RefuseStaleLine
    \/ AckLine
    \/ ConfirmMajorityLine
    \/ ConfirmOperatorLine
    \/ FenceLine
    \/ AddMemberLine
    \/ RemoveMemberLine
    \/ Untraced
====
