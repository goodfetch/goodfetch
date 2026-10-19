use tokio::sync::oneshot;

use super::TaskOwner;

#[test]
fn stopping_an_owner_cancels_its_tasks_and_waits_for_them() {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root();
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
    let parent = TaskOwner::root();
    let child = parent.child();

    child.cancel();

    assert_eq!(
        (child.token().is_cancelled(), parent.token().is_cancelled()),
        (true, false)
    );
}

#[test]
fn cancelling_a_parent_cancels_its_child() {
    let parent = TaskOwner::root();
    let child = parent.child();

    parent.cancel();

    assert!(child.token().is_cancelled());
}
