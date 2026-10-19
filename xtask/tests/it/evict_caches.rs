//! Cache priority within GitHub's 10 GB (ADR 0061; TESTING.md, Sharding,
//! caching and reports): once the repository's caches pass 9 GB, `xtask
//! evict-caches` deletes the least recently used caches that are not the
//! pull-request tier's Linux caches, until they fit, through GitHub's cache
//! interface.

use rstest::rstest;

use crate::github::GitHub;
use crate::workspace::{Outcome, Workspace};

const CACHES: &str = "/repos/goodfetch/goodfetch/actions/caches";

const GB: u64 = 1_000_000_000;

/// One cache as GitHub lists it.
fn cache(id: u64, key: &str, bytes: u64, last_accessed_at: &str) -> String {
    format!(
        r#"{{ "id": {id}, "ref": "refs/heads/main", "key": "{key}", "version": "v",
             "last_accessed_at": "{last_accessed_at}", "created_at": "2026-10-01T00:00:00Z",
             "size_in_bytes": {bytes} }}"#
    )
}

fn github(caches: &[String]) -> GitHub {
    let listed = format!(
        r#"{{ "total_count": {}, "actions_caches": [ {} ] }}"#,
        caches.len(),
        caches.join(", ")
    );
    // GitHub answers a deletion with no body; the stand-in's empty object
    // stands in for it.
    let mut routes = vec![(format!("{CACHES}?per_page=100&page=1"), listed)];
    routes.extend((1..=caches.len()).map(|id| (format!("DELETE {CACHES}/{id}"), "{}".to_owned())));
    let routes: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), body.as_str()))
        .collect();
    GitHub::serving(&routes)
}

fn evict(github: &GitHub) -> Outcome {
    Workspace::from_fixture("empty").xtask(&[
        "evict-caches",
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ])
}

/// The paths the command deleted, in order.
fn deleted(github: &GitHub) -> Vec<String> {
    github
        .writes()
        .into_iter()
        .map(|request| {
            assert_eq!(request.method, "DELETE");
            request.path
        })
        .collect()
}

#[test]
fn caches_past_9_gb_lose_their_least_recently_used_ones_until_they_fit() {
    let github = github(&[
        cache(
            1,
            "v0-rust-release-suite-linux-x64-gnu-Linux-x64-a-b",
            3 * GB,
            "2026-10-03T00:00:00Z",
        ),
        cache(
            2,
            "v0-rust-release-suite-windows-x64-Windows_NT-x64-a-b",
            GB,
            "2026-10-01T00:00:00Z",
        ),
        cache(3, "xtask-Linux-X64-c", GB, "2026-10-02T00:00:00Z"),
        cache(
            4,
            "v0-rust-release-suite-macos-arm64-Darwin-arm64-a-b",
            5_500_000_000,
            "2026-10-04T00:00:00Z",
        ),
    ]);

    let outcome = evict(&github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        deleted(&github),
        vec![format!("{CACHES}/2"), format!("{CACHES}/3")]
    );
}

#[rstest]
#[case::under_9_gb(8 * GB)]
#[case::at_9_gb(9 * GB)]
fn caches_within_9_gb_are_kept(#[case] bytes: u64) {
    let github = github(&[
        cache(
            1,
            "v0-rust-release-suite-linux-x64-gnu-Linux-x64-a-b",
            bytes - GB,
            "2026-10-01T00:00:00Z",
        ),
        cache(2, "xtask-Linux-X64-c", GB, "2026-10-02T00:00:00Z"),
    ]);

    let outcome = evict(&github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(deleted(&github), Vec::<String>::new());
}

/// The pull-request tier's Linux caches take priority: they are never
/// deleted, even when they alone pass 9 GB; its caches of another OS are not.
#[test]
fn pull_request_linux_caches_are_never_deleted() {
    let github = github(&[
        cache(
            1,
            "v0-rust-pull-request-pre-merge-Linux-x64-a-b",
            6 * GB,
            "2026-10-01T00:00:00Z",
        ),
        cache(
            2,
            "pull-request-pnpm-Linux-x64-c",
            4 * GB,
            "2026-10-01T00:00:00Z",
        ),
        cache(
            3,
            "v0-rust-pull-request-pre-merge-Darwin-arm64-a-b",
            GB,
            "2026-10-02T00:00:00Z",
        ),
        cache(
            4,
            "v0-rust-release-suite-linux-x64-gnu-Linux-x64-a-b",
            GB,
            "2026-10-03T00:00:00Z",
        ),
    ]);

    let outcome = evict(&github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        deleted(&github),
        vec![format!("{CACHES}/3"), format!("{CACHES}/4")]
    );
}
