//! The `extraction` family (ADR 0025's Extraction cases, J-011): product
//! pages with schema.org data and Open Graph metadata, a shop whose
//! products only its hidden API holds, pages whose class names churn,
//! the same kind of entity under small structure changes, a huge page,
//! and one product's regional variants. Each known-right result reads the
//! pages the way a careful tool would; each false success reads them the
//! way a common shortcut does.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};
use crate::reading::{every, first, origin, start_url, task};

const SEED: u64 = 165;

/// The page at `path` on the task's own host name.
fn page_of(proving_ground: &ProvingGround, task: &Value, path: &str) -> String {
    let answer = proving_ground.get(&format!("{}{path}", origin(start_url(task))));
    assert_eq!(answer.status, 200, "{path}: {}", answer.body);
    answer.body
}

/// The pages each of the start page's `href="(prefix…)"` links reaches.
fn linked(proving_ground: &ProvingGround, task: &Value, prefix: &str) -> Vec<String> {
    let index = proving_ground.get(start_url(task)).body;
    every(&format!(r#"href="({prefix}[^"]+)""#), &index)
        .iter()
        .map(|path| page_of(proving_ground, task, path))
        .collect()
}

/// The ID, name and price a product page states with labels.
fn labelled(page: &str) -> Value {
    let plain = regex::Regex::new("<[^>]+>")
        .expect("a valid pattern")
        .replace_all(page, "");
    let name = regex::Regex::new(
        r"<h[12][^>]*>([^<]+)</h[12]>|Name</th><td>([^<]+)</td>|Name</dt><dd>([^<]+)</dd>",
    )
    .expect("a valid pattern")
    .captures(page)
    .and_then(|found| found.iter().skip(1).flatten().next())
    .expect("the page names its product")
    .as_str()
    .to_owned();
    json!({
        "id": first(r"SKU:?\s*(X-[0-9]{4})", &plain),
        "name": name,
        "price": first(r"Price:?\s*\$([0-9.]+)", &plain),
    })
}

mod metadata {
    use super::*;

    /// A product page's Record, from its JSON-LD and Open Graph metadata.
    fn read(page: &str) -> Value {
        let data: Value = serde_json::from_str(&first(
            r#"(?s)<script type="application/ld\+json">(.*?)</script>"#,
            page,
        ))
        .expect("the JSON-LD is JSON");
        json!({
            "id": data["sku"],
            "name": data["name"],
            "price": data["offers"]["price"],
            "currency": data["offers"]["priceCurrency"],
            "brand": data["brand"]["name"],
            "gtin": data["gtin13"],
            "rating": data["aggregateRating"]["ratingValue"],
            "description": first(r#"<meta property="og:description" content="([^"]+)">"#, page),
        })
    }

    fn known_right(proving_ground: &ProvingGround) -> Vec<Value> {
        let task = task(proving_ground, "extraction/1");
        linked(proving_ground, &task, "/products/")
            .iter()
            .map(|page| read(page))
            .collect()
    }

    #[test]
    fn product_pages_read_from_their_json_ld_and_open_graph_are_verified() {
        let proving_ground = ProvingGround::start(SEED);
        let records = known_right(&proving_ground);

        let answer = proving_ground.judge(&submission("extraction/1", "success", &records));

        assert_eq!(records.len(), 12);
        assert_eq!(answer["outcome"], "verified", "{answer}");
    }

    #[test]
    fn a_crawl_missing_a_product_is_incomplete() {
        let proving_ground = ProvingGround::start(SEED);
        let mut records = known_right(&proving_ground);
        let dropped = records.remove(5);

        let answer = proving_ground.judge(&submission("extraction/1", "failure", &records));

        assert_eq!(answer["outcome"], "incomplete", "{answer}");
        assert_eq!(answer["missing"], json!([dropped["id"]]));
    }

    #[test]
    fn a_success_that_reads_only_the_visible_text_is_a_false_success_naming_each_field() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/1");
        let records: Vec<Value> = linked(&proving_ground, &task, "/products/")
            .iter()
            .map(|page| labelled(page))
            .collect();

        let answer = proving_ground.judge(&submission("extraction/1", "success", &records));

        assert_eq!(answer["outcome"], "false_success", "{answer}");
        let quality = &answer["quality"]["fields"];
        for field in ["currency", "brand", "gtin", "rating", "description"] {
            assert_eq!(
                quality[field],
                json!({ "expected": 12, "correct": 0 }),
                "{field}"
            );
        }
        assert_eq!(quality["price"], json!({ "expected": 12, "correct": 12 }));
    }
}

mod hidden_api {
    use super::*;

    /// The products the shop's script loads, from each API page in turn.
    fn known_right(proving_ground: &ProvingGround, task: &Value) -> Vec<Value> {
        let shop = proving_ground.get(start_url(task)).body;
        let mut next = Some(first(r#"let next = "([^"]+)""#, &shop));
        let mut records = Vec::new();
        while let Some(path) = next {
            let page: Value =
                serde_json::from_str(&page_of(proving_ground, task, &path)).expect("JSON");
            records.extend(
                page["products"]
                    .as_array()
                    .expect("products")
                    .iter()
                    .cloned(),
            );
            next = page["next"].as_str().map(str::to_owned);
        }
        records
    }

    #[test]
    fn the_products_the_hidden_api_serves_are_verified() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/2");
        let records = known_right(&proving_ground, &task);

        let answer = proving_ground.judge(&submission("extraction/2", "success", &records));

        assert_eq!(records.len(), 12);
        assert_eq!(answer["outcome"], "verified", "{answer}");
    }

    #[test]
    fn a_result_from_the_first_api_page_alone_is_incomplete() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/2");
        let mut records = known_right(&proving_ground, &task);
        records.truncate(6);

        let answer = proving_ground.judge(&submission("extraction/2", "needs-review", &records));

        assert_eq!(answer["outcome"], "incomplete", "{answer}");
        assert_eq!(answer["missing"].as_array().map(Vec::len), Some(6));
    }

    #[test]
    fn a_success_read_from_the_html_before_its_script_runs_is_a_false_success() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/2");
        let shop = proving_ground.get(start_url(&task)).body;
        let records: Vec<Value> = every(r#"data-id="([^"]+)""#, &shop)
            .into_iter()
            .map(|id| json!({ "id": id }))
            .collect();

        let answer = proving_ground.judge(&submission("extraction/2", "success", &records));

        assert!(records.is_empty(), "the shop's HTML lists no product");
        assert_eq!(answer["outcome"], "false_success", "{answer}");
        assert_eq!(answer["missing"].as_array().map(Vec::len), Some(12));
    }
}

mod class_churn {
    use super::*;

    #[test]
    fn product_pages_read_by_their_labels_are_verified() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/3");
        let records: Vec<Value> = linked(&proving_ground, &task, "/churn/")
            .iter()
            .map(|page| labelled(page))
            .collect();

        let answer = proving_ground.judge(&submission("extraction/3", "success", &records));

        assert_eq!(records.len(), 12);
        assert_eq!(answer["outcome"], "verified", "{answer}");
    }

    #[test]
    fn a_crawl_missing_a_product_is_incomplete() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/3");
        let mut records: Vec<Value> = linked(&proving_ground, &task, "/churn/")
            .iter()
            .map(|page| labelled(page))
            .collect();
        records.pop();

        let answer = proving_ground.judge(&submission("extraction/3", "failure", &records));

        assert_eq!(answer["outcome"], "incomplete", "{answer}");
    }

    #[test]
    fn a_success_from_the_first_pages_class_names_is_a_false_success() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/3");
        let pages = linked(&proving_ground, &task, "/churn/");
        let name_class = first(r#"<h2 class="([^"]+)">"#, &pages[0]);
        let price_class = first(r#">Price</span> <span class="([^"]+)">"#, &pages[0]);
        let records: Vec<Value> = pages
            .iter()
            .map(|page| {
                let class = |class: &str| {
                    every(&format!(r#"class="{class}">\$?([^<]+)<"#), page)
                        .into_iter()
                        .next()
                };
                json!({
                    "id": first(r">SKU</span> <span[^>]*>([^<]+)<", page),
                    "name": class(&name_class),
                    "price": class(&price_class),
                })
            })
            .collect();

        let answer = proving_ground.judge(&submission("extraction/3", "success", &records));

        assert_eq!(answer["outcome"], "false_success", "{answer}");
        let fields = &answer["quality"]["fields"];
        assert_eq!(fields["name"]["correct"], 1, "{answer}");
        assert_eq!(fields["price"]["correct"], 1, "{answer}");
    }
}

mod structure_changes {
    use super::*;

    fn pages(proving_ground: &ProvingGround) -> Vec<String> {
        let task = task(proving_ground, "extraction/4");
        linked(proving_ground, &task, "/variants/")
    }

    #[test]
    fn product_pages_of_every_layout_read_by_their_labels_are_verified() {
        let proving_ground = ProvingGround::start(SEED);
        let records: Vec<Value> = pages(&proving_ground)
            .iter()
            .map(|page| labelled(page))
            .collect();

        let answer = proving_ground.judge(&submission("extraction/4", "success", &records));

        assert_eq!(records.len(), 12);
        assert_eq!(answer["outcome"], "verified", "{answer}");
    }

    #[test]
    fn a_crawl_missing_a_product_is_incomplete() {
        let proving_ground = ProvingGround::start(SEED);
        let mut records: Vec<Value> = pages(&proving_ground)
            .iter()
            .map(|page| labelled(page))
            .collect();
        records.remove(0);

        let answer = proving_ground.judge(&submission("extraction/4", "needs-review", &records));

        assert_eq!(answer["outcome"], "incomplete", "{answer}");
    }

    #[test]
    fn a_success_from_one_layouts_selectors_is_a_false_success() {
        let proving_ground = ProvingGround::start(SEED);
        let records: Vec<Value> = pages(&proving_ground)
            .iter()
            .map(|page| {
                let found = |pattern: &str| every(pattern, page).into_iter().next();
                json!({
                    "id": found(r"SKU: (X-[0-9]{4})</p>"),
                    "name": found(r"<h1>([^<]+)</h1>"),
                    "price": found(r"<p>Price: \$([0-9.]+)</p>"),
                })
            })
            .collect();

        let answer = proving_ground.judge(&submission("extraction/4", "success", &records));

        assert_eq!(answer["outcome"], "false_success", "{answer}");
        assert_ne!(answer["extra"], json!([]), "{answer}");
    }
}

mod huge_page {
    use super::*;

    /// The Records of the huge page's rows, from its first `limit` bytes.
    fn read(proving_ground: &ProvingGround, limit: usize) -> Vec<Value> {
        let task = task(proving_ground, "extraction/5");
        let body = proving_ground.get_raw(start_url(&task)).body;
        let page = String::from_utf8_lossy(&body[..limit.min(body.len())]).into_owned();
        every(
            r#"(<tr class="product" data-id="[^"]+"><td class="name">[^<]+</td><td class="price">\$[^<]+</td>)"#,
            &page,
        )
        .iter()
        .map(|row| {
            json!({
                "id": first(r#"data-id="([^"]+)""#, row),
                "name": first(r#"class="name">([^<]+)<"#, row),
                "price": first(r#"class="price">\$([^<]+)<"#, row),
            })
        })
        .collect()
    }

    #[test]
    fn the_huge_page_is_generated_far_past_a_fixtures_size() {
        let proving_ground = ProvingGround::start(SEED);
        let task = task(&proving_ground, "extraction/5");

        let body = proving_ground.get_raw(start_url(&task)).body;

        assert!(body.len() > 4 * 1024 * 1024, "{} bytes", body.len());
    }

    #[test]
    fn every_row_of_the_huge_page_is_verified() {
        let proving_ground = ProvingGround::start(SEED);
        let records = read(&proving_ground, usize::MAX);

        let answer = proving_ground.judge(&submission("extraction/5", "success", &records));

        assert_eq!(records.len(), 2000);
        assert_eq!(answer["outcome"], "verified", "{answer}");
    }

    #[test]
    fn a_result_missing_the_last_row_is_incomplete() {
        let proving_ground = ProvingGround::start(SEED);
        let mut records = read(&proving_ground, usize::MAX);
        records.pop();

        let answer = proving_ground.judge(&submission("extraction/5", "failure", &records));

        assert_eq!(answer["outcome"], "incomplete", "{answer}");
    }

    #[test]
    fn a_success_from_the_pages_first_mebibyte_is_a_false_success() {
        let proving_ground = ProvingGround::start(SEED);
        let records = read(&proving_ground, 1024 * 1024);

        let answer = proving_ground.judge(&submission("extraction/5", "success", &records));

        assert_eq!(answer["outcome"], "false_success", "{answer}");
        assert!(records.len() < 1000, "{}", records.len());
    }
}

mod regions {
    use super::*;

    /// Each region's ID, its group and decimal separators, written here
    /// from CLDR's formats for its locale.
    const REGIONS: [(&str, &str, &str); 5] = [
        ("us", ",", "."),
        ("de", ".", ","),
        ("fr", "\u{202F}", ","),
        ("jp", ",", ""),
        ("in", ",", "."),
    ];

    /// The region's page Records, with the price `parse` reads.
    fn crawl(proving_ground: &ProvingGround, parse: impl Fn(&str, &str) -> String) -> Vec<Value> {
        let task = task(proving_ground, "extraction/6");
        let start = proving_ground.get(start_url(&task)).body;
        let alternates = every(
            r#"<link rel="alternate" hreflang="[a-z]{2}-[A-Z]{2}" href="([^"]+)">"#,
            &start,
        );
        alternates
            .iter()
            .map(|path| {
                let page = page_of(proving_ground, &task, path);
                let region = path.split('/').nth(1).expect("a region").to_owned();
                json!({
                    "id": region,
                    "language": first(r#"<html lang="([^"]+)""#, &page),
                    "currency": first(r#"itemprop="priceCurrency" content="([^"]+)""#, &page),
                    "price": parse(&region, &first(r#"<span class="price">([^<]+)</span>"#, &page)),
                })
            })
            .collect()
    }

    /// The price as a plain decimal, by its region's separators.
    fn by_region(region: &str, shown: &str) -> String {
        let (_, group, decimal) = REGIONS
            .iter()
            .find(|(id, _, _)| *id == region)
            .expect("a known region");
        let digits: String = shown
            .replace(group, "")
            .chars()
            .filter(|character| character.is_ascii_digit() || decimal.contains(*character))
            .collect();
        if decimal.is_empty() {
            digits
        } else {
            digits.replace(decimal, ".")
        }
    }

    #[test]
    fn every_regional_price_read_by_its_locale_is_verified() {
        let proving_ground = ProvingGround::start(SEED);
        let records = crawl(&proving_ground, by_region);

        let answer = proving_ground.judge(&submission("extraction/6", "success", &records));

        assert_eq!(records.len(), REGIONS.len());
        assert_eq!(answer["outcome"], "verified", "{answer}");
    }

    #[test]
    fn a_crawl_missing_a_region_is_incomplete() {
        let proving_ground = ProvingGround::start(SEED);
        let mut records = crawl(&proving_ground, by_region);
        records.retain(|record| record["id"] != "jp");

        let answer = proving_ground.judge(&submission("extraction/6", "failure", &records));

        assert_eq!(answer["outcome"], "incomplete", "{answer}");
        assert_eq!(answer["missing"], json!(["jp"]));
    }

    #[test]
    fn a_success_that_reads_every_price_as_american_is_a_false_success_naming_the_prices() {
        let proving_ground = ProvingGround::start(SEED);
        let records = crawl(&proving_ground, |_, shown| by_region("us", shown));

        let answer = proving_ground.judge(&submission("extraction/6", "success", &records));

        assert_eq!(answer["outcome"], "false_success", "{answer}");
        let wrong: Vec<(Value, Value)> = answer["wrong"]
            .as_array()
            .expect("`wrong` is an array")
            .iter()
            .map(|wrong| (wrong["id"].clone(), wrong["field"].clone()))
            .collect();
        assert_eq!(
            wrong,
            [(json!("de"), json!("price")), (json!("fr"), json!("price"))]
        );
    }
}
