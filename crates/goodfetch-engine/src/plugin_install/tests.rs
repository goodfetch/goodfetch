use goodfetch_protocol::browser_cache::DownloadFailure;
use rstest::rstest;

use super::{PullFailure, status_failure};
use crate::http_client;

/// The registry's failures, each a named case `artifact_download_failed`
/// carries, and an unknown status its own case that keeps the answer's
/// text.
///
/// Covers: story:7-81
#[rstest]
#[case::absent(404, PullFailure::Absent, DownloadFailure::HttpStatus { status: 404 })]
#[case::unauthorized(401, PullFailure::Unauthorized(401), DownloadFailure::HttpStatus { status: 401 })]
#[case::denied(403, PullFailure::Unauthorized(403), DownloadFailure::HttpStatus { status: 403 })]
#[case::rate_limited(429, PullFailure::RateLimited, DownloadFailure::HttpStatus { status: 429 })]
#[case::server_error(500, PullFailure::ServerError(500), DownloadFailure::HttpStatus { status: 500 })]
#[case::unavailable(503, PullFailure::ServerError(503), DownloadFailure::HttpStatus { status: 503 })]
#[case::unrecognized(
    418,
    PullFailure::Unrecognized { status: 418, text: "teapot".to_owned() },
    DownloadFailure::HttpStatus { status: 418 }
)]
#[case::redirect(
    307,
    PullFailure::Unrecognized { status: 307, text: "teapot".to_owned() },
    DownloadFailure::HttpStatus { status: 307 }
)]
fn a_status_the_registry_answers_is_a_named_failure(
    #[case] status: u16,
    #[case] expected: PullFailure,
    #[case] carried: DownloadFailure,
) {
    let failure = status_failure(status, "teapot".to_owned());

    assert_eq!(failure, expected);
    assert_eq!(failure.download_failure(), carried);
}

/// A failure before any answer is the registry unreachable, or TLS.
///
/// Covers: story:7-81
#[rstest]
#[case::network(http_client::Failure::Network, PullFailure::Unreachable)]
#[case::server_name(http_client::Failure::ServerName, PullFailure::Tls)]
fn a_failure_before_any_answer_is_a_named_failure(
    #[case] failure: http_client::Failure,
    #[case] expected: PullFailure,
) {
    let failure = PullFailure::from(failure);

    assert_eq!(failure, expected);
    assert_eq!(failure.download_failure(), DownloadFailure::Network);
}
