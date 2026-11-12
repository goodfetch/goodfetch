use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use tokio::sync::{mpsc, oneshot};

use super::{BlockingPolicy, OwnerKind, TaskOwner};

#[test]
fn stopping_an_owner_cancels_its_tasks_and_waits_for_them() {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (finished, observed) = oneshot::channel();
        let token = owner.token().clone();
        owner.spawn(async move {
            token.cancelled().await;
            let _ = finished.send("stopped after its cancellation");
        });

        owner.stop().await;

        assert_eq!(observed.await, Ok("stopped after its cancellation"));
    });
}

#[test]
fn cancelling_a_child_leaves_its_parent_running() {
    let parent = TaskOwner::root(BlockingPolicy::Inline);
    let child = parent.child();

    child.cancel();

    assert_eq!(
        (child.token().is_cancelled(), parent.token().is_cancelled()),
        (true, false)
    );
}

#[test]
fn cancelling_a_parent_cancels_its_child() {
    let parent = TaskOwner::root(BlockingPolicy::Inline);
    let child = parent.child();

    parent.cancel();

    assert!(child.token().is_cancelled());
}

#[test]
fn a_child_keeps_its_parents_blocking_policy() {
    let parent = TaskOwner::root(BlockingPolicy::Threads);

    assert_eq!(parent.child().policy(), BlockingPolicy::Threads);
}

#[rstest]
#[case::on_a_blocking_thread(BlockingPolicy::Threads)]
#[case::inline(BlockingPolicy::Inline)]
fn blocking_work_answers_what_it_returns(#[case] policy: BlockingPolicy) {
    let answer = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(policy);
        let answer = owner.blocking(|| 6 * 7).await;
        owner.stop().await;
        answer
    });

    assert_eq!(answer, 42);
}

#[rstest]
#[case::on_a_blocking_thread(BlockingPolicy::Threads)]
#[case::inline(BlockingPolicy::Inline)]
#[should_panic(expected = "the work's own panic")]
fn a_panic_in_blocking_work_resumes_in_its_caller(#[case] policy: BlockingPolicy) {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(policy);
        owner.blocking(|| panic!("the work's own panic")).await
    });
}

/// The actor gets what its mailbox holds at once, in order, and its owner
/// waits for it to end once every sender is gone.
#[rstest]
#[case::on_a_dedicated_thread(BlockingPolicy::Threads)]
#[case::as_a_task(BlockingPolicy::Inline)]
fn an_actor_handles_its_mailbox_in_groups_and_ends_with_its_senders(
    #[case] policy: BlockingPolicy,
) {
    let groups = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(policy);
        let (sender, mailbox) = mpsc::channel(8);
        let (seen, mut groups) = mpsc::unbounded_channel();
        for message in [1, 2, 3] {
            sender.send(message).await.unwrap();
        }
        owner
            .actor("test-actor", mailbox, move |group: Vec<i32>| {
                let _ = seen.send(group);
            })
            .unwrap();

        drop(sender);
        owner.wait().await;

        let mut all = Vec::new();
        while let Ok(group) = groups.try_recv() {
            all.push(group);
        }
        all
    });

    assert_eq!(groups, [vec![1, 2, 3]]);
}

/// Stopping an owner ends its actor once its tasks have ended, whoever
/// still holds a sender (#1763): the actor handles what was queued before,
/// a task's own send during its clean-up included, and a later send is
/// refused.
#[rstest]
#[case::on_a_dedicated_thread(BlockingPolicy::Threads)]
#[case::as_a_task(BlockingPolicy::Inline)]
fn stopping_an_owner_ends_its_actor_after_its_tasks_though_a_sender_remains(
    #[case] policy: BlockingPolicy,
) {
    let (stopped, handled, refused) = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(policy);
        let (sender, mailbox) = mpsc::channel(8);
        let (seen, mut groups) = mpsc::unbounded_channel();
        owner
            .actor("test-actor", mailbox, move |group: Vec<i32>| {
                let _ = seen.send(group);
            })
            .unwrap();
        let (token, cleaning_up) = (owner.token().clone(), sender.clone());
        owner.spawn(async move {
            token.cancelled().await;
            cleaning_up.send(2).await.unwrap();
        });
        sender.send(1).await.unwrap();

        let stopped = match policy {
            // Inline, every step runs on the paused runtime, whose clock
            // moves on only once nothing is left to run.
            BlockingPolicy::Inline => tokio::time::timeout(Duration::from_mins(1), owner.stop())
                .await
                .is_ok(),
            BlockingPolicy::Threads => {
                owner.stop().await;
                true
            }
        };
        let refused = sender.send(3).await.is_err();
        let mut handled = Vec::new();
        while let Ok(group) = groups.try_recv() {
            handled.extend(group);
        }
        (stopped, handled, refused)
    });

    assert_eq!((stopped, handled, refused), (true, vec![1, 2], true));
}

/// What an analysis on the CPU pool told the test.
#[derive(Debug)]
enum Analysed {
    Started(usize),
    Ended(usize),
}

/// On the paused runtime, a CPU pool of two permits runs two of three
/// analyses at once: the third waits for a permit, and starts only once
/// one of the first two returned.
#[test]
fn no_more_analyses_than_the_cpu_pools_permits_run_at_once() {
    let (waiting, most) = goodfetch_test_support::runtime().block_on(async {
        let owner = Arc::new(TaskOwner::root_on_cores(
            BlockingPolicy::Threads,
            NonZeroUsize::new(2).unwrap(),
        ));
        let (told, mut events) = mpsc::unbounded_channel();
        let mut releases = Vec::new();
        for analysis in 0..3 {
            let (release, go_on) = oneshot::channel::<()>();
            releases.push(Some(release));
            let (on, told) = (Arc::clone(&owner), told.clone());
            owner.spawn(async move {
                on.analysis(move || {
                    let _ = told.send(Analysed::Started(analysis));
                    let _ = go_on.blocking_recv();
                    let _ = told.send(Analysed::Ended(analysis));
                })
                .await;
            });
        }
        drop(told);
        let (mut running, mut most) = (Vec::new(), 0);
        while running.len() < 2 {
            if let Some(Analysed::Started(analysis)) = events.recv().await {
                running.push(analysis);
            }
        }
        // A third analysis that a pool let through starts within this
        // short real wait; one the pool holds never does.
        #[expect(
            clippy::disallowed_methods,
            reason = "the analyses run on blocking threads, which this real wait gives the time to start one the pool should hold"
        )]
        std::thread::sleep(Duration::from_millis(100));
        let mut waiting = true;
        while let Ok(event) = events.try_recv() {
            if let Analysed::Started(analysis) = event {
                waiting = false;
                running.push(analysis);
            }
        }
        most = most.max(running.len());
        for analysis in running.clone() {
            if let Some(release) = releases[analysis].take() {
                let _ = release.send(());
            }
        }
        while let Some(event) = events.recv().await {
            match event {
                Analysed::Started(analysis) => {
                    running.push(analysis);
                    most = most.max(running.len());
                    if let Some(release) = releases[analysis].take() {
                        let _ = release.send(());
                    }
                }
                Analysed::Ended(analysis) => running.retain(|other| *other != analysis),
            }
        }
        owner.stop().await;
        (waiting, most)
    });

    assert_eq!((waiting, most), (true, 2));
}

#[test]
fn an_analysis_runs_inline_under_the_inline_policy() {
    let answer = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let caller = std::thread::current().id();
        owner
            .analysis(move || std::thread::current().id() == caller)
            .await
    });

    assert!(answer);
}

/// How long an Inline test waits, on the paused clock, for what an owner
/// owes: never once means the test fails rather than hangs. A test on
/// threads waits without it, since the paused clock moves on while a
/// thread of its own still runs.
const OWED: Duration = Duration::from_mins(1);

/// Awaits `owed` within [`OWED`] under [`BlockingPolicy::Inline`], and
/// without a limit under [`BlockingPolicy::Threads`]: whether it came.
async fn within<F: Future<Output = ()>>(policy: BlockingPolicy, owed: F) -> bool {
    match policy {
        BlockingPolicy::Inline => tokio::time::timeout(OWED, owed).await.is_ok(),
        BlockingPolicy::Threads => {
            owed.await;
            true
        }
    }
}

/// A child owner's task that panics is told to its parent, which can also
/// read it at once, and then stops the child: the stop ends, a sibling task
/// that waits for its cancellation included (ADR 0148).
#[rstest]
#[case::on_threads(BlockingPolicy::Threads)]
#[case::inline(BlockingPolicy::Inline)]
fn a_child_task_that_panics_is_told_to_its_parent_which_stops_the_child(
    #[case] policy: BlockingPolicy,
) {
    let (told, stopped) = goodfetch_test_support::runtime().block_on(async {
        let parent = TaskOwner::root(policy);
        let child = parent.child();
        let token = child.token().clone();
        child.spawn(async move { token.cancelled().await });
        child.spawn(async { panic!("the task's own panic") });

        let told = within(policy, child.panicked()).await && child.has_panicked();
        let stopped = within(policy, child.stop()).await;
        parent.stop().await;
        (told, stopped)
    });

    assert_eq!((told, stopped), (true, true));
}

/// The panic of a task whose owner's parent has no reaction yet, a Run's,
/// is logged at `error` with the owner's kind, its only trace (ADR 0148).
#[test]
fn a_task_that_panics_is_logged_at_error_with_its_owners_kind() {
    let (told, logged) = goodfetch_test_support::logged(|| {
        goodfetch_test_support::runtime().block_on(async {
            let parent = TaskOwner::root(BlockingPolicy::Inline);
            let run = parent.child_for(OwnerKind::Run);
            run.spawn(async { panic!("the task's own panic") });
            let told = within(BlockingPolicy::Inline, run.panicked()).await;
            run.stop().await;
            parent.stop().await;
            told
        })
    });

    let logged = String::from_utf8_lossy(&logged);
    let told_lines: Vec<&str> = logged
        .lines()
        .filter(|line| line.contains("goodfetch_engine::tasks"))
        .collect();
    assert_eq!(
        (
            told,
            told_lines.len(),
            told_lines
                .iter()
                .all(|line| line.contains("ERROR") && line.contains(r#"owner="run""#)),
        ),
        (true, 1, true),
        "{logged}"
    );
}

/// A child owner's actor whose handler panics is told to its parent, which
/// then stops the child, on the actor's own thread as when it runs as a
/// task.
#[rstest]
#[case::on_a_dedicated_thread(BlockingPolicy::Threads)]
#[case::as_a_task(BlockingPolicy::Inline)]
fn a_child_actor_that_panics_is_told_to_its_parent_which_stops_the_child(
    #[case] policy: BlockingPolicy,
) {
    let (told, stopped) = goodfetch_test_support::runtime().block_on(async {
        let parent = TaskOwner::root(policy);
        let child = parent.child();
        let (sender, mailbox) = mpsc::channel(8);
        child
            .actor("test-actor", mailbox, |_: Vec<i32>| {
                panic!("the actor's own panic")
            })
            .unwrap();
        sender.send(1).await.unwrap();

        let told = within(policy, child.panicked()).await;
        let stopped = within(policy, child.stop()).await;
        parent.stop().await;
        (told, stopped)
    });

    assert_eq!((told, stopped), (true, true));
}

/// Blocking work that panics once its caller was dropped, which only a
/// blocking thread allows, is told to its owner's parent, which then stops
/// the owner.
#[test]
fn blocking_work_that_panics_after_its_caller_dropped_is_told_to_its_parent() {
    let stopped = goodfetch_test_support::runtime().block_on(async {
        let parent = TaskOwner::root(BlockingPolicy::Threads);
        let child = parent.child();
        let (started, starting) = oneshot::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let caller = child.blocking(move || {
            let _ = started.send(());
            let _ = released.recv();
            panic!("the work's own panic")
        });
        tokio::select! {
            () = caller => unreachable!("the work waits for its release"),
            _ = starting => {}
        }
        release.send(()).unwrap();

        child.panicked().await;
        child.stop().await;
        parent.stop().await;
        true
    });

    assert!(stopped);
}

/// An owner none of whose tasks panicked is never told so.
#[test]
fn an_owner_whose_tasks_ended_is_not_told_of_a_panic() {
    let told = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        owner.spawn(async {});
        owner.stop().await;
        tokio::time::timeout(OWED, owner.panicked()).await.is_ok() || owner.has_panicked()
    });

    assert!(!told);
}
