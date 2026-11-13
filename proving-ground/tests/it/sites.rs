//! The `scale.test/sites/<size>/` sites (ADR 0134): a site of any size,
//! each page linking only its own site's, which `just compare`
//! crawls with Foxhound and Spider at 1,000 pages. They belong to no task
//! and declare no Feature (#1574).

use std::collections::BTreeSet;

use crate::crawler::Crawler;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 164;

/// The pages a crawl from `/sites/<size>/1` reached, as sorted URLs.
fn site_crawl(proving_ground: &ProvingGround, size: u64) -> (usize, BTreeSet<String>) {
    let origin = format!("http://scale.test:{}", proving_ground.port());
    let mut crawler = Crawler::new(proving_ground);
    // The site's pages have no script, so the crawl reads only `<a href>`s.
    crawler.follow_scripts = false;
    let pages = crawler.crawl(&format!("{origin}/sites/{size}/1"));
    let urls = pages.iter().map(|page| page.url.clone()).collect();
    (pages.len(), urls)
}

/// The URLs of the pages of the site of `size` pages.
fn site_pages(proving_ground: &ProvingGround, size: u64) -> BTreeSet<String> {
    (1..=size)
        .map(|page| {
            format!(
                "http://scale.test:{}/sites/{size}/{page}",
                proving_ground.port()
            )
        })
        .collect()
}

/// The 1,000-page site Spider's comparison crawls (ADR 0134): a crawl of
/// its origin from `/sites/1000/1` reaches each of its pages once, each
/// answered `200` with no redirect, and nothing else; the page past its
/// last is none.
#[test]
fn a_crawl_of_the_thousand_page_site_reaches_each_page_once_and_nothing_else() {
    let proving_ground = ProvingGround::start(SEED);

    let crawled = site_crawl(&proving_ground, 1000);
    let past = proving_ground.get(&format!(
        "http://scale.test:{}/sites/1000/1001",
        proving_ground.port()
    ));

    assert_eq!(
        (crawled, past.status),
        ((1000, site_pages(&proving_ground, 1000)), 404)
    );
}

/// A site's size is in its path, so a smaller site holds fewer pages,
/// each linking only its own site's.
#[test]
fn a_site_of_another_size_holds_that_many_pages() {
    let proving_ground = ProvingGround::start(SEED);

    let crawled = site_crawl(&proving_ground, 25);

    assert_eq!(crawled, (25, site_pages(&proving_ground, 25)));
}

/// The `href`s of a page's links, as its markup writes them.
fn hrefs(body: &str) -> Vec<String> {
    regex::Regex::new(r#"<a href="([^"]*)""#)
        .expect("a valid expression")
        .captures_iter(body)
        .map(|captures| captures[1].to_owned())
        .collect()
}

/// The multi-domain site's page `page` of `size`, on the host of its
/// domain: `polite-<d>.localhost`, `d` being `page` modulo 50, from 1.
fn polite_url(proving_ground: &ProvingGround, size: u64, page: u64) -> String {
    format!(
        "http://polite-{}.localhost:{}/sites/{size}/{page}",
        (page - 1) % 50 + 1,
        proving_ground.port()
    )
}

/// The polite scenario's multi-domain site (#1903): a crawl from its first
/// page that follows every link across its hosts and obeys each host's
/// robots.txt reaches each of its 1,000 pages once, each on its own
/// domain's host, 20 on each of 50 domains, and every robots.txt holds
/// back the one other page they link, so a crawl that obeys none fetches
/// a page of no site.
#[test]
fn a_polite_crawl_of_the_multi_domain_site_reaches_each_page_on_its_own_domain() {
    let proving_ground = ProvingGround::start(SEED);
    let first = polite_url(&proving_ground, 1000, 1);
    let mut seen = BTreeSet::from([first.clone()]);
    let mut queue = vec![first];
    let mut held_back = BTreeSet::new();
    let mut robots = BTreeSet::new();
    while let Some(url) = queue.pop() {
        let page = proving_ground.get(&url);
        assert_eq!(page.status, 200, "{url}");
        let origin = url[..url.find("/sites/").expect("a site page")].to_owned();
        robots.insert(proving_ground.get(&format!("{origin}/robots.txt")).body);
        for href in hrefs(&page.body) {
            let link = if href.starts_with('/') {
                format!("{origin}{href}")
            } else {
                href
            };
            if link.ends_with("/private") {
                held_back.insert(link);
            } else if seen.insert(link.clone()) {
                queue.push(link);
            }
        }
    }
    let domains = seen
        .iter()
        .map(|url| {
            url.split_once("://")
                .expect("a URL")
                .1
                .split(':')
                .next()
                .expect("a host")
        })
        .collect::<BTreeSet<&str>>()
        .len();
    let private = proving_ground.get(&held_back.first().expect("a private page").clone());

    assert_eq!(
        (seen, domains, held_back.len(), robots, private.status),
        (
            (1..=1000)
                .map(|page| polite_url(&proving_ground, 1000, page))
                .collect(),
            50,
            50,
            BTreeSet::from(["User-agent: *\nDisallow: /sites/*/private\n".to_owned()]),
            200
        )
    );
}

/// A page of the multi-domain site asked for on another domain's host is
/// none, so a crawl that sends a page to the wrong host fetches no page.
#[test]
fn a_multi_domain_page_on_another_domains_host_is_none() {
    let proving_ground = ProvingGround::start(SEED);
    let port = proving_ground.port();

    let answers = [
        proving_ground
            .get(&polite_url(&proving_ground, 1000, 3))
            .status,
        proving_ground
            .get(&format!("http://polite-4.localhost:{port}/sites/1000/3"))
            .status,
        proving_ground
            .get(&format!("http://polite-51.localhost:{port}/sites/1000/51"))
            .status,
    ];

    assert_eq!(answers, [200, 404, 404]);
}

/// The headless browser scenario's script-rendered site (#1903): a page's
/// markup links only home, so a crawl that runs no script reaches the
/// first page alone, while a browser sees its links to its ten children.
#[test]
fn the_script_rendered_sites_links_are_seen_only_by_a_browser() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = format!("http://rendered.localhost:{}", proving_ground.port());
    let page = proving_ground.get(&format!("{origin}/sites/1000/2"));
    let mut plain = Crawler::new(&proving_ground);
    plain.follow_scripts = false;
    let reached = plain.crawl(&format!("{origin}/sites/25/1")).len();

    let rendered = crate::browser::run(
        "comparison",
        "links",
        &format!("{origin}/sites/1000/2"),
        serde_json::json!({}),
    );

    assert_eq!(
        (page.status, hrefs(&page.body), reached, &rendered["links"]),
        (
            200,
            vec!["/sites/1000/1".to_owned()],
            1,
            &serde_json::json!(
                std::iter::once(format!("{origin}/sites/1000/1"))
                    .chain((12..=21).map(|child| format!("{origin}/sites/1000/{child}")))
                    .collect::<Vec<_>>()
            )
        )
    );
}

/// The realistic-page site (#2316): the scale site's tree, each page
/// carrying an article of tens of kilobytes and 30 to 100 links, as a real
/// site's does (crawlee-rs publishes its figures on pages of 58 KB). A
/// page's header, menu and footer repeat the same links, and every page
/// whose number ends in 7 is linked only through its old path, which
/// answers `301` to it. A crawl that follows redirects still reaches each
/// page once.
#[test]
fn the_realistic_sites_pages_are_tens_of_kilobytes_with_30_to_100_links() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = format!("http://realistic.localhost:{}", proving_ground.port());
    let pages: Vec<String> = (1..=1000)
        .map(|page| proving_ground.get(&format!("{origin}/sites/1000/{page}")))
        .map(|answer| {
            assert_eq!(answer.status, 200);
            answer.body
        })
        .collect();
    let mut sizes: Vec<usize> = pages.iter().map(String::len).collect();
    sizes.sort_unstable();
    let links: Vec<Vec<String>> = pages.iter().map(|body| hrefs(body)).collect();
    let outside: Vec<usize> = links
        .iter()
        .map(Vec::len)
        .filter(|count| !(30..=100).contains(count))
        .collect();
    let repeated = links
        .iter()
        .filter(|hrefs| hrefs.iter().collect::<BTreeSet<_>>().len() < hrefs.len())
        .count();
    let moved = proving_ground.get(&format!("{origin}/sites/1000/moved/7"));
    let mut crawler = Crawler::new(&proving_ground);
    crawler.follow_scripts = false;
    let urls: Vec<String> = crawler
        .crawl(&format!("{origin}/sites/1000/1"))
        .into_iter()
        .map(|page| page.url)
        .collect();
    let reached: BTreeSet<String> = urls.iter().cloned().collect();

    assert!(
        sizes[sizes.len() / 2] >= 20_000,
        "median {}",
        sizes[sizes.len() / 2]
    );
    assert_eq!(
        (
            outside,
            repeated,
            moved.status,
            moved.header("location"),
            urls.len(),
            reached,
        ),
        (
            Vec::new(),
            1000,
            301,
            Some("/sites/1000/7"),
            1000,
            (1..=1000)
                .map(|page| format!("{origin}/sites/1000/{page}"))
                .collect(),
        )
    );
}

/// White space collapsed, Unicode's included, and trimmed.
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The article site (#2136), for the extraction scenario: the scale site's
/// tree, each page an article of a few paragraphs inside `<main>`, with a
/// header, a nav, an aside and a footer around it, each holding a marker of
/// its own, and one ad slot among the paragraphs that EasyList's
/// `##.ad-slot` matches. Some titles carry entities and extra white space.
/// `GET /article-answers/<size>` holds each page's right answer: its title,
/// entity-decoded and white space collapsed, and its paragraphs, with the
/// boilerplate's markers and the ad's.
#[test]
fn the_article_sites_pages_hold_an_article_among_boilerplate_and_an_ad_and_its_answers_are_right() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = format!("http://article.localhost:{}", proving_ground.port());
    let answers = proving_ground.control("/article-answers/1000").json();
    let markers: Vec<&str> = answers["boilerplate"]
        .as_array()
        .expect("the boilerplate's markers")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let ad = answers["ad"].as_str().expect("the ad's marker");
    let mut crawler = Crawler::new(&proving_ground);
    crawler.follow_scripts = false;
    let reached: BTreeSet<String> = crawler
        .crawl(&format!("{origin}/sites/1000/1"))
        .into_iter()
        .map(|page| page.url)
        .collect();
    let mut sizes = Vec::new();
    let mut raw_titles = Vec::new();
    let mut wrong = Vec::new();

    for page in 1..=1000_usize {
        let body = proving_ground
            .get(&format!("{origin}/sites/1000/{page}"))
            .body;
        sizes.push(body.len());
        raw_titles.push(
            regex::Regex::new("<title>([^<]*)</title>")
                .expect("a valid expression")
                .captures(&body)
                .expect("a title")[1]
                .to_owned(),
        );
        let document = dom_query::Document::from(body.as_str());
        let text = |selector: &str| collapsed(&document.select(selector).text());
        let paragraphs: Vec<String> = document
            .select("main > article > p")
            .iter()
            .map(|paragraph| collapsed(&paragraph.text()))
            .collect();
        let main = text("main");
        let around = [
            "body > header",
            "body > nav",
            "body > aside",
            "body > footer",
        ]
        .map(text);
        let answer = &answers["pages"][page - 1];
        let right = answer["path"] == format!("/sites/1000/{page}")
            && answer["title"] == text("title")
            && answer["paragraphs"] == serde_json::json!(paragraphs)
            && (3..=8).contains(&paragraphs.len())
            && markers.len() == 4
            && markers
                .iter()
                .zip(&around)
                .all(|(marker, text)| text.contains(marker) && !main.contains(marker))
            && text("main > article > .ad-slot").contains(ad)
            && !paragraphs.iter().any(|paragraph| paragraph.contains(ad));
        if !right {
            wrong.push(page);
        }
    }

    sizes.sort_unstable();
    let past = proving_ground.control("/article-answers/10001");
    assert_eq!(
        (
            wrong,
            reached,
            answers["pages"].as_array().map(Vec::len),
            past.status
        ),
        (
            Vec::new(),
            (1..=1000)
                .map(|page| format!("{origin}/sites/1000/{page}"))
                .collect(),
            Some(1000),
            404
        )
    );
    assert!(
        (2_000..=8_000).contains(&sizes[500]),
        "median {}",
        sizes[500]
    );
    assert!(raw_titles.iter().any(|title| title.contains("&amp;")));
    assert!(raw_titles.iter().any(|title| title.contains("&nbsp;")));
    assert!(raw_titles.iter().any(|title| title.contains("  ")));
}

/// The comparison's sites are observed, so `just compare`'s same-work check
/// reads what each tool fetched from them.
#[test]
fn the_comparisons_sites_are_observed() {
    let proving_ground = ProvingGround::start(SEED);
    let port = proving_ground.port();
    proving_ground.get(&polite_url(&proving_ground, 1000, 7));
    proving_ground.get(&format!("http://rendered.localhost:{port}/sites/1000/7"));
    proving_ground.get(&format!("http://realistic.localhost:{port}/sites/1000/7"));
    proving_ground.get(&format!("http://article.localhost:{port}/sites/1000/7"));

    let observations = proving_ground.observations();

    let hosts: BTreeSet<&str> = observations["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .filter(|request| request["target"] == "/sites/1000/7")
        .filter_map(|request| request["host"].as_str())
        .collect();
    assert_eq!(
        hosts,
        BTreeSet::from([
            "article.localhost",
            "polite-7.localhost",
            "realistic.localhost",
            "rendered.localhost"
        ])
    );
}
