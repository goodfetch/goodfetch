use super::{
    CRAWLEE_RS_COMMIT, CRAWLEE_VERSION, PAGESIFT_VERSION, PLAYWRIGHT_VERSION,
    SCRAPY_PLAYWRIGHT_VERSION, SCRAPY_VERSION, SPIDER_VERSION, Work,
};

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

/// The trials declare the crawlee-rs commit its side builds: the `rev` of
/// `xtask/compare-crawlee-rs/Cargo.toml`'s `crawlee` dependency.
#[test]
fn the_declared_crawlee_rs_is_the_commit_its_side_pins() {
    let manifest: toml::Table = include_str!("../../../compare-crawlee-rs/Cargo.toml")
        .parse()
        .unwrap();

    assert_eq!(
        manifest["dependencies"]["crawlee"]["rev"].as_str(),
        Some(CRAWLEE_RS_COMMIT)
    );
}

/// The trials declare the pagesift its side builds: the exact version
/// `xtask/compare-pagesift/Cargo.toml` holds each of pagesift's crates to.
#[test]
fn the_declared_pagesift_is_the_version_its_side_pins() {
    let manifest: toml::Table = include_str!("../../../compare-pagesift/Cargo.toml")
        .parse()
        .unwrap();
    let pinned = |name: &str| manifest["dependencies"][name].as_str().map(str::to_owned);

    let exact = Some(format!("={PAGESIFT_VERSION}"));
    assert_eq!(
        ["pagesift", "pagesift-sys", "pagesift-cdp", "pagesift-net"].map(pinned),
        [exact.clone(), exact.clone(), exact.clone(), exact]
    );
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

/// The trials declare the Playwright both sides' headless browser crawls
/// install, and the scrapy-playwright Scrapy's does: each side's exact pin.
#[test]
fn the_declared_playwright_is_the_one_each_side_pins() {
    let package: serde_json::Value =
        serde_json::from_str(include_str!("../../../compare-crawlee/package.json")).unwrap();
    let project: toml::Table = include_str!("../../../compare-scrapy/pyproject.toml")
        .parse()
        .unwrap();
    let pinned = |name: &str| -> Vec<String> {
        project["project"]["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|dependency| {
                Some(
                    dependency
                        .as_str()?
                        .strip_prefix(&format!("{name}=="))?
                        .to_owned(),
                )
            })
            .collect()
    };

    assert_eq!(
        (
            package["dependencies"]["playwright"].as_str(),
            pinned("playwright"),
            pinned("scrapy-playwright"),
        ),
        (
            Some(PLAYWRIGHT_VERSION),
            vec![PLAYWRIGHT_VERSION.to_owned()],
            vec![SCRAPY_PLAYWRIGHT_VERSION.to_owned()],
        )
    );
}

/// What a strace run's later runs are skipped on: whether it fetched the
/// site's own pages, each once or more, and no other.
#[test]
fn a_crawl_fetched_the_site_when_it_reached_each_of_its_pages_and_no_other() {
    let whole = Work {
        pages: 4,
        requests: 4,
        ..Work::default()
    };

    assert!(whole.is_the_site_of(4));
    assert!(
        Work {
            requests: 5,
            ..whole.clone()
        }
        .is_the_site_of(4)
    );
    assert!(
        !Work {
            pages: 3,
            missing: vec!["/sites/4/4".to_owned()],
            ..whole.clone()
        }
        .is_the_site_of(4)
    );
    assert!(
        !Work {
            extra: vec!["/sites/4/9".to_owned()],
            ..whole
        }
        .is_the_site_of(4)
    );
}
