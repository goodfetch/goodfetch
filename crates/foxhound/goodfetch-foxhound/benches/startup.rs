//! The startup of Foxhound's executable (ADRs 0071 and 0094), under
//! gungraun's Callgrind: an Attached engine over stdio answering `hello` and
//! exiting at the end of its input, and `goodfetch-foxhound --version`.

use gungraun::prelude::*;
use gungraun::{Stdin, Stdio};

/// A Foxhound Client's `hello`, then the end of input.
const HELLO: &str = "benches/fixtures/hello.jsonl";

#[binary_benchmark]
fn attached_engine_answers_hello() -> gungraun::Command {
    gungraun::Command::new(env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .arg("attached-engine")
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
