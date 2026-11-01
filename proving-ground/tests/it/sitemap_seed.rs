//! The `sitemap-seed` family: a site whose robots.txt cross-submits a
//! sitemap on a partner host and disallows `/private/`, whose index lists
//! two sitemaps, one relative, whose sitemaps list its pages, a partner's
//! page and a page no rule would match, one listing a page robots.txt
//! disallows, and whose pages answer on each of
//! its hosts, all on the port the request named; and an index of a plain
//! and a gzip sitemap beside the files no crawl should read: one past
//! sitemaps.org's 50 MB, a decompression bomb, broken XML, a 500, a 404
//! and the `sitemaps` family's index past its 50,000 URLs.
//! Covers: story:2-33

use std::io::Read as _;

use regex::Regex;

use crate::proving_ground::ProvingGround;

const SEED: u64 = 903;

/// Each `<loc>` of `xml`, in order.
fn locs(xml: &str) -> Vec<String> {
    Regex::new("<loc>([^<]+)</loc>")
        .expect("the pattern reads")
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
        locs(&get("sitemap-seed.test", "/sitemaps/private.xml").body),
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
            vec![
                on("sitemap-seed.test", "/books/20"),
                on("sitemap-seed.test", "/private/books/21"),
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
        status("sitemap-seed.test", "/private/books/21"),
        status("sitemap-seed.test", "/books/01"),
        status("sitemap-partner.test", "/robots.txt"),
    ];

    assert_eq!(statuses, [200, 200, 200, 200, 200, 404, 404]);
}

fn gunzip(body: &[u8]) -> Vec<u8> {
    let mut text = Vec::new();
    flate2::read::GzDecoder::new(body)
        .read_to_end(&mut text)
        .expect("the body is gzip");
    text
}

/// The limits index lists a plain sitemap and a gzip one, each of two
/// books, beside a sitemap one byte past sitemaps.org's 50 MB, a small
/// gzip file decoding to 10 MiB, broken XML, a 500, a 404 and the
/// `sitemaps` family's index of 50,048 sitemaps.
#[test]
fn the_limits_index_lists_good_sitemaps_beside_the_files_past_the_limits() {
    let proving_ground = ProvingGround::start(SEED);
    let port = proving_ground.port().to_owned();
    let raw = |path: &str| proving_ground.get_raw(&proving_ground.url("sitemap-seed.test", path));
    let on = |host: &str, path: &str| format!("http://{host}:{port}{path}");

    let index = raw("/limits-index.xml");
    let plain = raw("/sitemaps/plain.xml");
    let packed = raw("/sitemaps/packed.xml.gz");
    let oversized = raw("/sitemaps/oversized.xml");
    let bomb = raw("/sitemaps/bomb.xml.gz");
    let broken = raw("/sitemaps/broken.xml");

    assert_eq!(
        (
            locs(&String::from_utf8(index.body).unwrap()),
            locs(&String::from_utf8(plain.body).unwrap()),
            packed.content_type.as_deref(),
            locs(&String::from_utf8(gunzip(&packed.body)).unwrap()),
            oversized.body.len(),
            locs(&String::from_utf8(oversized.body).unwrap()),
            bomb.body.len() < 64 * 1024,
            gunzip(&bomb.body).len(),
            broken.status,
            raw("/sitemaps/error.xml").status,
            raw("/sitemaps/missing.xml").status,
        ),
        (
            vec![
                on("sitemap-seed.test", "/sitemaps/plain.xml"),
                on("sitemap-seed.test", "/sitemaps/packed.xml.gz"),
                on("sitemaps.test", "/sitemap-index.xml"),
                on("sitemap-seed.test", "/sitemaps/oversized.xml"),
                on("sitemap-seed.test", "/sitemaps/bomb.xml.gz"),
                on("sitemap-seed.test", "/sitemaps/broken.xml"),
                on("sitemap-seed.test", "/sitemaps/error.xml"),
                on("sitemap-seed.test", "/sitemaps/missing.xml"),
            ],
            vec![
                on("sitemap-seed.test", "/books/11"),
                on("sitemap-seed.test", "/books/12"),
            ],
            Some("application/gzip"),
            vec![
                on("sitemap-seed.test", "/books/13"),
                on("sitemap-seed.test", "/books/14"),
            ],
            52_428_801,
            vec![on("sitemap-seed.test", "/books/15")],
            true,
            10 * 1024 * 1024,
            200,
            500,
            404,
        )
    );
}
