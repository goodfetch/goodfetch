//! The `sitemap-seed` family: a site whose robots.txt cross-submits a
//! sitemap on a partner host and disallows `/private/`, whose index lists
//! two sitemaps, one relative, whose sitemaps list its pages, a partner's
//! page and a page no rule would match, and whose pages answer on each of
//! its hosts, all on the port the request named.
//! Covers: story:2-33

use regex::Regex;

use crate::proving_ground::ProvingGround;

const SEED: u64 = 903;

/// Each `<loc>` of `xml`, in order.
fn locs(xml: &str) -> Vec<String> {
    Regex::new("<loc>([^<]+)</loc>")
        .unwrap()
        .captures_iter(xml)
        .map(|found| found[1].to_owned())
        .collect()
}

#[test]
fn the_robots_txt_cross_submits_the_partners_sitemap_and_the_sitemaps_list_the_pages() {
    let proving_ground = ProvingGround::start(SEED);
    let port = proving_ground.port().to_owned();
    let get = |host: &str, path: &str| proving_ground.get(&proving_ground.url(host, path));
    let on = |host: &str, path: &str| format!("http://{host}:{port}{path}");

    let robots = get("sitemap-seed.test", "/robots.txt");
    let read = [
        locs(&get("sitemap-seed.test", "/sitemap-index.xml").body),
        locs(&get("sitemap-seed.test", "/sitemaps/books.xml").body),
        locs(&get("sitemap-seed.test", "/sitemaps/more.xml").body),
        locs(&get("sitemap-partner.test", "/cross.xml").body),
    ];

    assert_eq!(
        robots.body,
        format!(
            "User-agent: *\nDisallow: /private/\n\nSitemap: {}\n",
            on("sitemap-partner.test", "/cross.xml")
        )
    );
    assert_eq!(
        read,
        [
            vec![
                on("sitemap-seed.test", "/sitemaps/books.xml"),
                "sitemaps/more.xml".to_owned()
            ],
            vec![
                on("sitemap-seed.test", "/books/1"),
                on("sitemap-seed.test", "/books/2"),
                on("sitemap-seed.test", "/authors/1"),
                on("sitemap-seed.test", "/about"),
                on("sitemap-partner.test", "/books/9"),
            ],
            vec![
                "../books/3".to_owned(),
                on("sitemap-seed.test", "/books/1"),
                on("sitemap-seed.test", "/authors/2"),
            ],
            vec![
                on("sitemap-seed.test", "/books/4"),
                on("sitemap-partner.test", "/books/5"),
            ],
        ]
    );
}

#[test]
fn each_page_answers_on_every_host_and_any_other_path_is_not_found() {
    let proving_ground = ProvingGround::start(SEED);
    let status =
        |host: &str, path: &str| proving_ground.get(&proving_ground.url(host, path)).status;

    let statuses = [
        status("sitemap-seed.test", "/books/1"),
        status("sitemap-partner.test", "/books/5"),
        status("sitemap-seed.test", "/authors/2"),
        status("sitemap-seed.test", "/about"),
        status("sitemap-seed.test", "/books/01"),
        status("sitemap-partner.test", "/robots.txt"),
    ];

    assert_eq!(statuses, [200, 200, 200, 200, 404, 404]);
}
