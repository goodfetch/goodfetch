# Can Spider extract main text and title like Foxhound?

Research for [#2152](https://github.com/goodfetch/goodfetch/issues/2152) (map #2141, problem #2136), 2026-11-06.

## Answer

Yes, with a second crate, and the output is close to Foxhound's but not the same.
Spider 2.53.9 has no main-text step of its own: no Cargo feature of `spider` does it.
Spider's separate crate `spider_transformations` 2.39.13 does it, through its own Readability port, `llm_readability` 0.0.17.
Spider's page metadata already gives the title. Building the harness with `spider_transformations` cost about 60 µs per page on the scale site (measured under load, see below).

Three gaps change the work, so the harness has to handle them:

1. **The text contains the title.** `transform_content` with `readability: true` and `ReturnFormat::Text` extracts text from the HTML that `llm_readability` returns. That HTML is wrapped in a reader page whose `<head>` holds `<title>`. So each page's text starts with its title (`"Olive Globe 2\nOlive Globe 2"`). The harness must take the text from `llm_readability`'s `Product.text` instead (see "How the harness would call it").
2. **The algorithm is different.** `llm_readability` is "a rewrite of `readability-rs`". That crate descends from the older arc90 algorithm. `dom_smoothie`, Foxhound's default, "closely follows the implementation of readability.js" (Mozilla). On the Proving ground's scale-site pages the two pick different nodes. `llm_readability` keeps the item's `<h1>`. Plain `dom_smoothie` keeps the `<nav>` link list (page 1) or just `Home` (a leaf page).
3. **The title is raw.** Spider's `Metadata.title` is the raw `<title>` text chunk from `lol_html`. It is not entity-decoded, trimmed or whitespace-collapsed: `" Odd &amp;  Title "`. Foxhound collapses white space and decodes it. The scale site's titles are plain names, so in practice this changes nothing there.

## Sources

All read in the local Cargo registry (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`), at the versions crates.io served on 2026-11-06.

| Claim | Source |
|---|---|
| `spider` 2.53.9 has no transformation, readability or main-content feature | `spider-2.53.9/Cargo.toml` `[features]`. The only mention is a doc comment on `Page::get_content`, `src/page.rs:7116-7121`: "transformed locally with `spider_transformations`" |
| `spider_transformations` latest is 2.39.13 (MIT), repo `spider-rs/spider_transformations` (not archived, last push 2026-10-08), crate built from commit `5da9b2e2` | `cargo info spider_transformations`, `.cargo_vcs_info.json`, `gh api repos/spider-rs/spider_transformations` |
| It depends on `spider = "2"` with `default-features = false` (resolves to the harness's pinned 2.53.9), `llm_readability 0.0.17` (`tokio` feature), `fast_html2md 0`, `lol_html 2` | `spider_transformations-2.39.13/Cargo.toml` |
| Features: default `serde` (turns on `spider/serde`); optional `audio`, `document`, `screenshot` (Chrome), `balance`, `indexmap`. It **does not compile without `serde`**: `chunking.rs` uses `spider::serde` unconditionally (4 errors at `src/transformation/chunking.rs:5` and `:65`) | `cargo info`; probe build |
| API: `transform_content(&Page, &TransformConfig, &Option<String> encoding, &Option<SelectorConfiguration>, &Option<Vec<String>> ignore_tags) -> String`; `transform_content_input(TransformInput, &TransformConfig)`; async `transform_content_send(...)`. `TransformConfig { readability, return_format, filter_images, clean_html, filter_svg, main_content }`; `ReturnFormat::{Raw, Bytes, Text, Html2Text, Markdown, CommonMark, XML, Screenshot, Empty}` | `src/transformation/content.rs:29-125, 655-760, 969` |
| Pipeline: remove ignored tags with `lol_html`, then `llm_readability::extractor::extract` (falls back to the input HTML on error), then `spider::utils::clean_html` if `clean_html`, then the format | `content.rs:681-745` |
| `main_content: true` removes `nav`, `header:first-of-type`, `footer`, `body > aside:not(:first-of-type)` before readability | `content.rs:253-271` (`build_static_vector`) |
| `ReturnFormat::Text` = `lol_html` text of every element except `script, style, svg, noscript`, joined by spaces, then `sift()` (whitespace collapse) | `src/transformation/text_extract.rs:9-82` |
| `llm_readability` returns `Product { content, text }`. `content` is the top candidate wrapped in a reader page with `<title>{title}</title>` in its head. `text` is the candidate's own text | `llm_readability-0.0.17/src/extractor.rs:35-125` |
| `llm_readability`'s rules: one pass with no char threshold and no retry. Preprocess drops `script, link, style` and nodes whose `id`/`class` match UNLIKELY (`comment, foot, header, menu, sidebar, sponsor, popup, …`) and not LIKELY. It scores candidates with POSITIVE/NEGATIVE class regexes and link density. Clean drops `noscript, meta, object, svg, footer, aside`, and conditionally `form, table, ul, div` | `src/scorer.rs:18-31, 185-240, 348-449` |
| `dom_smoothie` follows Readability.js: 5 top candidates, a 500-character threshold with retries under looser `GrabFlags` | `dom_smoothie-0.18.2/README.md`, `src/config.rs:8-110` |
| Spider's title: `metadata_handlers` sets `meta_title` from the `head title` text chunk (raw source text, last non-empty chunk). It is filled in the HTTP mode with `basic` features | `spider-2.53.9/src/page.rs:3971-3990, 5336-5350`; probe crawl: 1,001 of 1,001 pages had a title |

## What Foxhound does (for comparison)

Source: `origin/spec/1577`.

`just compare`'s extraction cell asks Foxhound for `formats: { text: true }` and the field `title` (CSS `title`) (`xtask/src/compare/foxhound.rs:205-218`).
The engine runs these steps (`crates/goodfetch-extractor/src/formats.rs:167-240`, `main_content.rs`, `extractor.rs`):

1. The Clean-up rules remove matched elements (cosmetic filters) from the parsed tree.
2. The main-content step answers nothing when the body holds no text outside `script, style, noscript, template, [hidden], nav, aside`, ARIA landmark roles and the page's own `header`/`footer`.
3. Otherwise it runs `dom_smoothie::Readability::with_document` on the tree.
4. When Readability finds no text, it falls back to the body without those landmarks.
5. The text is read block by block, with paragraphs one blank line apart.

`spider_transformations` has no counterpart for the Clean-up rules, the empty answer or the landmark fallback. `main_content: true` is a smaller, fixed selector list applied before readability rather than after it.

## Measurement (throwaway probe, deleted)

Setup:

- A probe crate outside the workspace (`/Users/jb/agent-targets/spider-extract`, deleted afterwards), built `--release` (thin LTO) through `load-guard`.
- It reused `xtask/compare-spider/Cargo.lock`, so `spider` stayed at 2.53.9, with `spider_transformations` 2.39.13 and `dom_smoothie` 0.18.2 added.
- The pages copy the Proving ground's scale-site tree template (`proving-ground/src/crawl/scale.rs::tree_page` with `crawl.rs::document`), with made-up names and a fan-out of 10. That gave 1,001 pages, the last one with an entity and extra spaces in its title.
- A loopback server inside the probe served the pages.
- The machine is the Hetzner CCX43 (16 vCPU), at **load 22**, so the absolute figures are noisy. The ratios are more reliable than the microseconds.

| Step, per page (20 × 1,001 pages, one thread) | µs/page |
|---|---|
| `transform_content_input`, Text, no readability | 24.2 |
| … readability + Text | 67.8 |
| … `main_content` + readability + Text | 45.6 |
| `dom_smoothie 0.18.2` `Readability::new(html).parse()` (reference only: it parses again, unlike Foxhound's `with_document`) | 88.7 |
| `transform_content(&Page)` readability + Text, inside a Spider crawl's page reader (16 concurrent) | 59.8 |

The whole crawl of 1,001 pages took 93 ms with the transformation in the reader.

Outputs from the probe:

| Page | readability + Text (`spider_transformations`) | `dom_smoothie` text (plain call) |
|---|---|---|
| 1 (root, 11 links) | `"Hazel Lamp 1\nHazel Lamp 1"` (title + h1) | the nav: `Home`, the 11 link names |
| 500 (leaf) | `"Mellow Mirror 500\nMellow Mirror 500"` | `Home` |
| 1001 (`<title> Odd &amp;  Title </title>`) | `"Odd & Title Plush Desk 1001"` | `Plush Desk 1001 … Home` |

Metadata title for page 1001: `" Odd &amp;  Title "`. `dom_smoothie`'s title for it: `"Odd &  Title"`.

## How the harness would call it

In `xtask/compare-spider` (its own lock file, so the Products' lock is untouched):

```toml
spider_transformations = "=2.39.13"   # default feature `serde` is required
```

Two options, per page in `Fetched::add` for the `extraction` scenario:

- **Recommended:** call `spider_transformations`'s own readability crate directly, which avoids the title prefix: `llm_readability::extractor::extract(&mut page.get_html_bytes_u8(), url)`, where `url` is the `&Url` inside `page.get_url_parsed_ref()` (an `&Option<Url>`; `spider-2.53.9/src/page.rs:7035, 7171`) → `Product.text`. Pin `llm_readability = "=0.0.17"` beside it. Then collapse white space as Foxhound does. The title stays `page.get_metadata().title`, trimmed, collapsed and entity-decoded.
- `transform_content(&page, &TransformConfig { readability: true, return_format: ReturnFormat::Text, ..Default::default() }, &None, &None, &None)`, then strip the leading title line. This is brittle: when `sift()` collapses the newline, the title and the text are joined by a space, as on page 1001.

The function names in the first option come from the source as read. The exact `Page` byte accessor was not compiled in that form, so it is **unverified**.

Spider Cloud's `SpiderCloudConfig::with_return_format` does this server-side. It is a paid API and is declined (ADR 0076).

## What this changes for #2136

- With `llm_readability`, the cell compares "fetch, parse, Readability-family main text, title" on both sides. That is closer than raw HTML, but it is not the same rules: no Clean-up rules, no landmark handling, and an older Readability algorithm. The cell's label should say so. On this site the extracted texts also differ in content (h1 against nav links), so a same-output check would fail.
- Whether Foxhound's *actual* records on the scale site hold the nav text, as plain `dom_smoothie` does here, was **not verified**: Foxhound was not run. Clean-up rules or the composition may change the result. The `same_work` check does not read the extracted text.

## Unverified

- Foxhound's real extraction output and per-page cost on the scale site. The `dom_smoothie` row is a stand-in, with an extra parse.
- The absolute timings: the machine was at load 22 of 16 vCPUs, and the run was a single one with no repetition across runs.
- The recommended direct `llm_readability` call has not been compiled in the harness.
- The fake names and fan-out only approximate the real scale site (its seeded names and its `SITE_FAN_OUT` were not copied).
