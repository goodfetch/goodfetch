//! The LazyFS build's own logic (ADR 0060): `xtask lazyfs fetch` keeps the
//! pinned source archives only when each download's SHA-256 is its pin, so
//! a changed download fails before the deny, and `xtask lazyfs build`
//! refuses to build from a kept archive that is not its pin. The downloads
//! come from a local stand-in of their release host; the build itself runs
//! where LazyFS does, on Linux, and is proven by the LazyFS tests it serves.

use crate::github::GitHub;
use crate::workspace::{Workspace, finding};

/// The stand-in archives, with their SHA-256 from `shasum -a 256`.
const LAZYFS: &str = "stand-in lazyfs.tar.gz";
const LAZYFS_SHA256: &str = "52a53df515eb8ab443a21ed69923e16eb0d39d8d909c4fa38ae05a135ec87474";
const TOML11: &str = "stand-in toml11.tar.gz";
const TOML11_SHA256: &str = "01336bd55eb71440806f3b983881e367327e4c76101653319ed8b5297cf51aa3";
const SPDLOG: &str = "stand-in spdlog.tar.gz";
const SPDLOG_SHA256: &str = "1719069cf203cc4bb216ab7e1b6b9dc9a88442d2b3908d8781728580a0672b21";

const SOURCES_FILE: &str = "test-support/lazyfs/sources.toml";

fn release_host() -> GitHub {
    GitHub::serving(&[
        ("/lazyfs.tar.gz", LAZYFS),
        ("/toml11.tar.gz", TOML11),
        ("/spdlog.tar.gz", SPDLOG),
    ])
}

/// A workspace whose sources file pins the three stand-in archives, LazyFS's
/// at `lazyfs_sha256`.
fn pinned(host: &GitHub, lazyfs_sha256: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    let url = host.url();
    let source = |name: &str, sha256: &str| {
        format!("[[source]]\nname = \"{name}\"\nurl = \"{url}/{name}\"\nsha256 = \"{sha256}\"\n\n")
    };
    workspace.write(
        SOURCES_FILE,
        [
            source("lazyfs.tar.gz", lazyfs_sha256),
            source("toml11.tar.gz", TOML11_SHA256),
            source("spdlog.tar.gz", SPDLOG_SHA256),
        ]
        .concat()
        .as_bytes(),
    );
    workspace
}

#[test]
fn fetch_keeps_each_source_archive_whose_digest_is_its_pin() {
    let host = release_host();
    let workspace = pinned(&host, LAZYFS_SHA256);

    let outcome = workspace.xtask(&["lazyfs", "fetch"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        (
            workspace.read("target/lazyfs/sources/lazyfs.tar.gz"),
            workspace.read("target/lazyfs/sources/toml11.tar.gz"),
            workspace.read("target/lazyfs/sources/spdlog.tar.gz"),
        ),
        (
            Some(LAZYFS.as_bytes().to_vec()),
            Some(TOML11.as_bytes().to_vec()),
            Some(SPDLOG.as_bytes().to_vec()),
        )
    );
}

#[test]
fn fetch_fails_on_a_source_archive_whose_digest_differs_from_its_pin() {
    let host = release_host();
    let workspace = pinned(&host, TOML11_SHA256);

    let outcome = workspace.xtask(&["lazyfs", "fetch"]);

    assert_eq!(
        (
            outcome.code,
            outcome.findings(),
            workspace.read("target/lazyfs/sources/lazyfs.tar.gz")
        ),
        (1, vec![finding("digest-mismatch", "lazyfs.tar.gz")], None)
    );
}

#[test]
fn build_refuses_a_kept_source_archive_that_is_not_its_pin() {
    let host = release_host();
    let workspace = pinned(&host, LAZYFS_SHA256);
    assert_eq!(workspace.xtask(&["lazyfs", "fetch"]).code, 0);
    workspace.write("target/lazyfs/sources/spdlog.tar.gz", b"a changed archive");

    let outcome = workspace.xtask(&["lazyfs", "build"]);

    assert_eq!(
        (
            outcome.code,
            outcome.findings(),
            workspace.read("target/lazyfs/bin/lazyfs")
        ),
        (1, vec![finding("not-pinned", "spdlog.tar.gz")], None)
    );
}
