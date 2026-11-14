//! The `page-load` pages (issue 2319): each serves its 150 subresources,
//! `/plain` all on its own host and `/ads` 30 of them on six ad hosts, as
//! the comparison's page-load cell counts them. That the embedded Filter
//! lists block those six hosts is goodfetch-clean-up-rules' test.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::proving_ground::ProvingGround;
use crate::raw_clients;

const SEED: u64 = 2319;
const HOST: &str = "page-load.test";

/// A subresource attribute or a style sheet's `url(…)`.
static SUBRESOURCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:src|href)="([^"]+)"|url\("([^"]+)"\)"#).expect("the pattern compiles")
});

/// What one page's subresources came to.
#[derive(Debug, PartialEq, Eq)]
struct Counted {
    /// The page's status.
    status: Option<u16>,
    /// Its subresources, each once.
    subresources: usize,
    /// Those that answered 200.
    served: usize,
    /// Those on its own host.
    own: usize,
    /// The other hosts its subresources are on.
    other_hosts: BTreeSet<String>,
}

/// `path` on `page-load.test`, its subresources fetched as a browser would
/// ask for them.
fn counted(proving_ground: &ProvingGround, path: &str) -> Counted {
    let page = raw_clients::get(proving_ground, HOST, path, &[]);
    let html = String::from_utf8_lossy(page.body()).into_owned();
    let urls: BTreeSet<(String, String)> = SUBRESOURCE
        .captures_iter(&html)
        .filter_map(|found| found.get(1).or_else(|| found.get(2)))
        .map(|found| {
            let url = found.as_str();
            match url.strip_prefix("http://") {
                Some(rest) => {
                    let (authority, path) =
                        rest.split_once('/').expect("every subresource has a path");
                    let host = authority
                        .split_once(':')
                        .map_or(authority, |(host, _)| host);
                    (host.to_owned(), format!("/{path}"))
                }
                None => (HOST.to_owned(), url.to_owned()),
            }
        })
        .collect();
    let mut counted = Counted {
        status: page.status(),
        subresources: urls.len(),
        served: 0,
        own: 0,
        other_hosts: BTreeSet::new(),
    };
    for (host, path) in &urls {
        if raw_clients::get(proving_ground, host, path, &[]).status() == Some(200) {
            counted.served += 1;
        }
        if host == HOST {
            counted.own += 1;
        } else {
            counted.other_hosts.insert(host.clone());
        }
    }
    counted
}

#[test]
fn the_plain_page_serves_150_subresources_all_on_its_own_host() {
    let proving_ground = ProvingGround::start(SEED);

    assert_eq!(
        counted(&proving_ground, "/plain"),
        Counted {
            status: Some(200),
            subresources: 150,
            served: 150,
            own: 150,
            other_hosts: BTreeSet::new(),
        }
    );
}

#[test]
fn the_page_with_ads_serves_30_of_its_150_from_six_ad_hosts() {
    let proving_ground = ProvingGround::start(SEED);

    assert_eq!(
        counted(&proving_ground, "/ads"),
        Counted {
            status: Some(200),
            subresources: 150,
            served: 150,
            own: 120,
            other_hosts: [
                "ads.pubmatic.com",
                "fastlane.rubiconproject.com",
                "ib.adnxs.com",
                "js-sec.casalemedia.com",
                "u.openx.net",
                "z.moatads.com",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    );
}
