# The product's chain on WebMainBench: ADR 0007's converter switch rule (D-markdown), November 26, 2026

[#818](https://github.com/goodfetch/goodfetch/issues/818) applies ADR 0007's switch rule: with htmd's two measured defects fixed, the converter switches to mdka if mdka still leads by more than 0.03 on WebMainBench. The [October 10 re-run](extraction-rerun-2026-10-10.md) compared the converters on rs-trafilatura's content HTML, before the fixes; ADR 0007 asks for the adopted dom-smoothie then htmd chain to be measured again before the rule is applied. This run measures `goodfetch-extractor` itself.

**Verdict: the converter stays htmd.** On the product's main content, mdka leads the product's htmd by **+0.011** in Markdown structure F1 (95% CI −0.003 to +0.026, Wilcoxon p = 0.088), not more than 0.03. The fixes are worth +0.039 over plain htmd on the same HTML.

These are lab readings, not Goodfetch figures: until the owner answers #818's question on public corpora's labels (ADR 0025 is not amended), no WebMainBench score is published as a Goodfetch figure.

## What was measured

- **The product's chain.** The lab's Rust adapter (`adapters/extract-rust`) calls `goodfetch-extractor`'s `formats` from the repository's crates at commit `868f21035`, on each page as served (`Serialization::Served`), with no Clean-up rule and `markdown.content: "main"`. It reads the main content as Markdown and as HTML; both come from the same nodes.
  - `goodfetch`: the product's Markdown, dom-smoothie (the default `content.extractor`, default attempts) then htmd with goodfetch's element handlers (the two fixes of #783 and the heading-with-`<br>` one).
  - `goodfetch-htmd`, `goodfetch-mdka`: the same main-content HTML through plain htmd 0.5.6 or mdka 3.3.0.
  - `goodfetch-trafilatura`, `goodfetch-trafilatura-mdka`: the same with `content.extractor: trafilatura`.
  - `rs-trafilatura-htmd`, `rs-trafilatura-mdka`: the October 10 chains, rs-trafilatura's content HTML through plain htmd or mdka, kept as the reference.
- **The repository's patched crates.** A `[patch.crates-io]` applies only in the workspace that declares it, so the lab's manifest declares the repository's four vendored crates (dom_smoothie 0.18.2+goodfetch.1, rs-trafilatura 0.2.2+goodfetch.1, html-cleaning, quick_html2md); the lab's own `rs-trafilatura-*` modes read the patched crates too. The lab's exact pins of htmd, lol_html and serde_json were raised to the product's (0.5.6, 3.0.2, 1.0.154). In the lab, `goodfetch-extractor`'s dependency tree differs from the repository's only in crates that read neither HTML nor Markdown (jiff, postcard, psl, uuid, syn and a few others); every HTML and Markdown crate is the same version.
- **Corpus.** WebMainBench, OpenDataLab's 545-page set at Hugging Face revision `5da0972e`, its file's SHA-256 checked against `corpora/webmainbench.manifest.json`. Structure F1 counts the 494 pages whose expected Markdown has structure; word F1 counts all 545. The corpus was downloaded once at that pinned revision; the measurement read the pages from disk and reached no live site.
- **Metrics.** Markdown structure F1 and word F1 as the lab defines them (`src/extraction-markdown.ts`), paired per page by `tools/extraction-paired.ts`: mean difference with a 95% block bootstrap interval, Hodges-Lehmann, Wilcoxon signed-rank, and wins, losses and ties.
- **Run.** `markdown-wmb-goodfetch-2026-11-26T09-31-57-694Z-25379f`, campaign `campaigns/v2/extraction/markdown-wmb-goodfetch.json`, 7 chains × 64 shards = 448 cells, all verified. Host: the owner's rented gate host (ADR 0076), granted `local-dev` after `local-controlled` was refused under the other agents' load. F1 scores do not depend on the host; times are indications only.
- **Lab workaround.** `pnpm lab run` imports the workflow lane's SQLite binding even for an extraction campaign, and building it needs a Go toolchain download. The run loaded a module hook that resolves that one binding to an empty module; no extraction code reads it.

## Markdown structure F1 (494 pages)

Paired against the product (`goodfetch`, 0.324):

| Chain | Mean | Δ vs product (95% CI) | Wilcoxon p | Wins / losses / ties |
| --- | --- | --- | --- | --- |
| goodfetch (dom-smoothie, htmd with fixes) | 0.324 | — | — | — |
| goodfetch-mdka | 0.335 | **+0.011 (−0.003 to +0.026)** | 0.088 | 42 / 31 / 421 |
| goodfetch-htmd (plain htmd) | 0.285 | −0.039 (−0.053 to −0.026) | < 0.001 | 7 / 55 / 432 |
| goodfetch-trafilatura | 0.462 | +0.139 (+0.108 to +0.170) | < 0.001 | 237 / 73 / 184 |
| goodfetch-trafilatura-mdka | 0.481 | +0.157 (+0.124 to +0.191) | < 0.001 | 256 / 81 / 157 |
| rs-trafilatura-htmd | 0.410 | +0.086 (+0.052 to +0.121) | < 0.001 | 203 / 105 / 186 |
| rs-trafilatura-mdka | 0.455 | +0.132 (+0.097 to +0.167) | < 0.001 | 227 / 88 / 179 |

By page type (simple 149, mid 201, hard 144): goodfetch 0.353 / 0.361 / 0.241; goodfetch-mdka 0.357 / 0.372 / 0.259; goodfetch-trafilatura 0.571 / 0.448 / 0.370.

Other pairs:

- **With the trafilatura extractor**, mdka leads the product's htmd by +0.018 (+0.003 to +0.035; Wilcoxon p = 0.112; 55 / 56 / 383): also under 0.03.
- **The reference reproduces October 10 exactly.** rs-trafilatura-mdka against rs-trafilatura-htmd: +0.045 (+0.027 to +0.064), the figure ADR 0007 cites.
- **The fixes close most of that gap.** Against rs-trafilatura's content, the fixed htmd (`goodfetch-trafilatura`, 0.462) leads plain htmd by +0.052 (+0.037 to +0.069) and sits level with mdka (0.455; −0.007, −0.025 to +0.011).

## Word F1 of the Markdown (545 pages)

Paired against the product (0.776):

| Chain | Mean | Δ vs product (95% CI) |
| --- | --- | --- |
| goodfetch-htmd | 0.775 | −0.000 (−0.001 to +0.000) |
| goodfetch-mdka | 0.763 | −0.012 (−0.015 to −0.010) |
| goodfetch-trafilatura | 0.741 | −0.034 (−0.052 to −0.017) |
| goodfetch-trafilatura-mdka | 0.749 | −0.026 (−0.045 to −0.009) |
| rs-trafilatura-htmd | 0.808 | +0.032 (+0.012 to +0.052) |
| rs-trafilatura-mdka | 0.808 | +0.032 (+0.012 to +0.052) |

mdka loses words on 451 of 545 pages against the product's htmd: the switch would trade a structure lead within noise for a measured word loss.

## Decision and findings

- **Converter: htmd stays** (ADR 0007's rule). mdka's lead on the product's chain is +0.011, under the 0.03 threshold, and its interval includes zero; recorded in ADR 0007's note for #818. The fixture suite is untouched.
- **Finding, not a decision of this ticket: the extractors trade structure for words.** On the product's own chain, the trafilatura extractor leads dom-smoothie by 0.139 in structure F1, while dom-smoothie leads it by 0.034 in word F1 of the Markdown. The lab's rs-trafilatura chains, which convert rs-trafilatura's content HTML directly, score higher on words (0.808) than both the product's dom-smoothie Markdown (0.776) and its trafilatura Markdown (0.741); this run does not say which step of `formats` accounts for the difference. ADR 0007's extractor default rests on word F1 of the extracted text, a different reading from word F1 of the Markdown; this run changes no default, and is input for #185 and #2191's reading of extraction quality.

## Data

Per-chain summaries and paired comparisons: [`markdown-wmb-goodfetch-2026-11-26.json`](markdown-wmb-goodfetch-2026-11-26.json) (structure F1, against the product), [`…-vs-trafilatura`](markdown-wmb-goodfetch-vs-trafilatura-2026-11-26.json) (against the product's trafilatura chain), [`…-vs-rs-trafilatura`](markdown-wmb-goodfetch-vs-rs-trafilatura-2026-11-26.json) (against the October reference) and [`markdown-wmb-goodfetch-wordf1-2026-11-26.json`](markdown-wmb-goodfetch-wordf1-2026-11-26.json) (word F1).

To reproduce, from `rnd/lab/`: build the adapter (`cargo build --release --manifest-path adapters/extract-rust/Cargo.toml`), prepare WebMainBench as the [October 10 report](extraction-rerun-2026-10-10.md) did, run `pnpm lab run campaigns/v2/extraction/markdown-wmb-goodfetch.json` (with the SQLite binding built, or resolved to an empty module as above), then `node tools/extraction-paired.ts results/<run> goodfetch <out.json> markdown` (or `f1` for word F1).
