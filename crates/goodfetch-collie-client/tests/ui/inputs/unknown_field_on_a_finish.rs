// An input names only the fields its operation takes (ADR 0082): a field
// the catalog does not give `run.finish` has no setter on its builder.
fn finish(run: &goodfetch_collie_client::Run) {
    let _finish = run.finish().carbon(1.5);
}

fn main() {}
