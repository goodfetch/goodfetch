//! A test module of its own file: no use of the driver's either.

use goodfetch_cdp::generated::page;

#[test]
fn reloads_again() {
    let _reload = page::ReloadParams::default();
}
