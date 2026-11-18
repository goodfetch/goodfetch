//! The startup of Foxhound's executable (ADRs 0071 and 0094), under
//! gungraun's Callgrind: an Attached engine over stdio answering `hello` and
//! exiting at the end of its input, and `goodfetch-foxhound --version`.
//!
//! The Attached engine runs in its sandbox, on a Goodfetch home its setup
//! makes there, whose user configuration gives it one worker thread and one
//! blocking thread (`governor.worker_threads` and `governor.blocking_threads`).
//! As it starts, the engine hands the sandbox probe's fork and the Resource
//! governor's first sample to blocking threads, and reads its input on one:
//! on the default two workers and four blocking threads, how much of that
//! work ran before the end of its input stopped the engine depended on
//! timing, and Callgrind counted the probe's child from the parent's counts
//! at the fork, so the count spread 13.74% across 20 runs (#2752). On one
//! thread of each, the blocking work runs in the order it was handed over,
//! the input last. A home of its own also keeps the run out of the user's
//! data folder, whose list of installed executables, which the engine reads
//! and rewrites as it starts, every executable run on the machine changes.

use std::path::Path;

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::USER_CONFIGURATION;
use goodfetch_protocol::home::{USER_CONFIGURATION_FILE, per_major};
use goodfetch_protocol::settings::HOME_VARIABLE;
use gungraun::prelude::*;
use gungraun::{Sandbox, Stdin, Stdio};

/// A Foxhound Client's `hello`, then the end of input.
const HELLO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/fixtures/hello.jsonl");

/// The Attached engine's Goodfetch home, in its sandbox.
const HOME: &str = "home";

/// Makes [`HOME`] in the sandbox, with one worker thread and one blocking
/// thread in its user configuration.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark's setup writes its home before the measured run, on no async worker"
)]
fn one_thread_each() {
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version");
    let folder = per_major(Path::new(HOME), &version);
    std::fs::create_dir_all(&folder).expect("invariant: the sandbox takes a folder");
    let configuration = format!(
        "version = \"{}\"\n\n[settings.governor]\nworker_threads = 1\nblocking_threads = 1\n",
        USER_CONFIGURATION.version
    );
    std::fs::write(folder.join(USER_CONFIGURATION_FILE), configuration)
        .expect("invariant: the sandbox takes a file");
}

#[binary_benchmark(
    setup = one_thread_each(),
    config = BinaryBenchmarkConfig::default().sandbox(Sandbox::new(true))
)]
fn attached_engine_answers_hello() -> gungraun::Command {
    gungraun::Command::new(env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .arg("attached-engine")
        .env(HOME_VARIABLE, HOME)
        .stdin(Stdin::from(Stdio::File(HELLO.into())))
        .build()
}

#[binary_benchmark]
fn version() -> gungraun::Command {
    gungraun::Command::new(env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .arg("--version")
        .build()
}

binary_benchmark_group!(
    name = startup,
    benchmarks = [attached_engine_answers_hello, version]
);

main!(binary_benchmark_groups = startup);
