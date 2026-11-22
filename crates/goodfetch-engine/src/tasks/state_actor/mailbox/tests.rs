use std::time::Duration;

use super::Mailbox;

/// What the tests' actor holds: the jobs it ran.
type Ran = Vec<u32>;

/// A job whose place was taken before the actor ended, and sent after the
/// actor dropped its side and the jobs it held, as a caller does when the
/// actor's thread ends between those two steps (issue 3327), is refused:
/// no one will run it.
#[test]
fn a_job_sent_once_its_actor_ended_is_refused() {
    let answer = goodfetch_test_support::runtime().block_on(async {
        let (mailbox, jobs, running) = Mailbox::<Ran>::open(1);
        let place = mailbox.reserve().await.expect("the actor runs");
        drop((jobs, running));
        // The clock is paused: the timeout ends a wait nothing else ends.
        tokio::time::timeout(
            Duration::from_secs(60),
            place.send(|ran: &mut Ran| ran.len()),
        )
        .await
    });

    assert_eq!(answer, Ok(None));
}

/// A job the actor ran before it ended answers what it returned.
#[test]
fn a_job_run_before_its_actor_ended_is_answered() {
    let answer = goodfetch_test_support::runtime().block_on(async {
        let (mailbox, mut jobs, running) = Mailbox::<Ran>::open(1);
        let place = mailbox.reserve().await.expect("the actor runs");
        let actor = async move {
            let mut ran = vec![1];
            let job = jobs.recv().await.expect("the job was sent");
            job(&mut ran);
            drop(running);
        };
        tokio::join!(place.send(|ran: &mut Ran| ran.len()), actor).0
    });

    assert_eq!(answer, Some(1));
}
