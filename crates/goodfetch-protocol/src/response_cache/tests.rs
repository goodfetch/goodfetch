use super::{CACHE_MODE, CacheMode};
use crate::catalog::ValueKind;

/// Covers: story:6-38
#[test]
fn the_cache_is_off_by_default() {
    assert_eq!((CACHE_MODE.default)(), serde_json::json!("off"));
    assert_eq!(CacheMode::parse("off"), Some(CacheMode::Off));
}

#[test]
fn the_setting_offers_every_mode_this_minor_builds_and_no_other() {
    let ValueKind::Choice(values) = CACHE_MODE.kind else {
        panic!("`cache.mode` is a choice");
    };
    let modes: Vec<&str> = CacheMode::ALL.into_iter().map(CacheMode::as_str).collect();
    assert_eq!(values, modes.as_slice());
    assert_eq!(modes, ["off", "revalidate", "replay"]);
}
