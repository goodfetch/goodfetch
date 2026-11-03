use super::{CRAWLEE_VERSION, SCRAPY_VERSION, SPIDER_VERSION};

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

/// The trials declare the Crawlee its side installs: `@crawlee/cheerio`'s
/// exact version in `xtask/compare-crawlee/package.json`.
#[test]
fn the_declared_crawlee_is_the_one_its_side_pins() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../compare-crawlee/package.json")).unwrap();

    assert_eq!(
        manifest["dependencies"]["@crawlee/cheerio"].as_str(),
        Some(CRAWLEE_VERSION)
    );
}

/// The trials declare the Scrapy its side installs: the `scrapy==<version>`
/// of `xtask/compare-scrapy/pyproject.toml`.
#[test]
fn the_declared_scrapy_is_the_one_its_side_pins() {
    let manifest: toml::Table = include_str!("../../../compare-scrapy/pyproject.toml")
        .parse()
        .unwrap();
    let dependencies = manifest["project"]["dependencies"].as_array().unwrap();
    let pinned: Vec<_> = dependencies
        .iter()
        .filter_map(|dependency| dependency.as_str()?.strip_prefix("scrapy=="))
        .collect();

    assert_eq!(pinned, [SCRAPY_VERSION]);
}
