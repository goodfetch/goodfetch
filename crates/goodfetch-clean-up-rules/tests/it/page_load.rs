//! `judge` on the Proving ground's `page-load` pages (issue 2319), which
//! `xtask compare`'s page-load cell loads: the embedded Filter lists, at
//! their defaults, block none of `/plain`'s 150 subresources, and of
//! `/ads`' exactly the 30 on its six ad hosts.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use goodfetch_clean_up_rules::matching::{Choices, Request, ResourceKind, Verdict, judge};
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_test_support::{ProvingGround, TestEnv};

const SEED: u64 = 2319;
const HOST: &str = "page-load.test";

static LISTS: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("the embedded Filter lists load"));

/// What the lists did with one page's subresources.
#[derive(Debug, PartialEq, Eq)]
struct Judged {
    /// Its subresources, each once.
    subresources: usize,
    /// The hosts of those the lists block.
    blocked_hosts: BTreeSet<String>,
    /// Those the lists block.
    blocked: usize,
    /// Those on its own host the lists let through.
    own_allowed: usize,
}

/// Each URL `html` asks for: an attribute's `src` or `href`, or a style
/// sheet's `url(…)`, made absolute on `origin`.
fn subresources(html: &str, origin: &str) -> BTreeSet<String> {
    ["src=\"", "href=\"", "url(\""]
        .into_iter()
        .flat_map(|opening| {
            html.match_indices(opening).filter_map(move |(at, _)| {
                let rest = &html[at + opening.len()..];
                rest.split_once('"').map(|(url, _)| url)
            })
        })
        .map(|url| {
            if url.starts_with('/') {
                format!("{origin}{url}")
            } else {
                url.to_owned()
            }
        })
        .collect()
}

fn kind(url: &str) -> ResourceKind {
    match url.rsplit_once('.').map(|(_, extension)| extension) {
        Some("js") => ResourceKind::Script,
        Some("css") => ResourceKind::Stylesheet,
        Some("gif") => ResourceKind::Image,
        Some("woff2") => ResourceKind::Font,
        other => panic!("a subresource of an unknown kind: {other:?} in {url}"),
    }
}

/// `path` on `page-load.test`, each subresource judged as a browser with
/// the default Filter lists would ask for it.
fn judged(proving_ground: &ProvingGround, path: &str) -> Judged {
    let origin = proving_ground.url(HOST, "");
    let page_url = format!("{origin}{path}");
    let html = ureq::get(format!("http://{}{path}", proving_ground.address()))
        .header(
            "Host",
            format!("{HOST}:{}", proving_ground.address().port()),
        )
        .call()
        .and_then(|mut response| response.body_mut().read_to_string())
        .expect("the Proving ground serves the page");
    let choices = Choices {
        ads: true,
        trackers: true,
        annoyances: true,
        ..Choices::default()
    };
    let urls = subresources(&html, &origin);
    let mut judged = Judged {
        subresources: urls.len(),
        blocked_hosts: BTreeSet::new(),
        blocked: 0,
        own_allowed: 0,
    };
    for url in &urls {
        let host = url
            .strip_prefix("http://")
            .and_then(|rest| rest.split([':', '/']).next())
            .expect("every subresource is http");
        let verdict = judge(
            &Request {
                url,
                source_url: &page_url,
                kind: kind(url),
                method: "GET",
            },
            &NeverBlock::new(),
            Some(&LISTS),
            &choices,
        );
        if matches!(verdict, Verdict::Blocked(_) | Verdict::StandIn { .. }) {
            judged.blocked += 1;
            judged.blocked_hosts.insert(host.to_owned());
        } else if host == HOST {
            judged.own_allowed += 1;
        }
    }
    judged
}

#[test]
fn the_filter_lists_block_none_of_the_plain_pages_150_subresources() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).expect("the Proving ground starts");

    assert_eq!(
        judged(&proving_ground, "/plain"),
        Judged {
            subresources: 150,
            blocked_hosts: BTreeSet::new(),
            blocked: 0,
            own_allowed: 150,
        }
    );
}

#[test]
fn the_filter_lists_block_the_30_subresources_on_the_ad_pages_six_ad_hosts() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).expect("the Proving ground starts");

    assert_eq!(
        judged(&proving_ground, "/ads"),
        Judged {
            subresources: 150,
            blocked_hosts: [
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
            blocked: 30,
            own_allowed: 120,
        }
    );
}
