// A call's timeout is a `Duration`, never a bare number whose unit a
// reader guesses (ADR 0080): Playwright's `30_000` does not compile.
fn goto(page: &goodfetch_collie_client::Page) {
    let _goto = page.goto("https://example.test/").timeout(30_000);
}

fn main() {}
