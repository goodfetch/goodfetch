// A boolean is never positional (ADR 0099): `run.abort`'s `forwarded` is
// a builder method, so passing it as an argument fails to compile.
fn abort(run: &goodfetch_collie_client::Run) {
    let _abort = run.abort(true);
}

fn main() {}
