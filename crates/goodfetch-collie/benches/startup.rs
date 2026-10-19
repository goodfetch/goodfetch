//! The startup of Collie's executable (ADRs 0071 and 0094), under gungraun's
//! Callgrind: `goodfetch-collie --version`. An Attached engine answering
//! `hello` over stdio (`benches/fixtures/hello.jsonl`) spread 14.78% across
//! its 20 admission runs, not under 0.5%, so it waits outside the gate until
//! it is fixed (an open criterion of #214).

use gungraun::prelude::*;

#[binary_benchmark]
fn version() -> gungraun::Command {
    gungraun::Command::new(env!("CARGO_BIN_EXE_goodfetch-collie"))
        .arg("--version")
        .build()
}

binary_benchmark_group!(name = startup, benchmarks = [version]);

main!(binary_benchmark_groups = startup);
