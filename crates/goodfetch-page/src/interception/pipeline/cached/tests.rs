//! What the cache stage sends without a browser: the conditions a stale
//! entry's revalidation carries.

use super::conditional;
use crate::cdp::Header;

fn header(name: &str, value: &str) -> Header {
    Header {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

/// A stale entry's revalidation carries the cache's conditions in place of
/// the page's own, whatever their case, and keeps the other headers.
#[test]
fn a_revalidation_carries_the_caches_conditions_in_place_of_the_pages() {
    let sent = vec![
        header("Accept", "text/css"),
        header("If-None-Match", "\"page\""),
    ];

    let carried = conditional(
        sent,
        &[("if-none-match".to_owned(), "\"stored\"".to_owned())],
    );

    assert_eq!(
        carried
            .iter()
            .map(|header| (header.name.as_str(), header.value.as_str()))
            .collect::<Vec<_>>(),
        [("Accept", "text/css"), ("if-none-match", "\"stored\"")]
    );
}
