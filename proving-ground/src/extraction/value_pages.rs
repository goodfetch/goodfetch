//! The `extraction` family's value page (ticket #1002; ADR 0123): one
//! product page, in German, whose values a code-free crawl's fields read
//! with Value transforms: a name, a price printed for people beside its
//! machine-readable value (`<data value>`), a date and two date-times in
//! `<time datetime>` (one stating no offset, one stating its own), a SKU
//! inside a sentence, and a relative link to a manual. No task lists it;
//! a test seeds it by its path.

/// The path of the page.
pub(super) const PATH: &str = "/values";

const PAGE: &str = r#"<!doctype html>
<html lang="de-DE">
<head><meta charset="utf-8"><title>Delta-Drachen | Northwind</title></head>
<body>
<main>
<article class="product">
<h1>
  Delta-Drachen
</h1>
<p class="sku">Artikelnummer: DK-0042 (neu)</p>
<p>Preis: <data class="price" value="1099.98">1.099,98&nbsp;€</data></p>
<p>Erschienen am <time class="released" datetime="2026-02-01">1. Februar 2026</time>.</p>
<p>Lieferbar ab <time class="available" datetime="2026-02-01T10:00">1. Februar 2026, 10:00 Uhr</time>.</p>
<p>Zuletzt geändert <time class="updated" datetime="2026-02-01T10:00:00+05:30">1. Februar 2026, 10:00 Uhr IST</time>.</p>
<p><a class="manual" href="manuals/delta.pdf">Anleitung</a></p>
</article>
</main>
</body>
</html>
"#;

/// The page at `path`; `None` for any other path.
pub(super) fn page(path: &str) -> Option<String> {
    (path == PATH).then(|| PAGE.to_owned())
}
