// Extraction and conversion adapter for the lab's JSONL protocol. The mode is chosen by the first argument.
//
// Main content (text):
//   rs-trafilatura, dom-smoothie (Readability port), scraper-text (all body text, no extraction),
//   htmd (whole page to Markdown), lol-html (streaming boilerplate removal, see `lol` below).
// Markdown of the main content:
//   rs-trafilatura-md (its own converter, quick_html2md), dom-smoothie-md (its own Markdown text mode),
//   rs-trafilatura-<converter> (rs-trafilatura's content HTML through htmd, html2md, fast-html2md or mdka),
//   lol-html-htmd (the lol-html selection through htmd).
// The product's chain, goodfetch-extractor's `formats` (#818): goodfetch (its main content as Markdown, dom-smoothie then
//   htmd with goodfetch's element handlers), goodfetch-<converter> (the same main-content HTML through htmd, html2md,
//   fast-html2md or mdka); goodfetch-trafilatura and goodfetch-trafilatura-<converter> the same with
//   `content.extractor: trafilatura`.
// Converters alone, on the whole input (for the curated fixtures): conv-htmd, conv-html2md, conv-fast-html2md,
//   conv-mdka, conv-quick-html2md.
// Structural fields (selector speed and agreement): scraper-fields, lol-html-fields.
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::time::Instant;

fn read_page(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    if path.ends_with(".gz") {
        flate2::read::GzDecoder::new(file).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    } else {
        std::io::BufReader::new(file).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn trafilatura(html: &str, markdown: bool) -> Result<rs_trafilatura::ExtractResult, String> {
    let options = rs_trafilatura::Options { output_markdown: markdown, ..Default::default() };
    rs_trafilatura::extract_with_options(html, &options).map_err(|error| error.to_string())
}

fn convert(converter: &str, html: &str) -> Result<String, String> {
    match converter {
        "htmd" => htmd::convert(html).map_err(|error| error.to_string()),
        "html2md" => Ok(html2md::parse_html(html)),
        // fast_html2md's default path is its lol_html rewriter (the one spider_transformations uses).
        "fast-html2md" => Ok(fast_html2md::rewrite_html(html, false)),
        "mdka" => Ok(mdka::html_to_markdown(html)),
        "quick-html2md" => Ok(quick_html2md::html_to_markdown(html)),
        other => Err(format!("unknown converter {other}")),
    }
}

fn extract(mode: &str, html: &str, url: Option<&str>) -> Result<String, String> {
    let url = url.filter(|u| u.starts_with("http"));
    match mode {
        "rs-trafilatura" => trafilatura(html, false).map(|result| result.content_text),
        "rs-trafilatura-md" => trafilatura(html, true).map(|result| result.content_markdown.unwrap_or_default()),
        "dom-smoothie" | "dom-smoothie-md" => {
            let config = dom_smoothie::Config { text_mode: if mode == "dom-smoothie-md" { dom_smoothie::TextMode::Markdown } else { dom_smoothie::TextMode::Raw }, ..Default::default() };
            let mut readability = dom_smoothie::Readability::new(html, url, Some(config)).map_err(|error| error.to_string())?;
            readability.parse().map(|article| article.text_content.to_string()).map_err(|error| error.to_string())
        }
        "scraper-text" => {
            let document = scraper::Html::parse_document(html);
            let body = scraper::Selector::parse("body").expect("valid selector");
            let root = document.select(&body).next().unwrap_or_else(|| document.root_element());
            Ok(root.text().collect::<Vec<_>>().join(" "))
        }
        "htmd" => convert("htmd", html),
        "lol-html" => lol::main_html(html).and_then(|selected| lol::text(&selected)),
        "lol-html-htmd" => lol::main_html(html).and_then(|selected| convert("htmd", &selected)),
        "scraper-fields" => Ok(fields_scraper(html).to_string()),
        "lol-html-fields" => lol::fields(html).map(|fields| fields.to_string()),
        other => {
            if let Some(rest) = other.strip_prefix("goodfetch") {
                let (built_in, rest) = match rest.strip_prefix("-trafilatura") {
                    Some(rest) => (goodfetch_extractor::BuiltIn::Trafilatura, rest),
                    None => (goodfetch_extractor::BuiltIn::DomSmoothie, rest),
                };
                let read = product::main_content(html, url, built_in);
                return match rest.strip_prefix('-') {
                    Some(converter) => convert(converter, &read.html),
                    None if rest.is_empty() => Ok(read.markdown),
                    None => Err(format!("unknown extractor {other}")),
                };
            }
            if let Some(converter) = other.strip_prefix("rs-trafilatura-") {
                let content = trafilatura(html, false)?.content_html.unwrap_or_default();
                return convert(converter, &content);
            }
            if let Some(converter) = other.strip_prefix("conv-") {
                return convert(converter, html);
            }
            Err(format!("unknown extractor {other}"))
        }
    }
}

// goodfetch-extractor's `formats` on a page as served, with no Clean-up rule and the built-in main-content step
// `content.extractor` names (dom-smoothie, the default, with its default attempts; or rs-trafilatura): the main content
// as Markdown and as HTML, both read from the same nodes.
mod product {
    use goodfetch_extractor::{BuiltIn, Chosen, Context, Document, Serialization, formats};
    use goodfetch_protocol::page::{Asked, FormatsRequest, HtmlFormat, MarkdownContent, MarkdownFormat, MarkdownOptions};

    pub struct MainContent { pub markdown: String, pub html: String }

    pub fn main_content(html: &str, url: Option<&str>, built_in: BuiltIn) -> MainContent {
        let url = url.unwrap_or("https://example.invalid/");
        let request = FormatsRequest {
            markdown: Some(MarkdownFormat::Options(MarkdownOptions { content: MarkdownContent::Main })),
            html: Some(HtmlFormat::Asked(Asked)),
            ..FormatsRequest::default()
        };
        let document = Document { html, url, base_url: url, serialization: Serialization::Served };
        let context = Context { extractor: Chosen { built_in, ..Chosen::default() }.extractor(), ..Context::default() };
        let read = formats(&document, &request, &context);
        MainContent { markdown: read.markdown.unwrap_or_default(), html: read.html.unwrap_or_default() }
    }
}

pub fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Fields read with html5ever's tree (the decision's reference parser): first <title>, meta description, canonical link,
// og:title, every <h1> text, every a[href] and img[src] in document order.
fn fields_scraper(html: &str) -> Value {
    let document = scraper::Html::parse_document(html);
    let select = |css: &str| scraper::Selector::parse(css).expect("valid selector");
    let first_attr = |css: &str, name: &str| document.select(&select(css)).next().and_then(|el| el.value().attr(name)).map(str::to_string);
    json!({
        "title": document.select(&select("title")).next().map(|el| collapse(&el.text().collect::<String>())),
        "description": first_attr("meta[name=\"description\"]", "content"),
        "canonical": first_attr("link[rel=\"canonical\"]", "href"),
        "ogTitle": first_attr("meta[property=\"og:title\"]", "content"),
        "h1": document.select(&select("h1")).map(|el| collapse(&el.text().collect::<String>())).collect::<Vec<_>>(),
        "links": document.select(&select("a[href]")).filter_map(|el| el.value().attr("href").map(str::to_string)).collect::<Vec<_>>(),
        "images": document.select(&select("img[src]")).filter_map(|el| el.value().attr("src").map(str::to_string)).collect::<Vec<_>>(),
    })
}

// lol-html is a streaming rewriter: it sees each tag and text chunk once, builds no tree and cannot score content.
// The main-content pipeline built on it is three streaming passes plus string slicing:
//   1. scan: count <main>/[role=main] and <article> elements;
//   2. rewrite: remove boilerplate elements with their content (tags, ARIA roles, hidden elements, class/id tokens, and
//      <header> outside main/article), and wrap the content of every scope element (main/[role=main] if any, else article)
//      in marker comments;
//   3. slice the rewritten document between the markers (the whole document when there is no scope);
//   then the text is read by a last lol-html pass, or the slice goes to htmd for Markdown.
mod lol {
    use lol_html::html_content::{ContentType, Element, TextChunk};
    use lol_html::{RewriteStrSettings, doc_text, element, end_tag, rewrite_str, text};
    use serde_json::{Value, json};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const START: &str = "<!--goodfetch-scope-start-->";
    const END: &str = "<!--goodfetch-scope-end-->";
    const BOILERPLATE_TAGS: &str = "script, style, noscript, template, svg, canvas, iframe, object, embed, nav, footer, aside, form, button, select, textarea, dialog, menu";
    const BOILERPLATE_ATTRS: &str = "[role=navigation], [role=banner], [role=contentinfo], [role=complementary], [role=search], [role=dialog], [role=alertdialog], [role=menu], [role=menubar], [aria-hidden=true], [hidden]";
    // Whole tokens of class or id, split on whitespace, '-' and '_'.
    const BOILERPLATE_TOKENS: &[&str] = &["nav", "navbar", "navigation", "menu", "footer", "sidebar", "breadcrumb", "breadcrumbs", "cookie", "cookies", "consent", "banner", "advert", "advertisement", "ads", "ad", "social", "share", "sharing", "related", "newsletter", "subscribe", "popup", "modal", "masthead", "topbar", "toolbar", "promo", "sponsored", "skip"];

    type Handler<'h> = (std::borrow::Cow<'static, lol_html::Selector>, lol_html::ElementContentHandlers<'h>);
    fn settings<'h>(handlers: Vec<Handler<'h>>) -> RewriteStrSettings<'h, 'static> {
        handlers.into_iter().fold(RewriteStrSettings::new(), |settings, handler| settings.append_element_content_handler(handler))
    }

    fn boilerplate_token(element: &Element) -> bool {
        ["class", "id"].iter().filter_map(|name| element.get_attribute(name)).any(|value| {
            value.split(|c: char| c.is_whitespace() || c == '-' || c == '_').any(|token| BOILERPLATE_TOKENS.contains(&token.to_ascii_lowercase().as_str()))
        })
    }

    fn scope_selector(html: &str) -> Result<Option<&'static str>, String> {
        let (mains, articles) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let (m, a) = (mains.clone(), articles.clone());
        rewrite_str(html, settings(vec![
                element!("main, [role=main]", move |_el| { m.set(m.get() + 1); Ok(()) }),
                element!("article", move |_el| { a.set(a.get() + 1); Ok(()) }),
            ])).map_err(|error| error.to_string())?;
        Ok(if mains.get() > 0 { Some("main, [role=main]") } else if articles.get() > 0 { Some("article") } else { None })
    }

    pub fn main_html(html: &str) -> Result<String, String> {
        let scope = scope_selector(html)?;
        // Depth inside main/article, so that a <header> holding an article's title is kept.
        let depth = Rc::new(Cell::new(0i32));
        let (d1, d2) = (depth.clone(), depth.clone());
        let mut handlers = vec![
            element!("main, [role=main], article", move |el| {
                d1.set(d1.get() + 1);
                let d = d1.clone();
                if el.can_have_content() { el.on_end_tag(end_tag!(move |_end| { d.set(d.get() - 1); Ok(()) }))?; }
                Ok(())
            }),
            element!(BOILERPLATE_TAGS, |el| { el.remove(); Ok(()) }),
            element!(BOILERPLATE_ATTRS, |el| { el.remove(); Ok(()) }),
            element!("header", move |el| { if d2.get() == 0 { el.remove(); } Ok(()) }),
            element!("[class], [id]", |el| { if el.tag_name() != "body" && el.tag_name() != "html" && boilerplate_token(el) { el.remove(); } Ok(()) }),
        ];
        if let Some(selector) = scope {
            handlers.push(element!(selector, |el| {
                if el.can_have_content() { el.prepend(START, ContentType::Html); el.append(END, ContentType::Html); }
                Ok(())
            }));
        }
        let rewritten = rewrite_str(html, settings(handlers)).map_err(|error| error.to_string())?;
        if scope.is_none() { return Ok(rewritten); }
        // Concatenate the outermost scope segments; nested scopes are inside them already.
        let (mut out, mut level, mut rest) = (String::new(), 0usize, rewritten.as_str());
        loop {
            let next_start = rest.find(START);
            let next_end = rest.find(END);
            match (next_start, next_end) {
                (Some(s), Some(e)) if s < e => { if level > 0 { out.push_str(&rest[..s]); } level += 1; rest = &rest[s + START.len()..]; }
                (_, Some(e)) => { if level > 0 { out.push_str(&rest[..e]); out.push('\n'); level -= 1; } rest = &rest[e + END.len()..]; }
                (Some(s), None) => { if level > 0 { out.push_str(&rest[..s]); } level += 1; rest = &rest[s + START.len()..]; }
                (None, None) => { if level > 0 { out.push_str(rest); } break; }
            }
        }
        // A scope removed with a boilerplate ancestor leaves no segment: fall back to the cleaned document.
        Ok(if out.trim().is_empty() { rewritten } else { out })
    }

    const BLOCKS: &str = "p, div, li, ul, ol, dl, dt, dd, h1, h2, h3, h4, h5, h6, br, hr, tr, td, th, table, section, article, main, blockquote, pre, figure, figcaption, header, address";

    // Text of an HTML fragment, read by lol-html: chunks are raw, so entities are decoded here; block elements separate words.
    // A document text handler, not text!("*"): the slice of a scope starts inside <main>/<article>, so text sitting directly
    // in the scope has no enclosing element in the fragment and an element text handler would drop it.
    pub fn text(html: &str) -> Result<String, String> {
        let buffer = Rc::new(RefCell::new(String::new()));
        let (b1, b2) = (buffer.clone(), buffer.clone());
        let settings = settings(vec![element!(BLOCKS, move |_el| { b1.borrow_mut().push('\n'); Ok(()) })])
            .append_document_content_handler(doc_text!(move |chunk: &mut TextChunk| { b2.borrow_mut().push_str(chunk.as_str()); Ok(()) }));
        rewrite_str(html, settings).map_err(|error| error.to_string())?;
        let raw = buffer.borrow();
        Ok(html_escape::decode_html_entities(&*raw).into_owned())
    }

    // The same fields as fields_scraper, read in one streaming pass.
    pub fn fields(html: &str) -> Result<Value, String> {
        let title = Rc::new(RefCell::new((0usize, String::new())));
        let h1 = Rc::new(RefCell::new(Vec::<String>::new()));
        let attrs = Rc::new(RefCell::new((None::<String>, None::<String>, None::<String>, Vec::<String>::new(), Vec::<String>::new())));
        let (t1, t2, h_1, h_2) = (title.clone(), title.clone(), h1.clone(), h1.clone());
        let (a1, a2, a3, a4, a5) = (attrs.clone(), attrs.clone(), attrs.clone(), attrs.clone(), attrs.clone());
        rewrite_str(html, settings(vec![
                element!("title", move |_el| { t1.borrow_mut().0 += 1; Ok(()) }),
                text!("title", move |chunk| { let mut t = t2.borrow_mut(); if t.0 == 1 { t.1.push_str(chunk.as_str()); } Ok(()) }),
                element!("h1", move |_el| { h_1.borrow_mut().push(String::new()); Ok(()) }),
                text!("h1", move |chunk| { if let Some(last) = h_2.borrow_mut().last_mut() { last.push_str(chunk.as_str()); } Ok(()) }),
                element!("meta[name=\"description\"]", move |el| { let mut a = a1.borrow_mut(); if a.0.is_none() { a.0 = el.get_attribute("content"); } Ok(()) }),
                element!("link[rel=\"canonical\"]", move |el| { let mut a = a2.borrow_mut(); if a.1.is_none() { a.1 = el.get_attribute("href"); } Ok(()) }),
                element!("meta[property=\"og:title\"]", move |el| { let mut a = a3.borrow_mut(); if a.2.is_none() { a.2 = el.get_attribute("content"); } Ok(()) }),
                element!("a[href]", move |el| { if let Some(href) = el.get_attribute("href") { a4.borrow_mut().3.push(href); } Ok(()) }),
                element!("img[src]", move |el| { if let Some(src) = el.get_attribute("src") { a5.borrow_mut().4.push(src); } Ok(()) }),
            ])).map_err(|error| error.to_string())?;
        let decode = |s: &str| super::collapse(&html_escape::decode_html_entities(s));
        let raw = |s: &str| html_escape::decode_html_entities(s).into_owned();
        let title = title.borrow();
        let attrs = attrs.borrow();
        Ok(json!({
            "title": if title.0 > 0 { Some(decode(&title.1)) } else { None },
            // Attribute values come back raw from lol-html; entities are decoded as an HTML parser would.
            "description": attrs.0.as_deref().map(raw), "canonical": attrs.1.as_deref().map(raw), "ogTitle": attrs.2.as_deref().map(raw),
            "h1": h1.borrow().iter().map(|s| decode(s)).collect::<Vec<_>>(),
            "links": attrs.3.iter().map(|s| raw(s)).collect::<Vec<_>>(), "images": attrs.4.iter().map(|s| raw(s)).collect::<Vec<_>>(),
        }))
    }
}

fn respond(id: Value, result: Result<Value, String>) {
    let message = match result {
        Ok(result) => json!({"version": 1, "id": id, "result": result}),
        Err(error) => json!({"version": 1, "id": id, "error": error}),
    };
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{message}");
    let _ = stdout.flush();
}

const VERSIONS: &[(&str, &str)] = &[("rs-trafilatura", "0.2.2"), ("dom_smoothie", "0.18.2"), ("scraper", "0.27.0"), ("htmd", "0.5.6"), ("lol_html", "3.0.2"), ("html2md", "0.2.17"), ("fast_html2md", "0.0.63"), ("mdka", "3.3.0"), ("quick_html2md", "0.2.1"), ("html-escape", "0.2.15")];

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    // A one-shot form for checks outside the protocol: goodfetch-lab-extract <mode> --file <path>
    if std::env::args().nth(2).as_deref() == Some("--file") {
        let path = std::env::args().nth(3).unwrap_or_default();
        match read_page(&path).and_then(|html| extract(&mode, &html, None)) {
            Ok(text) => println!("{text}"),
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&line) else { break };
        let id = request["id"].clone();
        if request["version"] != 1 || !id.is_u64() {
            break;
        }
        match request["method"].as_str().unwrap_or("") {
            "hello" => respond(id, Ok(json!({"backend": mode, "protocol": 1, "capabilities": ["extract"], "runtime": "rust", "maxConcurrency": 1, "versions": VERSIONS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>()}))),
            "prepare" => respond(id, Ok(json!({"ready": true}))),
            "run" => {
                let params = &request["params"];
                let result = (|| {
                    let html = read_page(params["path"].as_str().ok_or("missing path")?)?;
                    let began = Instant::now();
                    let text = extract(&mode, &html, params["url"].as_str())?;
                    Ok(json!({"status": "ok", "text": text, "metrics": {"extractMs": began.elapsed().as_secs_f64() * 1000.0, "htmlBytes": html.len()}}))
                })();
                respond(id, result);
            }
            "close" => {
                respond(id, Ok(json!({"closed": true})));
                return;
            }
            _ => respond(id, Err("unknown method".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::lol;

    fn words(text: &str) -> Vec<&str> { text.split_whitespace().collect() }

    #[test]
    fn lol_text_keeps_text_directly_inside_the_scope() {
        let selected = lol::main_html("<body><nav>Menu</nav><main>Intro <p>Para</p> tail</main></body>").unwrap();
        assert_eq!(words(&lol::text(&selected).unwrap()), ["Intro", "Para", "tail"]);
    }

    #[test]
    fn lol_text_separates_blocks_and_decodes_entities() {
        assert_eq!(words(&lol::text("<p>a&amp;b</p><p>c</p>").unwrap()), ["a&b", "c"]);
    }
}
