//! Every (state, event) pair of the Effect's table, each a named case
//! (ADR 0128): a late event's allowed row changes nothing.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use rstest::rstest;

use super::EffectEvent::{
    Declare, DeclareRepeatable, DidNotHappen, Fail, Happened, Interrupt, LeaveUncertain, Return,
    Send,
};
use super::EffectState::{
    Absent, Done, Open, OpenRepeatable, Reconciled, Sent, SentRepeatable, Uncertain,
};
use super::{EffectEvent, EffectState, Next, apply};
use crate::transition::Refusal;

/// What a pair answers: the next state or nothing, or who is refused.
type Expected = Result<Next, Refusal>;

const UNCHANGED: Expected = Ok(Next::Unchanged);
const DROPPED: Expected = Ok(Next::Dropped);
/// A declaration that meets a body still running: only a bug reaches it.
const BUG: Expected = Err(Refusal::Internal);
/// A declaration of an uncertain key raises `effect_uncertain`.
const UNCERTAIN: Expected = Err(Refusal::Program("effect_uncertain"));

#[expect(
    clippy::unnecessary_wraps,
    reason = "a case's expected answer is an `Expected`, whichever row it is"
)]
const fn to(state: EffectState) -> Expected {
    Ok(Next::To(state))
}

/// Covers: error:effect_uncertain
#[rstest]
#[case::absent_declare(Absent, Declare, to(Open))]
#[case::absent_declare_repeatable(Absent, DeclareRepeatable, to(OpenRepeatable))]
#[case::absent_send(Absent, Send, UNCHANGED)]
#[case::absent_return(Absent, Return, UNCHANGED)]
#[case::absent_fail(Absent, Fail, UNCHANGED)]
#[case::absent_interrupt(Absent, Interrupt, UNCHANGED)]
#[case::absent_happened(Absent, Happened, UNCHANGED)]
#[case::absent_did_not_happen(Absent, DidNotHappen, UNCHANGED)]
#[case::absent_leave_uncertain(Absent, LeaveUncertain, UNCHANGED)]
#[case::open_declare(Open, Declare, BUG)]
#[case::open_declare_repeatable(Open, DeclareRepeatable, BUG)]
#[case::open_send(Open, Send, to(Sent))]
#[case::open_return(Open, Return, to(Done))]
#[case::open_fail(Open, Fail, DROPPED)]
#[case::open_interrupt(Open, Interrupt, DROPPED)]
#[case::open_happened(Open, Happened, UNCHANGED)]
#[case::open_did_not_happen(Open, DidNotHappen, UNCHANGED)]
#[case::open_leave_uncertain(Open, LeaveUncertain, UNCHANGED)]
#[case::open_repeatable_declare(OpenRepeatable, Declare, BUG)]
#[case::open_repeatable_declare_repeatable(OpenRepeatable, DeclareRepeatable, BUG)]
#[case::open_repeatable_send(OpenRepeatable, Send, to(SentRepeatable))]
#[case::open_repeatable_return(OpenRepeatable, Return, to(Done))]
#[case::open_repeatable_fail(OpenRepeatable, Fail, DROPPED)]
#[case::open_repeatable_interrupt(OpenRepeatable, Interrupt, DROPPED)]
#[case::open_repeatable_happened(OpenRepeatable, Happened, UNCHANGED)]
#[case::open_repeatable_did_not_happen(OpenRepeatable, DidNotHappen, UNCHANGED)]
#[case::open_repeatable_leave_uncertain(OpenRepeatable, LeaveUncertain, UNCHANGED)]
#[case::sent_declare(Sent, Declare, BUG)]
#[case::sent_declare_repeatable(Sent, DeclareRepeatable, BUG)]
#[case::sent_send(Sent, Send, UNCHANGED)]
#[case::sent_return(Sent, Return, to(Done))]
#[case::sent_fail(Sent, Fail, to(Uncertain))]
#[case::sent_interrupt(Sent, Interrupt, to(Uncertain))]
#[case::sent_happened(Sent, Happened, UNCHANGED)]
#[case::sent_did_not_happen(Sent, DidNotHappen, UNCHANGED)]
#[case::sent_leave_uncertain(Sent, LeaveUncertain, UNCHANGED)]
#[case::sent_repeatable_declare(SentRepeatable, Declare, BUG)]
#[case::sent_repeatable_declare_repeatable(SentRepeatable, DeclareRepeatable, BUG)]
#[case::sent_repeatable_send(SentRepeatable, Send, UNCHANGED)]
#[case::sent_repeatable_return(SentRepeatable, Return, to(Done))]
#[case::sent_repeatable_fail(SentRepeatable, Fail, DROPPED)]
#[case::sent_repeatable_interrupt(SentRepeatable, Interrupt, DROPPED)]
#[case::sent_repeatable_happened(SentRepeatable, Happened, UNCHANGED)]
#[case::sent_repeatable_did_not_happen(SentRepeatable, DidNotHappen, UNCHANGED)]
#[case::sent_repeatable_leave_uncertain(SentRepeatable, LeaveUncertain, UNCHANGED)]
#[case::done_declare(Done, Declare, UNCHANGED)]
#[case::done_declare_repeatable(Done, DeclareRepeatable, UNCHANGED)]
#[case::done_send(Done, Send, UNCHANGED)]
#[case::done_return(Done, Return, UNCHANGED)]
#[case::done_fail(Done, Fail, UNCHANGED)]
#[case::done_interrupt(Done, Interrupt, UNCHANGED)]
#[case::done_happened(Done, Happened, UNCHANGED)]
#[case::done_did_not_happen(Done, DidNotHappen, UNCHANGED)]
#[case::done_leave_uncertain(Done, LeaveUncertain, UNCHANGED)]
#[case::uncertain_declare(Uncertain, Declare, UNCERTAIN)]
#[case::uncertain_declare_repeatable(Uncertain, DeclareRepeatable, UNCERTAIN)]
#[case::uncertain_send(Uncertain, Send, UNCHANGED)]
#[case::uncertain_return(Uncertain, Return, UNCHANGED)]
#[case::uncertain_fail(Uncertain, Fail, UNCHANGED)]
#[case::uncertain_interrupt(Uncertain, Interrupt, UNCHANGED)]
#[case::uncertain_happened(Uncertain, Happened, to(Reconciled))]
#[case::uncertain_did_not_happen(Uncertain, DidNotHappen, DROPPED)]
#[case::uncertain_leave_uncertain(Uncertain, LeaveUncertain, UNCHANGED)]
#[case::reconciled_declare(Reconciled, Declare, UNCHANGED)]
#[case::reconciled_declare_repeatable(Reconciled, DeclareRepeatable, UNCHANGED)]
#[case::reconciled_send(Reconciled, Send, UNCHANGED)]
#[case::reconciled_return(Reconciled, Return, UNCHANGED)]
#[case::reconciled_fail(Reconciled, Fail, UNCHANGED)]
#[case::reconciled_interrupt(Reconciled, Interrupt, UNCHANGED)]
#[case::reconciled_happened(Reconciled, Happened, UNCHANGED)]
#[case::reconciled_did_not_happen(Reconciled, DidNotHappen, UNCHANGED)]
#[case::reconciled_leave_uncertain(Reconciled, LeaveUncertain, UNCHANGED)]
fn each_pair_of_the_effects_table(
    #[case] state: EffectState,
    #[case] event: EffectEvent,
    #[case] expected: Expected,
) {
    let answered = apply(state, event).map_err(|refused| {
        assert_eq!((refused.state, refused.event), (state, event));
        refused.refusal
    });
    assert_eq!(answered, expected);
}

/// The journal reads back each state it writes, the repeatable ones by
/// their flag.
#[rstest]
fn the_journal_reads_back_each_state_it_writes(
    #[values(
        Open,
        OpenRepeatable,
        Sent,
        SentRepeatable,
        Done,
        Uncertain,
        Reconciled
    )]
    state: EffectState,
) {
    let repeatable = matches!(state, OpenRepeatable | SentRepeatable);
    let stored = state.stored().unwrap();
    assert_eq!(EffectState::of(stored, repeatable), Some(state));
}
