//! The advisory snapshot the gate's audit reads (ADR 0064): `xtask
//! advisories fetch` keeps RustSec's database at its pinned commit and OSV's
//! databases at their pinned SHA-256, so `ci/audit.sh` checks the lockfiles
//! against data that only a commit moves. RustSec's stand-in is a local Git
//! repository, and OSV's a local stand-in of its release host.

use rstest::rstest;

use crate::github::GitHub;
use crate::workspace::{Workspace, finding};

const PINS_FILE: &str = "ci/advisories.toml";

/// Where cargo-deny 0.20 looks for RustSec's database under its `db-path`,
/// and cargo-audit is pointed with `--db`.
const CHECKOUT: &str = "target/advisories/rustsec/advisory-db-3157b0e258782691";

/// An advisory of the RustSec stand-in, and the SHA-256 of `NPM_DATABASE`
/// (`shasum -a 256`).
const ADVISORY: &str = "crates/smallvec/RUSTSEC-2021-0003.md";
const NPM_DATABASE: &str = "stand-in npm all.zip";
const NPM_DATABASE_SHA256: &str =
    "94caeb9e9d79e7f4a22b7af149cb9cbba92dbc6e5816e64fdde642a2148a1b3e";

/// A Git repository standing in for RustSec's, whose one commit holds
/// `ADVISORY` with `text`; returns it with the commit.
fn rustsec(text: &str) -> (Workspace, String) {
    let repository = Workspace::from_fixture("empty");
    repository.write(ADVISORY, text.as_bytes());
    let commit = repository.commit_all();
    (repository, commit)
}

/// A workspace whose pins file pins `commit` of `repository` and the npm
/// database `host` serves.
fn pinned(repository: &Workspace, commit: &str, host: &GitHub) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    let url = host.url();
    workspace.write(
        PINS_FILE,
        format!(
            "[rustsec]\nrepository = \"file://{}\"\ncommit = \"{commit}\"\n\n\
             [[osv]]\nname = \"npm.zip\"\nurl = \"{url}/npm/all.zip\"\nsha256 = \"{NPM_DATABASE_SHA256}\"\n",
            repository.path().display()
        )
        .as_bytes(),
    );
    workspace
}

fn osv_host() -> GitHub {
    GitHub::serving(&[("/npm/all.zip", NPM_DATABASE)])
}

/// Runs `git <args>` in the kept checkout as [`Workspace::git`] runs it: a
/// cleared environment, so a `GIT_DIR` the test inherits (a pre-push hook's)
/// never points it at another repository, no user or system configuration
/// and a fixed author. Returns its output.
fn in_checkout(workspace: &Workspace, args: &[&str]) -> String {
    workspace.git(&[&["-C", CHECKOUT], args].concat())
}

/// The commit the kept checkout has checked out.
fn checked_out(workspace: &Workspace) -> String {
    in_checkout(workspace, &["rev-parse", "HEAD"])
        .trim()
        .to_owned()
}

#[test]
fn fetch_keeps_the_pinned_rustsec_commit_and_osv_database() {
    let (repository, commit) = rustsec("smallvec's advisory");
    let host = osv_host();
    let workspace = pinned(&repository, &commit, &host);

    let outcome = workspace.xtask(&["advisories", "fetch"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        (
            checked_out(&workspace),
            workspace.read(&format!("{CHECKOUT}/{ADVISORY}")),
            workspace.read("target/advisories/osv/npm.zip"),
        ),
        (
            commit,
            Some(b"smallvec's advisory".to_vec()),
            Some(NPM_DATABASE.as_bytes().to_vec()),
        )
    );
}

/// A checkout moved off its pin, as an online `cargo deny check` moves the
/// database under its `db-path`, or with a file changed, goes back to the
/// pinned commit unchanged.
#[rstest]
#[case::moved_to_a_newer_commit(true)]
#[case::with_a_changed_file(false)]
fn fetch_puts_a_changed_checkout_back_at_its_pinned_commit(#[case] commit_the_change: bool) {
    let (repository, commit) = rustsec("smallvec's advisory");
    let host = osv_host();
    let workspace = pinned(&repository, &commit, &host);
    assert_eq!(workspace.xtask(&["advisories", "fetch"]).code, 0);
    workspace.write(&format!("{CHECKOUT}/{ADVISORY}"), b"a newer advisory");
    if commit_the_change {
        in_checkout(
            &workspace,
            &["commit", "--quiet", "--all", "--message", "Newer"],
        );
    }

    let outcome = workspace.xtask(&["advisories", "fetch"]);

    assert_eq!(
        (
            outcome.code,
            checked_out(&workspace),
            workspace.read(&format!("{CHECKOUT}/{ADVISORY}")),
        ),
        (0, commit, Some(b"smallvec's advisory".to_vec())),
        "{}",
        outcome.stderr
    );
}

#[test]
fn fetch_fails_on_a_commit_the_repository_lacks() {
    let (repository, _) = rustsec("smallvec's advisory");
    let host = osv_host();
    let workspace = pinned(&repository, &"0".repeat(40), &host);

    let outcome = workspace.xtask(&["advisories", "fetch"]);

    assert_eq!(
        (
            outcome.code,
            outcome.findings(),
            workspace.read(&format!("{CHECKOUT}/{ADVISORY}")),
        ),
        (1, vec![finding("missing-commit", "rustsec")], None)
    );
}
