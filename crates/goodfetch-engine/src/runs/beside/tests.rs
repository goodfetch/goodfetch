//! The model-based test of the calls that wait beside the Runs' registry
//! (ADR 0145, issue 2258; TESTING.md, Model-based tests): starts, resumes,
//! stops, ends, discards and a drain arrive in any order, while the disk
//! work of a start, a resume or an ended Run's report ends in any order,
//! and the loop takes each through [`Queues`] as the registry's loop does.
//! Every answer is the one the same calls give one at a time in the order
//! they came; no Run has the disk work of two calls at once, so one start
//! runs per Run ID; a stop, an end or a discard of a Run whose start is in
//! flight waits for it; a drain waits for every start in flight; and no
//! call waits once no disk work is in flight. 16 cases in the Quick suite
//! and 256 in the Pre-merge suite (`proptest_config`).

use std::collections::BTreeMap;

use goodfetch_protocol::run::RunId;
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};

use super::{Order, Queues, Reach};

/// A call on the registry.
#[derive(Clone, Debug)]
enum Call {
    /// A start under the Run ID given, or one the registry generates.
    Start(Option<RunId>),
    Resume(RunId),
    Stop(RunId),
    End(RunId),
    Discard(RunId),
    Drain,
}

impl Call {
    /// The Run the call names.
    fn run_id(&self) -> Option<&RunId> {
        match self {
            Self::Start(run_id) => run_id.as_ref(),
            Self::Resume(run_id)
            | Self::Stop(run_id)
            | Self::End(run_id)
            | Self::Discard(run_id) => Some(run_id),
            Self::Drain => None,
        }
    }

    /// Which calls it waits behind, as `Command::reach` says of the
    /// registry's.
    fn reach(&self) -> Reach {
        match self {
            Self::Start(None) => Reach::InOrder,
            Self::Drain => Reach::Every,
            named => Reach::Run(
                named
                    .run_id()
                    .expect("invariant: every other call names its Run")
                    .clone(),
            ),
        }
    }
}

/// What the registry answers.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Answer {
    New(RunId),
    Resumed,
    Live,
    Ended,
    Stopped,
    Report,
    NotFound,
    InUse,
    Discarded,
    Draining,
    Drained(Vec<RunId>),
    Nothing,
}

/// Where a Run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Live,
    Ended,
}

/// What the registry keeps: its Runs, whether it drains, and how many Run
/// IDs it generated.
#[derive(Clone, Debug, Default)]
struct Registry {
    runs: BTreeMap<RunId, Held>,
    draining: bool,
    generated: u32,
}

impl Registry {
    fn live(&self, run_id: &RunId) -> bool {
        self.runs.get(run_id) == Some(&Held::Live)
    }

    /// The next Run ID the registry generates.
    fn generate(&mut self) -> RunId {
        self.generated += 1;
        RunId::parse(&format!("g{}", self.generated)).unwrap()
    }

    /// Whether the loop answers `call` without disk work, as the
    /// registry's does: a drain, an end, a discard, the calls while it
    /// drains, and those on a live Run.
    fn answers_now(&self, call: &Call) -> bool {
        match call {
            Call::Drain | Call::End(_) | Call::Discard(_) => true,
            Call::Start(_) | Call::Resume(_) if self.draining => true,
            Call::Start(None) => false,
            Call::Start(Some(run_id)) | Call::Resume(run_id) | Call::Stop(run_id) => {
                self.live(run_id)
            }
        }
    }

    /// What `call` answers and changes, taken alone.
    fn decide(&mut self, call: &Call) -> Answer {
        match call {
            Call::Start(_) | Call::Resume(_) if self.draining => Answer::Draining,
            Call::Start(None) => {
                let run_id = self.generate();
                self.decide(&Call::Start(Some(run_id)))
            }
            Call::Start(Some(run_id)) => match self.runs.get(run_id) {
                Some(Held::Live) => Answer::Live,
                Some(Held::Ended) => Answer::Ended,
                None => {
                    self.runs.insert(run_id.clone(), Held::Live);
                    Answer::New(run_id.clone())
                }
            },
            Call::Resume(run_id) => match self.runs.get(run_id) {
                Some(Held::Live) => Answer::Live,
                Some(Held::Ended) => {
                    self.runs.insert(run_id.clone(), Held::Live);
                    Answer::Resumed
                }
                None => Answer::NotFound,
            },
            Call::Stop(run_id) => match self.runs.get(run_id) {
                Some(Held::Live) => {
                    self.runs.insert(run_id.clone(), Held::Ended);
                    Answer::Stopped
                }
                Some(Held::Ended) => Answer::Report,
                None => Answer::NotFound,
            },
            Call::End(run_id) => {
                if self.live(run_id) {
                    self.runs.insert(run_id.clone(), Held::Ended);
                }
                Answer::Nothing
            }
            Call::Discard(run_id) => match self.runs.get(run_id) {
                Some(Held::Live) => Answer::InUse,
                Some(Held::Ended) => {
                    self.runs.remove(run_id);
                    Answer::Discarded
                }
                None => Answer::NotFound,
            },
            Call::Drain => {
                self.draining = true;
                let mut stopped = Vec::new();
                for (run_id, held) in &mut self.runs {
                    if *held == Held::Live {
                        *held = Held::Ended;
                        stopped.push(run_id.clone());
                    }
                }
                Answer::Drained(stopped)
            }
        }
    }
}

/// What the reference model knows: the answers of the calls so far, each
/// taken alone in the order they came.
#[derive(Clone, Debug, Default)]
struct Reference {
    registry: Registry,
    answers: Vec<Answer>,
}

#[derive(Clone, Debug)]
enum Transition {
    /// A call arrives.
    Send(Call),
    /// The disk work in flight `index` modulo how many are ends, if any.
    Finish(usize),
}

fn run_id() -> impl Strategy<Value = RunId> {
    prop_oneof![Just("a"), Just("b"), Just("c")].prop_map(|run_id| RunId::parse(run_id).unwrap())
}

fn call() -> impl Strategy<Value = Call> {
    prop_oneof![
        3 => proptest::option::weighted(0.8, run_id()).prop_map(Call::Start),
        2 => run_id().prop_map(Call::Resume),
        2 => run_id().prop_map(Call::Stop),
        1 => run_id().prop_map(Call::End),
        1 => run_id().prop_map(Call::Discard),
        1 => Just(Call::Drain),
    ]
}

impl ReferenceStateMachine for Reference {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        Just(Self::default()).boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            3 => call().prop_map(Transition::Send),
            2 => (0_usize..8).prop_map(Transition::Finish),
        ]
        .boxed()
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        if let Transition::Send(call) = transition {
            let answer = state.registry.decide(call);
            state.answers.push(answer);
        }
        state
    }
}

/// The loop under test: its queues, the registry it decides on, and the
/// disk work in flight beside it, each with its Run, its call and the
/// place of its answer.
struct Subject {
    registry: Registry,
    queues: Queues<(usize, Call)>,
    in_flight: Vec<(RunId, usize, Call)>,
    answers: BTreeMap<usize, Answer>,
    sent: usize,
}

impl Subject {
    /// Whether the disk work of a call on `run_id` is in flight.
    fn busy(&self, run_id: &RunId) -> bool {
        self.in_flight.iter().any(|(busy, ..)| busy == run_id)
    }

    /// Takes the call `index`, from `order`, then every call ready, as the
    /// registry's loop does before it reads its mailbox again.
    fn take(&mut self, index: usize, call: Call, order: Order) {
        let mut next = Some(((index, call), order));
        while let Some(((index, call), order)) = next {
            let reach = call.reach();
            if let Some((index, call)) = self.queues.unless_waiting((index, call), reach, order) {
                self.answer(index, call);
            }
            next = self.queues.next();
        }
    }

    /// Answers the call `index` now, or sends its disk work beside.
    fn answer(&mut self, index: usize, call: Call) {
        if let Some(run_id) = call.run_id() {
            assert!(
                !self.busy(run_id),
                "{call:?} was taken while a call on its Run was in flight"
            );
        }
        if matches!(call, Call::Drain) {
            assert!(
                self.in_flight.is_empty(),
                "a drain was taken with {:?} in flight",
                self.in_flight
            );
        }
        if self.registry.answers_now(&call) {
            let answer = self.registry.decide(&call);
            self.answers.insert(index, answer);
            return;
        }
        let call = match call {
            Call::Start(None) => Call::Start(Some(self.registry.generate())),
            named => named,
        };
        let run_id = call
            .run_id()
            .expect("invariant: disk work is of one Run")
            .clone();
        assert!(!self.busy(&run_id), "two starts of {run_id} at once");
        self.queues.busy(&run_id);
        self.in_flight.push((run_id, index, call));
    }

    /// Ends the disk work `which` modulo how many are in flight: its
    /// decision is taken, and the calls on its Run that waited go next.
    fn finish(&mut self, which: usize) {
        if self.in_flight.is_empty() {
            return;
        }
        let (run_id, index, call) = self.in_flight.remove(which % self.in_flight.len());
        let answer = self.registry.decide(&call);
        self.answers.insert(index, answer);
        self.queues.done(&run_id);
        if let Some(((index, call), order)) = self.queues.next() {
            self.take(index, call, order);
        }
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Reference;

    fn init_test(_: &Reference) -> Self {
        Self {
            registry: Registry::default(),
            queues: Queues::new(),
            in_flight: Vec::new(),
            answers: BTreeMap::new(),
            sent: 0,
        }
    }

    fn apply(mut subject: Self, _: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Send(call) => {
                let index = subject.sent;
                subject.sent += 1;
                subject.take(index, call, Order::Arrived);
            }
            Transition::Finish(which) => subject.finish(which),
        }
        subject
    }

    fn check_invariants(subject: &Self, reference: &Reference) {
        for (index, answer) in &subject.answers {
            assert_eq!(
                answer, &reference.answers[*index],
                "call {index} answered otherwise than alone"
            );
        }
        if subject.in_flight.is_empty() {
            assert_eq!(
                subject.answers.len(),
                reference.answers.len(),
                "a call waits with no disk work in flight"
            );
        }
    }

    fn teardown(mut subject: Self, reference: Reference) {
        while !subject.in_flight.is_empty() {
            subject.finish(0);
        }
        Self::check_invariants(&subject, &reference);
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    #[test]
    fn calls_beside_the_registry_answer_as_they_would_one_at_a_time_in_the_order_they_came(
        sequential 1..40 => Subject
    );
}
