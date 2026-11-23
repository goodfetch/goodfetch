//! Each Product's rendering of a docs page, as the docs generator writes it
//! before the build (ADR 0097), and a package README's, which is its
//! Product's as written.
//!
//! Until the generator exists, this is its rendering, and its syntax is the
//! generator's to keep or replace:
//! - a page's front matter lists the Products that carry it,
//!   `products: [foxhound]`; a page that lists none is both Products';
//! - a part only some Products carry sits between `{/* products: foxhound */}`
//!   and `{/* end products */}`, each alone on its line;
//! - a part marked as naming a Product sits between `{/* names: foxhound */}`
//!   and `{/* end names */}`, and is left as written;
//! - elsewhere, `{{executable}}`, `{{npm}}`, `{{import}}`, `{{connection}}`,
//!   `{{engine_path_variable}}`, `{{installer}}` and `{{image}}` write the
//!   rendering's Product's names (ADR 0090).
//!
//! A rendered line keeps its line in the source, so every finding and
//! manifest entry names the line a writer edits.

use serde::Deserialize;

use crate::check::Finding;
use crate::products::Product;

/// The finding kind of a page the rendering cannot read.
pub(super) const MALFORMED: &str = "docs-samples";

/// One Product's rendering of a page or README.
pub(crate) struct Rendering {
    pub(crate) product: Product,
    pub(crate) lines: Vec<Line>,
}

/// One line of a rendering.
pub(crate) struct Line {
    pub(crate) text: String,
    /// Its line in the source, from 1.
    pub(crate) source: usize,
    /// The Products whose renderings carry it, Collie first.
    pub(super) carriers: Vec<Product>,
    /// The Product the part it sits in is marked as naming.
    pub(super) names: Option<Product>,
}

#[derive(Deserialize)]
struct FrontMatter {
    #[serde(default)]
    products: Option<Vec<Product>>,
}

/// A part a marker opened, and the line of its marker.
struct Open<T> {
    value: T,
    line: usize,
}

/// A part marker, alone on its line.
enum Marker {
    Products(Vec<Product>),
    EndProducts,
    Names(Product),
    EndNames,
}

/// A README as written: its package's Product's alone.
pub(super) fn readme(product: Product, text: &str) -> Rendering {
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, text)| Line {
            text: text.to_owned(),
            source: index + 1,
            carriers: vec![product],
            names: None,
        })
        .collect();
    Rendering { product, lines }
}

/// Each Product's rendering of the page at `page`, Collie's first, or what
/// keeps it from rendering.
pub(crate) fn page(page: &str, text: &str) -> Result<Vec<Rendering>, Vec<Finding>> {
    let source: Vec<&str> = text.lines().collect();
    let (page_products, body_start) = front_matter(page, &source)?;
    let mut findings = Vec::new();
    let mut products: Option<Open<Vec<Product>>> = None;
    let mut names: Option<Open<Product>> = None;
    let mut lines = Vec::new();
    for (index, text) in source.iter().enumerate().skip(body_start) {
        let number = index + 1;
        let malformed =
            |message: &str| Finding::new(MALFORMED, format!("{page}:{number}"), message);
        match marker(text) {
            Some(Err(message)) => findings.push(malformed(&message)),
            Some(Ok(Marker::Products(value))) => {
                if products.is_some() {
                    findings.push(malformed("a products part opens inside another"));
                }
                products = Some(Open {
                    value,
                    line: number,
                });
            }
            Some(Ok(Marker::EndProducts)) => {
                if products.take().is_none() {
                    findings.push(malformed("`end products` closes no products part"));
                }
            }
            Some(Ok(Marker::Names(value))) => {
                if names.is_some() {
                    findings.push(malformed("a names part opens inside another"));
                }
                names = Some(Open {
                    value,
                    line: number,
                });
            }
            Some(Ok(Marker::EndNames)) => {
                if names.take().is_none() {
                    findings.push(malformed("`end names` closes no names part"));
                }
            }
            None => lines.push(Line {
                text: (*text).to_owned(),
                source: number,
                carriers: page_products
                    .iter()
                    .copied()
                    .filter(|product| {
                        products
                            .as_ref()
                            .is_none_or(|part| part.value.contains(product))
                    })
                    .collect(),
                names: names.as_ref().map(|part| part.value),
            }),
        }
    }
    for line in [products.map(|part| part.line), names.map(|part| part.line)]
        .into_iter()
        .flatten()
    {
        findings.push(Finding::new(
            MALFORMED,
            format!("{page}:{line}"),
            "the part this marker opens is never closed",
        ));
    }
    if !findings.is_empty() {
        return Err(findings);
    }
    Ok(page_products
        .iter()
        .map(|&product| Rendering {
            product,
            lines: lines
                .iter()
                .filter(|line| line.carriers.contains(&product))
                .map(|line| Line {
                    text: match line.names {
                        Some(_) => line.text.clone(),
                        None => with_names(&line.text, product),
                    },
                    source: line.source,
                    carriers: line.carriers.clone(),
                    names: line.names,
                })
                .collect(),
        })
        .collect())
}

/// The Products the page's front matter lists, Collie first, and the index
/// of the first line after it.
fn front_matter(page: &str, source: &[&str]) -> Result<(Vec<Product>, usize), Vec<Finding>> {
    if source.first() != Some(&"---") {
        return Ok((Product::ALL.to_vec(), 0));
    }
    let malformed = |message: String| vec![Finding::new(MALFORMED, format!("{page}:1"), message)];
    let Some(end) = source.iter().skip(1).position(|line| *line == "---") else {
        return Err(malformed("the front matter is never closed".to_owned()));
    };
    let yaml = source[1..=end].join("\n");
    let products = if yaml.trim().is_empty() {
        None
    } else {
        serde_saphyr::from_str::<FrontMatter>(&yaml)
            .map_err(|error| malformed(format!("reading the front matter: {error}")))?
            .products
    };
    let products = match products {
        None => Product::ALL.to_vec(),
        Some(listed) if listed.is_empty() => {
            return Err(malformed("`products` lists no Product".to_owned()));
        }
        Some(listed) => Product::ALL
            .into_iter()
            .filter(|product| listed.contains(product))
            .collect(),
    };
    Ok((products, end + 2))
}

/// The part marker `line` is, if it is one, or what is wrong with it.
fn marker(line: &str) -> Option<Result<Marker, String>> {
    let comment = line.trim().strip_prefix("{/*")?.strip_suffix("*/}")?.trim();
    match comment {
        "end products" => return Some(Ok(Marker::EndProducts)),
        "end names" => return Some(Ok(Marker::EndNames)),
        _ => {}
    }
    let (kind, value) = comment.split_once(':')?;
    let products = value
        .split(',')
        .map(|word| {
            Product::named(word.trim()).ok_or_else(|| format!("`{}` is no Product", word.trim()))
        })
        .collect::<Result<Vec<_>, _>>();
    match kind.trim() {
        "products" => Some(products.map(Marker::Products)),
        "names" => Some(products.and_then(|products| match products[..] {
            [product] => Ok(Marker::Names(product)),
            _ => Err("a names part names one Product".to_owned()),
        })),
        _ => None,
    }
}

/// `text` with each name marker written as `product`'s name.
pub(crate) fn with_names(text: &str, product: Product) -> String {
    product
        .names()
        .by_marker()
        .into_iter()
        .fold(text.to_owned(), |text, (marker, name)| {
            text.replace(&format!("{{{{{marker}}}}}"), name)
        })
}
