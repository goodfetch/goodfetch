use super::{TASK_REQUESTS, TASK_SECONDS, bound};

/// A task's crawl goes on under both bounds, and is stopped at either,
/// saying which, at the requests it had sent.
#[test]
fn a_tasks_crawl_is_stopped_at_its_requests_or_its_seconds() {
    assert_eq!(
        [
            bound(TASK_SECONDS - 1, TASK_REQUESTS - 1),
            bound(12, TASK_REQUESTS + 40),
            bound(TASK_SECONDS, 37),
        ],
        [
            None,
            Some("stopped after 1040 requests, the most a task may take".to_owned()),
            Some("stopped after 300 s, the longest a task may take, at 37 requests".to_owned()),
        ]
    );
}
