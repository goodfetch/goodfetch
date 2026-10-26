//! The extractor's benchmark scenario (ADR 0071; spec #580's performance
//! budgets): `formats` over a pinned page of at least 1 MiB, every format
//! of the request asked at once. The page is a literal block repeated: a
//! head with Open Graph, Twitter and JSON-LD metadata, then articles with
//! headings, paragraphs of links, e-mail addresses and telephone numbers,
//! an image, a table, a microdata item and a navigation list.

use std::fmt::Write as _;

use goodfetch_extractor::{Context, Document, Serialization, formats};
use goodfetch_protocol::page::{
    Asked, FormatsRequest, HtmlFormat, MarkdownContent, MarkdownFormat, MarkdownOptions,
};

/// The page's URL, its base URL too.
const URL: &str = "https://kites.test/journal/bridle";

/// The size the page reaches at least, in bytes.
const SIZE: usize = 1 << 20;

/// The page's head.
const HEAD: &str = r#"<!doctype html>
<html lang="en-GB">
<head>
<meta charset="utf-8">
<title>The bridle journal | Northwind</title>
<meta name="description" content="Every bridle we rigged, measured and flew.">
<link rel="canonical" href="/journal/bridle">
<meta property="og:title" content="The bridle journal">
<meta property="og:image" content="https://cdn.kites.test/bridle.jpg">
<meta name="twitter:card" content="summary_large_image">
<script type="application/ld+json">{"@context": "https://schema.org", "@type": "Blog", "name": "The bridle journal"}</script>
</head>
<body>
<nav><ul><li><a href="/">Home</a></li><li><a href="/kites">Kites</a></li><li><a href="/journal">Journal</a></li></ul></nav>
<main>
"#;

/// One article, `{n}` its number.
const ARTICLE: &str = r#"<article itemscope itemtype="https://schema.org/BlogPosting">
<h2 itemprop="headline">Bridle {n}: a three-leg rig</h2>
<p>We rigged bridle {n} with <a href="/lines/{n}">a polyester line</a> and flew it in a <em>steady</em> wind of 12 knots; read <a href="https://elsewhere.test/wind?day={n}">the forecast</a> we followed.</p>
<p>Questions go to <a href="mailto:bridles@kites.test">our riggers</a> or rigger{n}@kites.test, and the workshop answers on 020 7031 3000 and +33 1 42 68 53 00.</p>
<img src="/images/bridle-{n}.jpg" alt="Bridle {n} at rest">
<table><caption>Bridle {n}'s legs</caption><thead><tr><th>Leg</th><th>cm</th></tr></thead><tbody><tr><th>Front</th><td>42</td></tr><tr><th>Back</th><td>57</td></tr></tbody></table>
<p>Follow the build on <a href="https://github.com/northwind">GitHub</a> and <a href="https://x.com/northwind">X</a>.</p>
</article>
"#;

/// The page's tail.
const TAIL: &str = "</main>\n<footer><p>Northwind, 2026</p></footer>\n</body>\n</html>\n";

/// The pinned page: [`ARTICLE`] repeated after [`HEAD`] until the page
/// holds at least [`SIZE`] bytes.
pub(crate) fn page() -> String {
    let mut html = String::from(HEAD);
    let mut number = 0;
    while html.len() < SIZE {
        number += 1;
        html.push_str(&ARTICLE.replace("{n}", &number.to_string()));
    }
    html.push_str(TAIL);
    html
}

/// What the request asks: every format, Markdown's `fit` content and the
/// main content's HTML.
fn every_format() -> FormatsRequest {
    FormatsRequest {
        markdown: Some(MarkdownFormat::Options(MarkdownOptions {
            content: MarkdownContent::Fit,
        })),
        html: Some(HtmlFormat::Asked(Asked)),
        text: Some(Asked),
        links: Some(Asked),
        images: Some(Asked),
        media: Some(Asked),
        tables: Some(Asked),
        metadata: Some(Asked),
        contacts: Some(Asked),
    }
}

/// Reads every format of `html`; the length of the result's JSON, so
/// nothing is optimized away.
pub(crate) fn read_every_format(html: &str) -> usize {
    let document = Document {
        html,
        url: URL,
        base_url: URL,
        serialization: Serialization::Dom,
    };
    let read = formats(&document, &every_format(), &Context::default());
    let mut written = String::new();
    write!(written, "{read:?}").expect("invariant: writing to a String succeeds");
    written.len()
}
