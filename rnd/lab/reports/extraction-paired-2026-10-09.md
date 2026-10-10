# Extraction: paired per-page analysis (D-extraction), October 9, 2026

The mechanical verdict of D-extraction remains `insufficient-n`, because the lane aggregates F1 by page type: seven types, hence seven blocks. Yet the natural unit of extraction is the page. `tools/extraction-paired.ts` recomputes the F1 of each page from the predictions recorded by the retained run, using the lane's definition. The extractors are then compared on the per-page F1 **difference**. A log-ratio would discard the pages where an extractor scores 0, and would favor the one that fails.

Run: `extraction-quality-test-2026-10-08T18-00-46-463Z-751a55`, WCXB held-out test split, 511 pages. The recomputed means are identical to those of the run (0.884 for rs-trafilatura), which validates the recomputation. Data: [`extraction-paired-2026-10-09.json`](extraction-paired-2026-10-09.json), against `scraper-text`, and [`extraction-paired-vs-trafilatura-2026-10-09.json`](extraction-paired-vs-trafilatura-2026-10-09.json), against rs-trafilatura.

## Against rs-trafilatura

| Extractor | Mean F1 | Mean difference (95% CI, bootstrap) | Hodges–Lehmann | Pages where rs-trafilatura wins / loses / ties | Wilcoxon |
| --- | --- | --- | --- | --- | --- |
| rs-trafilatura | 0.884 | — | — | — | — |
| Readability | 0.828 | −0.056 (−0.078 to −0.034) | −0.024 | 352 / 125 / 34 | p < 10⁻⁹ |
| dom-smoothie | 0.823 | −0.061 (−0.083 to −0.038) | −0.024 | 351 / 124 / 36 | p < 10⁻⁹ |
| scraper-text (html5ever, the decision's reference) | 0.719 | −0.165 (−0.186 to −0.144) | −0.160 | 469 / 38 / 4 | p < 10⁻⁹ |
| htmd (Markdown) | 0.523 | −0.361 (−0.383 to −0.339) | −0.364 | 482 / 23 / 6 | p < 10⁻⁹ |

## Reading

- **rs-trafilatura is the best main-content extractor, page by page, not just on average.** It beats the html5ever reference on 469 pages out of 511, and each of the next two on about 350 pages.
- The gap with Readability and dom-smoothie is small at the median (Hodges–Lehmann −0.024) and larger on average (−0.06). On most pages, they are close; on a few, they fail clearly.
- htmd converts to Markdown without isolating the main content. It is an output format, not an extractor: its place is after extraction.

## What this decides

The extraction chain proposed for D-extraction:
1. rs-trafilatura (trafilatura class) isolates the main content;
2. htmd converts it to Markdown when that format is requested;
3. html5ever and scraper are used for structural parsing (selectors, fields), not for content selection.

F1 does not depend on the host. The registry verdict remains `insufficient-n`, since its rule counts blocks by page type, but the per-page analysis settles it. Final adoption remains with the project owner.

## Limitations

- A single corpus (WCXB); WebMainBench remains to be added for cross-checking.
- Word F1 on the main-content text; the quality of the produced Markdown (headings, lists, tables) is not measured here.
