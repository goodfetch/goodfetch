# Extraction re-run, lol-html, Markdown quality and WebMainBench (D-extraction, D-markdown), October 10, 2026

This re-run adds four things to the [October 9 paired analysis](extraction-paired-2026-10-09.md): a lol-html adapter, a second corpus (WebMainBench), the first measurement of Markdown quality, and a field-extraction speed comparison between lol-html and html5ever/scraper. The campaigns, the Markdown oracle, the fixtures and the decision changes were committed before any measurement (commit `504c843`). Every cell of every run is `verified`.

## Host and versions

- Host class: **local-dev**. `pnpm lab preflight local-controlled` refused the host: load 1.27 per CPU (limit 0.35) and 637% foreign CPU (limit 150%), with other agents running. The campaigns requested `local-controlled` with downgrade allowed, and were granted `local-dev`. F1 and Markdown scores do not depend on the host. Times are relative indications on a loaded machine (validity class C).
- Crates: rs-trafilatura 0.2.2, dom_smoothie 0.18.2, **scraper 0.27.0** (was 0.24.0), htmd 0.5.5, lol_html 3.0.1, html2md 0.2.17, fast_html2md 0.0.63, mdka 3.3.0, quick_html2md 0.2.1, html-escape 0.2.15. These were the latest versions on crates.io on October 10, except quick_html2md, pinned to the version rs-trafilatura uses. Node: @mozilla/readability 0.6.0 on linkedom 0.18.13, Node 25.5.0. Markdown oracle parser: marked 18.1.0.
- The re-run reproduces the October 8 scores of the unchanged extractors exactly on WCXB, including scraper-text after the scraper upgrade: rs-trafilatura 0.884, Readability 0.828, dom-smoothie 0.823, scraper-text 0.719, htmd 0.523.

## Runs

| Run | Content | Cells |
| --- | --- | --- |
| `extraction-rerun-wcxb-test-2026-10-10T15-13-38-928Z-b8a0cb` | WCXB held-out test, 511 pages, 6 extractors, 64 shards | 384 verified |
| `extraction-rerun-wmb-2026-10-10T15-14-27-287Z-8ec849` | WebMainBench, 545 pages, 6 extractors, 64 shards | 384 verified |
| `markdown-wmb-2026-10-10T15-15-30-344Z-d7971d` | WebMainBench Markdown structure, 8 converters or chains | 512 verified |
| `markdown-fixtures-2026-10-10T15-13-30-714Z-1256be` | 30 curated fixtures, 5 converters alone | 30 verified |
| `parsing-fields-wcxb-test-2026-10-10T15-17-10-238Z-d929b5` | Fields: lol-html against scraper, WCXB test | 128 verified |

Per-page data: [`extraction-rerun-wcxb-2026-10-10.json`](extraction-rerun-wcxb-2026-10-10.json), [`…-vs-trafilatura`](extraction-rerun-wcxb-vs-trafilatura-2026-10-10.json), [`extraction-rerun-wmb-2026-10-10.json`](extraction-rerun-wmb-2026-10-10.json), [`…-vs-trafilatura`](extraction-rerun-wmb-vs-trafilatura-2026-10-10.json), [`markdown-wmb-2026-10-10.json`](markdown-wmb-2026-10-10.json), [`markdown-wmb-wordf1-2026-10-10.json`](markdown-wmb-wordf1-2026-10-10.json), [`parsing-fields-2026-10-10.json`](parsing-fields-2026-10-10.json).

## What the lol-html pipeline does

lol-html is a streaming rewriter. It sees each tag and text chunk once, builds no tree, and so cannot score content. The adapter (`adapters/extract-rust`, mode `lol-html`) is the most reasonable pipeline built on it alone:

1. **Scan pass:** count `main`/`[role=main]` and `article` elements.
2. **Rewrite pass:** remove the boilerplate elements and their content.
   - Tags: `script, style, noscript, template, svg, canvas, iframe, object, embed, nav, footer, aside, form, button, select, textarea, dialog, menu`.
   - ARIA roles: navigation, banner, contentinfo, complementary, search, dialog, alertdialog, menu, menubar; also `[aria-hidden=true]` and `[hidden]`.
   - Any element whose class or id contains a whole boilerplate token (nav, menu, footer, sidebar, breadcrumb, cookie, consent, banner, ad, ads, advert, social, share, related, newsletter, subscribe, popup, modal, masthead, topbar, toolbar, promo, sponsored, skip).
   - `header` elements outside `main`/`article`. A depth counter, kept by end-tag handlers, keeps an article's own header.
   - The pass also wraps the content of every scope element in marker comments: `main`/`[role=main]` if present, otherwise `article`.
3. **Slice:** keep the text between the markers, or the whole cleaned document when the page has no scope element.
4. **Output:** a last lol-html pass reads the text, decoding entities itself, since lol-html returns raw chunks and raw attribute values. For Markdown (`lol-html-htmd`), the slice goes to htmd.

**Defect found in review, fixed after these runs.** In the runs above, the last pass read text with an element text handler (`text!("*")`). Text sitting directly inside `<main>` or `<article>`, outside any child element, has no enclosing element once the scope is sliced out, so it was dropped: `<main>Intro <p>Para</p> tail</main>` gave only "Para". The adapter now reads text with a document text handler, with a unit test. The pull request #87 review measured the impact by wrapping the slice in a `<div>`: 1 WCXB page of 511 changes, none of 545 on WebMainBench, and the mean F1 is unchanged to four decimals. The figures below come from the runs at `504c843` and were not re-run.

lol-html cannot weigh text density, link density or position, so it cannot isolate content inside a `div` soup. It also cannot tell a forum post from a comment block, and it misses `<header>` elements left implicitly closed. Its end-tag handlers do not fire for implicitly closed elements.

## Main content: word F1, per-page paired analysis

Each cell is the difference in per-page F1 (95% bootstrap CI) against the html5ever reference (`scraper-text`, all body text), the decision's baseline.

| Extractor | WCXB F1 (511) | Δ vs scraper-text | WebMainBench F1 (545) | Δ vs scraper-text | Median ms/page WCXB |
| --- | --- | --- | --- | --- | --- |
| rs-trafilatura | **0.884** | +0.165 (0.144 to 0.186) | 0.820 | +0.241 (0.214 to 0.268) | 14.5 |
| Readability (Node) | 0.828 | +0.109 (0.088 to 0.130) | **0.847** | +0.268 (0.242 to 0.294) | 18.3 |
| dom-smoothie | 0.823 | +0.104 (0.083 to 0.125) | 0.846 | +0.267 (0.241 to 0.293) | 2.3 |
| lol-html (streaming) | 0.716 | −0.004 (−0.039 to 0.030) | 0.746 | +0.167 (0.138 to 0.196) | 1.1 |
| scraper-text (reference) | 0.719 | — | 0.579 | — | 0.9 |
| htmd (whole page) | 0.523 | −0.196 (−0.206 to −0.187) | 0.370 | −0.209 (−0.222 to −0.195) | 1.3 |

Against rs-trafilatura:
- On WCXB, Readability scores −0.056 (−0.078 to −0.034), dom-smoothie −0.061 (−0.083 to −0.038) and lol-html −0.169 (−0.203 to −0.138).
- On WebMainBench, Readability scores **+0.027 (0.009 to 0.045)** and dom-smoothie **+0.025 (0.008 to 0.044)**: rs-trafilatura loses on 262 and 261 of 545 pages, and wins on 225 and 224. The Hodges–Lehmann shift is +0.004 for both, so the median page is nearly tied. lol-html scores −0.074 (−0.098 to −0.051).

Per type:
- On WCXB, lol-html is flat across types, 0.68 to 0.74. It beats the reference on collection (0.683 against 0.492), forum, listing, product and service pages, and loses on article (0.715 against 0.805) and documentation pages (0.695 against 0.781).
- On WebMainBench, every extractor drops from simple to hard pages: rs-trafilatura goes from 0.888 to 0.751, dom-smoothie from 0.906 to 0.802, lol-html from 0.822 to 0.656.

**Ground-truth provenance to keep in mind.**
- *WCXB.* In the test split's ground-truth files, the field `_internal.source` is `"rs-trafilatura-draft"` on 504 of 511 pages; the other 7 are 5 without the field, 1 `verified-gt` and 1 `human-verified`. The repository README (commit `c039d5e`) describes a different first step: "LLM-assisted drafting", with Claude generating the initial annotations, then four human review passes and automated checks. The lab has not established which of the two describes the drafts; the field is the per-file record. The README's step 4, "adversarial review", flags files where the top extractors score low F1 for re-examination: a second way extractor output can shape the ground truth. WCXB and rs-trafilatura have the same author, Murrough Foley (WCXB `LICENSE` and README citation; rs-trafilatura 0.2.2 `Cargo.toml`).
- *WebMainBench.* The ground truth comes from OpenDataLab. Its dataset card (revision `5da0972e`) says the full set was annotated by humans at HTML tag level, in three rounds, and that the 545-page Markdown was "manually calibrated"; it does not say how the Markdown was produced before calibration. As far as the card states, it is not derived from any candidate tested here. The dataset accompanies OpenDataLab's own extractor, MinerU-HTML (Dripper), which is not tested here.

The reversal between the two corpora (rs-trafilatura first on WCXB, Readability and dom-smoothie slightly ahead on WebMainBench) is consistent with an rs-trafilatura bias in WCXB's ground truth. We have not tested whether that bias causes the reversal.

### Mechanical verdict (D-extraction)

The decision now pairs 64 shards per corpus. The rule is non-inferiority against html5ever-scraper with Δ = 5%, and the primary stratum is WCXB. `pnpm lab verdict` gives:

| Option | WCXB, ratio of shard means [90% CI] | Outcome | WebMainBench | Outcome |
| --- | --- | --- | --- | --- |
| lol-html-streaming (primary comparison) | 0.979 [0.939 ; 1.019] | insufficient-n (64 of 103) | 1.304 [1.250 ; 1.361] | insufficient-n (64 of 112) |
| trafilatura-class | 1.231 [1.210 ; 1.252] | **better** | 1.445 [1.390 ; 1.504] | insufficient-n (64 of 101) |
| dom-smoothie | 1.145 [1.118 ; 1.173] | **better** | 1.490 [1.435 ; 1.549] | insufficient-n (64 of 97) |
| readability | 1.152 [1.125 ; 1.179] | **better** | 1.491 [1.434 ; 1.553] | insufficient-n (64 of 97) |
| htmd | 0.724 [0.712 ; 0.735] | **worse** | 0.629 [0.610 ; 0.648] | **worse** |

The `oracle-exact` gate passes everywhere. On WebMainBench, the shard means are more dispersed than on WCXB, so more blocks are required than 64 shards provide. The per-page analysis above, with 545 pairs, is the robustness check.

## Markdown quality (D-markdown), never measured before

**Oracle, declared before measuring** (`src/extraction-markdown.ts`):
- The output and the ground-truth Markdown are read by the same GFM parser, marked 18.1.0.
- Both are reduced to four element types: heading texts (any level), list item texts (nested lists counted apart), table cell texts (pipe or HTML tables) and code block contents (fenced or indented). Before matching, entities are decoded, inline markup and escapes are dropped, and text is lowercased with whitespace collapsed.
- Elements are matched as multisets. A page's structure score is the micro F1 over the four types; links (text and target) are scored apart.
- Two sources of truth:
  - **30 curated fixtures** (`fixtures/markdown-structure/`): six categories, with hand-written expected Markdown, test each converter alone on clean HTML.
  - **WebMainBench's 545 pages**, whose `groundtruth_content` is calibrated Markdown, test whole chains. The converters all receive rs-trafilatura's content HTML, so that only the conversion differs.

### Converters alone, 30 curated fixtures (pooled F1 per element type)

| Converter | Headings | Lists | Tables | Code | Links |
| --- | --- | --- | --- | --- | --- |
| htmd 0.5.5 | 0.96 | 1.00 | 0.97 | 0.93 | 1.00 |
| mdka 3.3.0 | 0.96 | 1.00 | 0.94 | **1.00** | 1.00 |
| html2md 0.2.17 | **1.00** | 1.00 | 0.95 | 0.75 | 1.00 |
| quick_html2md 0.2.1 (rs-trafilatura's) | 0.96 | 1.00 | **1.00** | 0.88 | 0.86 |
| fast_html2md 0.0.63 (lol_html rewriter, used by spider_transformations) | 0.88 | 0.93 | **0.00** | **0.00** | 1.00 |

Failures seen on the fixtures:
- **htmd** writes `<pre>` without `<code>` as a plain paragraph, which loses the code block. It also drops cells of a table whose rows start with `<th>` row headers, losing "8 W" and "Colour".
- **mdka** turns that same row-header table into paragraphs.
- **html2md** splits a cell containing `|` into two cells. In code blocks, it collapses consecutive blank lines and truncates a block that contains a backtick fence.
- **quick_html2md** truncates the same backtick block, drops links inside table cells, and percent-encodes parentheses in URLs. The encoded form is equivalent; the oracle counts it as a miss.
- **fast_html2md** produces no parseable table: rows are glued to the preceding list, and pipes have no separator row. It emits code as inline code in broken fences, without indentation or blank lines. It leaves `&eacute;` and `&euro;` undecoded in headings, cuts a heading at `<br>`, merges a paragraph into a list item, and drops a list inside a blockquote.
- Every converter except html2md cuts a heading at `<br>`: the fixture expects the second line too.

### Chains on WebMainBench, per-page structure F1 (494 pages with structure)

| Chain | Structure F1 | Δ vs rs-trafilatura → htmd (95% CI) | Pages won / lost / tied | Word F1 (545) |
| --- | --- | --- | --- | --- |
| rs-trafilatura → mdka | **0.455** | +0.045 (0.027 to 0.064) | 81 / 42 / 371 | 0.808 |
| rs-trafilatura → html2md | 0.444 | +0.034 (0.017 to 0.051) | 81 / 41 / 372 | 0.784 |
| lol-html → htmd | 0.425 | +0.015 (−0.011 to 0.042) | 125 / 130 / 239 | 0.613 |
| rs-trafilatura → htmd (baseline) | 0.410 | — | — | 0.808 |
| rs-trafilatura native (quick_html2md) | 0.357 | −0.053 (−0.078 to −0.028) | 88 / 148 / 258 | 0.792 |
| rs-trafilatura → fast_html2md | 0.352 | −0.058 (−0.079 to −0.038) | 64 / 134 / 296 | 0.806 |
| dom-smoothie Markdown mode | 0.331 | −0.079 (−0.116 to −0.041) | 115 / 210 / 169 | 0.789 |
| htmd on the whole page | 0.251 | −0.159 (−0.188 to −0.131) | 117 / 277 / 100 | 0.370 |

Pooled per element type (F1):
- **Headings** are close across chains, 0.65 to 0.70. Whole-page htmd reaches 0.45: it finds 93% of the headings but adds many.
- **Lists:** 0.83 to 0.88 with rs-trafilatura, 0.78 for lol-html → htmd. dom-smoothie's Markdown mode keeps only 25% of the list items.
- **Tables:** 0.92 to 0.93 for every chain except fast_html2md, which scores 0.00.
- **Code:** mdka 0.74, html2md 0.62, htmd 0.31, quick_html2md 0.22, fast_html2md 0.13.

The absolute values are low for every chain, partly because the WebMainBench ground truth is strict. It sometimes wraps non-code text in code fences, and it drops nearly all links. The comparison between chains, made on the same pages, is the reliable part.

### Mechanical verdict (D-markdown)

The rule is superiority against rs-trafilatura → htmd with Δ = 5%, on 64 shards. Every option comes out `insufficient-n`, because Markdown scores vary more between shards than the 64 shards can resolve: 124 to 1,029 blocks would be required. The ratio estimates are:

| Option | Ratio [95% CI] | Required |
| --- | --- | --- |
| mdka | 1.131 [1.070 ; 1.199] | 180 |
| html2md | 1.079 [1.030 ; 1.132] | 124 |
| lol-html-htmd | 1.093 [0.998 ; 1.211] | 524 |
| trafilatura-native | 0.871 [0.800 ; 0.951] | 406 |
| fast-html2md | 0.851 [0.781 ; 0.926] | 381 |
| dom-smoothie-markdown | 0.815 [0.714 ; 0.938] | 1,029 |
| htmd-whole-page | 0.610 [0.547 ; 0.678] | 641 |

The per-page paired analysis, with 494 pairs, separates mdka and html2md from htmd (CIs above zero), and places quick_html2md, fast_html2md and dom-smoothie below it.

## lol-html for selectors and fields (D-extraction, field precision and recall)

On the 511 WCXB test pages, `lol-html-fields` reads the same fields as `scraper-fields` in one streaming pass: first `title`, meta description, canonical, og:title, `h1` texts, `a[href]`, `img[src]`. The html5ever tree is the reference.

- **Agreement:**
  - all fields are identical on 484 of 511 pages;
  - field precision is 0.9986 and field recall 0.9959;
  - title, canonical, og:title, h1 and images agree on all 511 pages.
- **Where they differ:**
  - *Links* differ on 13 pages, for two reasons; page 4787 has both, so 12 + 2 − 1 = 13:
    - On 12 pages, the html5ever tree holds more `a` elements than lol-html sees, with nothing extra on the lol-html side. On page 4442, checked by hand, the source has 3 `<a>` tags for a given URL and the tree 7. This is consistent with html5ever cloning misnested `<a>` elements, as its tree-construction rules (adoption agency, reconstruction of formatting elements) require. lol-html counts source tags, html5ever DOM elements: neither is an error, they define "a link" differently.
    - On 2 pages, `href` values contain raw carriage returns, which html5ever normalizes and lol-html does not: 110 `href` values on page 4071, where the link counts are otherwise equal (621 each), and 1 on page 4787, which is also among the 12.
  - *Description* differs on 14 pages, all only by CR/CRLF: html5ever normalizes newlines when it preprocesses the input, lol-html returns the raw value.
- **Speed:** per page, lol-html takes 0.27 of scraper's time (CI 0.265 to 0.276), with a median of 0.25 ms against 0.93 ms. That is about 3.7 times faster on a local-dev host.

## What this changes

- **lol-html can** read selector-addressed fields and attributes in one streaming pass, about 3.7 times faster than building the html5ever tree, with 99.6% or more agreement. It is a credible fast path for fields and links when DOM-normalization semantics are not required. It can also remove boilerplate in a stream; its selection, through htmd, gives Markdown structure on par with rs-trafilatura → htmd (+0.015, CI −0.011 to 0.042), with much lower word F1 (0.613 against 0.808).
- **lol-html cannot** replace a main-content extractor. Its streaming heuristic reaches only 0.716 F1 on WCXB, the level of all body text, and 0.746 on WebMainBench, 0.07 to 0.10 below the scoring extractors. Its rewriter-based Markdown converter in fast_html2md is the weakest converter measured.
- **Main content:** rs-trafilatura stays first on WCXB, but WebMainBench does not confirm its lead. Readability and dom-smoothie are slightly better there (+0.025 to +0.027, nearly tied at the median), and dom-smoothie is 5 to 6 times faster than rs-trafilatura per page on this host (median 2.3 against 14.5 ms on WCXB). The choice between rs-trafilatura and dom-smoothie is therefore not settled by quality alone.
- **Markdown:** with the same content HTML, mdka and html2md produce better structure than htmd on WebMainBench. On the fixtures, htmd has two concrete defects (code in `<pre>` without `<code>`, tables with row headers), and html2md is weak on code. rs-trafilatura's own Markdown output and fast_html2md are worse than htmd.

Final adoption remains with the project owner.

## Not done, limits

- WebMainBench: only the 545-page subset with calibrated Markdown was used (109 MB, Apache-2.0, from Hugging Face at revision `5da0972e`). The full 7,809-page file (1.35 GB) has tag-level annotations, not Markdown, and was left out. The paper's ROUGE-5 metric and edit-distance metrics were not reproduced; word F1 and the structure oracle are the lab's own.
- Word F1 tokenizes on `\w+`, so a run of Chinese characters counts as one token. About 27 pages are in Chinese, and some pages without a language tag likely are too.
- The structure oracle matches text exactly after normalization. Encoded but equivalent URLs and slightly different cell text count as misses.
- The timings come from a local-dev host under load (other agents running). They are paired per page, but not qualified as local-controlled.
- `parsing-representative` (malformed HTML, large DOM, 1 to 1,000 MiB streaming) is still not built. The field comparison above uses WCXB pages only.
- **lol-html's rules are not validated on held-out pages.** The boilerplate tags, ARIA roles, class/id tokens and scope rules were committed 9 seconds before the first run, and which pages they were developed on was not recorded. The WCXB test split and WebMainBench were both available locally while they were written, so neither corpus can be treated as held out for lol-html. The rules are generic (no site- or page-specific entry), which limits but does not exclude fitting.
- **WebMainBench's HTML carried the annotators' answer key.** On 544 of 545 pages, the elements selected as main content have `cc-select="true"` and the class `mark-selected`, and every page has an injected `<style id="cc-extraStyle">` (in `<body>` on page 0313). No candidate or lab code reads them, so these results are not affected through them. The prepare tool now strips them and refuses a page that still has any; see `corpora/webmainbench.manifest.json`, field `annotationMarkers`. The runs above read the unstripped HTML.
