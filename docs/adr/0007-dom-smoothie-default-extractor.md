# dom-smoothie is the default main-content extractor, not rs-trafilatura

The default chain is dom-smoothie for main content, htmd for Markdown, html5ever/scraper for selectors and fields, and lol-html for the fast streaming read of links, meta tags and title. rs-trafilatura, the R&D's choice, ships as a selectable alternative behind the same extractor interface. On the independent WebMainBench corpus (545 pages) dom-smoothie beats rs-trafilatura by 0.025 word F1 (CI above zero) and runs 5 to 6 times faster; rs-trafilatura's 0.06 lead on WCXB does not hold, because 504 of WCXB's 511 answer keys were drafted from rs-trafilatura's own output. Decided in [Adopt collection decisions](https://github.com/Jbmanllr/automan/issues/71), on the lab re-run of [PR 87](https://github.com/Jbmanllr/automan/pull/87).

## Considered Options

- **rs-trafilatura as default**: rejected. One maintainer, no commit since April 2026, three bugs filed in October, and machine-learning layers that Python trafilatura does not have. It is vendored and pinned.
- **lol-html for main content**: rejected. It is a streaming rewriter with no document tree and reached only all-body-text quality on WCXB; it reads fields at 0.27 of scraper's time with 99.9% precision, which is what it is adopted for.
- **mdka or html2md for Markdown**: not adopted yet. mdka leads htmd by 0.045 on WebMainBench, short of a formal verdict. htmd (112 dependent crates) is kept and its two measured defects are fixed upstream: code in `<pre>` without `<code>`, and tables with row headers. If mdka still leads by more than 0.03 after those fixes, the converter switches.

## Consequences

- The 30 hand-written Markdown fixtures become a V1 conformance suite.
- No surveyed competitor uses a trafilatura-class extractor: Jina and Apify use Readability, Firecrawl a fixed list of selectors to remove, Crawl4AI its own pruning filters, and Crawlee, Scrapy and Stagehand none in their core.
