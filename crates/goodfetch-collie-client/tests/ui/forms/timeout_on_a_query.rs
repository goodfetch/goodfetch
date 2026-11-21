// A question that never waits is a `Query`, which has no `timeout` (ADR
// 0099): `page.read` answers at once.
use std::collections::BTreeMap;
use std::time::Duration;

fn read(page: &goodfetch_collie_client::Page) {
    let _read = page.read(BTreeMap::new()).timeout(Duration::from_secs(1));
}

fn main() {}
