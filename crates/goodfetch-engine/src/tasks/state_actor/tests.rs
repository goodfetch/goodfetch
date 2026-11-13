use std::sync::Arc;

use rstest::rstest;

use super::StateActor;
use crate::tasks::{BlockingPolicy, TaskOwner};

/// What the tests' actor holds: the jobs it ran.
type Ran = Vec<u32>;

fn actor(owner: &Arc<TaskOwner>) -> StateActor<Ran> {
    StateActor::of("test-state-actor", Arc::clone(owner))
}

/// Each job runs on the state the earlier ones left, in order, and its
/// answer comes back to its caller.
#[rstest]
#[case::on_threads(BlockingPolicy::Threads)]
#[case::inline(BlockingPolicy::Inline)]
fn jobs_run_in_order_on_the_state_the_actor_keeps(#[case] policy: BlockingPolicy) {
    let answers = goodfetch_test_support::runtime().block_on(async {
        let owner = Arc::new(TaskOwner::root(policy));
        let actor = actor(&owner);
        let mut answers = Vec::new();
        for job in 1..=3 {
            answers.push(
                actor
                    .run(move |ran: &mut Ran| {
                        ran.push(job);
                        ran.clone()
                    })
                    .await,
            );
        }
        owner.stop().await;
        answers
    });

    assert_eq!(answers, [Ok(vec![1]), Ok(vec![1, 2]), Ok(vec![1, 2, 3])]);
}

/// Under the Inline policy the actor is a task of the runtime its first
/// job ran on: once that runtime ended, the next job starts it again on its
/// own, from a fresh state, its owner still running.
#[test]
fn an_actor_whose_runtime_ended_starts_again_on_the_next() {
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let actor = actor(&owner);
    let run = || {
        goodfetch_test_support::runtime().block_on(actor.run(|ran: &mut Ran| {
            ran.push(1);
            ran.len()
        }))
    };

    let (first, second) = (run(), run());

    assert_eq!((first, second), (Ok(1), Ok(1)));
}

/// Once its owner is cancelled, no job starts the actor.
#[test]
fn an_actor_whose_owner_stopped_runs_no_job() {
    let answer = goodfetch_test_support::runtime().block_on(async {
        let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let actor = actor(&owner);
        actor.run(|ran: &mut Ran| ran.push(1)).await.unwrap();
        owner.stop().await;
        actor.run(|ran: &mut Ran| ran.len()).await
    });

    assert!(answer.is_err(), "{answer:?}");
}

/// An actor whose job panicked is not started again in place (ADR 0148):
/// its owner's parent ends the owner, and until then every job is refused.
#[rstest]
#[case::on_threads(BlockingPolicy::Threads)]
#[case::inline(BlockingPolicy::Inline)]
fn an_actor_whose_job_panicked_runs_no_later_job(#[case] policy: BlockingPolicy) {
    let (panicked, later) = goodfetch_test_support::runtime().block_on(async {
        let parent = TaskOwner::root(policy);
        let owner = Arc::new(parent.child());
        let actor = actor(&owner);
        let panicked = actor
            .run(|_: &mut Ran| -> usize { panic!("the job's own panic") })
            .await;
        let later = actor.run(|ran: &mut Ran| ran.len()).await;
        owner.stop().await;
        parent.stop().await;
        (panicked, later)
    });

    assert!(panicked.is_err(), "{panicked:?}");
    assert!(later.is_err(), "{later:?}");
}
