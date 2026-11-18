//! The startup of Collie's executable (ADRs 0071 and 0094), under gungraun's
//! Callgrind: an Attached engine over stdio answering `hello` and exiting at
//! the end of its input, and `goodfetch-collie --version`.
//!
//! The Attached engine runs in gungraun's sandbox, on a Goodfetch home the
//! benchmark's setup makes there. Its user configuration gives the engine one
//! worker thread and one blocking thread (`governor.worker_threads` and
//! `governor.blocking_threads`) and an hour between the Resource governor's
//! samples (`governor.sample_interval`). As it starts, the engine hands the
//! sandbox probe's fork, the governor's first sample and the reads of its
//! input to blocking threads. On the default two workers and four blocking
//! threads, how much of that work ran before the end of its input stopped the
//! engine depended on timing, and Callgrind counts the probe's forked child as
//! a process of its own, so the count spread 14.78% across 20 runs (#2752).
//! On one blocking thread that work runs one piece at a time, and with an
//! hour between samples a slow run takes no second sample. A home of its own
//! also keeps the run off the user's Goodfetch home, whose list of installed
//! executables, which the engine reads and rewrites as it starts, every
//! executable run on the machine changes.

use std::path::Path;

use goodfetch_protocol::formats::USER_CONFIGURATION;
use goodfetch_protocol::settings::HOME_VARIABLE;
use goodfetch_test_client::HomeLayout;
use gungraun::prelude::*;
use gungraun::{Sandbox, Stdin, Stdio};

/// Collie's `hello`, then the end of input.
const HELLO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/fixtures/hello.jsonl");

/// The Attached engine's Goodfetch home, in gungraun's sandbox.
const HOME: &str = "home";

/// Makes [`HOME`] in the sandbox, whose user configuration gives the engine
/// one thread of each kind and an hour between the governor's samples.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark's setup writes its home before the measured run, on no async worker"
)]
fn pinned_home() {
    let layout = HomeLayout::of(Path::new(HOME));
    std::fs::create_dir_all(layout.per_major()).expect("invariant: the sandbox takes a folder");
    let configuration = format!(
        "version = \"{}\"\n\n[settings.governor]\nworker_threads = 1\nblocking_threads = 1\nsample_interval = 3600000\n",
        USER_CONFIGURATION.version
    );
    std::fs::write(layout.user_configuration(), configuration)
        .expect("invariant: the sandbox takes a file");
}

#[binary_benchmark(
    setup = pinned_home(),
    config = BinaryBenchmarkConfig::default().sandbox(Sandbox::new(true))
)]
fn attached_engine_answers_hello() -> gungraun::Command {
    gungraun::Command::new(env!("CARGO_BIN_EXE_goodfetch-collie"))
        .arg("attached-engine")
        .env(HOME_VARIABLE, HOME)
        .stdin(Stdin::from(Stdio::File(HELLO.into())))
        .build()
}

#[binary_benchmark]
fn version() -> gungraun::Command {
    gungraun::Command::new(env!("CARGO_BIN_EXE_goodfetch-collie"))
        .arg("--version")
        .build()
}

binary_benchmark_group!(
    name = startup,
    benchmarks = [attached_engine_answers_hello, version]
);

main!(binary_benchmark_groups = startup);
