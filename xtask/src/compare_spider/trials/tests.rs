use super::SPIDER_VERSION;

/// The Spider version Spider's side pins in its manifest, `=<version>`.
fn pinned(manifest: &str) -> Option<String> {
    let manifest: toml::Table = manifest.parse().ok()?;
    let requirement = manifest
        .get("dependencies")?
        .get("spider")?
        .get("version")?;
    Some(requirement.as_str()?.strip_prefix('=')?.to_owned())
}

/// The trials declare the Spider their side really builds: a bump of
/// `xtask/compare-spider/Cargo.toml`'s pin alone fails here, so the report
/// never names another Spider than the one that ran.
#[test]
fn the_declared_spider_is_the_one_its_side_pins() {
    let manifest = include_str!("../../../compare-spider/Cargo.toml");

    assert_eq!(pinned(manifest).as_deref(), Some(SPIDER_VERSION));
}
