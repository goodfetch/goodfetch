# Extraction quality on WCXB, October 8, 2026

Measurement of D-extraction and D-light-engine on the WCXB corpus (Murrough Foley, 2026, CC-BY-4.0, commit pinned in `corpora/wcxb.manifest.json`). 2,008 pages, 7 page types. The metrics are those of its `evaluate.py`: word-level F1 against the main content, share of required snippets found, share of boilerplate snippets present. The lab's scores are identical to those of `evaluate.py` to the fourth decimal place for all five extractors.

Reference score on the **held-out test set** (511 pages), run `extraction-quality-test-2026-10-08T18-00-46-463Z-751a55`; the development set (1,497 pages, run `extraction-quality-dev-2026-10-08T17-58-37-038Z-ee347f`) gives the same ranking.

| Extractor | Article | Collection | Documentation | Forum | Listing | Product | Service | Overall F1 | Required snippets | Boilerplate present | Failures | ms per page (median, articles) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rs-trafilatura | 0.951 | 0.775 | 0.973 | 0.787 | 0.745 | 0.755 | 0.832 | **0.884** | 0.858 | 0.099 | 0 | 11.3 |
| dom-smoothie | 0.935 | 0.585 | 0.860 | 0.700 | 0.616 | 0.638 | 0.786 | **0.823** | 0.739 | 0.078 | 3 | 1.9 |
| readability | 0.935 | 0.590 | 0.880 | 0.715 | 0.616 | 0.637 | 0.796 | **0.828** | 0.747 | 0.087 | 0 | 9.1 |
| scraper-text | 0.805 | 0.492 | 0.781 | 0.635 | 0.612 | 0.536 | 0.666 | **0.719** | 0.762 | 0.953 | 0 | 0.8 |
| htmd | 0.631 | 0.247 | 0.600 | 0.423 | 0.373 | 0.329 | 0.441 | **0.523** | 0.763 | 0.964 | 0 | 1.1 |
- **rs-trafilatura** (Rust port of trafilatura) dominates, especially outside articles: collections, forums, listings and products, where Readability-type extractors fall behind.
- **dom_smoothie** (Rust port of Readability) matches Readability.js, with a median of about 2 ms per page versus 9 ms for Readability on linkedom. It fails on a few pages that Readability handles.
- The two references without extraction (all the text, htmd conversion of the whole page) let almost all the boilerplate through: they bound the gain of a real main-content extraction.
- Timings are measured on a Mac in use (`local-dev`) and are only indicative; quality, on the other hand, does not depend on the machine.

Lightpanda (D-light-engine) is not measured yet: it requires an out-of-process engine, to be integrated as an adapter.
