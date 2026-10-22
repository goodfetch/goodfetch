# dom-smoothie is the default main-content extractor, not rs-trafilatura

The default chain is dom-smoothie for main content, htmd for Markdown, html5ever/scraper for selectors and fields, and lol-html for the fast streaming read of links, meta tags and title. rs-trafilatura, the R&D's choice, ships as a selectable alternative behind the same extractor interface. On the independent WebMainBench corpus (545 pages) dom-smoothie beats rs-trafilatura by 0.025 word F1 (CI above zero) and runs 5 to 6 times faster; rs-trafilatura's 0.06 lead on WCXB is not confirmed on an independent corpus. The reversal is consistent with a bias in WCXB, whose answer keys were drafted from rs-trafilatura's own output for 504 of 511 pages before review, but that cause was not tested. Decided in [Adopt collection decisions](https://github.com/goodfetch/goodfetch/issues/71), on the lab re-run of [PR 87](https://github.com/goodfetch/goodfetch/pull/87).

## Considered Options

- **rs-trafilatura as default**: rejected. One maintainer, no commit since April 2026, three bugs filed in October, and machine-learning layers that Python trafilatura does not have. It ships vendored in a fork Goodfetch can patch, pinned to a reviewed version.
- **lol-html for main content**: rejected. It is a streaming rewriter with no document tree and reached only all-body-text quality on WCXB; it reads fields at 0.27 of scraper's time with 99.6% or more agreement, which is what it is adopted for.
- **mdka or html2md for Markdown**: not adopted yet. mdka leads htmd by 0.045 on WebMainBench, short of a formal verdict. htmd (112 dependent crates) is kept and its two measured defects are fixed upstream: code in `<pre>` without `<code>`, and tables with row headers. If mdka still leads by more than 0.03 after those fixes, the converter switches.
  > Note (agent decision, accepted by the owner on 2026-10-22; [#783](https://github.com/goodfetch/goodfetch/issues/783)): until htmd takes them upstream, `goodfetch-extractor` carries both fixes as htmd element handlers of its own, each with a fixture that fails without it, and a third the 30 fixtures need, which every converter but html2md lacked: a heading with a `<br>` stays one heading. Offering the fixes upstream is a pull request to htmd's repository, a public action an agent does not take. The 30 fixtures are the Proving ground's `extraction/7` task, judged by Markdown structure as the lab's oracle judged them, with a heading's level, a code block's language and a link's relative or absolute form not counted, and case and paragraphs counted.

## Consequences

- The 30 hand-written Markdown fixtures become a V1 conformance suite.
- The Markdown converters were compared on rs-trafilatura's content HTML; the adopted dom-smoothie then htmd chain is measured again before the switch rule above is applied.
- No surveyed competitor uses a trafilatura-class extractor: Jina and Apify use Readability, Firecrawl a fixed list of selectors to remove, Crawl4AI its own pruning filters, and Crawlee, Scrapy and Stagehand none in their core.
- Before the extractor, the engine removes the elements matched by the cosmetic rules of the Clean-up rules, in every tier; raw HTML, WARC, Run traces, the lol-html link reading above and the user's own selectors see the page untouched (ADR 0047).
