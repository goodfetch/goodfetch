//! The list of installed executables' writer for the Compatibility corpus
//! (ADR 0066): records, through the engine's own writer at ADR 0059's test
//! `Clock` start, a Collie executable that a Daemon runs from its versioned
//! copy and a program that links Foxhound through its Rust crate, then
//! copies the list into the folder given as the only argument as
//! `installed-executables.json`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::home::Home;
use goodfetch_engine::installed::InstalledList;
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable};
use goodfetch_protocol::point_in_time;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let list = InstalledList::of(&Home::new(scratch.path().to_owned()));
    let start = point_in_time::from_rfc3339(START).expect("invariant: an RFC 3339 time");
    let version = ProductVersion::parse("0.0.0").expect("invariant: a Product version");
    let browser = ArtifactId {
        kind: ArtifactKind::Browser,
        name: "chrome-for-testing".to_owned(),
        build: "141.0.7390.54-linux64".to_owned(),
    };
    let collie = InstalledExecutable {
        kind: ExecutableKind::ProductExecutable,
        product: Product::Collie,
        product_version: version.clone(),
        path: "/home/someone/project/node_modules/.bin/goodfetch-collie".to_owned(),
        size: 41_943_040,
        modified_at: point_in_time::from_rfc3339("2026-01-02T08:00:00Z")
            .expect("invariant: an RFC 3339 time"),
        pins: vec![browser.clone()],
        copy: Some(ArtifactId {
            kind: ArtifactKind::ExecutableCopy,
            name: "goodfetch-collie".to_owned(),
            build: "0.0.0-0123456789abcdef".to_owned(),
        }),
        last_seen: start,
    };
    let linked = InstalledExecutable {
        kind: ExecutableKind::LinkedProgram,
        product: Product::Foxhound,
        product_version: version,
        path: "/home/someone/crawler/target/release/crawler".to_owned(),
        size: 52_428_800,
        modified_at: point_in_time::from_rfc3339("2026-01-03T10:30:00Z")
            .expect("invariant: an RFC 3339 time"),
        pins: vec![browser],
        copy: None,
        last_seen: start,
    };
    for executable in [&collie, &linked] {
        list.record(executable)
            .expect("invariant: the scratch folder is writable");
    }
    std::fs::copy(list.path(), output.join("installed-executables.json"))
        .expect("invariant: the output folder is writable");
}
