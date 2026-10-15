//! Snippet for Results, outcomes and errors (#466): match on the code or group, read outcomes as values.

use goodfetch_sdk_sketch::{
    AiWorkResult, Ended, Error, ErrorCode, ErrorDetails, ErrorGroup, CrawlRequestOutcome, CrawlRequestOutcomeKind,
};

/// A failure is an `Err`; the page call itself is #472's.
fn on_failure(err: &Error) -> Option<Vec<String>> {
    match (err.group(), err.code(), err.details()) {
        (_, ErrorCode::TargetNotUnique, Some(ErrorDetails::TargetNotUnique(d))) => Some(d.matches.clone()),
        (ErrorGroup::DeadlineExceeded, _, _) => err.limit().map(|l| vec![l.setting.clone()]),
        (ErrorGroup::Page, _, _) => err.observed().map(|o| vec![o.to_owned()]),
        _ => None, // `#[non_exhaustive]`: a newer engine's codes land here, in their group
    }
}

/// Outcomes are values; how a program reads them is #478's.
fn count(outcomes: Vec<CrawlRequestOutcome>) -> (u32, u32, u32, u32) {
    let (mut ok, mut failed, mut skipped, mut blocked) = (0, 0, 0, 0);
    for outcome in outcomes {
        match outcome.kind {
            CrawlRequestOutcomeKind::Succeeded { .. } => ok += 1,
            CrawlRequestOutcomeKind::Failed(err) => {
                failed += 1;
                let _ = (err.code(), outcome.attempts, outcome.attempt_errors.len());
            }
            CrawlRequestOutcomeKind::Skipped { .. } => skipped += 1,
            CrawlRequestOutcomeKind::Blocked(_) => blocked += 1,
            _ => {}
        }
    }
    (ok, failed, skipped, blocked)
}

/// C-GOOD-ERR: the error crosses threads and tasks.
fn assert_good_error<E: std::error::Error + Send + Sync + 'static>() {}

/// AI work: the Verdict is a value until the program asks for it as an error. The Agent's API is #474's.
fn ai_work(task: AiWorkResult<String>) -> Result<String, Error> {
    if let Ended::Stopped(reason) = &task.ended {
        let _ = (reason, &task.partial);
    }
    task.require_verified()
}

fn main() {
    assert_good_error::<Error>();
    let _ = (on_failure, count, ai_work);
}
