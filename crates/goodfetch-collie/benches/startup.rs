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
//! input to blocking threads. On two workers and four blocking threads, the
//! defaults then, how much of that work ran before the end of its input
//! stopped the engine depended on timing, and Callgrind counts the probe's
//! forked child as a process of its own, so the count spread 14.78% across 20
//! runs (#2752).
//! On one blocking thread that work runs one piece at a time, and with an
//! hour between samples a slow run takes no second sample. A home of its own
//! also keeps the run off the user's Goodfetch home, whose list of installed
//! executables, which the engine reads and rewrites as it starts, every
//! executable run on the machine changes.
//!
//! On one thread of each kind, 2 of 20 runs still stood out, the spread
//! 1.63% (#2752). In one, the engine's last two threads ran their exit, each
//! about 40,000 instructions of mimalloc's teardown under glibc's
//! `__nptl_deallocate_tsd`, before the process ended, which on other runs
//! they do not reach. Callgrind leaves that function out with
//! `--toggle-collect`, which also starts every thread with counting off, and
//! gungraun passes its toggles after every other option, so
//! `--collect-atstart=yes` cannot turn it back on. Counting is turned on in
//! the functions that hold all of a thread's work instead: the loader's
//! `_dl_start` and `_dl_init`, then `__libc_start_main` on the main thread,
//! and `start_thread` on each other thread, where `__nptl_deallocate_tsd`
//! turns it off again. Only the loader's entry, about 40 instructions, goes
//! uncounted. The spread fell to 0.02% across 20 runs of one build.
//! In the other, the probe forked early: the forked child's count starts
//! from Callgrind's counts of every thread at the fork, so an early fork
//! counts fewer instructions. No Callgrind option leaves those inherited
//! counts out; a fork early by 160,000 instructions came once in more
//! than 80 runs, the other early ones by under 30,000.

use std::path::Path;

use goodfetch_protocol::formats::USER_CONFIGURATION;
use goodfetch_protocol::settings::HOME_VARIABLE;
use goodfetch_test_client::HomeLayout;
use gungraun::prelude::*;
use gungraun::{Callgrind, Sandbox, Stdin, Stdio};

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
    config = BinaryBenchmarkConfig::default()
        .sandbox(Sandbox::new(true))
        .tool(Callgrind::with_args([
            "--toggle-collect=_dl_start",
            "--toggle-collect=_dl_init",
            "--toggle-collect=__libc_start_main*",
            "--toggle-collect=start_thread",
            "--toggle-collect=__nptl_deallocate_tsd",
        ]))
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
