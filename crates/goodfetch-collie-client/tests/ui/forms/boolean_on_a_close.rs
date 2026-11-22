// A boolean is never positional (ADR 0099), on the blocking twin too:
// `page.close`'s `run_before_unload` is a builder method.
fn close(page: &goodfetch_collie_client::blocking::Page) {
    let _close = page.close(true);
}

fn main() {}
