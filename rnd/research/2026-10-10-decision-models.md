# Decision models for crawl, extraction, automation, vision and repair

Research of **October 10, 2026** for ticket [#92](https://github.com/Jbmanllr/automan/issues/92) (map [#61](https://github.com/Jbmanllr/automan/issues/61)). It feeds [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72), where the owner asked for this investigation before deciding how models are called. Like the rest of `rnd/`, this note is input for inspiration; it adopts no term or decision.

A **decision model** here means a model that picks among a closed set of candidates, returns a confidence, and can abstain. The note starts from the R&D notes of October 7 ([browser frameworks, "Jev and decision models"](2026-10-07-browser-frameworks.md), [direction, "Models: optional, replaceable and budgeted"](2026-10-07-direction-automan.md), [Stagehand contracts and alternatives](2026-10-07-stagehand-alternatives-followup.md)) and does not repeat the [AI browser frameworks survey](https://github.com/Jbmanllr/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md).

## Summary

- **Jev is a hosted, closed "System One" model** from TypeSafe: a `state` plus typed questions (Choice, Score, Noul) in, per-option probabilities and a confidence out, no text generation. `jev-1.13.0`, text only, 64k tokens, $0.042 per million input tokens, output free. Architecture, size and training data are undisclosed; TypeSafe publishes no standard benchmarks and no calibration metric by policy; no self-hosting is offered. Its own documentation lists its failure modes, including option-order bias, numbers and dates, large noisy states and adversarial content. [§2.1](#21-what-jev-is)
- **The category is no longer one vendor.** In the three weeks after Jev, Cloudflare (Clef 27B and Clef-Flash 9B), Perplexity (pplx-decider 27B) and OpenAI (Decisions API, public beta) shipped the same request shape, and independents released small Apache-2.0 models (decider 0.8B–4B, Jeff 0.8B) with GGUF builds. Clef, Decider, decider and Jeff weights are open. Mechanically they are LLM backbones run in prefill-only mode with a readout over the offered options. Almost every figure is self-reported; only decider publishes ECE and held-out splits, and it calls itself overconfident on hard sets. [§2.2](#22-the-class-grew-in-three-weeks-hosted-and-open-decision-models)
- **Stagehand PRs 2952, 2953, 2954, 2955 and 3103 are still open and unmerged** on 2026-10-10 (3103 is a draft); the stack now has eleven PRs, including a provider seam for four vendors and a `resolved | abstained(reason)` driver contract. npm `latest` (4.2.0) contains no `experimentalDecisions`. Its own figures still say "fast path, not replacement": act p50 1.97 s → 0.46 s, but 27/40 with the LLM disabled; thresholds tuned in-sample; the cached-target check never evaluated end to end. [§2.3](#23-stagehands-decision-stack-state-on-2026-10-10)
- **For DOM and accessibility-tree selection, "rank then choose" is the established pattern** (Mind2Web's DeBERTa ranker, Recall@50 85–89%). Fine-tuned selectors lose more than 10 points on unseen sites; offline scores overstate live ones; distilled small agents gain in their training environment and little elsewhere. No paper was found that calibrates element selection itself. [§3](#3-dom-and-accessibility-tree-selection-rerankers-distillation-locator-repair-abstention)
- **Vision grounders always answer.** ScreenSpot-v2 is saturated (93–96); ScreenSpot-Pro single-pass open models reach about 55–73, and everything above about 75 uses zoom or several calls. On OSWorld-G's refusal subset almost every model scores 0%. The usable confidence hooks are a separate verifier (GUI-Actor-Verifier-2B), detector boxes (OmniParser, now fully MIT) and cross-model agreement. [§4](#4-gui-vision-grounding-models)
- **For crawl and extraction the evidence favours rules plus cheap classifiers.** Challenge and paywall detection are rule problems (`cf-mitigated: challenge`, CAPTCHA iframe hosts, JSON-LD); a bare password-field rule is weak for login intent (60.8% vs 95% for PhishIntention). Every measured crawl-priority gain scores fetched pages, not URLs. The one small extraction model with an extractive design (MinerU-HTML v1.0, Apache-2.0) trails rs-trafilatura on WCXB and is about 35× slower. [§5](#5-page-classifiers-crawl-scoring-and-extraction-models)
- **A Rust engine can run all of this locally**: `ort` for cross-encoders and classifiers (22 MB int8 MiniLM, about 1 ms per pair on CPU), `llama-cpp-2` for GGUF small LMs and VLMs (no semver; pin exactly), candle, tract or burn as pure-Rust alternatives. Whether GGUF conversions keep a decision model's readout and temperature is unverified. [§6](#6-running-small-models-locally-from-a-rust-engine)
- **Synthesis and lab.** A decision provider fits as a rung between the fingerprint check and the LLM on the accessibility tree, and as a semantic "same target?" check inside cached replay and repair. Its contract options: candidates with stable ids in; `resolved { id, probabilities, confidence }` or typed `abstained { reason }` out; thresholds per provider, model version and decision kind fitted on held-out labels; journaled and counted like any model call. A lab campaign would need held-out sites, abstention and selective-risk metrics, and stress variants (twins, decoys, injection, long lists, "none" pages). [§7](#7-synthesis-for-automan-where-a-decision-provider-sits), [§8](#8-what-a-lab-campaign-would-have-to-measure)

## 1. How to read this note

- Sources were read on **2026-10-10** through `gh` (GitHub API), Hugging Face model cards and API, arXiv, vendor documentation and crates.io, unless a line says otherwise. Exa was unavailable; web search was used to find pages, then the primary page was read.
- **Self-reported** means the authors or vendor ran the benchmark on their own model. **Re-run by X** means another team published its own evaluation of the same checkpoint. **(unverified)** marks a claim not traced to a primary source. "Not found" means not found in the sources read, not proof of absence.
- Benchmarks are not comparable across rows when harnesses differ: the same vision checkpoint moves by 10 points or more between harnesses (§4), and decision-model suites differ per vendor.
- Model names such as GPT-6 Luna, Gemini 3.8 Flash or Qwen3.8 are the vendors' current names on the reading date, copied as written.
- Sections 2 to 6 are the facts per family; section 7 maps them onto Automan's ladder; section 8 lists what a lab campaign would have to measure; section 9 holds the caveats; section 10 the method.

## 2. Jev and the "System One" decision-model class

### 2.1 What Jev is

| Item | Fact | Source |
| --- | --- | --- |
| Vendor | TypeSafe AI. Co-founder Diogo Almeida, credited by TypeSafe as a co-inventor of RLHF. | [AI primer](https://docs.typesafe.ai/introduction/machine-learning-primer), [launch post](https://typesafe.ai/blog/introducing-system-one-models-and-jev) |
| Product | A hosted "System One" model: it takes a `state` (text, JSON or an array of text) and typed questions, and returns typed answers with probabilities. It **does not generate text**. | [System One](https://docs.typesafe.ai/concepts/system-one), [coding agents page](https://docs.typesafe.ai/introduction/coding-agents) |
| Question types | `Choice` (one option from a list: `choice`, per-option `probabilities`, `confidence`), `Score` (ordered rubric levels: expected `score`, `probabilities`, `confidence`), `Noul` (probability that a statement is true). Several questions per request, each evaluated in isolation against the same state. | [Primitives](https://docs.typesafe.ai/primitives), [Choice](https://docs.typesafe.ai/primitives/choice) |
| Confidence | Derived from the spread of the distribution, not a separate head: Noul `|2p−1|`; Choice `(p_max − 1/n)/(1 − 1/n)`; Score a normalised mean-absolute-deviation formula. | [Confidence](https://docs.typesafe.ai/confidence) |
| Current model | `jev-1.13.0` (aliases `jev-latest`, `jev-preview` both point to it). Text input only, no images. 64k tokens per request; 32k for state plus the longest question. English is the primary language; other languages "handled but not equally well". | [Models](https://docs.typesafe.ai/models) |
| Price | $0.042 per million input tokens; output free. Rate limits 100K tokens/s and 80 requests/s, "adjusting dynamically". | [Models](https://docs.typesafe.ai/models) |
| Latency | 70–500 ms end to end, measured by TypeSafe from West Coast laptops (self-reported). Cloudflare's own run measured Jev at 524.1 ms median, 536.0 ms p95. | [Launch post](https://typesafe.ai/blog/introducing-system-one-models-and-jev), [Clef-Flash model card](https://huggingface.co/Cloudflare/clef-flash) |
| Training | "RLCD", reinforcement learning for calibrated decisions; same weights for every account; no fine-tuning or LoRA on customer data. | [AI primer](https://docs.typesafe.ai/introduction/machine-learning-primer), [Models](https://docs.typesafe.ai/models) |
| Architecture, size, training data | **Not disclosed.** The launch post mentions a "new model architecture" and a "parallel sampler" without details; its FAQ asks where the training data comes from and does not answer. TypeSafe's GitHub org holds forks of vLLM and LLaDA (a diffusion LM); that is an observation, not evidence of Jev's architecture. | [Launch post](https://typesafe.ai/blog/introducing-system-one-models-and-jev), [TypeSafe GitHub org](https://github.com/typesafe-ai) |
| Benchmarks | TypeSafe publishes **no standard benchmark table** by policy and no calibration metric (ECE or similar). It publishes dated "workflow evals" scored against the average of two frontier LLMs, and headline speed/cost ratios (193.6× faster, 444.6× cheaper) that it calls "on the higher end". | [Antibenchmaxxing post](https://typesafe.ai/blog/antibenchmaxxing), [launch post](https://typesafe.ai/blog/introducing-system-one-models-and-jev), [Typesafe Evals](https://evals.typesafe.ai/) |
| License, self-hosting | **Proprietary, API only.** The enterprise page asks "Can we run Jev in our own VPC or on-premises?" without answering; zero data retention for enterprise customers. SDKs (Python, JS) and the `system-one-adapter-python` LLM emulator are MIT. | [Enterprise](https://typesafe.ai/enterprise), [Legal](https://docs.typesafe.ai/legal), [typesafe-sdk-python](https://github.com/typesafe-ai/typesafe-sdk-python), [system-one-adapter-python](https://github.com/typesafe-ai/system-one-adapter-python) |
| Access | Direct API (`POST https://api.typesafe.ai/v1/systemone`) and through OpenRouter (`/api/v1/systemone` and `/api/alpha/decisions`, model `typesafe/jev-1.13`). | [API reference](https://docs.typesafe.ai/api), [OpenRouter Jev guide](https://openrouter.ai/docs/guides/community/jev.md) |

**Known failure modes, stated by TypeSafe itself** for `jev-1.13` (reviewed 2026-10-02): literal reading of instructions; numbers, counting and date comparison; multi-hop indirection; accuracy falls as `state` grows with irrelevant detail ("context rot"); adversarial content in the state can move the answer (prompt injection is not handled); contradictory instructions and criteria; **option order bias toward the first option**; no generation. [Jev 1.13 jaggedness](https://docs.typesafe.ai/model-jaggedness/jev-1.13).

For Automan this list matters directly: page text is adversarial-capable untrusted input (Q28 in [#72](https://github.com/Jbmanllr/automan/issues/72)), candidate lists from an accessibility tree are long and noisy, and option order is something the engine controls, so it can shuffle and re-ask.

The TypeSafe cookbooks are the closest published evidence for selection over many candidates. A BM25 shortlist of 30 passages per query, re-ranked with one Noul per pair, raised top-1 from 5% to 18% and top-10 from 38% to 62% on 40 CLERC legal queries (self-reported, small sample). [Re-ranking cookbook](https://docs.typesafe.ai/cookbooks/rerank_typesafe). The launch post states up to 255 choices per question, with a two-stage scoring process beyond that. [Launch post](https://typesafe.ai/blog/introducing-system-one-models-and-jev).

### 2.2 The class grew in three weeks: hosted and open decision models

Jev opened the category on 2026-09-15 (third-party dating, e.g. [Sanity glossary](https://www.sanity.io/glossary/jev-typesafe-ai-model); unverified against a TypeSafe release note). By 2026-10-10 the same request shape (`state` + typed questions → probabilities) is served by several vendors and by open weights. Stagehand PR [#3102](https://github.com/browserbase/stagehand/pull/3102) already wraps four of them behind one codec.

| Model | Vendor | Weights / license | Base | Inputs | Price (input, per M tokens) | Published evidence | Source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Jev 1.13 | TypeSafe | closed, API only | undisclosed | text | $0.042 | no standard benchmarks; workflow evals | [Models](https://docs.typesafe.ai/models) |
| Clef (27B) and Clef-Flash (9B) | Cloudflare | **Apache-2.0**, HF; GGUF conversions by ggml-org and others | Qwen3.8-27B / Qwen3.5-9B, frozen backbone plus rank-256 adapters and a "joint schema head" scoring all options in one prefill pass | text, JSON, **images, video** | $0.24 / $0.09 on Workers AI (third-party reporting, unverified on a Cloudflare price page) | 43-benchmark "Decision Index" run by Cloudflare: e.g. BFCL 98.5/98.8 vs Jev 95.8; median latency 209.3 / 38.8 ms vs Jev 524.1 ms; Jev ahead on GPQA, MMLU-Pro, BBH. **Self-reported, no ECE published, no web-element benchmark.** | [Clef-Flash card](https://huggingface.co/Cloudflare/clef-flash), [Clef card](https://huggingface.co/Cloudflare/clef), [Cloudflare blog](https://blog.cloudflare.com/clef-decision-models), [ggml-org GGUF](https://huggingface.co/ggml-org/Clef-Flash-GGUF), [pricing report](https://www.developersdigest.tech/blog/cloudflare-clef-decision-models-2026) |
| pplx-decider v1 / v1.1 (27B) | Perplexity | **Apache-2.0**, HF (the card says the repo is gated) | Qwen3.8-27B with a 255-row decision readout and non-causal attention | text, images | $0.04 or $0.02 (third-party sources disagree; Perplexity's own doc page was not found) | Decision Index overall 61.56 (v1.1) vs Jev 57.9 (self-reported); ~49 GiB GPU memory | [v1.1 card](https://huggingface.co/perplexity-ai/pplx-decider-v1.1-27b), [OpenRouter listing](https://openrouter.ai/perplexity/pplx-decider-v1.1-27b) |
| GPT-6 Luna (Decisions API) | OpenAI | closed, API only, **public beta** | — | text, base64 images | $0.10 | "about 10x faster than the Responses API"; no calibration claim; answers can be `refusal`; advises thresholds from your own labels | [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions) |
| decider 0.8B / 2B / 4B / 35B-A3B, 2B-vision | Mapika (independent) | **Apache-2.0**, HF, GGUF | Qwen3.5 bases | text (vision variant: image) | self-hosted | **The only family found that publishes ECE and held-out splits**, plus web data: 2B v11 Mind2Web element choice 83.9% (in-task, Mind2Web is in its training mix), live MiniWoB++ 90.3% (22 click-only tasks), held-out ECE 0.156 ("still overconfident"); 3.2 ms per request on one B300 with CUDA graphs. Self-reported. | [decider-2b card](https://huggingface.co/Mapika/decider-2b), [repo](https://github.com/Mapika/decider) |
| Jeff v1.3 (0.8B) + LoRA adapters | firelex / jeffhub (independent) | Apache-2.0 weights, MIT code, GGUF | Qwen3.5-0.8B | text | self-hosted | general panel 78.6%, ECE 0.028 (base); per-task adapters reach 84.8–98.7% incl. a `nav` adapter (voice commands to on-screen items) at 0.23 s on an M4 Max; zero-shot on long option lists collapses without an adapter (legal-clauses 7.4%). Self-reported. | [jeff-base card](https://huggingface.co/mstrasser/jeff-base), [repo](https://github.com/firelex/jeff) |

Other names appear in these tables without primary sources found here: "Kev 9B", "DiffusionGemma Jev" and "Laya" (Cloudflare's comparison; Laya at 5.8 ms median but far lower accuracy) ([Clef-Flash card](https://huggingface.co/Cloudflare/clef-flash)), and a community "Jev Decision Index" ([HF space](https://huggingface.co/spaces/multimodalart/jev-decision-index), not inspected).

**What this means for "decision model" as a category.** Mechanically, the open variants are an LLM backbone run in prefill-only mode with a readout over the offered options (letter logits or a small head), a softmax per question, and a fitted temperature. That is the same thing a cross-encoder or an LLM with logprobs over option letters does; the differences are the training mix, the calibration step and the one-pass batching of many questions. The open cards make this explicit ([decider-2b](https://huggingface.co/Mapika/decider-2b): "the letter logits at an answer slot, softmaxed over the options"; [pplx-decider](https://huggingface.co/perplexity-ai/pplx-decider-v1.1-27b): a `[255, 5120]` decision head). A local Rust engine can therefore run this class through llama.cpp or ONNX (see §6), at least for the 0.8B–4B sizes.

### 2.3 Stagehand's decision stack: state on 2026-10-10

GitHub API, read on 2026-10-10: **#2952, #2953, #2954, #2955 and #3103 are all still open and unmerged.** #3103 is now a draft. The stack has grown to eleven PRs, all by `miguelg719`, last updated 2026-10-04 (#3107 on 2026-10-04 16:04 UTC). The npm `latest` of `@browserbasehq/stagehand` is **4.2.0** and its package contains no `experimentalDecisions` string (checked by unpacking the tarball). [npm](https://www.npmjs.com/package/@browserbasehq/stagehand)

| PR | State | What it does | Figures given in the PR (local runs on Browserbase, gemini-3.8-flash fallback, self-reported) |
| --- | --- | --- | --- |
| [#2951](https://github.com/browserbase/stagehand/pull/2951) | open | Reports Chrome's AX `editable` property per node so the "input" candidate view can include `contenteditable`. | — |
| [#2952](https://github.com/browserbase/stagehand/pull/2952) | open | Jev client (8 s timeout, circuit breaker per endpoint+key), per-intent candidate views from the AX outline, twin disambiguation context (ancestors, heading, row/column, "n of m"), lexical pre-ranking (lists over 40 cut to 30), two questions per pick: `best` (no "none") and `strict` (with "none", vetoes above 0.9); ambiguity stops instead of guessing; page-state classifier (access denied, CAPTCHA, login wall). | — (library only) |
| [#2953](https://github.com/browserbase/stagehand/pull/2953) | open | `act()` as a decision tree: intent → arguments (parsed, or a span of the instruction from a tiny LLM call) → pick → existing action → deterministic read-back → LLM fallback seeded with the shortlist. | act suite 39/40 → 118/120 (3 trials); p50 1.97 s → 0.46 s; 4 of 147 acts needed the LLM. Breadth 35/40 → 39/40; 204/240 → 229/240 on two models. **LLM disabled: act 27/40, breadth 26/40.** Thresholds tuned in-sample; breadth tasks and scripts not published. |
| [#2954](https://github.com/browserbase/stagehand/pull/2954) | open | `observe()` through the picker; **cached-action check**: one yes/no "does the element this selector resolves to still match?" before replay. Errors and timeouts never block replay. | observe 9/12 → 10/12; 11/16 answered in 0.18 s vs 1.99 s. Cache-check threshold 0.35 from API probes; **no end-to-end eval of the cache path**. |
| [#2955](https://github.com/browserbase/stagehand/pull/2955) | open | `extract()` "pick and copy": the model picks elements, code copies values; lists by repeating-group detection; schema validation plus a completion gate, else LLM. | 69/75 vs 23/25 baseline (both 92%, same failures); 37/75 extracts with no LLM in ~0.5 s vs 4.37 s. LLM disabled: 36/75. |
| [#2960](https://github.com/browserbase/stagehand/pull/2960) | open | Adds BM25's best candidate to the 30-item lexical shortlist. | Real snapshots: 50/50 recall for every scorer; "no evidence of an accuracy gain". |
| [#2988](https://github.com/browserbase/stagehand/pull/2988) | open, draft | Picks a page's WebMCP tool and fills arguments as spans of the instruction; refuses when the instruction names a control. | Offline, 379 synthetic requests: 50% finished by the decision model alone, 96.8% of those right (3 wrong tools accepted); p50 301 ms. LLM baseline 96.3% at p50 1.38 s. Thresholds picked on this set. |
| [#2993](https://github.com/browserbase/stagehand/pull/2993) | open, draft | Sends the intent request during the DOM-settle wait. | Decision-model act median total 1001 ms, of which ~560 ms is settle and snapshot. |
| [#2994](https://github.com/browserbase/stagehand/pull/2994) | open, draft | Target readiness and an occlusion guard (`elementFromPoint`) before pointer acts. A page-level "is it loaded?" decision was **measured and removed**: content alone carries no loadedness. | Load-race suite 15×3: LLM 18/43 at 7.5 s p50; decisions 23/45 at 5.1 s. Six tasks fail in every arm (modals, PerimeterX CAPTCHA, SPA without handlers): "a click that did nothing is returned as success". |
| [#3102](https://github.com/browserbase/stagehand/pull/3102) | open, draft | Provider seam: TypeSafe (default), Cloudflare Clef, Perplexity Decider, OpenAI Decisions. Shared answer validation (choice must be an offered option, probabilities consistent), id aliasing, splitting at provider question caps (64/128). | Live: TypeSafe passes (`jev-1.13.0`, 203 ms); OpenAI key got 403; Clef and Decider checked against docs only. "Thresholds were tuned on Jev; other models' probabilities are not calibrated the same way." |
| [#3103](https://github.com/browserbase/stagehand/pull/3103) | open, **draft** | Explicit `stagehand.experimentalDecisions.act / observe / extract` namespace in TS, Python and Go; plain methods never call the decision model. | — |
| [#3107](https://github.com/browserbase/stagehand/pull/3107) | open, draft | Drivers: a driver returns `resolved` or `abstained` with a reason; a fallback chain is itself a driver; services never know which driver they hold. | 23 driver tests, mutation-checked. |

The [browser-use/jev-ultrafast](https://github.com/browser-use/jev-ultrafast) repository (MIT, created 2026-09-16, 22.5k stars on 2026-10-10) is the other public integration: an indexed element table, one Jev request answering operation and target "heads" speculatively, a small LLM only for typed text, and executor-side freshness and occlusion checks. Its evidence is explicit about scope: 3 runs of one Google Flights task (median 9.45 s → 7.09 s) and "not a general reliability benchmark"; `DONE` still needs independent verification; shadow roots, frames, canvas, uploads and popups are out of scope. Community ports exist (Chrome extension, MCP server, iOS simulator) ([search](https://github.com/search?q=jev-ultrafast&type=repositories)).

**Reading.** The Stagehand stack is the most detailed public design of a decision-model rung for browser automation, and its own numbers say what the R&D notes already said: a fast path in front of an LLM, not a replacement (27/40 and 26/40 with the LLM off), tuned in-sample, with the cache-check path unevaluated. Two design points carry over to Automan regardless of vendor: the explicit `resolved | abstained(reason)` driver contract (#3107), and the answer validation layer that treats an out-of-set or inconsistent answer as a typed error (#3102).

## 3. DOM and accessibility-tree selection, rerankers, distillation, locator repair, abstention

Scope: research and products that bear on a decision provider that takes a closed candidate set and returns a choice, a calibrated confidence and an abstention. Sources are primary (arXiv papers, Hugging Face model cards and API, GitHub, vendor documentation), checked on 2026-10-10. "Self-reported" means the authors ran the benchmark themselves. "(unverified)" means I could not trace the claim to a primary source.

### 3.1 Element selection and grounding on DOM or accessibility tree

**Mechanism shared by most DOM work: rank, then choose.** Mind2Web's MindAct has two stages. First, a small cross-encoder ([DeBERTa-v3-base, 86M parameters, Sentence-Transformers CrossEncoder](https://arxiv.org/abs/2306.06070)) scores each DOM element against the task and keeps the top 50. Second, an LM runs multiple-choice QA over groups of 5 candidates plus a "None" option, iterating until one element remains or every option is rejected ([Mind2Web §3.2](https://arxiv.org/abs/2306.06070)). That "None" option is a built-in abstention. Dataset: over 2,000 tasks from 137 websites in 31 domains ([Mind2Web](https://arxiv.org/abs/2306.06070)). The ranker reaches Recall@50 of 88.9 / 85.3 / 85.7 % on Cross-Task / Cross-Website / Cross-Domain ([Mind2Web §4.3](https://arxiv.org/abs/2306.06070)), which caps step accuracy: if the target is not in the candidate set, the step fails.

Mind2Web offline results (self-reported; Ele. Acc = element accuracy, Step SR = step success rate):

| Model (all on MindAct top-50 unless noted) | Cross-Task Ele. Acc / Step SR | Cross-Website | Cross-Domain | Source |
|---|---|---|---|---|
| DeBERTa ranker used as classifier | 26.8 / – | 21.6 / – | 24.5 / – | [Mind2Web T2](https://arxiv.org/abs/2306.06070) |
| MindAct Flan-T5-XL (3B, fine-tuned) | 55.1 / 52.0 | 42.0 / 38.9 | 42.1 / 39.6 | [Mind2Web T2](https://arxiv.org/abs/2306.06070) |
| MindAct GPT-4, 3-shot (50 tasks, top-10) | 41.6 / 36.2 | 35.8 / 30.1 | 37.1 / 26.4 | [Mind2Web T2](https://arxiv.org/abs/2306.06070) |
| WebGUM-XL (3B, transferred from MiniWoB) | 57.2 / 53.7 | 45.3 / 41.6 | 43.9 / 41.4 | [WebGUM T3](https://arxiv.org/abs/2305.11854) |
| Synapse (GPT-3.5, top-3 + trajectory exemplars + memory) | 34.0 / 30.6 | 29.1 / 24.2 | 29.6 / 26.4 | [Synapse T1](https://arxiv.org/abs/2306.07863) |
| SeeAct GPT-4V with "Choices" grounding (30-task subsets) | 46.4 / 40.2 | 38.0 / 32.4 | 42.4 / 36.8 | [SeeAct T2](https://arxiv.org/abs/2401.01614) |
| AutoWebGLM (ChatGLM3-6B), Step SR only | – / 66.4 | – / 56.4 | – / 55.8 | [AutoWebGLM T3](https://arxiv.org/abs/2404.03648) |
| ScribeAgent-Large (Qwen2.5-32B, own production data, no Mind2Web training) | 53.5 / 51.2 | 53.4 / 51.3 | 53.3 / 51.2 | [ScribeAgent](https://arxiv.org/abs/2411.15004) |

**Out-of-distribution (OOD) gaps.**
- On Mind2Web, every fine-tuned model loses more than 10 points of Step SR from Cross-Task to Cross-Website or Cross-Domain. Cross-Website and Cross-Domain perform about the same, which the authors read as "the challenges primarily stem from the diversity in website designs and interaction logic rather than domain specifics" ([Mind2Web §4.3](https://arxiv.org/abs/2306.06070)).
- SeeAct finds that in-context learning with large models generalizes better to unseen websites, while fine-tuning keeps an edge on seen websites ([SeeAct](https://arxiv.org/abs/2401.01614)). Its best grounding strategy still sits 20–30 % below oracle grounding, and set-of-mark prompting "turns out to be not effective for web agents" ([SeeAct](https://arxiv.org/abs/2401.01614)).
- ScribeAgent stays flat across the three splits (above) because it never trained on Mind2Web's seen tasks. Its training data is about 6B tokens of workflow data from over 250 domains, and the dataset is proprietary ([ScribeAgent](https://arxiv.org/abs/2411.15004)). On WebArena it reports 51.3 % (7B) and 53 % (32B), but those runs use a GPT-4o-based multi-agent setup, not the small model alone ([ScribeAgent](https://arxiv.org/abs/2411.15004)).

**WebLINX and its Dense Markup Ranker (DMR).**
- Dataset: about 100K interactions from 2,300 expert demonstrations on more than 150 websites ([WebLINX](https://arxiv.org/abs/2402.05930)).
- DMR is a bi-encoder: a fine-tuned all-MiniLM-L6-v2 trained with a cosine-similarity MSE loss. It averages 186 ms per turn, against 916 ms for the MindAct DeBERTa cross-encoder, on an AMD EPYC 7453 CPU with an RTX A6000 GPU ([WebLINX App. B.4](https://arxiv.org/abs/2402.05930)).
- Recall@10 (self-reported, Table 13):

| Ranker | In-domain | Test-OOD aggregate | Test-CAT (unseen subcategory) |
|---|---|---|---|
| MiniLM (DMR) | 74.27 | 51.87 | 44.05 |
| DeBERTa (MindAct cross-encoder) | 76.86 | 54.78 | 48.43 |
| BGE-small | 74.44 | 50.01 | 43.61 |
| GTE-base | 73.24 | 48.16 | 42.74 |

- Downstream, the overall score falls from IID to OOD for every fine-tuned model: Sheared-LLaMA 2.7B 37.4 → 25.0, Llama-2-13B 37.0 → 25.2, MindAct 3B 25.7 → 20.9. Zero-shot GPT-4V scores 10.4 OOD ([WebLINX T4](https://arxiv.org/abs/2402.05930)). Unseen subcategories (for example a restaurant booking versus a medical appointment) are the hardest split ([WebLINX T5](https://arxiv.org/abs/2402.05930)).

**Offline vs live.**
- Mind2Web-Live / WebCanvas: 542 tasks with 2,439 key-node checks ([WebCanvas](https://arxiv.org/abs/2406.12373)). About a year after release, 96 of 780 Mind2Web tasks (12 %) had expired on the live sites. The MindAct model beat GPT-3.5/4 on the static test set but "lagged behind in 2024 online evaluations" ([WebCanvas §1](https://arxiv.org/abs/2406.12373)). Best agent: 23.1 % task success ([WebCanvas](https://arxiv.org/abs/2406.12373)).
- Online-Mind2Web: 300 tasks on 136 sites with human evaluation. SeeAct 30.7 %, Browser Use 30.0 %, Claude Computer Use 3.7 at 56.3 %, OpenAI Operator 61.3 %. These are far below earlier self-reported WebVoyager numbers ([Online-Mind2Web T2](https://arxiv.org/abs/2504.01382)). The automatic judge WebJudge-7B agrees with humans 87 % of the time ([Online-Mind2Web T3](https://arxiv.org/abs/2504.01382)).

**Other systems.**
- **Steward:** filters the DOM to about 15 elements and uses GPT-3.5/GPT-4-family models. Reported: 81.44 % top-1 action+element accuracy over the top 5 elements, 46.70 % per-step accuracy on Mind2Web, 40 % task completion, 8.52–10.14 s and $0.028 per action, and about 4.8 s with caching ([Steward](https://arxiv.org/abs/2409.15441)). Its abstract and body disagree on cached cost ($0.022 vs $0.013 per action).
- **AXNav** (Apple): replays accessibility tests on iOS from natural language with an LLM multi-agent system. It is not a DOM ranker ([AXNav](https://arxiv.org/abs/2310.02424)).
- "Agent-as-a-Judge" (arXiv 2410.10934) targets judging code-generation agents, not element selection (unverified; not read). WebJudge, above, is the web-specific judge.

**Small open models on end-to-end benchmarks.** WebArena's original best GPT-4 agent scored 14.41 % against 78.24 % for humans ([WebArena](https://arxiv.org/abs/2307.13854)). VisualWebArena's best was 16.4 % against 88.7 % for humans ([VisualWebArena](https://arxiv.org/abs/2401.13649)). Small-model numbers are in §3. AutoWebGLM-6B reports 18.2 % on WebArena ([AutoWebGLM T4](https://arxiv.org/abs/2404.03648)).

**Latency and cost.**
- AutoWebGLM measured about 2.4 s per prediction, which is about 48 % of step time ([AutoWebGLM T5](https://arxiv.org/abs/2404.03648)).
- On the Mind2Web multiple-choice format, GPT-3.5 often picks "None" ([Mind2Web §4.3](https://arxiv.org/abs/2306.06070)). This is an uncalibrated over-abstention failure mode.

**Licenses.** Mind2Web code is open on [GitHub](https://github.com/OSU-NLP-Group/Mind2Web) (license not checked, unverified).

### 3.2 Cross-encoders and rerankers usable for candidate ranking

Parameter counts, licenses and ONNX files come from the [Hugging Face API](https://huggingface.co/api/models/BAAI/bge-reranker-v2-m3). All benchmark numbers are self-reported on model cards.

| Model | Params | License | ONNX | Context / notes | Published quality / speed |
|---|---|---|---|---|---|
| [cross-encoder/ms-marco-MiniLM-L6-v2](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2) | 22.7M | Apache-2.0 | official (O1–O4, quantized) | English, MS MARCO | NDCG@10 TREC DL19 74.30; 1,800 docs/s on V100 GPU |
| [cross-encoder/ms-marco-MiniLM-L12-v2](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L12-v2) | 33.4M | Apache-2.0 | official | same | 74.31; 960 docs/s on V100 ([card table](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2)) |
| [BAAI/bge-reranker-base](https://huggingface.co/BAAI/bge-reranker-base) | 278M | MIT | official | zh/en | "fast inference" (card, no figure) |
| [BAAI/bge-reranker-v2-m3](https://huggingface.co/BAAI/bge-reranker-v2-m3) | 568M | Apache-2.0 | community: [onnx-community/bge-reranker-v2-m3-ONNX](https://huggingface.co/onnx-community/bge-reranker-v2-m3-ONNX) | multilingual | recommended for efficiency by BAAI |
| [BAAI/bge-reranker-v2-gemma](https://huggingface.co/BAAI/bge-reranker-v2-gemma) | 2.5B | Apache-2.0 on the repo; Gemma base terms may also apply (unverified) | none | LLM reranker | recommended for quality |
| [jinaai/jina-reranker-v2-base-multilingual](https://huggingface.co/jinaai/jina-reranker-v2-base-multilingual) | 278M | **CC-BY-NC-4.0** | official (fp16/int8/bnb4) | 1,024 tokens, sliding window | function-calling and code reranking benchmarks on the card |
| [jinaai/jina-reranker-v3](https://huggingface.co/jinaai/jina-reranker-v3) | 0.6B (Qwen3-0.6B) | **CC-BY-NC-4.0** | community only | listwise, up to 64 docs in 131K context | BEIR 61.94 ([paper](https://arxiv.org/abs/2509.25085)); successor [v3.5](https://huggingface.co/jinaai/jina-reranker-v3.5), also CC-BY-NC |
| [mixedbread-ai/mxbai-rerank-base-v2](https://huggingface.co/mixedbread-ai/mxbai-rerank-base-v2) | 0.5B | Apache-2.0 | community | RL-trained (ProRank/GRPO), 100+ languages | BEIR 55.57, "latency 0.67 s" on A100 (unit of work not stated) |
| [mixedbread-ai/mxbai-rerank-large-v2](https://huggingface.co/mixedbread-ai/mxbai-rerank-large-v2) | 1.5B | Apache-2.0 | none | same | BEIR 57.49, 0.89 s on A100 |
| [mixedbread-ai/mxbai-rerank-xsmall-v1](https://huggingface.co/mixedbread-ai/mxbai-rerank-xsmall-v1) | 70.8M | Apache-2.0 | official (+quantized) | English | – |
| [Qwen/Qwen3-Reranker-0.6B](https://huggingface.co/Qwen/Qwen3-Reranker-0.6B) / [4B](https://huggingface.co/Qwen/Qwen3-Reranker-4B) / [8B](https://huggingface.co/Qwen/Qwen3-Reranker-8B) | 0.6B / 4.0B / 8.2B | Apache-2.0 | community for 0.6B ([onnx-community](https://huggingface.co/onnx-community/Qwen3-Reranker-0.6B-ONNX)) | 32K context; instruction-aware; score = P("yes") vs P("no") token | MTEB-R 65.80 / 69.76 / 69.02 |
| [Alibaba-NLP/gte-reranker-modernbert-base](https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base) | 150M | Apache-2.0 | official | 8,192 tokens, English | BEIR 56.19 |
| [Alibaba-NLP/gte-multilingual-reranker-base](https://huggingface.co/Alibaba-NLP/gte-multilingual-reranker-base) | 306M | Apache-2.0 | none in repo | multilingual | MTEB-R 59.51 ([Qwen3 card](https://huggingface.co/Qwen/Qwen3-Reranker-0.6B)) |

**Notes.**
- Cross-card numbers do not agree. For Qwen3-Reranker-0.6B, [Jina's card](https://huggingface.co/jinaai/jina-reranker-v3) gives BEIR 56.28 while [Qwen's card](https://huggingface.co/Qwen/Qwen3-Reranker-0.6B) gives MTEB-R 65.80, so neither should be treated as independent reproduction.
- **CPU latency.** Sentence-Transformers benchmarked CrossEncoder backends on an i7-13700K for MiniLM-L6, bge-reranker-base, mxbai-rerank-large-v1 and bge-reranker-v2-m3. Its recommendations: `openvino-qint8` if a small accuracy loss is acceptable, otherwise `openvino` on Intel or `onnx-O3`. fp16/bf16 on CPU runs at about 0.17× speed ([sbert efficiency docs](https://sbert.net/docs/cross_encoder/usage/efficiency.html)). The exact ms figures are only in a chart, so no per-pair CPU ms is quoted here.
- **Web element ranking specifically:**
  - Mind2Web DeBERTa-v3-base cross-encoder ([Mind2Web](https://arxiv.org/abs/2306.06070)).
  - WebLINX DMR MiniLM bi-encoder ([WebLINX](https://arxiv.org/abs/2402.05930)).
  - A community re-train: [torontodeveloper/mind2web-candidate-ranker](https://huggingface.co/torontodeveloper/mind2web-candidate-ranker) (MIT, DeBERTa-v3-base). It self-reports recall@10 of 92.6–93.0 % and acc@1 of 50.9–60.2 % per split. Its eval candidate pool is not stated as the full DOM, so it is not comparable to Recall@50 above. It warns that DeBERTa-v3 gives NaN logits in fp16.
  - A 2026 study trained a DeBERTa filter on Mind2Web: 81 % top-10 recall on Mind2Web validation but 0.63 recall transferred to WebArena. Keeping 50 % of AX-tree elements raised Llama-2-70B from 1.97 % to 2.96 % but lowered GPT-3.5 from 6.65 % to 5.84 %, because filtering breaks layout context ([arXiv 2609.27770](https://arxiv.org/html/2609.27770v1)).
  - I found no published fine-tune of bge, Jina, mxbai, Qwen3 or GTE rerankers for web element ranking (searched; absence not proven).
- **Abstention support.** Pointwise rerankers emit independent relevance scores: a logit, or P(yes) for Qwen3. None ships a calibrated "none of these" output. Calibration would need a held-out fit, for example temperature scaling or a conformal threshold (§5).

### 3.3 Distilling LLM decisions into small local web agents

| Work | Teacher / data source | Student | In-distribution result | Held-out / OOD result | Cost |
|---|---|---|---|---|---|
| [Synatra](https://arxiv.org/abs/2409.15637) | LLM-synthesized demos from tutorials and other indirect knowledge (100k) | CodeLlama-7B | Mind2Web 6.62 → 15.85 % step-level score; MiniWoB++ 23.04 → 38.20 | WebArena 0.00 → 6.28 % (GPT-3.5: 6.16, GPT-4: 14.41) | $0.031 per demo, "3 % the cost of human demonstrations" |
| [NNetNav](https://arxiv.org/abs/2410.02907) | Exploration with retroactive relabeling by the base LLM (unsupervised) | Llama-3.1-8B, 10k demos | WebArena >16 % (+15 pts over zero-shot) | WebVoyager (live sites) 35 % (+31 pts) | – |
| [WebRL](https://arxiv.org/abs/2411.02337) | Self-evolving online curriculum RL plus an outcome reward model, on WebArena sites | Llama-3.1-8B / GLM-4-9B / Llama-3.1-70B | WebArena-Lite (165 tasks) 4.8 → 42.4 %, 6.1 → 43.0 %, 49.1 % (GPT-4-Turbo 17.6 %, GPT-4o 13.9 %) | none reported off WebArena sites | – |
| [AgentTrek](https://arxiv.org/abs/2412.09605) | Tutorial-guided replay with a VLM agent and a VLM evaluator | Qwen2.5-7B / 32B | – | WebArena (called "out-of-domain" by the authors) 10.46 / 22.40 % | $0.55 per trajectory |
| [Explorer](https://arxiv.org/abs/2502.11357) | 94K LLM-explored multimodal trajectories, 49K URLs | Phi-3.5V 4B / Qwen2-VL-7B | Multimodal-Mind2Web Step SR up to 53.2 / 56.7 / 53.0 (task / website / domain) when also trained on the Mind2Web train set | Mind2Web-Live task SR 18.1 / 19.3 % (GPT-4o 25.3 %) | $0.28 per trajectory |
| [OS-Genesis](https://arxiv.org/abs/2412.19723) | Reverse task synthesis plus a trajectory reward model | Qwen2-VL-7B | – | WebArena 5.39 → 10.79 % | – |
| [OpenWebVoyager](https://arxiv.org/abs/2410.19609) | GPT-4o-guided exploration and feedback over iterations | Idefics2-8B | Mind2Web cross-task 6.3 → 19.6 % | Mind2Web cross-website (unseen) 6.6 → 10.4 % | – |
| [Go-Browse](https://arxiv.org/abs/2506.03533) | Graph-search exploration on WebArena, 10K successful trajectories | Qwen-2.5-7B | WebArena 8.3 → 21.7 % (NNetNav-7B 18.8, GPT-4o-mini 19.3) | Online-Mind2Web 5.33 % (NNetNav-7B 4.00, GPT-4o-mini 9.33) | – |
| [InSTA](https://arxiv.org/abs/2502.06776) | Qwen3-235B-generated tasks on 150k live sites, LLM judge with 82.6 % accuracy | Qwen3-1.7B | 56.9 % on its own task distribution, 94.7 % of Gemini 2.5 Flash (self-reported) | Step accuracy +89.5 % / +122.1 % relative on data-limited Mind2Web / WebLINX | – |

Pattern: distilled small models gain a lot on the environment they were trained in (WebRL: +37.6 pts on WebArena-Lite). Gains shrink sharply on held-out live sites: Go-Browse falls from 21.7 % on WebArena to 5.33 % on Online-Mind2Web, and OpenWebVoyager gains +13.3 on cross-task but only +3.8 on cross-website. These are all self-reported, and only Online-Mind2Web and Mind2Web-Live are independent benchmarks.

**Shipped small "selector / decision" models.**
- **Browser Use × TypeSafe "Jev Ultrafast"** (MIT, Sept 2026). Each observation becomes an indexed element table. TypeSafe's Jev returns an operation plus per-operation targets in one request, with a `BLOCKED` operation. A small LLM writes text only for typing ([repo](https://github.com/browser-use/jev-ultrafast)). Jev itself is a hosted API ([docs](https://docs.typesafe.ai/introduction)). Its model and calibration are not public in the repo (unverified).
- **[Mapika/decider-2b](https://huggingface.co/Mapika/decider-2b)** (Apache-2.0; Qwen3.5-2B-Base; describes itself as an "open reproduction" of Jev). It returns a probability distribution over caller-defined options in one forward pass and has `abstain_below=t`. Training includes Mind2Web element choice and an injected abstain option in 10 % of questions. Self-reported results:
  - Mind2Web 83.9 % on 1,770 rows. The candidate-set size is not stated, Mind2Web is in its training mix, and this is not comparable to MindAct element accuracy.
  - Regression-set ECE 0.038, but ECE 0.156–0.175 on hard held-out sets, where the card says the model is "still overconfident".
  - 3.2 ms p50 for a single request on a B300 GPU (no CPU figure).
- **Stagehand / Skyvern:** I found no small selector model released (searched; absence not proven).

### 3.4 Semantic drift and locator repair

| Approach | Mechanism | Dataset | Accuracy | Latency / cost | How the repaired target is validated |
|---|---|---|---|---|---|
| [Similo](https://arxiv.org/abs/2208.00677) (TOSEM 2023) | Weighted similarity over many stored locator attributes; pick the best-scoring candidate | 598 element cases, 40 websites, old vs new release | 12 % failed vs 24 % for the multi-locator baseline (LML) | ~3 ms per locate | Offline against a human-labeled ground-truth element in the new release |
| [VON Similo](https://arxiv.org/abs/2301.03863) | Merges visually overlapping DOM nodes into one visual element before scoring | 1,163 element pairs, 40 popular apps | 94.7 % vs Similo 83.8 % | – | Same ground-truth pairs |
| [VON Similo LLM](https://arxiv.org/abs/2310.02046) | VON Similo top-10 → GPT-4 picks one (one-shot, JSON candidates, with a stated rationale) | 804 pairs, 48 apps (versions 12–60 months apart) | 95.0 % vs 91.3 %; failures 70 → 40 (−43 %) | 1,934 ms vs 29 ms per localization; $35.86 for 804 | Match against oracle XPath. More than 10 candidates hurt GPT-4 (format errors, confusion) |
| [LTR ranking for Similo](https://research.chalmers.se/en/publication/544187) (JSS 2025) | Learning-to-rank over Similo features | 1,163 pairs, 40 apps | top-1 up to 88.4 %, top-3 up to 94.79 % | – | Ground-truth pairs |
| [Erratum](https://arxiv.org/abs/2106.04916) | Flexible tree matching to restrict the search space | realistic and synthetic mutations of production apps | +67 % accuracy over WATER | "avoids" brute-force scan (no figure) | Benchmark oracle |
| [Healenium](https://github.com/healenium/healenium-web) (Apache-2.0, EPAM, active Oct 2026) | Stores the last successful locator path; on `NoSuchElement` runs a comparison ("LSC" in the docs) of the current page vs the stored path and ranks candidates ([docs](https://healenium.io/docs/how_healenium_works)) | – | no published accuracy | – | `score-cap` threshold (default example 0.5); report with screenshot and a feedback button; `@DisableHealing` for negative checks ([README](https://github.com/healenium/healenium-web)) |
| Testim Smart Locators | Weights "hundreds of attributes" per element; falls back to other attributes; auto-improves a locator when its score drops below 70 % ([Tricentis docs](https://docs.tricentis.com/testim/content/overview/testim-overview/testim-automate.htm)) | – | none published | – | Confidence score; side-by-side evaluation before updating (vendor claim) |
| mabl auto-heal | Element history of 30+ attributes and ancestors; candidates searched within the ancestor; GenAI "advanced auto-heal" if no strong match ([mabl docs](https://help.mabl.com/docs/how-auto-heal-works)) | – | none published | – | Heal is saved only if the test, including its assertions, passes. No healing for explicit CSS/XPath steps ([mabl docs](https://help.mabl.com/docs/how-auto-heal-works)) |

**Caveats.**
- An LLM healer study on Supabase Studio and Grafana reports 55–68 % recovery vs 23 % for Healenium, with about 26 % "passed but wrong" heals. It is known only from [press coverage](https://www.analyticsinsight.net/artificial-intelligence/suneet-malhotra-why-self-healing-test-systems-still-need-human-judgment) (unverified; no paper found).
- An accessibility-tree, non-LLM healer reports 31/31 passes on one demo shop, a narrow validation ([arXiv 2603.20358](https://arxiv.org/abs/2603.20358)).
- Academic work validates repairs only offline against labeled pairs. In products, validation is a score threshold (Healenium, Testim) or a downstream assertion (mabl). None checks semantic equivalence of the healed target before the effect.

### 3.5 Abstention and calibration for selection

| Work | Setting | Mechanism | Guarantee / metrics | Key finding |
|---|---|---|---|---|
| [Kumar et al. 2023](https://arxiv.org/abs/2305.18404) | LLM multiple-choice QA (MMLU) | Split-conformal prediction over option softmax | coverage, set size | Set size tracks accuracy, so it is usable for selective classification. Coverage depends on exchangeability, which was tested on out-of-subject questions |
| [KnowNo](https://arxiv.org/abs/2307.01928) | LLM robot planner, multiple-choice over candidate plans | Conformal set; ask a human when the set has more than one option | task-success level guaranteed, help rate | Statistical guarantee on completion with minimal help, without fine-tuning |
| [Prune 'n Predict / CP-OPT](https://arxiv.org/abs/2501.00555) | MCQA and ToolAlpaca tool choice | Learned conformity scores; re-prompt with only the conformal set | coverage, set size | Up to 13 % smaller sets; up to 4.6 % relative accuracy gain |
| [SafeGround](https://arxiv.org/abs/2602.02419) | GUI grounding (ScreenSpot-Pro) | Spatial dispersion of sampled clicks; calibrated threshold | false discovery rate (FDR) control, AUROC | Abstain and defer to a stronger model; up to +5.38 pts system accuracy |
| [ARGUS](https://arxiv.org/abs/2606.25760) | 27 uncertainty-quantification (UQ) methods × 4 GUI VLMs × 4 datasets, plus closed APIs | logit, sampling, hidden-state, P(True), verbalized, conformal | AUROC, AURC, ECE, Brier, selective execution, conformal coverage | UQ rankings transfer within a model (ρ up to 0.969) but not to closed vendors (mean ρ +0.08). Conformal click regions lose coverage under interface mismatch |
| [PACE](https://arxiv.org/abs/2609.24277) | Coordinate-token GUI agents | Place-weighted digit entropy, single pass | AUROC, selective accuracy | Beats K-sample baselines at a fraction of the cost |
| [HyperClick / self-critiqued RL](https://arxiv.org/abs/2510.27266) | GUI grounding | RL with a confidence term | ECE | Models are more confident than their accuracy warrants on ScreenSpot-Pro |
| [Confidence Dichotomy](https://arxiv.org/abs/2601.07264) | Tool-use agents | RL jointly rewarding accuracy and calibration | ECE, Brier, AUROC | Evidence tools such as web search induce overconfidence; verification tools reduce it |
| [Xiong et al.](https://arxiv.org/abs/2306.13063) / [Tian et al.](https://arxiv.org/abs/2305.14975) | Verbalized confidence | Prompting, sampling consistency | ECE, AUROC | Verbalized confidence clusters at 80–100 % (overconfident). For RLHF models it is still often better calibrated than token probabilities (~50 % relative ECE reduction) |

Gap: I found no paper that calibrates or conformalizes **DOM/AX-tree element selection** on Mind2Web or WebArena (searched; absence not proven). The closest are MindAct's untrained "None" option ([Mind2Web](https://arxiv.org/abs/2306.06070)) and decider-2b's trained abstain option and temperature calibration ([card](https://huggingface.co/Mapika/decider-2b), self-reported). Two consistent risks across these papers: calibration fitted on one model or interface does not transfer ([ARGUS](https://arxiv.org/abs/2606.25760)), and conformal guarantees need exchangeability between calibration and live traffic ([Kumar et al.](https://arxiv.org/abs/2305.18404)). Website drift breaks that by construction ([WebCanvas](https://arxiv.org/abs/2406.12373)). Metrics to adopt: ECE/Brier for calibration, AUROC for error discrimination, risk–coverage/AURC and selective accuracy for abstention, set size and coverage for conformal methods, and FDR for risk-controlled acting.

## 4. GUI vision grounding models

Scope: models that turn a screenshot plus an instruction into a click target, for use as Automan's opt-in last rung after the accessibility tree. Conventions:

- **self-reported**: the number comes from the model authors' own card, paper or blog.
- **re-run by X**: another team published its own evaluation of the same checkpoint, which is the closest thing to independent reproduction in this field.
- **(unverified)**: not traced to a primary source.

Licenses come from the Hugging Face API `license:` tag and the model card, both checked on 2026-10-10.

**Leaderboard caveat.** The ScreenSpot-Pro leaderboard ([gui-agent.github.io/grounding-leaderboard](https://gui-agent.github.io/grounding-leaderboard/), data in [GUI-Agent/grounding-leaderboard `results/screenspot_pro.json`](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)) is maintained by the ScreenSpot-Pro authors. Most entries are submitted by the model authors themselves. Some submissions are re-scored by the maintainers against the official ground truth, for example the HuzzleWorld entries, as the [2026-09-10 commit message](https://github.com/GUI-Agent/grounding-leaderboard/commits/main/results/screenspot_pro.json) describes. Treat a leaderboard row as self-reported unless a separate paper re-ran the model.

### Benchmarks referenced

- **ScreenSpot (v1)**: introduced with SeeClick ([arXiv 2401.10935](https://arxiv.org/abs/2401.10935)). About 1.2k instructions across mobile, desktop and web.
- **ScreenSpot-v2**: a corrected relabel of v1, published with OS-Atlas ([arXiv 2410.23218](https://arxiv.org/abs/2410.23218)). Top models now score 93–96%, so the benchmark is effectively saturated.
- **ScreenSpot-Pro**: high-resolution professional apps (23 applications, 5 industries, 3 operating systems) with small targets ([arXiv 2504.07981](https://arxiv.org/abs/2504.07981)). When it launched, the best model scored only 18.9%. A Chinese-instruction variant is evaluated with [`run_ss_pro_cn.sh`](https://github.com/likaixin2000/ScreenSpot-Pro-GUI-Grounding).
- **OSWorld-G**: 564 samples in five categories: text matching, element recognition, layout, fine-grained manipulation, and **refusal** ([arXiv 2505.13227](https://arxiv.org/abs/2505.13227)). OSWorld-G-Refined (OSWorld-G-R) is a relabelled version.
- **VenusBench-GD**: includes "refusal grounding" for infeasible instructions ([UI-Venus-2 report, arXiv 2609.00028](https://arxiv.org/abs/2609.00028)).
- **UI-Vision**: cluttered desktop layouts. It is cited in the [Phi-Ground report](https://arxiv.org/abs/2507.23779) and the [UI-Venus-2 card](https://huggingface.co/inclusionAI/UI-Venus-2-9B).

---

### UI-TARS (ByteDance Seed): 1.0, 1.5, UI-TARS-2

- **Mechanism.** A native end-to-end agent VLM that outputs actions with absolute or relative coordinates as text ([UI-TARS paper, arXiv 2501.12326](https://arxiv.org/abs/2501.12326)).
  - UI-TARS 1.0 comes in 2B, 7B and 72B, built on Qwen2-VL.
  - UI-TARS-1.5 adds RL-trained reasoning before acting ([UI-TARS-1.5-7B card](https://huggingface.co/ByteDance-Seed/UI-TARS-1.5-7B)).
- **Accuracy, self-reported.**
  - The flagship UI-TARS-1.5 scores 94.2 on ScreenSpot-v2 and 61.6 on ScreenSpot-Pro.
  - The released UI-TARS-1.5-7B scores 49.6 on ScreenSpot-Pro and 27.5 on OSWorld ([card](https://huggingface.co/ByteDance-Seed/UI-TARS-1.5-7B)).
  - The flagship is not open. The card says ByteDance offers only "early research access".
  - UI-TARS 1.0: 7B scores 91.6 on ScreenSpot-v2 and 35.7 on ScreenSpot-Pro; 72B scores 90.3 and 38.1, as tabulated in the [GUI-Actor card](https://huggingface.co/microsoft/GUI-Actor-7B-Qwen2.5-VL).
- **Re-runs by other teams.** These are lower than the self-reported 7B figures.
  - **Salesforce GTA1** re-ran UI-TARS-1.5-7B: 89.7 on ScreenSpot-v2, 42.0 on ScreenSpot-Pro, 52.8 on OSWorld-G and 64.2 on OSWorld-G-R ([GTA1-7B card](https://huggingface.co/Salesforce/GTA1-7B)).
  - **H Company** lists 39.0 on ScreenSpot-Pro and reproduced 61.4 on OSWorld-G ([Holo1.5 card](https://huggingface.co/Hcompany/Holo1.5-7B)).
  - **Jedi/OSWorld-G authors** re-ran UI-TARS-7B (47.5) and UI-TARS-72B (57.1) on OSWorld-G ([arXiv 2505.13227](https://arxiv.org/abs/2505.13227)).
- **UI-TARS-2.**
  - The technical report ([arXiv 2509.02544](https://arxiv.org/abs/2509.02544)) reports agent scores only: 88.2 Online-Mind2Web, 47.5 OSWorld, 50.6 WindowsAgentArena, 73.3 AndroidWorld. It gives no ScreenSpot number in the abstract.
  - No UI-TARS-2 weights exist on Hugging Face: `ByteDance-Seed/UI-TARS-2` returns not-found from the HF API, and the [ByteDance-Seed org listing](https://huggingface.co/ByteDance-Seed) shows no successor as of 2026-10-10.
  - No 2026 UI-TARS successor with open weights was found (unverified beyond HF and GitHub checks).
- **License.**
  - UI-TARS-1.5-7B weights: Apache-2.0 ([HF](https://huggingface.co/ByteDance-Seed/UI-TARS-1.5-7B)).
  - Code: Apache-2.0 ([bytedance/UI-TARS](https://github.com/bytedance/UI-TARS)).
  - The desktop app is Apache-2.0 ([UI-TARS-desktop](https://github.com/bytedance/UI-TARS-desktop)).
- **Deployment.**
  - HF transformers (Qwen2.5-VL architecture) and vLLM.
  - Community GGUF, for example [mradermacher/UI-TARS-1.5-7B-GGUF](https://huggingface.co/mradermacher/UI-TARS-1.5-7B-GGUF).
  - MLX builds exist ([mlx-community/UI-TARS-1.5-7B-8bit](https://huggingface.co/mlx-community/UI-TARS-1.5-7B-8bit)).
  - No official Ollama library entry: `ollama.com/library/ui-tars` returns 404.
- **Abstention.** None. The model emits a point.

### OmniParser (Microsoft): v1, v2

- **Mechanism.** Unlike every other model here, OmniParser does not output coordinates from a VLM. It is a screen *parser*:
  - an interactable-region detector (YOLO) plus an icon captioner (Florence-2) turn a screenshot into a list of boxes with captions and OCR text;
  - a separate LLM then picks an element ID ([arXiv 2408.00203](https://arxiv.org/abs/2408.00203)).
  - Its output is structurally close to an accessibility tree, which makes it the most natural vision fallback for an AX-first engine: it gives element boxes with detector confidence scores, not a bare point.
- **Accuracy and latency, self-reported.**
  - v1 improved GPT-4V on ScreenSpot ([arXiv 2408.00203](https://arxiv.org/abs/2408.00203)).
  - v2 scores 39.6 average on ScreenSpot-Pro (paired with GPT-4o per the blog).
  - v2 is "60% improvement in latency" over v1, at **0.6 s/frame on A100 and 0.8 s on a single 4090** ([OmniParser-v2.0 card](https://huggingface.co/microsoft/OmniParser-v2.0)).
  - Not independently re-run on the ScreenSpot-Pro leaderboard (no entry in [screenspot_pro.json](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)).
- **License, which matters here.**
  - `icon_detect` (YOLOv8 / Ultralytics) is **AGPL-3.0**.
  - `icon_caption` (Florence-2) is MIT.
  - A newer **`icon_detect_v3`**, fine-tuned from YOLOv9-E on the MIT-licensed MultimediaTechLab/YOLO, is **MIT**. The card says pairing it with `icon_caption` gives "an entirely MIT licensed" pipeline, shipped as TorchScript with no Ultralytics at runtime ([card](https://huggingface.co/microsoft/OmniParser-v2.0)).
  - The repository code is **CC-BY-4.0** ([GitHub API, microsoft/OmniParser](https://github.com/microsoft/OmniParser)).
- **Deployment.**
  - PyTorch/TorchScript.
  - Community ONNX export of the detector ([onnx-community/OmniParser-v2.0_icon_detect](https://huggingface.co/onnx-community/OmniParser-v2.0_icon_detect)), CoreML and MLX ports. The ONNX export fits a Rust `ort` runtime.
- **Failure modes and confidence.** The card notes the detector is "deliberately low confidence on small UI elements", with the default threshold at 0.05. Raising it above about 0.1 "starts dropping real elements" ([card](https://huggingface.co/microsoft/OmniParser-v2.0)). It does produce per-box scores, but they are uncalibrated detector scores.

### ShowUI (Show Lab, NUS)

- **Mechanism.** A 2B vision-language-action model on Qwen2-VL-2B. It applies UI-guided visual token selection and outputs a normalized [0,1] point ([arXiv 2411.17465](https://arxiv.org/abs/2411.17465); [card](https://huggingface.co/showlab/ShowUI-2B)).
- **Accuracy.**
  - Self-reported 75.1 on ScreenSpot v1 ([arXiv 2411.17465](https://arxiv.org/abs/2411.17465)).
  - **Re-run by the ScreenSpot-Pro authors: 7.7 on ScreenSpot-Pro** ([leaderboard data](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)) and 77.3 on ScreenSpot-v2 ([v2 data](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_v2.json)). It collapses on high-resolution professional apps.
- **License.** Weights MIT ([HF](https://huggingface.co/showlab/ShowUI-2B)); code Apache-2.0 ([GitHub](https://github.com/showlab/ShowUI)).
- **Deployment.** Transformers; community GGUF ([abdulraheem01/ShowUI-2B-GGUF](https://huggingface.co/abdulraheem01/ShowUI-2B-GGUF)).

### SeeClick, OS-Atlas, Aguvis, UGround (2024 generation)

These four models were all re-run by later teams, so they are the best-reproduced numbers in the field, and they are now far below the state of the art.

| Model | Mechanism | Self-reported | Re-runs by other teams |
|---|---|---|---|
| **SeeClick** (Qwen-VL, about 9.6B) ([arXiv 2401.10935](https://arxiv.org/abs/2401.10935)) | Text coordinates | Introduced ScreenSpot | ScreenSpot-v2 53.9 and ScreenSpot-Pro 1.1, by the [ScreenSpot-Pro maintainers](https://github.com/GUI-Agent/grounding-leaderboard) |
| **OS-Atlas** (Qwen2-VL 7B; InternVL 4B) ([arXiv 2410.23218](https://arxiv.org/abs/2410.23218)) | Text coordinates / boxes | Introduced ScreenSpot-v2 | 7B: ScreenSpot-Pro 18.9 ([leaderboard](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)); OSWorld-G 27.7 ([Jedi paper](https://arxiv.org/abs/2505.13227)) |
| **Aguvis** (Qwen2-VL 7B/72B) ([arXiv 2412.04454](https://arxiv.org/abs/2412.04454)) | Pure-vision agent; pyautogui-style text actions | ScreenSpot 84.4 (7B), 89.2 (72B) | ScreenSpot-v2 86.0, re-run by **Microsoft GUI-Actor** ([card](https://huggingface.co/microsoft/GUI-Actor-7B-Qwen2.5-VL)); OSWorld-G 38.7 ([Jedi paper](https://arxiv.org/abs/2505.13227)) |
| **UGround-V1** (Qwen2-VL 2B/7B/72B) ([arXiv 2410.05243](https://arxiv.org/abs/2410.05243)) | Text coordinates | ScreenSpot-Pro 31.1 (7B), 34.5 (72B) | ScreenSpot-v2 87.6 re-run by **GUI-Actor**; OSWorld-G 36.4 ([Jedi paper](https://arxiv.org/abs/2505.13227)) |

- **Licenses.**
  - SeeClick: code Apache-2.0 ([GitHub](https://github.com/njucckevin/SeeClick)); the HF repo has no license tag ([HF](https://huggingface.co/cckevinn/SeeClick)).
  - OS-Atlas-7B: Apache-2.0 ([HF](https://huggingface.co/OS-Copilot/OS-Atlas-Base-7B)).
  - Aguvis: no license tag on [HF](https://huggingface.co/xlangai/Aguvis-7B-720P) or the [GitHub repo](https://github.com/xlang-ai/aguvis), so the license is unclear (unverified).
  - UGround-V1-7B: Apache-2.0; UGround-V1-72B: the Qwen ("tongyi-qianwen") license ([HF API](https://huggingface.co/osunlp/UGround-V1-72B)); code MIT ([GitHub](https://github.com/OSU-NLP-Group/UGround)).
- **Deployment.** All four are Qwen-VL derivatives, so transformers and vLLM work. Community GGUF exists for UGround ([mradermacher/UGround-V1-7B-GGUF](https://huggingface.co/mradermacher/UGround-V1-7B-GGUF)).

### Jedi / OSWorld-G (XLANG, HKU)

- **Mechanism.** Qwen2.5-VL 3B/7B fine-tuned on the 4M-example synthetic Jedi grounding dataset. It outputs coordinates ([arXiv 2505.13227](https://arxiv.org/abs/2505.13227)).
- **Accuracy, self-reported.** Jedi-7B scores 91.7 on ScreenSpot-v2, 39.5 on ScreenSpot-Pro and 54.1 on OSWorld-G. Jedi-3B scores 88.6, 36.1 and 50.9.
- **Agentic effect, self-reported.** Pairing a general model with Jedi raised OSWorld success "from 5% to 27%".
- **Re-runs by other teams.** GUI-Actor and GTA1 both tabulate Jedi-7B at 91.7 and 39.5, but these look copied from the paper rather than re-run ([GUI-Actor card](https://huggingface.co/microsoft/GUI-Actor-7B-Qwen2.5-VL); [GTA1 card](https://huggingface.co/Salesforce/GTA1-7B)).
- **Key finding for abstention: OSWorld-G's Refusal subset (54 samples).**
  - Gemini-2.5-Pro scored 38.9%, Seed1.5-VL 18.5%, Jedi-3B, Jedi-7B and OS-Atlas-7B each 7.4%, and every other model 0%.
  - The authors say refusal "remains a significant challenge."
  - Fine-grained manipulation is the weakest category, at 46.9 for Jedi-7B versus 65.9 on text matching ([arXiv 2505.13227](https://arxiv.org/abs/2505.13227)).
- **License.** Weights Apache-2.0 ([HF](https://huggingface.co/xlangai/Jedi-7B-1080p)); code Apache-2.0 ([GitHub](https://github.com/xlang-ai/OSWorld-G)).
- **Deployment.** Transformers and vLLM; community GGUF ([mradermacher/Jedi-7B-1080p-GGUF](https://huggingface.co/mradermacher/Jedi-7B-1080p-GGUF)).

### GUI-Actor (Microsoft): attention-based, with a verifier

- **Mechanism.** GUI-Actor is *coordinate-free*. An `<ACTOR>` token attends over visual patch tokens through an attention action head, which yields one or more candidate regions in a single forward pass ([arXiv 2506.03143](https://arxiv.org/abs/2506.03143)).
- **Separate verifier.** [GUI-Actor-Verifier-2B](https://huggingface.co/microsoft/GUI-Actor-Verifier-2B), built on UI-TARS-2B-SFT, takes the instruction plus a screenshot with a red circle drawn at the candidate. It outputs True or False, and "you can also use the probability of each label to score the sample."
  - This is the clearest published **abstention/confidence hook** among the open grounders. It can also verify points produced by *other* grounders.
- **Accuracy, self-reported** ([card](https://huggingface.co/microsoft/GUI-Actor-7B-Qwen2.5-VL)):

  | Model | ScreenSpot-Pro | ScreenSpot-v2 |
  |---|---|---|
  | GUI-Actor-7B (Qwen2.5-VL) | 44.6 | 92.1 |
  | GUI-Actor-7B + Verifier | 47.7 | 92.5 |
  | GUI-Actor-3B + Verifier | 45.9 | 92.4 |

  The authors report better generalization to unseen resolutions. With the backbone frozen, training only the head (about 100M parameters) was enough to be competitive.
- **Re-runs.** GUI-G2 tabulates GUI-Actor-7B at 92.1 on ScreenSpot-v2, but this appears copied from the paper rather than re-run ([GUI-G2 card](https://huggingface.co/inclusionAI/GUI-G2-7B)). On the leaderboard it sits at 44.6 on ScreenSpot-Pro (submitted, [data](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)).
- **License.** Weights MIT ([HF](https://huggingface.co/microsoft/GUI-Actor-7B-Qwen2.5-VL)); code MIT ([GitHub](https://github.com/microsoft/GUI-Actor)).
- **Deployment.** Transformers with custom model code plus flash-attention. A community GGUF exists ([adamlonsdale/GUI-Actor-7B-Qwen2.5-VL-gguf](https://huggingface.co/adamlonsdale/GUI-Actor-7B-Qwen2.5-VL-gguf)), but whether llama.cpp runs the custom action head is (unverified). Expect only the backbone to work.

### GTA1 (Salesforce)

- **Mechanism.** RL (GRPO) with a click-success reward on Qwen2.5-VL / UI-TARS-1.5 / OpenCUA backbones. It outputs coordinates. The paper also covers test-time scaling of the *planner*, where a judge selects among candidate actions ([arXiv 2507.05791](https://arxiv.org/abs/2507.05791)).
- **Accuracy, self-reported** ([card](https://huggingface.co/Salesforce/GTA1-7B)):

  | Model | ScreenSpot-v2 | ScreenSpot-Pro | OSWorld-G | OSWorld-G-R |
  |---|---|---|---|---|
  | GTA1-7B | 93.4 | 55.5 | 60.1 | 68.8 |
  | GTA1-32B | 95.2 | 63.6 | 65.2 | 72.2 |

  In agent use, GTA1-7B-2507 paired with o3 reaches 45.2 on OSWorld.
- **Re-runs.** Later papers cite GTA1-32B (63.6) as a baseline in [MAI-UI](https://arxiv.org/abs/2512.22047); no independent re-run was found (unverified).
- **License.** Salesforce/GTA1-7B is MIT ([HF](https://huggingface.co/Salesforce/GTA1-7B)); the earlier HelloKKMe/GTA1-7B is Apache-2.0 ([HF](https://huggingface.co/HelloKKMe/GTA1-7B)); the [GitHub repo](https://github.com/Yan98/GTA1) has no license file.
- **Deployment.** Transformers and vLLM; GGUF ([mradermacher/GTA1-7B-2507-GGUF](https://huggingface.co/mradermacher/GTA1-7B-2507-GGUF)).

### Phi-Ground (Microsoft)

- **Mechanism.** Built on Phi-3.5-vision (about 4B) with a **fixed input canvas**:
  - Phi-Ground-4B-7C uses 1008×672 and outputs a box ×1000.
  - Phi-Ground-Any-4B (2026) uses 1680×1008 and outputs a point in [0,10000] ([Phi-Ground card](https://huggingface.co/microsoft/Phi-Ground), [Phi-Ground-Any card](https://huggingface.co/microsoft/Phi-Ground-Any)).
  - The strict preprocessing (pad to canvas, instruction-first prompt) is a portability constraint.
- **Accuracy, self-reported.** End-to-end it scores 43.2 on ScreenSpot-Pro and 27.2 on UI-Vision. The paper states that end-to-end grounders are "still … less than 65%" on hard benchmarks and "far from being ready for deployment" ([arXiv 2507.23779](https://arxiv.org/abs/2507.23779)). Not re-run by others (unverified).
- **License.** Weights MIT; code MIT ([GitHub](https://github.com/microsoft/Phi-Ground)).
- **Deployment.** Transformers (pinned 4.43) and vLLM. No GGUF found.

### GUI-G1 / GUI-G² (Zhejiang U. / inclusionAI)

- **GUI-G1** analyses R1-Zero-style RL for grounding ([arXiv 2505.15810](https://arxiv.org/abs/2505.15810)). Its leaderboard entry is 35.9 on ScreenSpot-Pro ([data](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)).
- **GUI-G²** uses Gaussian point and coverage rewards on Qwen2.5-VL-7B ([arXiv 2507.15846](https://arxiv.org/abs/2507.15846)).
  - The card reports ScreenSpot-v2 per-platform tables ([card](https://huggingface.co/inclusionAI/GUI-G2-7B)).
  - The paper reports about 47.5 on ScreenSpot-Pro (unverified: I did not extract this figure from the primary table).
- **License.** Weights Apache-2.0 ([HF](https://huggingface.co/inclusionAI/GUI-G2-7B)).
- **Deployment.** Transformers; GGUF ([mradermacher/GUI-G2-7B-GGUF](https://huggingface.co/mradermacher/GUI-G2-7B-GGUF)).

### Holo (H Company): Holo1 → 1.5 → 2 → 3 / 3.1 → 4

- **Mechanism.** Qwen-based VLMs fine-tuned (SFT, then GRPO) for UI localization, UI question answering and navigation. They output coordinates.
  - Holo1.5 is "natively high-resolution (up to 3840 × 2160)" ([Holo1.5 card](https://huggingface.co/Hcompany/Holo1.5-7B)).
  - Holo2 adds **agentic localization**: iterative refinement over up to 3 steps ([Holo2-235B card](https://huggingface.co/Hcompany/Holo2-235B-A22B)).
- **Holo1** (3B/7B, Qwen2.5-VL) is the model behind Surfer-H ([arXiv 2506.02865](https://arxiv.org/abs/2506.02865)). Its ScreenSpot-Pro score is only 26.06 (7B), per H's own table ([Holo1.5 card](https://huggingface.co/Hcompany/Holo1.5-7B)).
- **Holo1.5, self-reported** ([card](https://huggingface.co/Hcompany/Holo1.5-7B)). The card marks which competitor numbers H reproduced in-house.

  | Model | ScreenSpot-v2 | ScreenSpot-Pro | OSWorld-G |
  |---|---|---|---|
  | Holo1.5-3B | 91.66 | 51.49 | 61.57 |
  | Holo1.5-7B | 93.31 | 57.94 | 66.27 |
  | Holo1.5-72B | 94.41 | 63.25 | 71.80 |

- **Holo2, self-reported** ([Holo2-8B card](https://huggingface.co/Hcompany/Holo2-8B), [Holo2-235B card](https://huggingface.co/Hcompany/Holo2-235B-A22B)):

  | Model | ScreenSpot-v2 | ScreenSpot-Pro | ScreenSpot-Pro (agentic, 3 steps) | OSWorld-G |
  |---|---|---|---|---|
  | Holo2-4B | 93.2 | 57.2 | 68.6 | — |
  | Holo2-8B | 93.2 | 58.9 | 71.4 | 70.1 |
  | Holo2-30B-A3B | 94.9 | 66.1 | 75.2 | 76.1 |
  | Holo2-235B-A22B | 95.9 | 70.6 | 78.5 | 79.0 |

- **Re-runs.** **inclusionAI (UI-Venus-2)** re-ran Holo2-30B-A3B on VenusBench-GD (59.5) and UI-Vision (40.9). It cites, rather than re-runs, Holo2's ScreenSpot-Pro of 66.1 ([arXiv 2609.00028](https://arxiv.org/abs/2609.00028)).
- **Holo3 / Holo3.1 / Holo4 (2026).**
  - Holo3-35B-A3B (2026-03, Apache-2.0): 77.8% OSWorld-Verified; Holo3-122B-A10B: 78.85% ([hcompany.ai/holo3](https://hcompany.ai/holo3)).
  - Holo3.1 (0.8B/4B/9B/35B-A3B, Apache-2.0, 2026-05) adds mobile support, function calling and official FP8/NVFP4/**Q4 GGUF** builds ([Holo-3.1-9B card](https://huggingface.co/Hcompany/Holo-3.1-9B)). It reports a step time of "6.8s to 3.3s" on DGX Spark with NVFP4 ([hcompany.ai/holo3.1](https://www.hcompany.ai/holo3.1)).
  - Holo4 (2026-09-24): **Holo4-27B** (Qwen3.8 dense, **CC-BY-NC-4.0**) and **Holo4-35B-A3B** (Qwen3.6 MoE, **Apache-2.0**), each with official GGUF, FP8 and NVFP4 ([Holo4-35B-A3B card](https://huggingface.co/Hcompany/Holo4-35B-A3B), [Holo4-27B card](https://huggingface.co/Hcompany/Holo4-27B)).
    - Agent scores, self-reported: Holo4-27B 85.2% on OSWorld at $0.08/task; Holo4-35B-A3B 80.8% ([blog](https://hcompany.ai/newsroom/holo4)).
    - **ScreenSpot-Pro and OSWorld-G numbers for Holo3, 3.1 and 4 appear only in images** (unverified; not extractable as text).
  - Holotron4-30B-A3B is under the NVIDIA Open Model Agreement ([card](https://huggingface.co/Hcompany/Holotron4-30B-A3B)).
- **Hosted cost.** On the H Models API, Holo4-27B costs $0.40 / $3.00 per 1M input/output tokens and Holo4-35B-A3B costs $0.30 / $2.00 ([blog](https://hcompany.ai/newsroom/holo4)). The API has a dedicated [element-localization endpoint](https://hub.hcompany.ai/models-api/element-localization).
- **License (mixed: check per size).**

  | License | Models |
  |---|---|
  | Apache-2.0 | Holo1-7B, Holo1.5-7B, Holo2-4B, Holo2-8B, Holo3-35B-A3B, Holo3.1 (all sizes), Holo4-35B-A3B |
  | "other" (inherits Qwen research terms) | Holo1-3B, Holo1.5-3B |
  | CC-BY-NC-4.0 | Holo1.5-72B, Holo2-30B-A3B, Holo2-235B-A22B, Holo4-27B |

  The Holo2-235B card body says both "research-only license" and "License: Apache 2.0", but the HF tag is cc-by-nc-4.0 (HF API, 2026-10-10).
- **Abstention / confidence.** No calibrated confidence is documented on the cards (unverified for the API).

### UI-Venus (Ant Group / inclusionAI): 1.0, 1.5, 2

- **Mechanism.** RL-fine-tuned Qwen VLMs that output a point.
  - UI-Venus-Ground-7B and 72B (2025) are grounding-only ([arXiv 2508.10833](https://arxiv.org/abs/2508.10833)).
  - 1.5 (2B/8B/30B-A3B, Qwen3-VL) is a unified agent ([arXiv 2602.09082](https://arxiv.org/abs/2602.09082)).
  - 2 (9B released, 27B family, Qwen3.5) adds web, desktop and CAPTCHA tasks ([arXiv 2609.00028](https://arxiv.org/abs/2609.00028)).
- **Accuracy, self-reported.**
  - UI-Venus-72B: 95.3 on ScreenSpot-v2 and 61.9 on ScreenSpot-Pro.
  - UI-Venus-1.5: ScreenSpot-Pro 57.7 / 68.4 / 69.6 for 2B / 8B / 30B-A3B; 30B-A3B also scores 96.2 on ScreenSpot-v2 ([1.5-8B card](https://huggingface.co/inclusionAI/UI-Venus-1.5-8B)).
  - UI-Venus-2-9B: 73.0 on ScreenSpot-Pro, 78.5 on OSWorld-G-R, 77.1 on VenusBench-GD. UI-Venus-2-27B: 74.1 on ScreenSpot-Pro ([UI-Venus-2-9B card](https://huggingface.co/inclusionAI/UI-Venus-2-9B)).
- **Re-runs.** **H Company** reproduced UI-Venus-72B in-house: 77.0 WebClick, 75.58 Showdown, 70.40 OSWorld-G ([Holo1.5 card](https://huggingface.co/Hcompany/Holo1.5-7B)).
- **Abstention.**
  - **VenusBench-GD includes refusal grounding** for infeasible instructions, and the 1.5 card calls it "refusal-aware evaluation" ([card](https://huggingface.co/inclusionAI/UI-Venus-1.5-8B)).
  - The UI-Venus-2 card reports safety attack-success rates of 11.3% on OSHarm and 48.8% on OSBlind.
- **License.**
  - Ground-7B/72B and 1.5: Apache-2.0.
  - **UI-Venus-2-9B: no license tag.** The card says the weight license "is pending final confirmation", that upstream statements conflict, and that Apache-2.0 "has intentionally not been carried over" ([card](https://huggingface.co/inclusionAI/UI-Venus-2-9B)).
- **Deployment.** vLLM (official instructions); GGUF ([bartowski/UI-Venus-2-9B-GGUF](https://huggingface.co/bartowski/UI-Venus-2-9B-GGUF)); MLX.

### GUI-Owl / GUI-Owl-1.5 (Alibaba mPLUG / Tongyi) and derivatives

- **Mechanism.** Native GUI agents on Qwen3-VL (2B/4B/8B/32B/235B, instruct and thinking variants) that output coordinates. They support optional **crop/zoom-in two-stage refinement** ([arXiv 2602.16855](https://arxiv.org/abs/2602.16855); [card](https://huggingface.co/mPLUG/GUI-Owl-1.5-8B-Instruct)).
- **Accuracy, self-reported** ([paper HTML](https://arxiv.org/html/2602.16855v1)). Thinking variants ground *worse* than instruct variants: 57.6 vs 71.1 on ScreenSpot-Pro at 8B.

  | Model | ScreenSpot-v2 | ScreenSpot-Pro | ScreenSpot-Pro (zoom-in) | OSWorld-G |
  |---|---|---|---|---|
  | GUI-Owl-1.5-8B-Instruct | 93.7 | 71.1 | 77.8 | 65.8 |
  | GUI-Owl-1.5-32B-Instruct | 95.3 | 72.9 | 80.3 | 66.8 |

- **Derivatives that top the 2026 ScreenSpot-Pro leaderboard.** All are self-submitted.
  - **Indeed-UI-32B** (GUI-Owl-1.5-32B + high-resolution grounding data): 73.3 on ScreenSpot-Pro, **82.7 with zoom-in**, 95.8 on ScreenSpot-v2, 68.6 on OSWorld-G ([card](https://huggingface.co/intelligence-indeed/Indeed-UI-32B)). The card has a trailing `license: apache-2.0` but no HF license tag.
  - **KV-Ground-8B** (GUI-Owl-1.5 base): 73.2, or 80.5 with zoom-in. **CC-BY-NC-SA-4.0** ([HF](https://huggingface.co/vocaela/KV-Ground-8B-BaseGuiOwl1.5-0315)).
- **License.** GUI-Owl-1.5 weights MIT ([HF](https://huggingface.co/mPLUG/GUI-Owl-1.5-8B-Instruct)); code MIT ([X-PLUG/MobileAgent](https://github.com/X-PLUG/MobileAgent)).
- **Deployment.** vLLM, validated by the authors on A100 96 GB; GGUF ([mradermacher/GUI-Owl-1.5-8B-Instruct-GGUF](https://huggingface.co/mradermacher/GUI-Owl-1.5-8B-Instruct-GGUF)).

### MAI-UI (Alibaba Tongyi Lab)

- **Mechanism.** 2B/8B/32B/235B-A22B GUI agents trained with "Instruction-as-Reasoning", plus adaptive zoom-in ([arXiv 2512.22047](https://arxiv.org/abs/2512.22047)).
- **Accuracy, self-reported.** MAI-UI-32B scores 67.9 on ScreenSpot-Pro, or **73.5 with zoom-in**. The 8B scores 70.9 with zoom-in. The family scores 70.9 on OSWorld-G and 49.2 on UI-Vision.
- **Re-runs.** **inclusionAI** re-ran MAI-UI-8B on VenusBench-GD (65.2*) ([arXiv 2609.00028](https://arxiv.org/abs/2609.00028)).
- **License.** MAI-UI-2B and 8B: Apache-2.0 ([HF](https://huggingface.co/Tongyi-MAI/MAI-UI-8B)). The 32B weights are not on HF (author listing, 2026-10-10).
- **Deployment.** vLLM; GGUF ([mradermacher/MAI-UI-8B-GGUF](https://huggingface.co/mradermacher/MAI-UI-8B-GGUF)).

### Fara (Microsoft): Fara-7B, Fara1.5

- **Mechanism.** A vision-only web CUA ("sees the browser through screenshots, not the DOM or accessibility tree"). It emits `computer_use` tool calls with pixel coordinates ([Fara1.5-27B card](https://huggingface.co/microsoft/Fara1.5-27B); [arXiv 2606.20785](https://arxiv.org/abs/2606.20785); Fara-7B [arXiv 2511.19663](https://arxiv.org/abs/2511.19663)).
- **Grounding use.** The card says it "can also serve as a grounding model for other agents."
- **Accuracy, self-reported.** WebVoyager: 80.8 / 86.6 / 89.3 for 4B / 9B / 27B. Online-Mind2Web 72.3 for 27B. No ScreenSpot numbers on the card.
- **Abstention-like behavior.** Fara is trained to stop at "critical points" (missing information, ambiguity, irreversible actions). This is task-level, not grounding confidence.
- **Failure modes (card).** English-only training data; can be misled by deceptive rendering and prompt injection; hallucinates page state.
- **License.** MIT for weights (Fara-7B and Fara1.5) and code ([microsoft/fara](https://github.com/microsoft/fara)).
- **Deployment.** vLLM; GGUF ([bartowski/Fara1.5-4B-GGUF](https://huggingface.co/bartowski/Fara1.5-4B-GGUF)); Azure Foundry.

### Moondream (point / detect)

- **Mechanism.** A general small VLM with native `point`, `detect`, `query` and `caption` skills that return structured output.
  - Moondream 3.1 is a 9B-A2B MoE ([card](https://huggingface.co/moondream/moondream3.1-9B-A2B)).
  - moondream2 is 2B ([HF](https://huggingface.co/vikhyatk/moondream2)).
  - It is not GUI-specialised. **No ScreenSpot, ScreenSpot-Pro or OSWorld-G number was found in a primary source** (unverified).
  - `detect` returns boxes, so it can act as an OmniParser-like proposer, but untested on UIs.
- **License.** moondream2 Apache-2.0; moondream3-preview BSL-1.1 with a "No Third-Party Service" grant ([card](https://huggingface.co/moondream/moondream3-preview)).
  - Moondream 3.1 uses the source-available **Moondream License 1.0**. Its single restriction: you cannot offer general-purpose Moondream "as a hosted or managed service." Embedding it in a product is allowed ([license](https://moondream.ai/licenses/model/1.0)).
- **Deployment.**
  - The Photon local engine (NVIDIA Ampere+ and Apple Silicon) and Moondream Cloud ([card](https://huggingface.co/moondream/moondream3.1-9B-A2B)).
  - Official GGUF for moondream2 ([moondream/moondream2-gguf](https://huggingface.co/moondream/moondream2-gguf)).
  - An Ollama library entry exists ([ollama.com/library/moondream](https://ollama.com/library/moondream)).

### Qwen2.5-VL / Qwen3-VL as base grounders

Nearly every open grounder above is a Qwen-VL fine-tune. The base models ground natively, outputting absolute pixel coordinates (Qwen2.5-VL) or relative coordinates (Qwen3-VL).

**Qwen2.5-VL numbers disagree between sources, which shows how much the harness changes results:**

| Model | Benchmark | Score | Source |
|---|---|---|---|
| Qwen2.5-VL-72B | ScreenSpot-Pro | 43.6 | "Previous SOTA" in the [UI-TARS-1.5 card](https://huggingface.co/ByteDance-Seed/UI-TARS-1.5-7B) |
| Qwen2.5-VL-72B | ScreenSpot-Pro | 53.3 | [GTA1 card](https://huggingface.co/Salesforce/GTA1-7B) and [leaderboard](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json) |
| Qwen2.5-VL-72B | ScreenSpot-Pro | 55.6 | [Holo1.5 table](https://huggingface.co/Hcompany/Holo1.5-7B) |
| Qwen2.5-VL-7B | ScreenSpot-Pro | 26.8 – 29.0 | Same three sources |
| Qwen2.5-VL-7B | ScreenSpot-v2 | 85.6 – 88.8 | [GUI-Actor](https://huggingface.co/microsoft/GUI-Actor-7B-Qwen2.5-VL), [Holo1.5](https://huggingface.co/Hcompany/Holo1.5-7B) |

**Qwen3-VL**, cited in [GUI-Owl-1.5](https://arxiv.org/html/2602.16855v1), and also re-run by **H Company** for the Thinking variants ([Holo2-235B card](https://huggingface.co/Hcompany/Holo2-235B-A22B)):

| Model | ScreenSpot-v2 | ScreenSpot-Pro | OSWorld-G |
|---|---|---|---|
| Qwen3-VL-8B-Instruct | 94.4 | 54.6 | 54.8 |
| Qwen3-VL-32B-Instruct | 95.8 | 57.9 | 65.1 |
| Qwen3-VL-235B-A22B-Instruct | — | 62.0 | — |

**Re-runs.** **inclusionAI** re-ran Qwen3.6-27B: 65.2* on ScreenSpot-Pro ([arXiv 2609.00028](https://arxiv.org/abs/2609.00028)).

**License.**
- Qwen2.5-VL-7B and all Qwen3-VL sizes: Apache-2.0 ([HF](https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct)).
- Qwen2.5-VL-3B and 72B use the Qwen license (unverified for 3B; the [UGround-72B card](https://huggingface.co/osunlp/UGround-V1-72B) links the Qwen2-VL-72B license).

**Deployment.** The broadest support of any family: transformers, vLLM, llama.cpp/GGUF, and Ollama ([qwen2.5vl](https://ollama.com/library/qwen2.5vl), [qwen3-vl](https://ollama.com/library/qwen3-vl)).

---

### Proprietary baselines

**Anthropic Claude computer use**
- **Mechanism.** A client-side tool. Claude returns actions with pixel coordinates in screenshot space.
  - Current GA toolset: `computer_toolset_20260801`, with **`zoom` on by default** (it returns a full-resolution crop of a region).
  - The earlier `computer_20251124` and `computer_20250124` versions need beta headers ([docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)).
- **Image limits.** Opus 4.7+ accepts up to 2576 px on the long edge (about 3.75 MP); earlier models accept 1568 px (about 1.15 MP).
- **Cost.** About 4,500 input tokens of toolset overhead, plus 1,000–1,800 tokens per screenshot (same docs).
- **Accuracy.** Anthropic publishes no ScreenSpot number. Third-party measurements:
  - The original Claude computer use scored 17.1 on ScreenSpot-Pro ([leaderboard](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)).
  - Claude 3.7 scored 87.6 on ScreenSpot-v2 and 27.7 on ScreenSpot-Pro (cited in the [UI-TARS-1.5 card](https://huggingface.co/ByteDance-Seed/UI-TARS-1.5-7B)).
  - Sonnet 4 scored 19.1 on ScreenSpot-Pro ([Holo1.5 card](https://huggingface.co/Hcompany/Holo1.5-7B)).
  - Sonnet 4.5 scored 36.2, measured by Google DeepMind via the API (Gemini 3 Pro launch table; the [methodology PDF](https://storage.googleapis.com/deepmind-media/gemini/gemini_3_pro_model_evaluation.pdf) confirms Google computed ScreenSpot-Pro for competitors, while the 36.2 figure itself is in an image and is unverified).
- **Abstention.** No grounding confidence. The docs list "computer vision accuracy … might … hallucinate when outputting specific coordinates" as a limitation.

**OpenAI computer use**
- `computer-use-preview` (the CUA / Operator model) **was shut down on 2026-07-23**. The announcement was made 2026-04-22, and the listed substitute is `gpt-5.6-terra` ([deprecations](https://developers.openai.com/api/docs/deprecations)).
- The current guide uses `gpt-6-astra` and recommends **code execution (Playwright/PyAutoGUI) over the `computer` tool**, which "remains supported as an alternative" ([guide](https://developers.openai.com/api/docs/guides/tools-computer-use)).
- **Historical grounding (re-run by Jedi authors):** Operator scored 70.5 on ScreenSpot-v2, 36.6 on ScreenSpot-Pro and 40.6 on OSWorld-G ([arXiv 2505.13227](https://arxiv.org/abs/2505.13227)). OpenAI CUA was also listed at 87.9 / 23.4 in the [UI-TARS-1.5 card](https://huggingface.co/ByteDance-Seed/UI-TARS-1.5-7B).
- GPT-5-high (resized) scored 6.0 and GPT5-minimal (resized) 18.5 on ScreenSpot-Pro on the [leaderboard](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json). Frontier general models without a CUA harness ground poorly.
- **Abstention.** The `pending_safety_checks` mechanism of the preview model is not described on the current guide (unverified for current models).

**Google Gemini computer use**
- `gemini-2.5-computer-use-preview-10-2025` was browser-optimised ("not yet optimized for desktop OS-level control") with a per-step out-of-model safety service ([Google blog](https://blog.google/technology/google-deepmind/gemini-computer-use-model/)).
  - It is listed for shutdown on **2026-07-28**, replaced by `gemini-3.8-flash` ([deprecations](https://ai.google.dev/gemini-api/docs/deprecations)).
- **Current models.** Computer use is a built-in tool on `gemini-3.8-flash` (recommended), `gemini-3.5-flash-lite` and `gemini-3-flash-preview`. Environments: browser, mobile and desktop. Coordinates are on a normalized 0–999/1000 grid ([docs](https://ai.google.dev/gemini-api/docs/computer-use)).
- **Abstention.** **`safety_decision`** is the one documented per-action gate in this whole survey. A function call can carry `{"decision": "require_confirmation", "explanation": …}`, which the client must acknowledge with `safety_acknowledgement`; blocked actions should halt. It is a policy gate, not a grounding-confidence score.
- **Grounding.** Gemini 3 Pro reported **72.7 on ScreenSpot-Pro** (launch table image, unverified as text).
  - The [methodology PDF](https://storage.googleapis.com/deepmind-media/gemini/gemini_3_pro_model_evaluation.pdf) states this used a "capture screenshot" tool and `media_resolution` "extra_high", and that **"high" gives 60.5**.
  - Gemini-2.5-Pro scored 45.2 on OSWorld-G and was the best refuser, at 38.9 on the refusal subset ([arXiv 2505.13227](https://arxiv.org/abs/2505.13227)).

---

### 2026 state of ScreenSpot-Pro

As of the leaderboard commit of 2026-10-01 ([data](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)), all entries are submitted:

| Model | ScreenSpot-Pro |
|---|---|
| GroundCascade (Indeed-UI-32B + KV-Ground-8B), training-free cascade averaging 2.57 model calls | **85.1** |
| Indeed-UI-32B, zoom-in | 82.7 |
| Indeed-UI-8B, zoom-in | 81.0 |
| KV-Ground-8B + Qwen3.5-27B router | 80.9 |
| KV-Ground-8B, zoom-in | 80.5 |
| Holo2-235B-A22B, agentic | 78.5 |
| MAI-UI-32B (MVP) | 77.5 |

- **Best single-pass open model:** Holo2-235B-A22B at 70.6 on the leaderboard.
- **Best single-pass claims in papers, not on the leaderboard:**
  - Qwen-UI-Agent-27B at 76.6, cited in [UI-Venus-2](https://arxiv.org/abs/2609.00028) (the primary source was not traced: unverified).
  - UI-Venus-2-27B at 74.1.
  - GUI-Owl-1.5-32B at 72.9.
- **Pattern.** Every score above about 75 uses zoom-in, crop or multi-call refinement. The original paper's insight that "strategically reducing the search area enhances accuracy" (ScreenSeekeR, 48.1 at launch, [arXiv 2504.07981](https://arxiv.org/abs/2504.07981)) still drives the frontier.
- **ScreenSpot-v2 is saturated.** Leaderboard top: duvo-eye-1 at 95.1, UI-Venus-72B at 95.3 ([v2 data](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_v2.json)); papers report up to 96.2 (UI-Venus-1.5-30B).

### Known failure modes (cross-cutting)

- **High resolution and small icons.**
  - ScreenSpot-Pro icon accuracy trails text accuracy everywhere. Even the top GroundCascade entry scores icon 73.0 versus text 92.4 ([data](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)).
  - CAD (AutoCAD icons) and Creative apps are weakest across the board (same data).
  - OmniParser's detector is weak on small elements ([card](https://huggingface.co/microsoft/OmniParser-v2.0)).
- **Fine-grained manipulation and layout reasoning.** These are the lowest OSWorld-G categories ([arXiv 2505.13227](https://arxiv.org/abs/2505.13227)). Missing software knowledge hurts, and rephrasing instructions without that knowledge helps.
- **Non-English.**
  - Fara is English-only ([card](https://huggingface.co/microsoft/Fara1.5-27B)).
  - VenusBench-GD scores are English-instruction only ([UI-Venus-2 card](https://huggingface.co/inclusionAI/UI-Venus-2-9B)).
  - A ScreenSpot-Pro CN script exists, but few CN results are published ([repo](https://github.com/likaixin2000/ScreenSpot-Pro-GUI-Grounding)).
  - Chinese-app coverage is a stated focus of UI-Venus and MAI-UI.
- **Dark patterns.** SusBench (313 tasks, 55 sites, 9 pattern types injected into real sites) found agents and humans both most susceptible to **Preselection, Trick Wording and Hidden Information** ([arXiv 2510.11035](https://arxiv.org/abs/2510.11035)).
- **Harness sensitivity.**
  - The same checkpoint swings by 10 or more points between harnesses: UI-TARS-1.5-7B ScreenSpot-Pro ranges from 49.6 self-reported to 42.0 (GTA1) and 39.0 (H Company); Qwen2.5-VL-72B ranges from 43.6 to 55.6.
  - Muse-Glimmer's base parse rate was only 63% under a plain prompt ([lemuralabs card](https://huggingface.co/lemuralabs/Muse-Glimmer-30B-GUI-Grounding-Fast), third-party).
  - Implication: **re-benchmark on Automan's own screenshots** before trusting any number.
- **Refusal and abstention.** Most grounders always return a point. On OSWorld-G's refusal subset almost all scored 0%.
  - **GUI-Actor-Verifier-2B**: True/False probability on a drawn candidate point.
  - **VenusBench-GD and OSWorld-G**: benchmarks that measure refusal.
  - **Gemini `safety_decision`**: policy confirmation, not grounding confidence.
  - **OmniParser**: per-box detector scores, uncalibrated.
  - **Cross-model or zoom consistency gates**: as in GroundCascade's "self-consistency gate" and "cross-model agreement gate" ([leaderboard description](https://github.com/GUI-Agent/grounding-leaderboard/blob/main/results/screenspot_pro.json)), usable as a practical abstention signal.

---

### Comparison table

In the score columns, **SR** = self-reported. All "Reproduced?" judgments are traced in the per-model sections above.

| Model | Params | Mechanism | ScreenSpot-v2 | ScreenSpot-Pro | Other | License | Deployment | Reproduced? |
|---|---|---|---|---|---|---|---|---|
| [UI-TARS-1.5-7B](https://huggingface.co/ByteDance-Seed/UI-TARS-1.5-7B) | 7B | Agent VLM; text coordinates | 94.2 (flagship, SR); 89.7 GTA1 re-run | 49.6 SR; 42.0 GTA1; 39.0 H Co. | OSWorld-G 52.8 (GTA1) | Apache-2.0 | HF, vLLM, GGUF (community), MLX | Yes: GTA1, H Company; lower than SR |
| [UI-TARS-72B (1.0)](https://arxiv.org/abs/2501.12326) | 72B | Agent VLM; coordinates | 90.3 | 38.1 | OSWorld-G 57.1 (Jedi) | Apache-2.0 (unverified for 72B) | HF, vLLM | OSWorld-G by Jedi authors |
| [UI-TARS-2](https://arxiv.org/abs/2509.02544) | n/a | Agent | — | — | OSWorld 47.5, Online-M2W 88.2 (SR) | No open weights found | — | No |
| [OmniParser v2](https://huggingface.co/microsoft/OmniParser-v2.0) | YOLO + Florence-2 base | Detect + caption → LLM picks | — | 39.6 (with GPT-4o, SR) | 0.6 s/frame A100 | MIT with `icon_detect_v3`; AGPL-3.0 `icon_detect`; code CC-BY-4.0 | PyTorch/TorchScript, ONNX (community) | No |
| [ShowUI-2B](https://huggingface.co/showlab/ShowUI-2B) | 2B | Normalized point | 77.3 | 7.7 | ScreenSpot 75.1 (SR) | MIT (weights) | HF, GGUF (community) | Yes: ScreenSpot-Pro authors |
| [SeeClick](https://arxiv.org/abs/2401.10935) | 9.6B | Text coordinates | 53.9 | 1.1 | — | Code Apache-2.0; weights untagged | HF | Yes: ScreenSpot-Pro authors |
| [OS-Atlas-7B](https://huggingface.co/OS-Copilot/OS-Atlas-Base-7B) | 7B | Coordinates/boxes | 83.3 (leaderboard) – 87.1 (SR) | 18.9 | OSWorld-G 27.7 (Jedi) | Apache-2.0 | HF, vLLM | Yes: ScreenSpot-Pro authors, Jedi |
| [Aguvis-7B](https://arxiv.org/abs/2412.04454) | 7B | Pure-vision agent | 86.0 (GUI-Actor re-run) | 22.9 | OSWorld-G 38.7 (Jedi) | Unclear (no tag) | HF | Yes: GUI-Actor, Jedi |
| [UGround-V1-7B](https://huggingface.co/osunlp/UGround-V1-7B) | 7B | Text coordinates | 87.6 (GUI-Actor re-run) | 31.1 | OSWorld-G 36.4 (Jedi) | Apache-2.0 (72B: Qwen license) | HF, vLLM, GGUF | Yes: GUI-Actor, Jedi |
| [Jedi-7B](https://huggingface.co/xlangai/Jedi-7B-1080p) | 7B | Coordinates | 91.7 | 39.5 | OSWorld-G 54.1; refusal 7.4 | Apache-2.0 | HF, vLLM, GGUF | Cited, not re-run |
| [GUI-Actor-7B + Verifier](https://huggingface.co/microsoft/GUI-Actor-7B-Qwen2.5-VL) | 7B + 2B | Attention action head + True/False verifier | 92.5 | 47.7 | Verifier gives a probability | MIT | HF (custom code); GGUF action head (unverified) | Cited only |
| [GTA1-7B / 32B](https://huggingface.co/Salesforce/GTA1-7B) | 7B / 32B | RL-tuned coordinates | 93.4 / 95.2 | 55.5 / 63.6 | OSWorld-G 60.1 / 65.2 | MIT (Salesforce) | HF, vLLM, GGUF | Cited only |
| [Phi-Ground-4B](https://huggingface.co/microsoft/Phi-Ground) | 4B | Box/point on a fixed canvas | — | 43.2 (SR) | UI-Vision 27.2 | MIT | HF, vLLM | No |
| [GUI-G²-7B](https://huggingface.co/inclusionAI/GUI-G2-7B) | 7B | Gaussian-reward RL coordinates | see card | about 47.5 (unverified) | — | Apache-2.0 | HF, GGUF | No |
| [Holo1.5-7B](https://huggingface.co/Hcompany/Holo1.5-7B) | 7B | Coordinates, up to 4K input | 93.3 | 57.9 | OSWorld-G 66.3 | Apache-2.0 | HF, vLLM, GGUF (community) | No (H Co. reproduces others) |
| [Holo2-8B](https://huggingface.co/Hcompany/Holo2-8B) | 8B | Coordinates + agentic refinement | 93.2 | 58.9 / 71.4 agentic | OSWorld-G 70.1 | Apache-2.0 | HF, vLLM, GGUF (community) | No |
| [Holo2-235B-A22B](https://huggingface.co/Hcompany/Holo2-235B-A22B) | 235B MoE | Same | 95.9 | 70.6 / 78.5 agentic | OSWorld-G 79.0 | CC-BY-NC-4.0 | HF, vLLM | No |
| [Holo4-35B-A3B](https://huggingface.co/Hcompany/Holo4-35B-A3B) / [27B](https://huggingface.co/Hcompany/Holo4-27B) | 35B-A3B / 27B | Agent VLM + localization API | in image only | in image only | OSWorld 80.8 / 85.2 (SR) | Apache-2.0 / CC-BY-NC-4.0 | HF, official GGUF/FP8/NVFP4, H API | No |
| [UI-Venus-Ground-72B](https://huggingface.co/inclusionAI/UI-Venus-Ground-72B) | 72B | RL point | 95.3 | 61.9 | OSWorld-G 70.4 (H Co. re-run) | Apache-2.0 | HF, vLLM | Partly: H Company |
| [UI-Venus-2-9B](https://huggingface.co/inclusionAI/UI-Venus-2-9B) | 9B | Agent; point (reasoning off) | — | 73.0 | OSWorld-G-R 78.5; VenusBench-GD 77.1 | **Pending** (no tag) | vLLM, GGUF, MLX | No |
| [GUI-Owl-1.5-32B-Instruct](https://arxiv.org/abs/2602.16855) | 32B | Agent; coordinates + zoom | 95.3 | 72.9 / 80.3 zoom | OSWorld-G 66.8 | MIT | vLLM, GGUF (8B) | No |
| [Indeed-UI-32B](https://huggingface.co/intelligence-indeed/Indeed-UI-32B) | 32B | GUI-Owl-1.5 fine-tune + zoom | 95.8 | 73.3 / 82.7 zoom | OSWorld-G 68.6 | Apache-2.0 per card text (no tag) | HF | Leaderboard submission only |
| [MAI-UI-8B / 32B](https://arxiv.org/abs/2512.22047) | 8B / 32B | Agent; coordinates + adaptive zoom | — | 70.9 zoom (8B); 67.9 / 73.5 (32B) | OSWorld-G 70.9 | Apache-2.0 (2B/8B) | vLLM, GGUF | Partly: inclusionAI (VenusBench-GD) |
| [Fara1.5-27B](https://huggingface.co/microsoft/Fara1.5-27B) | 4B / 9B / 27B | Vision-only web CUA; pixel coordinates | — | — | WebVoyager 89.3; Online-M2W 72.3 | MIT | vLLM, GGUF, Azure | No |
| [Moondream 3.1](https://huggingface.co/moondream/moondream3.1-9B-A2B) | 9B-A2B | General `point`/`detect` | not reported | not reported | — | Moondream License 1.0 (no hosted resale); moondream2 Apache-2.0 | Photon, Cloud, GGUF (md2), Ollama | n/a |
| [Qwen3-VL-8B / 32B](https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct) | 8B / 32B | Base VLM; relative coordinates | 94.4 / 95.8 | 54.6 / 57.9 | OSWorld-G 54.8 / 65.1 | Apache-2.0 | HF, vLLM, llama.cpp, Ollama | Partly: H Company re-ran Thinking variants |
| [Qwen2.5-VL-7B / 72B](https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct) | 7B / 72B | Base VLM; absolute coordinates | 85.6–88.8 / 93.3 | 26.8–29.0 / 43.6–55.6 | — | Apache-2.0 (7B); Qwen license (72B) | HF, vLLM, llama.cpp, Ollama | Yes, many; wide spread |
| [Claude computer use](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool) | n/a | Hosted tool; pixel coordinates + zoom | 87.6 (3.7, cited) | 17.1 (orig.), 27.7 (3.7), 36.2 (Sonnet 4.5, Google-run) | ~4.5k-token overhead | Proprietary | API, Bedrock, Vertex | Yes: third parties (Google, H Co.) |
| [OpenAI CUA / computer tool](https://developers.openai.com/api/docs/guides/tools-computer-use) | n/a | Hosted tool; pixel coordinates (code-exec now preferred) | 70.5 (Operator, Jedi) | 36.6 (Operator, Jedi) | Preview shut down 2026-07-23 | Proprietary | API | Yes: Jedi authors |
| [Gemini computer use](https://ai.google.dev/gemini-api/docs/computer-use) | n/a | Hosted tool; 0–999 grid; `safety_decision` | — | 72.7 (3 Pro, extra_high) / 60.5 (high) | OSWorld-G 45.2, refusal 38.9 (2.5 Pro, Jedi) | Proprietary | API | Partly: Jedi authors (2.5 Pro) |

## 5. Page classifiers, crawl scoring and extraction models

Conventions: "self-reported" = benchmark run by the authors or vendor of the system. "(unverified)" = not traced to a primary source. Numbers from different benchmarks are not comparable (WCXB = word F1; WebMainBench = ROUGE-N F1; AEB = token F1).

### 5.1 Page-type, login, challenge, paywall and soft-404 classifiers

**Summary.** No credible public, general-purpose model exists for "is this a login page / bot challenge / paywall / soft-404". The useful work is (a) vendor-documented markers that make challenge detection a rule problem, (b) older, narrow academic classifiers, and (c) one recent seven-type page classifier that matches WCXB. A Hugging Face search for login-page, captcha-detection, soft404 and webpage-classification models returned only toy uploads with almost no downloads ([HF search, login-page](https://huggingface.co/api/models?search=login-page), [captcha-detection](https://huggingface.co/api/models?search=captcha-detection), [soft404](https://huggingface.co/api/models?search=soft404)).

| Task | Best public artefact | Mechanism / I/O | Reported accuracy | License / deployment | Notes and OOD risks |
|---|---|---|---|---|---|
| Page type (7 WCXB types) | [`web-page-classifier` 0.1.0](https://docs.rs/web-page-classifier) (Rust, by the WCXB author) | XGBoost on DOM numeric features plus title and description; also a URL-heuristic `classify_url`; also an extraction-quality predictor that estimates F1 | No accuracy published on the crate page ([docs.rs](https://docs.rs/web-page-classifier)); the [WCXB paper](https://arxiv.org/html/2605.21097) says rs-trafilatura routes by XGBoost page type but gives no classifier accuracy | MIT OR Apache-2.0, pure Rust | Taxonomy built from extraction failures, not topics ([WCXB](https://arxiv.org/html/2605.21097)). Same author as the benchmark, so any future number is self-reported |
| Page type (fine-grained) | [sdfprotocol/sdf-classify](https://huggingface.co/sdfprotocol/sdf-classify) | Qwen2.5-1.5B QLoRA; chat prompt to hierarchical type (10 parents, 50+ subtypes) | 95.2% exact match, self-reported, no test-set size given ([card](https://huggingface.co/sdfprotocol/sdf-classify)) | MIT; GGUF Q4_K_M (~0.94 GB), Ollama, vLLM | Trained on only 2,335 docs; no independent evaluation |
| Page type (commercial) | [Diffbot Analyze](https://docs.diffbot.com/reference/extract-analyze) | "Visually analyzes" a page, returns a type (article, product, discussion, image, video, list, event, or "other") and routes it to an extractor ([Diffbot docs](https://www.diffbot.com/docs/extract/analyze)) | Not published | Hosted API only | Closed model |
| Page type (commercial) | [Zyte API](https://docs.zyte.com/zyte-api/usage/extract/) | The caller chooses one field (`product`, `productList`, `productNavigation`, `article`, ...); the older `pageType` parameter was replaced ([Zyte migration](https://scrapinghub.readthedocs.io/zyte-api/migration/zyte/autoextract.html)) | Not published | Hosted | Zyte exposes no public auto-classifier; the caller declares the type |
| Login / credential page | [PhishIntention](https://www.usenix.org/conference/usenixsecurity22/presentation/liu-ruofan) (USENIX Security 2022) CRP classifier | Screenshot plus detected layout ("AWL": logo, input, button, label boxes); the CRP-transition locator then clicks into login pages ([paper](https://gangw.cs.illinois.edu/class/cs562/papers/PhishIntention-sec22.pdf)) | Test accuracy 95.0% for screenshot plus layout, 90.0% for layout only, 89.0% for screenshot only; an **input-box heuristic baseline reached 60.8%**. Test set of 901 pages, train set of 8,109 (self-reported, Table 5 and Appendix) | CC0-1.0 ([repo](https://github.com/lindsey98/PhishIntention)); PyTorch detectors; whole pipeline 0.58 s mean per page | Needs a rendered screenshot. Training pages are sampled from a phishing corpus, so the domain is login-heavy. The paper notes screenshot-only training overfits |
| Login / brand (related) | [Phishpedia](https://github.com/lindsey98/Phishpedia), VisualPhishNet | Logo detection plus Siamese brand matching; Phishpedia ran at 0.39 s mean per page vs 0.26 s for VisualPhishNet in the same comparison ([PhishIntention §6](https://gangw.cs.illinois.edu/class/cs562/papers/PhishIntention-sec22.pdf)) | Phishing detection, not login detection | Phishpedia CC0-1.0 | Detects brands, not login intent |
| Soft-404 | [TeamHG-Memex/soft404](https://github.com/TeamHG-Memex/soft404) | Text and structure vectoriser plus a classifier; HTML in, probability out | ROC AUC 0.995, PR AUC 0.990 (10-fold CV over 120k pages and 25k domains, self-reported) | README says MIT; the GitHub license field is empty; the README points to a maintained fork | CV mixes pages from the same domains, so cross-site generalisation is unmeasured |
| Soft-404 (academic) | Prieto, Álvarez, Cacheda 2014, "Soft-404 pages, a crawling problem" ([JDIM](https://dline.info/newl/node/440)) | Content heuristics plus a C4.5 tree | P 0.992 / R 0.980 on the authors' dataset; 7.35% of servers return 200 for unknown URLs (from search-result summary, **unverified** against the full text) | n/a | Old web |
| Soft-404 (rule) | Bar-Yossef et al. "random URL probe": request a surely-missing URL on the same host and compare it with the target ([patent describing it](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/7818312)) | Per-host fingerprint | n/a | Rule | Costs one extra request per host |
| Paywall | Papadopoulos et al., WWW 2020 ([arXiv 1903.01406](https://ar5iv.labs.arxiv.org/html/1903.01406)) | Random forest over textual ("subscribe", "remaining" in 87 languages), structural (reader-mode length change across cookie-jar and clean crawls) and visual (obscured or overlay text) features | P 0.77 / R 0.77 / F 0.75 / AUROC 0.74 (5-fold CV, self-reported) | No released model found | Weak model that needs repeated, instrumented visits |
| Paywall (rule) | Google paywall markup: `isAccessibleForFree: false` plus `hasPart` with a `cssSelector` ([Google](https://developers.google.com/search/docs/appearance/structured-data/paywalled-content)) | JSON-LD lookup | Exact when present | Rule | Only on publishers that annotate for Google |
| Bot challenge (Cloudflare) | Official header `cf-mitigated: challenge` on every Challenge Page type; the body is always text/html ([Cloudflare docs](https://developers.cloudflare.com/cloudflare-challenges/challenge-types/challenge-pages/detect-response)) | Header rule | Exact (vendor contract) | Rule | Status code alone is unreliable (403 or 503) |
| Bot challenge (others) | Firefox CAPTCHA detection: reCAPTCHA by iframe URL, Turnstile through a closed shadow root, DataDome by `message` events, hCaptcha by iframes, Arkose by iframe plus network ([Mozilla docs](https://firefox-source-docs.mozilla.org/toolkit/components/captchadetection/captcha-detection/index.html)) | DOM, iframe and network rules | n/a | MPL code | GeeTest not covered. DataDome markers (`x-datadome` header, `captcha-delivery.com` in a 403 body) come only from scraping vendors, **not DataDome docs (unverified)** ([Scrappey](https://scrappey.com/tools/antibot-detector/datadome)) |

No peer-reviewed classifier for challenge or interstitial pages was found. A 2025 news-crawl study reports that about 2.8% of 5,478 domains blocked OpenWPM entirely, which skews any training set toward sites that do not block (search summary of [Sivan-Sevilla 2025](https://corpus.lantern.io/findings/2025-sivan-sevilla-probing__bot-detection-biases-structural-crawl-dataset/), **unverified**).

**Implication for Automan.** Challenge and paywall detection are best treated as rule cascades: vendor headers, iframe or script hosts, JSON-LD, and a password-input check. PhishIntention's result shows that the bare "password/input present" rule is weak for *login intent* (60.8% vs 95%), but it is still a cheap first pass. The one place where a small learned model is well-supported is the page-type router feeding extraction (rs-trafilatura's XGBoost approach).

### 5.2 Learned link relevance and focused crawling

| Work | Scorer and what it scores | Metric and result | Cost / caveats |
|---|---|---|---|
| Chakrabarti, van den Berg, Dom 1999, [Focused crawling](https://www.math.unipd.it/~dulli/corso04/www1999f.pdf) (Computer Networks 31) | Topic classifier on fetched pages plus a hub "distiller"; topics defined by example documents | Harvest rate stays high while a standard crawler "loses its way" (qualitative curves) | Classic baseline; a cheap classifier per page |
| [Craw4LLM](https://arxiv.org/html/2502.13347v3) (Yu, Liu, Xiong, 2025; [code](https://github.com/cxcscmu/Craw4LLM), MIT) | DCLM fastText quality score (FineWeb-Edu also tested) **of the fetched page**: Score(u) = M(FetchPage(u)) | Matches indegree-crawl downstream LLM performance with 21% of the crawled documents (48% when counting all visited); >95% of oracle at 2.2% of the data (self-reported) | Simulation on ClueWeb22-A, about 900M pages. The authors note that a real crawler would have to fetch each candidate to score it |
| [Neural Prioritisation for Web Crawling](https://arxiv.org/html/2506.16146v2) (Pezzuti, MacAvaney, Tonellotto, ICTIR 2025) | QT5-Small quality estimator on the **fetched parent**; the child inherits the parent score (QFirst) or the minimum over its parents (QMin). The authors state URLs cannot be scored by content before download | Harvest rate up to +149% vs BFS (Researchy Questions), up to +20% (MS MARCO Web Search); speedup 1.10 to 1.60 (self-reported) | One forward pass per page; single-threaded simulation; BFS is the only baseline |
| [Data Scout](https://arxiv.org/abs/2609.05766) (Sept 2026) | LLM-generated queries produce seeds; a domain classifier (FineMath) probes a sample per **subdomain** and admits or rejects the whole subdomain | 21.9% of crawled pages are relevant vs 0.31% when filtering a web sample (~70x); 63.2% of pages are absent from Common Crawl (self-reported) | Classifier cost is amortised per subdomain, not per URL |
| [ThreatCrawl](https://arxiv.org/pdf/2304.11960) (2023) | BERT relevance classifier for cybersecurity pages | Reported as large harvest-rate gains vs a prior 1%-harvest crawler (self-reported, figure not extracted) | Domain-specific |
| [Crawl4AI adaptive crawling](https://docs.crawl4ai.com/core/adaptive-crawling/) (v0.9.x, Apache-2.0) | "Statistical" (term-based, no model) or "embedding" (embeddings plus LLM query expansion) strategy; links ranked by query relevance, expected information gain and URL depth; stops on coverage, consistency and saturation (confidence 0.7, max 20 pages, top-k 3 links) | **No benchmark published** | The embedding mode needs an embedding model and a chat LLM |
| [Firecrawl crawl `prompt`](https://docs.firecrawl.dev/features/crawl) | A natural-language prompt "to generate crawl options"; explicit parameters override it | No metrics | Mechanism undocumented (likely one LLM call producing include/exclude paths, **unverified**). Pricing: 1 credit per page, +4 credits per page for JSON ([pricing](https://www.firecrawl.dev/pricing)) |
| Fraunhofer LLM-guided focused crawler ([PDF](https://publica-rest.fraunhofer.de/server/api/core/bitstreams/2f9de95a-dce6-4a61-a3a2-223710f45084/content)) | LLM on-demand relevance estimate inside StormCrawler | No harvest numbers found | Venue and date unverified |

**Findings.** (1) Every measured gain comes from scoring **fetched content** and propagating that score to outlinks, not from a URL-only model. No 2024–2026 paper was found that reports precision at budget for a URL-only or anchor-text-only LLM scorer. (2) Cheap scorers (fastText, T5-small, XGBoost) carry the published results; generative LLMs appear as seed and query generators (Data Scout) or as unbenchmarked product features (Crawl4AI, Firecrawl). (3) All results are simulations on ClueWeb22 or self-reported, with no politeness or revisit modelling. Spider's "prompt to crawl" feature was not checked (**unverified**).

### 5.3 Main-content and extraction models

#### 5.3a Independent and cross-benchmark evidence (most important)

| System | WCXB dev word F1 ([Foley 2026](https://arxiv.org/html/2605.21097)) | WCXB test F1 | Latency (WCXB) | WebMainBench ROUGE-N F1 ([Dripper v3](https://arxiv.org/html/2511.23119v3)) |
|---|---|---|---|---|
| rs-trafilatura | 0.859 | 0.903 | 44 ms | n/a |
| MinerU-HTML / Dripper 0.6B | 0.827 | **not reported** | 1,570 ms (A100, vLLM batched) | 0.8779 (0.8925 with Trafilatura fallback), self-reported |
| Trafilatura | 0.791 | 0.841 | 97 ms | 0.6402 |
| Resiliparse | 0.797 | 0.817 | 28 ms | 0.6290 |
| dom-smoothie | 0.762 | 0.817 | 26 ms | n/a |
| ReaderLM-v2 1.5B | 0.741 | **not reported** | 10,410 ms | 0.2280 (TEXT mode) |
| Readability | 0.674 | 0.736 | 785 ms | 0.6542 ([MinerU-HTML README](https://github.com/opendatalab/MinerU-HTML)) |
| GPT-5 / DeepSeek-V3.2 inside the Dripper framework | n/a | n/a | n/a | 0.9024 / 0.9098 |

- **Neural models trail on non-article pages.** Per-type WCXB dev F1 for MinerU-HTML vs ReaderLM-v2: article 0.928/0.880, forum 0.794/0.589, product 0.619/0.464, collection 0.506/0.415, listing 0.710/0.578. MinerU-HTML trails the best heuristic on 6 of 7 types ([WCXB](https://arxiv.org/html/2605.21097)). WCXB is by the rs-trafilatura author, so it is not neutral, but its test split was held out.
- **The benchmarks disagree.** On WebMainBench (built by the MinerU-HTML team), the model beats Trafilatura by about 24 points. On WCXB it trails rs-trafilatura by 3 points on dev. ReaderLM-v2 collapses to 0.23 on WebMainBench but scores 0.74 on WCXB, which points to evaluation-mode sensitivity (markdown vs text, truncation).
- **Article-only benchmark (AEB).** On the [Zyte/ScrapingHub article benchmark](https://github.com/scrapinghub/article-extraction-benchmark) (MIT, latest run), F1 is rs_trafilatura 0.970, trafilatura 0.958, go_trafilatura 0.960, readability_js 0.947, dom_smoothie 0.865. No model-based extractor is listed, so the article case looks saturated for heuristics.
- Automan's own measurements (rs-trafilatura 0.884, Readability 0.828, dom-smoothie 0.823 word F1 on WCXB test) differ from the paper's (0.903 / 0.736 / 0.817). Versions and the scorer should be pinned before comparing.

#### 5.3b Model catalogue

| Model | Mechanism and I/O | Size / base | Accuracy (benchmark) | Latency / cost | License | Deployment | Grounding and failure modes |
|---|---|---|---|---|---|---|---|
| **Dripper / MinerU-HTML v1.0** ([paper](https://arxiv.org/html/2511.23119v3), [repo](https://github.com/opendatalab/MinerU-HTML), [HF](https://huggingface.co/opendatalab/MinerU-HTML)) | Simplify HTML into numbered blocks (mean 44,590 to 6,960 tokens, an 84% reduction), then **classify each block main/other** with regex-constrained decoding. Output is a subset of the source blocks, so it is **extractive and cannot hallucinate text** | Qwen3-0.6B, SFT on 986k pages labelled by DeepSeek-V3.2 | WebMainBench 0.878 (self-reported); WCXB dev 0.827 (third party) | 3.08 pages/s on one A100 (self-reported) | Apache-2.0 (v1.0 weights and code) | vLLM, Transformers, OpenAI-compatible; Trafilatura fallback built in | 32k context: pages over the limit score 0, and 29.3% of sampled Common Crawl pages exceed 32k tokens. Labels are LLM-generated. Weak on structured pages |
| **MinerU-HTML v1.1** ([HF](https://huggingface.co/opendatalab/MinerU-HTML-v1.1-hunyuan0.5B-compact), March 2026) | Same approach with a compact output format and regex constraints for the vLLM v1 backend | Hunyuan 0.5B, 256k context | WebMainBench v1.1 0.9001 (self-reported) | Not published | **Tencent Hunyuan Community License: the Territory excludes the EU, UK and South Korea**, and a licence is required above 100M MAU ([LICENSE](https://huggingface.co/opendatalab/MinerU-HTML-v1.1-hunyuan0.5B-compact/blob/main/LICENSE)) | vLLM | Not usable in the EU. Only the v1.0 (Qwen3, Apache) weights are usable there |
| **ReaderLM-v2** ([HF](https://huggingface.co/jinaai/ReaderLM-v2), [arXiv 2503.01151](https://arxiv.org/abs/2503.01151)) | **Generative** HTML to Markdown, or HTML plus JSON schema to JSON | 1.54B (Qwen2.5-1.5B), 512K context | Card: ROUGE-L 0.84 (md), JSON F1 0.81; paper claims 15–20% over GPT-4o (self-reported). Third party: WCXB 0.741, WebMainBench 0.228 | 10.4 s per page on A100 (WCXB). The Jina Reader `x-engine: readerlm-v2` costs 3x tokens ([card](https://huggingface.co/jinaai/ReaderLM-v2)) | **CC-BY-NC-4.0** (non-commercial) | GGUF, llama.cpp, Ollama, vLLM, SGLang, AWS and Azure marketplaces | Generative, so it can rewrite or drop text. The card relies on contrastive loss and a 1.08 repetition penalty against degeneration. Same licence for v1 [reader-lm-0.5b](https://huggingface.co/jinaai/reader-lm-0.5b) and [1.5b](https://huggingface.co/jinaai/reader-lm-1.5b); no 2026 ReaderLM found ([HF list](https://huggingface.co/api/models?search=reader&author=jinaai)) |
| **NuExtract 1.5 / 1.5-tiny** ([HF](https://huggingface.co/numind/NuExtract-1.5)) | Text plus JSON template to JSON | Phi-3.5-mini (3.8B) / Qwen2.5-0.5B ([tiny](https://huggingface.co/numind/NuExtract-1.5-tiny)) | No independent numbers found | n/a | MIT | Transformers, GGUF | Superseded |
| **NuExtract 2.0** ([8B card](https://huggingface.co/numind/NuExtract-2.0-8B)) | Text or image plus a typed template (`verbatim-string`, `string`, `integer`, enum, ...) to JSON; missing fields return null | 2B (Qwen2-VL-2B, MIT), 4B (Qwen2.5-VL-3B, **Qwen Research License**), 8B (Qwen2.5-VL-7B, MIT) | 8B ExtractBench mean 49.42 (card metadata) | n/a | MIT except the 4B | vLLM, GGUF | Casing and form drift between runs, per the card's own example. Not HTML-specific |
| **NuExtract3** ([HF](https://huggingface.co/numind/NuExtract3), April 2026) | Image and text vision-language model; extraction to JSON and document to Markdown; reasoning modes | ~4B (Qwen3.5-4B) | ExtractBench mean 82.35 (card metadata); internal benchmark 0.651 vs gemma-4-E4B 0.538 (self-reported) | n/a | Apache-2.0 | vLLM, FP8, W4A16, GGUF, MLX ([variants](https://huggingface.co/api/models?author=numind)) | `verbatim-string` gives copy-only fields. The card reports repetition loops in small reasoning models. HTML input is not documented |
| **Schematron-3B / 8B** ([3B card](https://huggingface.co/inference-net/Schematron-3B)) | Raw or lxml-cleaned HTML plus JSON Schema to strict JSON | Llama-3.2-3B / Llama-3.1-8B; 128K context | Gemini-2.5-Pro-judged score 1–5: 3B 4.41, 8B 4.64, GPT-4.1 4.74 (self-reported, LLM judge) | 3B costs about half of 8B; no $/token on the card | Llama 3.2 Community (3B); no license tag on the [8B](https://huggingface.co/api/models/inference-net/Schematron-8B) | Hosted at inference.net, vLLM, SGLang, GGUF | Static HTML only; large pages need truncation; the card says to validate outputs downstream. No grounding |
| **Osmosis-Structure-0.6B** ([HF](https://huggingface.co/osmosis-ai/Osmosis-Structure-0.6B)) | Free text plus schema to JSON (structures *model output*, not HTML) | Qwen3-0.6B | No scores for the 0.6B model itself | n/a | Apache-2.0 | SGLang, Ollama, GGUF | Off-target for page extraction |
| **ScrapeGraphAI-100k** ([arXiv 2602.15189](https://arxiv.org/abs/2602.15189)) | Dataset of 93,695 real (Markdown, prompt, schema, response) extraction events | A fine-tuned 1.7B model "narrows the gap" to a 30B model (abstract only, numbers **unverified**) | n/a | n/a | Dataset on HF | n/a | ScrapeGraphAI itself ([MIT](https://github.com/ScrapeGraphAI/Scrapegraph-ai)) is a wrapper around any LLM, including Ollama |
| **MarkupLM** ([arXiv 2110.08518](https://arxiv.org/abs/2110.08518), ACL 2022; [HF base](https://huggingface.co/microsoft/markuplm-base)) | Text plus markup-structure encoder (pre-2023 attribute extraction and web QA) | base / large | Abstract claims state of the art on document-understanding tasks (benchmark numbers not extracted) | Encoder: fast | No license tag on HF | Transformers | Needs a fine-tuning head per task |
| **DOM-LM** ([arXiv 2201.10608](https://arxiv.org/abs/2201.10608)) | Transformer over text and DOM tree | n/a | Claims to beat baselines on attribute extraction, open IE and QA | n/a | No released weights found | n/a | Research only |
| **BoilerNet** ([arXiv 2004.14294](https://arxiv.org/abs/2004.14294), WWW 2020 demo) / **Web2Text** ([arXiv 1801.02607](https://arxiv.org/abs/1801.02607), ECIR 2018) | Sequence labelling over DOM leaves / CNN potentials plus HMM over blocks; extractive | Small | State of the art on CleanEval in their time (numbers not extracted) | CPU-feasible | n/a | Research code | The precursors of Dripper's block-labelling approach |

**Hosted reader pricing.** Firecrawl: Free 1,000 credits; Standard $83/month for 100k credits; Scale $599/month for 1M credits; 1 credit per page, +4 for JSON ([pricing](https://www.firecrawl.dev/pricing)). Jina Reader: billed by output tokens, 10M free tokens per key, 20 RPM without a key and 500 RPM with a key; $/M tokens is not shown in page text ([Jina Reader](https://jina.ai/reader/)).

**Implication for Automan.** The only small model that adds evidence-backed value as an extractor is the **block-classification** design (Dripper / MinerU-HTML v1.0). It is extractive, so its output is a subset of the source and it does not hallucinate, and it is Apache-2.0. Even so, it is about 35x slower than rs-trafilatura on a GPU, has no held-out WCXB score, and is weaker on product, collection and listing pages. The pattern its own authors use (model with a heuristic fallback) points to a confidence-gated escalation: heuristic first, then the model on low-confidence or non-article pages. rs-trafilatura's extraction-quality predictor is a ready-made gate. Generative HTML-to-Markdown models (ReaderLM) are non-commercial and score worse.

### 5.4 Field-level grounding and validation (brief)

- **LangExtract** ([google/langextract](https://github.com/google/langextract), Apache-2.0) maps each extraction to a `char_interval` in the source. Values that cannot be located get `char_interval = None`, for example values copied from few-shot examples, and can be filtered out. It works with Gemini, OpenAI or local Ollama models, and handles long documents with chunking and multiple passes (example: `extraction_passes=3`). The matching algorithm (exact or fuzzy) is not documented in the README.
- **Instructor** ([docs](https://python.useinstructor.com/concepts/reask_validation/), MIT) uses Pydantic validators with `max_retries` to re-ask the model. Its documented context-validation example checks that a `supporting_quote` appears in the source after whitespace normalisation.
- **Model-side support.** NuExtract `verbatim-string` fields ([NuExtract3](https://huggingface.co/numind/NuExtract3)) request copy-only values. Dripper's block labels are grounded by construction. Schematron and ReaderLM JSON give no provenance, and their cards say to validate downstream ([Schematron](https://huggingface.co/inference-net/Schematron-3B)).
- **Engine pattern these sources support.** Treat every model-extracted field as untrusted until (a) it passes schema and type validation, and (b) string fields match normalised source text with a recorded span or selector. Numbers and dates should be re-parsed from the matched span. Fields that fail go to retry, fallback or null, never to silent acceptance. This mirrors LangExtract and Instructor; no benchmark of grounding-check recall or precision was found.

## 6. Running small models locally from a Rust engine

Research date: 2026-10-10. Crate versions and dates come from the crates.io API (`https://crates.io/api/v1/crates/<name>`), and repository activity comes from the GitHub API. The model sizes cover Automan's candidate decision models: cross-encoder rerankers, small classifiers, 0.5B–4B LMs and a 2B–7B vision grounder. "(unverified)" marks a claim that I could not trace to a primary source.

### Comparison table

| Runtime | Version / date | License | Formats | Accelerators | Vision / VLM support | Binary / footprint | Maturity notes |
|---|---|---|---|---|---|---|---|
| [`ort`](https://crates.io/crates/ort) (pykeio, ONNX Runtime bindings) | 2.0.0-rc.13, 2026-07-28 ([crates.io](https://crates.io/api/v1/crates/ort)); wraps ONNX Runtime 1.31.0 ([dist.tsv](https://github.com/pykeio/ort/blob/main/ort-sys/build/download/dist.tsv)) | MIT OR Apache-2.0 | ONNX | CPU, CoreML, CUDA, TensorRT, DirectML, OpenVINO, WebGPU (experimental), ROCm/MIGraphX, QNN, XNNPACK, NNAPI and others ([features](https://crates.io/api/v1/crates/ort/2.0.0-rc.13)) | Any VLM exported to ONNX. You write the decode loop yourself ([phi-3-vision example](https://github.com/pykeio/ort/tree/main/examples)) | Upstream ORT CPU archive: 10 MB on linux-x64 and 34 MB on osx-arm64, compressed ([ORT 1.31.0 release](https://github.com/microsoft/onnxruntime/releases/tag/v1.31.0)). CUDA builds: 233–429 MB | Docs call it "production-ready" even though the version is still an RC ([ort.pyke.io](https://ort.pyke.io/)). 21.5M total downloads. Very active |
| [`candle`](https://github.com/huggingface/candle) (Hugging Face) | candle-core 0.11.0, 2026-06-26 ([crates.io](https://crates.io/api/v1/crates/candle-core)) | MIT OR Apache-2.0 | safetensors, GGUF (quantized), partial ONNX | CPU (MKL / Accelerate), CUDA (+cuDNN, NCCL), Metal, WASM ([features](https://crates.io/api/v1/crates/candle-core/0.11.0)) | Qwen3-VL, LLaVA, Moondream, PaliGemma, Pixtral, PaddleOCR-VL, BLIP, CLIP/SigLIP, YOLOv8. No Florence-2, no Qwen2-VL ([models dir](https://github.com/huggingface/candle/tree/main/candle-transformers/src/models)) | Pure Rust, no C++ runtime to ship. No published size figure (unverified) | Pre-1.0. Commits weekly. Known CPU performance gaps ([#2877](https://github.com/huggingface/candle/issues/2877), [#1939](https://github.com/huggingface/candle/issues/1939)) |
| [`burn`](https://github.com/tracel-ai/burn) (Tracel) | 0.22.0, 2026-10-06; `burn-onnx` 0.22.0 ([crates.io](https://crates.io/api/v1/crates/burn-onnx)) | MIT OR Apache-2.0 | Native Burn modules. ONNX is converted to Rust code at build time | CubeCL: CUDA, ROCm, Metal, Vulkan, WebGPU, CPU (LLVM JIT). Flex: pure-Rust CPU, wasm, no_std. LibTorch deprecated ([README](https://github.com/tracel-ai/burn#supported-backends)) | Only if the ONNX graph imports cleanly. No ready-made VLM or LLM zoo (unverified) | Model compiled into the binary, no runtime to ship ([burn-onnx](https://github.com/tracel-ai/burn-onnx)) | Pre-1.0, with breaking backend churn (LibTorch removed, candle backend gone from 0.22 features). burn-onnx is young (137 stars) |
| [`llama-cpp-2`](https://github.com/utilityai/llama-cpp-rs) (utilityai) | 0.1.159, 2026-10-07 ([crates.io](https://crates.io/api/v1/crates/llama-cpp-2)) | MIT OR Apache-2.0 (llama.cpp itself is MIT) | GGUF | CPU (OpenMP), Metal, CUDA, Vulkan, ROCm, OpenCL, MKL; `dynamic-backends` ([features](https://crates.io/api/v1/crates/llama-cpp-2/0.1.159)) | Yes, through the `mtmd` feature ([`mtmd.rs`](https://github.com/utilityai/llama-cpp-rs/tree/main/llama-cpp-2/src)), which reaches libmtmd's VLMs: Qwen2/2.5-VL, Gemma 3/4, SmolVLM, InternVL, Moondream2 ([multimodal.md](https://github.com/ggml-org/llama.cpp/blob/master/docs/multimodal.md)) | Upstream CLI bundle (all tools): 11.5 MB macos-arm64 and 16.9 MB ubuntu-x64, compressed ([b11541](https://github.com/ggml-org/llama.cpp/releases/tag/b11541)) | "does not follow semver meaningfully" ([README](https://github.com/utilityai/llama-cpp-rs)). Goose pins an exact version because patch releases break the API ([Cargo.toml](https://github.com/aaif-goose/goose/blob/main/Cargo.toml)) |
| [`llama_cpp`](https://crates.io/crates/llama_cpp) (edgenai) | 0.3.2, 2024-04-29 | MIT OR Apache-2.0 | GGUF | CPU / GPU of its llama.cpp snapshot | No | n/a | Abandoned: last repo push 2024-06 ([GitHub](https://github.com/edgenai/llama_cpp-rs)) |
| [`mistral.rs`](https://github.com/EricLBuehler/mistral.rs) | GitHub v0.9.4, 2026-09-24. crates.io `mistralrs` is stuck at 0.8.1 (2026-04-02) because it depends on a git rev of candle ([Cargo.toml](https://github.com/EricLBuehler/mistral.rs/blob/master/Cargo.toml)) | MIT | safetensors with ISQ, GGUF, GPTQ, AWQ, HQQ, FP8, UQFF | CUDA (FlashAttention), Metal, CPU | Broad: Qwen2/2.5/3-VL, Gemma 3/3n/4, Idefics3/SmolVLM, Phi-3.5-vision, Llama 3.2 Vision, MiniCPM-o ([supported models](https://docs.mistralrs.dev/reference/supported-models/)) | Ships as a self-contained binary or server ([README](https://github.com/EricLBuehler/mistral.rs)); size not published (unverified) | Fast-moving, single main maintainer. Built on candle. No reranker support |
| [`tract`](https://github.com/sonos/tract) (Sonos) | 0.23.8, 2026-09-21 ([crates.io](https://crates.io/api/v1/crates/tract)) | MIT OR Apache-2.0 | ONNX, NNEF/tract-OPL, TFLite | CPU SIMD (x86, ARM), Metal, CUDA, WASM ([README](https://github.com/sonos/tract#runtimes)) | Only through ONNX graphs. Has a `causal_llm` example | Pure Rust, aimed at a "ship-tiny-runtime" story ([README](https://github.com/sonos/tract)) | Used in production at Sonos for wake-word and ASR. Active |
| [`fastembed`](https://github.com/Anush008/fastembed-rs) (Qdrant ecosystem) | 7.1.1, 2026-10-07 ([crates.io](https://crates.io/api/v1/crates/fastembed)) | Apache-2.0 | ONNX through `ort`. Candle for Qwen3 / nomic-v2 | Inherits from `ort` and candle | Image embeddings (ResNet50, CLIP, Qwen3-VL-Embedding) | Pulls ort plus hf-hub by default | Synchronous API with no Tokio dependency ([README](https://github.com/Anush008/fastembed-rs)) |
| [`rust-bert`](https://github.com/guillaume-be/rust-bert) | 0.23.0, 2024-09-29 | Apache-2.0 | libtorch (tch), optional ONNX via ort | libtorch CPU / CUDA | No | Requires libtorch (hundreds of MB, unverified) | Stale: no release for 2 years, last push 2026-01 |
| [HF TEI](https://github.com/huggingface/text-embeddings-inference) | v1.9.4, 2026-09-15 (not on crates.io) | Apache-2.0 | safetensors (candle), ONNX (ort) ([backends](https://github.com/huggingface/text-embeddings-inference/tree/main/backends)) | CPU, CUDA, Metal | No | Server or Docker image (`cpu-1.9`), not a library | Production HF server. Serves XLM-RoBERTa / GTE / ModernBERT rerankers ([README](https://github.com/huggingface/text-embeddings-inference)) |

### `ort` (ONNX Runtime bindings)

- **Status.** The latest version is 2.0.0-rc.13 (2026-07-28) and crates.io lists no stable 2.x release ([crates.io](https://crates.io/api/v1/crates/ort)). The maintainers still call 2.0.0-rc.13 "production-ready" and recommend it for new projects ([ort.pyke.io](https://ort.pyke.io/)). Named users:
  - Google's Magika, SurrealDB, Supabase edge functions and Wasmtime's WASI-NN ([ort.pyke.io](https://ort.pyke.io/)).
  - 632 reverse dependencies on crates.io, including `magika`, `pyannote-rs`, `paddle-ocr-rs` and `kreuzberg` ([reverse deps](https://crates.io/api/v1/crates/ort/reverse_dependencies)).
- **Execution providers (EPs).** These are Cargo features ([2.0.0-rc.13 features](https://crates.io/api/v1/crates/ort/2.0.0-rc.13)):
  - Main ones: `coreml`, `cuda`, `tensorrt`, `directml`, `openvino`, `webgpu`.
  - Also available: `rocm`, `migraphx`, `qnn`, `xnnpack`, `nnapi`, `onednn`, `acl` and others.
  - By default, a failed EP registration silently falls back to CPU. `.error_on_failure()` makes it return an error instead ([EP docs](https://ort.pyke.io/perf/execution-providers)). Automan should use it so the fallback is never silent.
  - WebGPU is labelled experimental and "may produce incorrect results or crash" ([EP docs](https://ort.pyke.io/perf/execution-providers)).
  - Prebuilt CUDA binaries need CUDA ≥ 13.2 and cuDNN ≥ 9.23 on `PATH` ([EP docs](https://ort.pyke.io/perf/execution-providers)).
- **Prebuilt binaries.** The default `download-binaries` feature fetches ORT 1.31.0 from pyke's CDN, with a SHA-256 per artifact ([dist.tsv](https://github.com/pykeio/ort/blob/main/ort-sys/build/download/dist.tsv)). Available targets:
  - `aarch64-apple-darwin` (+coreml, webgpu)
  - `x86_64` / `aarch64-unknown-linux-gnu` (none, openvino, webgpu, cuda13 + tensorrt)
  - `x86_64` / `aarch64-pc-windows-msvc` (directml, openvino, cuda)
  - iOS and Android
  - **No `x86_64-apple-darwin`.** Microsoft stopped shipping Intel-macOS binaries as of ORT 1.24.1, and ort follows suit ([issue #556](https://github.com/pykeio/ort/issues/556)). An Intel-Mac Automan build would need a self-compiled ORT.
- **Static vs dynamic linking.**
  - Static linking is preferred "if your execution providers support it". If both static and dynamic libraries are present, ort picks static ([linking docs](https://ort.pyke.io/setup/linking)).
  - `load-dynamic` loads the dylib at runtime, through `ort::init_from(path)` or `ORT_DYLIB_PATH`. The executable then still starts when the library is missing ([linking docs](https://ort.pyke.io/setup/linking)). This suits Automan: ship a small engine binary and fetch the ORT dylib on demand.
  - CUDA and TensorRT EPs are always loaded dynamically ([EP docs](https://ort.pyke.io/perf/execution-providers)).
- **Size.** Upstream ORT 1.31.0 archives, compressed ([release assets](https://github.com/microsoft/onnxruntime/releases/tag/v1.31.0)):

  | Build | Size |
  |---|---|
  | linux-x64 CPU | 10 MB |
  | linux-aarch64 | 9 MB |
  | osx-arm64 | 34 MB |
  | win-x64 | 79 MB |
  | linux CUDA 13 | 233 MB |
  | linux CUDA 12 | 429 MB |

  - The PyPI wheels (which also contain Python bindings) are 14–23 MB ([PyPI](https://pypi.org/pypi/onnxruntime/json)).
  - I found no primary figure for the uncompressed `libonnxruntime` size added to a static Rust binary. Assume roughly 15–30 MB on CPU (unverified).
  - ORT's custom "reduced operator" and minimal builds shrink this further ([ORT custom build](https://onnxruntime.ai/docs/build/custom.html)).
- **Cross-compilation pains.** Reported issues:
  - Cross-compiling to macOS from Linux ([#217](https://github.com/pykeio/ort/issues/217)).
  - Windows ARM cross-builds ([#338](https://github.com/pykeio/ort/issues/338)).
  - Embedded armv7 linker errors ([#369](https://github.com/pykeio/ort/issues/369)).
  - Static linking combined with `download-binaries` ([#372](https://github.com/pykeio/ort/issues/372)).
  - macOS shared-library "hell" ([#388](https://github.com/pykeio/ort/issues/388)).

  In practice, build each target natively in CI.
- **Determinism.** `SessionBuilder::with_deterministic_compute(true)` maps to ORT's `SetDeterministicCompute`. ort's doc comment says default kernels "may introduce slight variance" ([source](https://github.com/pykeio/ort/blob/main/src/session/builder/impl_options.rs)).
- **Alternative backends.** `ort-tract` and `ort-candle` implement the ORT C API on top of pure-Rust engines. They are labelled "experimental", and ort-candle has no CUDA support today ([backends](https://ort.pyke.io/backends)).
- **Model download helper.** The `fetch-models` feature pulls in `ureq` and `sha2` ([features](https://crates.io/api/v1/crates/ort/2.0.0-rc.13)).

### `candle` (Hugging Face)

- **Status.** candle-core 0.11.0 was released 2026-06-26 ([crates.io](https://crates.io/api/v1/crates/candle-core)). The repo has 21k stars and was last committed 2026-10-02 ([commits](https://github.com/huggingface/candle/commits/main)).
- **Backends.** Features are `cuda`, `cudnn`, `nccl`, `metal`, `mkl` and `accelerate` ([0.11.0 features](https://crates.io/api/v1/crates/candle-core/0.11.0)). There is also a WASM target ([README](https://github.com/huggingface/candle#features)).
- **Models relevant to Automan** ([`candle-transformers/src/models`](https://github.com/huggingface/candle/tree/main/candle-transformers/src/models)):
  - Encoders: `bert`, `xlm_roberta`, `modernbert`, `debertav2`, `distilbert`, `jina_bert`.
  - Language models: `qwen2`, `qwen3`, and quantized GGUF variants (`quantized_qwen2/3`, `quantized_llama`, `quantized_phi3`, `quantized_gemma3`, `quantized_lfm2`).
  - Vision and VLMs: `qwen3_vl`, `llava`, `moondream` (+quantized), `paligemma`, `pixtral`, `paddleocr_vl`, `siglip`/`clip`, `blip`, `trocr`, `yolo-v8` (examples).
  - **Not present:** Florence-2 (OmniParser's caption model) and Qwen2-VL / Qwen2.5-VL.
  - `xlm_roberta` covers the bge-reranker architecture. I found no dedicated reranker example (unverified).
- **Performance.**
  - CPU: an open issue reports candle inference "~8.5x slower than PyTorch on CPU" ([#2877](https://github.com/huggingface/candle/issues/2877)).
  - Quantized Metal was reported much slower than llama.cpp in 2024 ([#1939](https://github.com/huggingface/candle/issues/1939)).
  - CUDA: a maintainer later measured parity with llama.cpp, 69.4 tok/s on an RTX 2080 for Mistral-7B Q4_K_S ([#1939 comment](https://github.com/huggingface/candle/issues/1939)).
  - Recent work still targets CPU quantized matmul ([commit 2026-10-02](https://github.com/huggingface/candle/commits/main)).
- **Binary.** candle is pure Rust and needs no external runtime on CPU or Metal. I found no published binary-size figure (unverified).
- **Downstream users.** mistral.rs, Kyutai's `moshi`, `kalosm`, fastembed (for Qwen3 models), and burn's former candle backend ([reverse deps](https://crates.io/api/v1/crates/candle-core/reverse_dependencies)).

### `burn` (Tracel AI)

- **Status.** 0.22.0 was released 2026-10-06 ([crates.io](https://crates.io/api/v1/crates/burn)).
- **Backends** ([README](https://github.com/tracel-ai/burn#supported-backends)):
  - CubeCL: CUDA, ROCm, Metal, Vulkan, WebGPU, plus a CPU runtime that JIT-compiles kernels through LLVM.
  - `flex`: a pure-Rust eager CPU backend that also runs on wasm and `no_std`.
  - LibTorch was deprecated in 0.22.0 and removed from `main`.
  - 0.22.0 has no `candle` feature, but still has `ndarray` and `tch` ([features](https://crates.io/api/v1/crates/burn/0.22.0)).
- **ONNX import.** It moved to the separate [`burn-onnx`](https://github.com/tracel-ai/burn-onnx) repo, which includes `onnx-ir` and `burn-import` (all 0.22.0).
  - It converts an ONNX file into Rust source plus a weights file in `build.rs`, so there is "no ONNX runtime to ship".
  - It claims opsets 1–24, constant folding and attention fusion, and has imported a 28k-node SDXL UNet ([README](https://github.com/tracel-ai/burn-onnx)).
  - Operator coverage is listed in [SUPPORTED-ONNX-OPS.md](https://github.com/tracel-ai/burn-onnx/blob/main/SUPPORTED-ONNX-OPS.md).
- **Maturity.** Pre-1.0 with frequent backend churn. burn-onnx has 137 stars. Burn suits small classifiers compiled into the binary. For autoregressive LMs or VLMs I found no turnkey path (unverified).

### llama.cpp bindings (`llama-cpp-2`, `llama_cpp`)

- **`llama-cpp-2`.** 0.1.159 was released 2026-10-07, with near-daily releases ([crates.io](https://crates.io/api/v1/crates/llama-cpp-2)). Its README calls it "dead simple, very close to raw bindings, and does not follow semver meaningfully" ([README](https://github.com/utilityai/llama-cpp-rs)).
  - Features: `metal`, `cuda`, `vulkan`, `rocm`, `opencl`, `mkl`, `openmp`, `dynamic-backends`, `dynamic-link`, `system-ggml`, `mtmd` (multimodal), `llguidance` (constrained decoding) ([features](https://crates.io/api/v1/crates/llama-cpp-2/0.1.159)).
  - The sys crate download is 5.6 MB because it vendors the llama.cpp source ([crates.io](https://crates.io/api/v1/crates/llama-cpp-sys-2)).
- **Multimodal (mtmd).** The `mtmd.rs` module exposes libmtmd ([source](https://github.com/utilityai/llama-cpp-rs/tree/main/llama-cpp-2/src)). Upstream VLM support includes:
  - Qwen2-VL 2B/7B, Qwen2.5-VL 3B/7B, Gemma 3/4, SmolVLM 256M–2.2B, InternVL 1B–14B and Moondream2.
  - OCR-specialised models (PaddleOCR-VL, GLM-OCR, DeepSeek-OCR) ([multimodal.md](https://github.com/ggml-org/llama.cpp/blob/master/docs/multimodal.md)).
  - A vision model needs a separate `mmproj` GGUF ([multimodal.md](https://github.com/ggml-org/llama.cpp/blob/master/docs/multimodal.md)).
- **Production reference.** Goose's `goose-local-inference` crate runs GGUF through `llama-cpp-2`, with `mtmd` for image input, an optional `hf-hub` feature, and memory-aware model placement and eviction ([README](https://github.com/aaif-goose/goose/blob/main/crates/goose-local-inference/README.md)). It pins `=0.1.146` because the crate "has breaking changes in patch releases" ([Cargo.toml](https://github.com/aaif-goose/goose/blob/main/Cargo.toml)).
- **`llama_cpp` (edgenai).** Abandoned: last release 2024-04-29, last push 2024-06 ([GitHub](https://github.com/edgenai/llama_cpp-rs)).
- **Determinism.** Deterministic inference is not the default in llama.cpp. A "Deterministic inference mode (CUDA)" PR has been open since 2025-09 ([#16016](https://github.com/ggml-org/llama.cpp/pull/16016)).

### `mistral.rs`

- **Status.** GitHub v0.9.4 was released 2026-09-24, but the crates.io `mistralrs` crate is stuck at 0.8.1 (2026-04-02) ([crates.io versions](https://crates.io/api/v1/crates/mistralrs/versions)). The workspace depends on a git rev of candle ([Cargo.toml](https://github.com/EricLBuehler/mistral.rs/blob/master/Cargo.toml)), which crates.io cannot publish, so embedding the current version means a git dependency.
- **Features** ([README](https://github.com/EricLBuehler/mistral.rs)):
  - Quantization: ISQ (in-situ quantization of Hugging Face weights at load), GGUF 2–8 bit, GPTQ, AWQ, HQQ, FP8, UQFF.
  - Serving and decoding: PagedAttention, continuous batching, grammar / JSON-schema constrained decoding, OpenAI- and Anthropic-compatible server.
  - Hardware: Metal, CUDA, CPU.
- **Vision models.** Qwen2/2.5/3-VL, Gemma 3/3n/4, Idefics2/3 (SmolVLM), Phi-3.5-vision, Phi-4-multimodal, Llama 3.2 Vision, MiniCPM-o, LLaVA. Florence is absent ([supported models](https://docs.mistralrs.dev/reference/supported-models/)).
- **Limits.** Embeddings are limited to EmbeddingGemma and Qwen3-Embedding, and there are no rerankers ([supported models](https://docs.mistralrs.dev/reference/supported-models/)). Published benchmarks are CUDA only (GB10, B200, H100) and say nothing about CPU ([README](https://github.com/EricLBuehler/mistral.rs#benchmarks)).

### `tract` (Sonos)

- **Status.** 0.23.8 was released 2026-09-21. The `tract` crate is the authoritative public API, and the internal `tract-*` crates are not a stable surface ([README](https://github.com/sonos/tract)).
- **Runtimes.** CPU with hand-written SIMD (x86, ARMv6–8, SVE), Metal, CUDA and WASM ([README](https://github.com/sonos/tract#runtimes)).
- **Formats.** ONNX, NNEF/tract-OPL and TFLite.
- **Use.** Used in production at Sonos for wake-word and streaming ASR. It also runs LLM, Stable Diffusion and YOLOv8 examples ([README](https://github.com/sonos/tract)).
- **Fit.** It is the most self-contained pure-Rust ONNX option for small classifiers. ort's docs rate ort-tract's operator support as "great" ([backends](https://ort.pyke.io/backends)).

### `fastembed-rs`

- **Status.** 7.1.1 was released 2026-10-07 ([crates.io](https://crates.io/api/v1/crates/fastembed)). It is synchronous, has "No dependency on Tokio", and uses `ort` ([README](https://github.com/Anush008/fastembed-rs)).
- **Rerankers.** `BAAI/bge-reranker-base` (the default), `BAAI/bge-reranker-v2-m3`, `jinaai/jina-reranker-v1-turbo-en` and `jinaai/jina-reranker-v2-base-multilingual` ([README](https://github.com/Anush008/fastembed-rs#reranking)). The ms-marco MiniLM cross-encoder is not in the list.
- **Defaults.** The default features pull ORT binaries and `hf-hub` downloads. Optional features include `ort-load-dynamic`, `directml`, `webgpu`, and candle-backed `qwen3` / `metal` / `cuda` ([features](https://crates.io/api/v1/crates/fastembed/7.1.1)).

### `rust-bert`

- **Status.** The last release is 0.23.0 (2024-09-29) and the last repository push was 2026-01-13 ([GitHub](https://github.com/guillaume-be/rust-bert)).
- **Runtime.** It depends on libtorch via `tch`, with optional ONNX via ort ([reverse deps of ort](https://crates.io/api/v1/crates/ort/reverse_dependencies)).
- **Verdict.** Not recommended: it is stale and carries a heavy native runtime.

### Hugging Face Text Embeddings Inference (TEI)

- **Status.** v1.9.4 was released 2026-09-15 ([releases](https://github.com/huggingface/text-embeddings-inference/releases)). It is written in Rust, with `candle` and `ort` backends ([backends/](https://github.com/huggingface/text-embeddings-inference/tree/main/backends)).
- **Hardware.** Metal is supported for local Mac execution ([README](https://github.com/huggingface/text-embeddings-inference)).
- **Rerankers.** It serves XLM-RoBERTa (bge-reranker-base/large), GTE and ModernBERT rerankers ([README](https://github.com/huggingface/text-embeddings-inference#sequence-classification-and-re-ranking)).
- **Distribution.** It is a server or Docker image (`cpu-1.9`, `cpu-arm64-1.9`) and is not on crates.io. Automan could use it as a reference design or a sidecar, but not as an embedded library.

### Footprint data

**Model file sizes.** These come from the Hugging Face API tree listings.

| Model | fp32 / fp16 | int8 / Q8 | Q4 | License |
|---|---|---|---|---|
| ms-marco-MiniLM-L6-v2 (22.7M params) | 86.7 MiB fp32 safetensors and ONNX | 22.1 MiB `model_qint8_{arm64,avx2,avx512,avx512_vnni}.onnx` | n/a | Apache-2.0 ([HF](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2/tree/main/onnx)) |
| bge-reranker-base (278M) | 1,061 MiB fp32 | (unverified) | n/a | ([HF](https://huggingface.co/BAAI/bge-reranker-base/tree/main)) |
| bge-reranker-v2-m3 (568M) | 2,166 MiB fp32 | About 569 MB in a third-party int8 build ([search result, unverified](https://huggingface.co/onebrain-ai/onebrain-rerank-v1)) | n/a | Apache-2.0 ([HF](https://huggingface.co/BAAI/bge-reranker-v2-m3)) |
| Qwen2.5-0.5B-Instruct | 1,208 MiB fp16 GGUF | 644 MiB Q8_0 | 469 MiB Q4_K_M, 409 MiB Q4_0 | Apache-2.0 ([HF](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/tree/main)) |
| Qwen2.5-1.5B-Instruct | 3,396 MiB fp16 | 1,807 MiB Q8_0 | 1,066 MiB Q4_K_M | Apache-2.0 ([HF](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/tree/main)) |
| Qwen3-0.6B | — | 610 MiB Q8_0 | — | ([HF](https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/tree/main)) |
| ReaderLM-v2 (1.5B) | 2,944 MiB safetensors | — | — | **CC-BY-NC-4.0** ([HF](https://huggingface.co/jinaai/ReaderLM-v2)) |
| NuExtract-2.0-2B | 4,213 MiB safetensors | — | — | MIT for 2B and 8B; the 4B is Qwen Research ([HF](https://huggingface.co/numind/NuExtract-2.0-2B)) |
| Qwen2.5-VL-3B-Instruct | 5,892 MiB f16 + 1,276 MiB mmproj | 3,133 MiB Q8_0 + 806 MiB mmproj Q8 | 1,841 MiB Q4_K_M | **qwen-research** licence for 3B ([HF](https://huggingface.co/Qwen/Qwen2.5-VL-3B-Instruct)). 7B and Qwen3-VL-2B are Apache-2.0 ([HF](https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct)) |
| OmniParser v2 | icon_detect 38.7 MiB `.pt` (AGPL-3.0, YOLOv8). icon_caption (Florence-2-base) 1,034 MiB | — | — | MIT overall. The new `icon_detect_v3` (YOLOv9-E) is MIT ([HF card](https://huggingface.co/microsoft/OmniParser-v2.0)) |
| jina-reranker-v2-base-multilingual | — | — | — | **CC-BY-NC-4.0** ([HF](https://huggingface.co/jinaai/jina-reranker-v2-base-multilingual)) |

**Latency and throughput.**

- **MiniLM-L6 cross-encoder on CPU.**
  - Qdrant's 2026-09-25 benchmark ran on an M4 Max using all 14 CPU cores, with about 30 query-document pairs per request on scidocs-reranking ([Qdrant blog](https://qdrant.tech/blog/oxidizing-cross-encoders/)).
  - p50 was about 25–29 ms per request, which is about 0.85–0.96 ms per pair. These per-pair figures appear in search-result excerpts of the same post. p99 was 36 ms for the Rust `cross-encode-rs` (ort) and 49 ms for fastembed.
  - CoreML EP was much slower on this small model: a p50 of 213 ms versus 29 ms on the CPU EP ([Qdrant blog](https://qdrant.tech/blog/oxidizing-cross-encoders/)).
  - ONNX session creation took about 34–38 ms ([Qdrant blog](https://qdrant.tech/blog/oxidizing-cross-encoders/)).
- **Base-size multilingual reranker on CPU.** jina-reranker-v2-base had a p50 of about 131 ms per 30-pair request in Rust, against about 190 ms in Python ([Qdrant blog](https://qdrant.tech/blog/oxidizing-cross-encoders/)).
  - I found no primary absolute CPU latency for bge-reranker-v2-m3 (568M).
  - By parameter count, expect it to be several times slower than the base model (unverified).
- **Backend speedups.** Sentence-Transformers' CPU benchmark (i7-13700K; MiniLM-L6, bge-reranker-base, mxbai-rerank-large, bge-reranker-v2-m3) reports relative speedups only:
  - The int8 ONNX/OpenVINO variants give similar speedups.
  - fp16/bf16 on CPU runs at about 0.17x, so avoid it.
  - Recommendations are `openvino-qint8` when minor quality loss is acceptable, otherwise OpenVINO on Intel and `onnx-O3` elsewhere ([sbert efficiency](https://sbert.net/docs/cross_encoder/usage/efficiency.html)).
  - The published "1800 docs/s" figure for MiniLM-L6 is measured on a V100 GPU ([model card](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2)).
- **Small LMs (llama.cpp).**
  - Apple M1 Max CPU, 8 threads: Qwen3-0.6B Q4_0 generates about 202 tok/s and Qwen3-4B Q4_0 about 31.9 tok/s, with about 186 tok/s prefill on the 4B ([#27078](https://github.com/ggml-org/llama.cpp/issues/27078)).
  - Arm server CPUs, Qwen3-0.6B Q8_0: about 45–61 tok/s generation and 136–233 tok/s prefill on Neoverse N1 (Graviton2) and N2 ([#27078](https://github.com/ggml-org/llama.cpp/issues/27078)).
  - Apple Silicon Metal, Llama-2-7B Q4_0: 14 tok/s on M1, 24 tok/s on M4 and 83 tok/s on M4 Max (40-core GPU) ([discussion #4167](https://github.com/ggml-org/llama.cpp/discussions/4167)). Generation is memory-bandwidth bound, so a 0.5–1.5B Q4 model should run several times faster (unverified extrapolation).
  - I found no primary x86 desktop CPU figure for 0.5–1.5B models (unverified).
- **VLMs.** These are anecdotal figures from llama.cpp bug-report logs, not benchmarks:
  - Qwen2.5-VL-3B image-slice encode took 301–350 ms on an RTX 4070 ([#17534](https://github.com/ggml-org/llama.cpp/issues/17534)).
  - SmolVLM2-2.2B f16 took about 16 s per slice on an Intel Iris Plus laptop iGPU via Vulkan ([#17438](https://github.com/ggml-org/llama.cpp/issues/17438)).
  - A Qwen3.5-9B vision projector falling back to CPU took about 10–13 s per slice ([#21272](https://github.com/ggml-org/llama.cpp/issues/21272)).
  - Taken together: a 2–3B VLM is practical on a GPU or Metal, while on CPU it is a multi-second-per-screenshot operation (unverified for Metal 2–3B specifically).

### Packaging trade-offs

- **Bundled vs on-demand.**
  - Reranker int8 ONNX files are about 22 MB (MiniLM) and could be bundled.
  - Rerankers in the 278M–568M class (1–2 GB fp32) and any LM or VLM (0.4–7 GB) should be downloaded on demand.
  - The ORT dylib can also be on-demand through `load-dynamic` / `ort::init_from` ([linking](https://ort.pyke.io/setup/linking)).
- **Downloads.** `hf-hub` 1.0.0 (2026-07-10) has an async API on your own Tokio runtime and a `blocking` feature, supports Xet transfers, and is Apache-2.0 ([crates.io](https://crates.io/api/v1/crates/hf-hub), [README](https://github.com/huggingface/hf-hub)).
- **Checksums.**
  - The Hugging Face tree API exposes each LFS file's SHA-256 as `lfs.oid` ([example](https://huggingface.co/api/models/cross-encoder/ms-marco-MiniLM-L6-v2/tree/main/onnx)). Automan can pin `(repo, revision, path, sha256)` in a manifest.
  - ort's own binary downloads are SHA-256-verified ([dist.tsv](https://github.com/pykeio/ort/blob/main/ort-sys/build/download/dist.tsv)).
- **Weight licensing traps.**
  - ReaderLM-v2 and jina-reranker-v2 are CC-BY-NC-4.0.
  - Qwen2.5-VL-3B is under the qwen-research licence.
  - OmniParser's original `icon_detect` is AGPL-3.0, while the new `icon_detect_v3` is MIT.
  - Apache/MIT options: bge-reranker-v2-m3, MiniLM, Qwen2.5 text, Qwen2.5-VL-7B, Qwen3-VL-2B, NuExtract-2.0 2B and 8B (the 4B is under the Qwen Research licence).
  - Sources: the Hugging Face model cards in the footprint table above.
- **Sandboxing.**
  - The ONNX and GGUF parsers are native C/C++ (ORT, llama.cpp) and run in-process.
  - The pure-Rust options (tract, candle, burn-flex) reduce memory-safety exposure for untrusted model files (unverified as a security claim).
  - Running inference in a separate worker process is the conservative choice, as goose does by isolating providers (unverified).
- **Determinism.**
  - ORT: deterministic compute is opt-in ([ort source](https://github.com/pykeio/ort/blob/main/src/session/builder/impl_options.rs)).
  - llama.cpp: deterministic mode is still a CUDA-only open PR ([#16016](https://github.com/ggml-org/llama.cpp/pull/16016)).
  - Expect the same output on the same backend and thread count with greedy decoding, but not bit-identical output across CPU, Metal and CUDA, or across batch shapes (unverified generalisation).
  - Store model hash, runtime version and EP in the run record.
- **Tokio thread contention.**
  - Tokio's docs say to use `spawn_blocking` for blocking work. For CPU-bound work they suggest a separate pool such as rayon, returning results over a oneshot channel, or a dedicated runtime ([tokio docs](https://docs.rs/tokio/latest/tokio/index.html)).
  - ORT and llama.cpp run their own intra-op thread pools (fastembed exposes `with_intra_threads`, see [README](https://github.com/Anush008/fastembed-rs)). Size them explicitly so they do not oversubscribe the Tokio workers. A single dedicated inference thread or worker with a bounded queue (micro-batching) matches what cross-encode-rs does ([Qdrant blog](https://qdrant.tech/blog/oxidizing-cross-encoders/)).

### Rust projects running local models in production

- **HF TEI.** Rust, with candle and ort backends, serving rerankers ([GitHub](https://github.com/huggingface/text-embeddings-inference)).
- **Qdrant.** Maintains fastembed-rs (ort) ([GitHub](https://github.com/Anush008/fastembed-rs)) and published cross-encode-rs benchmarks ([blog](https://qdrant.tech/blog/oxidizing-cross-encoders/)). Qdrant's own server `Cargo.toml` shows no fastembed, ort or candle dependency ([Cargo.toml](https://github.com/qdrant/qdrant/blob/master/Cargo.toml)).
- **ort users.** Google Magika, SurrealDB, Supabase and Wasmtime WASI-NN ([ort.pyke.io](https://ort.pyke.io/)).
- **Goose (AAIF).** Embeds llama.cpp through `llama-cpp-2` with `mtmd` ([crate](https://github.com/aaif-goose/goose/tree/main/crates/goose-local-inference)).
- **Zed.** Its `llama_cpp` crate is an HTTP client to a llama.cpp server (it depends on `http_client`), not embedded inference. I found no candle dependency in Zed's workspace ([Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/llama_cpp/Cargo.toml)).
- **Spider.** AI features call remote OpenAI or Gemini through `async-openai` and `gemini-rust`. I found no local runtime ([Cargo.toml](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml)).
- **Browser Use.** I found no Rust runtime making local model calls (unverified, not investigated further).

## 7. Synthesis for Automan: where a decision provider sits

This section lays out options against the owner's answers so far in [#72](https://github.com/Jbmanllr/automan/issues/72): compiled plans with fingerprints, a whole-plan check before the first effect, one repair per plan per run, `observe` / `act` / `extract` / `check`, the accessibility tree by default, vision opt-in as the last rung, an engine-owned in-house provider layer (Q17, Q18 open). It adopts nothing.

### 7.1 What a "decision model" is, mechanically

Every system in this note that picks from a closed set does one of four things:

1. **Score each candidate independently** against the instruction: a cross-encoder (MiniLM, DeBERTa, bge-reranker), a pointwise LLM reranker reading P("yes") (Qwen3-Reranker), or a Noul per candidate. Output: one score per candidate, not a distribution; "none of these" needs a threshold fitted on held-out labels (§3.2).
2. **Read a distribution over the offered options in one pass**: the System One class (Jev, Clef, pplx-decider, decider, Jeff, OpenAI Decisions). An LLM backbone runs prefill only; a readout gives one logit per option; a softmax and a fitted temperature give probabilities (§2.2). A "none" option is just another option, and only as good as its training (decider-2b's card: chosen "when nothing on offer fits, not when the exact fine-grained label is merely absent").
3. **Ask a generative LLM** to name the option (MindAct's multiple-choice with "None", Stagehand's current `act`). Verbalized confidence clusters at 80–100% ([Xiong et al.](https://arxiv.org/abs/2306.13063)); token logprobs over option letters are the cheaper calibrated signal when a provider exposes them.
4. **Point at pixels** (vision grounders): a coordinate, usually without any confidence (§4). The only published hooks are a separate verifier (GUI-Actor-Verifier), detector box scores (OmniParser), cross-model or zoom agreement (GroundCascade), and sampled-click dispersion ([SafeGround](https://arxiv.org/abs/2602.02419)).

So "decision provider" is an interface over all four, not a vendor. Jev is one hosted implementation of family 2; the same request shape now has open Apache-2.0 implementations from 0.8B to 27B, some with GGUF builds a Rust engine can load through `llama-cpp-2` (§6). Whether a GGUF build preserves the decision readout and the fitted temperature has to be checked per model: the pplx-decider card warns that default causal inference "does not reproduce this checkpoint's evaluated setup" ([card](https://huggingface.co/perplexity-ai/pplx-decider-v1.1-27b)), and Clef uses a separate joint head file ([card](https://huggingface.co/Cloudflare/clef-flash)). Unverified for every GGUF conversion listed.

### 7.2 The ladder, and where the rung goes

The resolution ladder under discussion in #72, with the decision rung inserted:

| Rung | Input | What decides | What it costs (order of magnitude, sources in §2–§6) | Abstains when |
| --- | --- | --- | --- | --- |
| 0. Compiled selector | plan step | code | ~0 model cost | selector does not resolve, or resolves to several nodes |
| 1. Fingerprint check | resolved node + stored fingerprint | code (attribute similarity, Similo-style) | ~3 ms per locate in Similo ([paper](https://arxiv.org/abs/2208.00677)) | similarity under threshold, or a decoy matches better |
| 2. Decision model | instruction + candidate list from the accessibility tree (role view, context for twins) | family 1 or 2 above, local or hosted | MiniLM cross-encoder ~1 ms per pair on CPU; hosted System One 40–540 ms per request; local 2B decider 3 ms on a datacenter GPU, CPU unmeasured | top probability under threshold, margin to the second too small, "none" chosen, answer fails validation, option-order shuffle changes the answer |
| 3. LLM on the accessibility tree | instruction + snapshot (+ rung 2's shortlist) | generative LLM | ~0.5–5 s and tokens; Stagehand's own act p50 1.97 s | refuses, or its pick fails the postcondition |
| 4. Vision | screenshot (+ instruction) | computer-use tool or grounder | seconds per screenshot; ScreenSpot-Pro single-pass 40–73% for open 7–32B models | almost never on its own (refusal subsets near 0%) |

Two placements are possible and they are not exclusive:

- **As a rung** between fingerprint and LLM: it resolves the closed-set cases cheaply and hands the rest up. Stagehand's numbers show the shape of the gain and its limit: most acts resolved at rung 2, but with the LLM removed success fell to 27/40 and 26/40 ([#2953](https://github.com/browserbase/stagehand/pull/2953)).
- **As a check inside rungs 0–1**: the question "is the node this selector now resolves to still the target?" ([#2954](https://github.com/browserbase/stagehand/pull/2954)). Automan's lab already showed the fingerprint rejecting a decoy before any action on `decoy-swap` ([lab report](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/ai-real-models-2026-10-09.md)); a semantic check would target the case a fingerprint cannot see: same attributes, different meaning (a relabelled control, a price that changed). Stagehand ran this check with "no eval [that] exercises the cache path end to end".

### 7.3 Per use

| Use | Candidate set | Possible decision step | Evidence found | Main risk |
| --- | --- | --- | --- | --- |
| **Automation (`act`, `observe`)** | Accessibility-tree nodes filtered by intent (pointer, input, select, option), with ancestors, heading, row/column, "n of m" | Rank then choose: lexical or cross-encoder shortlist, then a choice with an explicit "none"; deterministic read-back after the effect | Stagehand #2953/#2954 (in-sample); Mind2Web ranker Recall@50 85–89% and the cross-site drop of >10 points; decider-2b Mind2Web 83.9% (in its training mix) | Out-of-site drift; option-order bias (Jev's own list); injected text in candidate labels |
| **Repair** | Candidates near the stored fingerprint (same form, same region, same role) | Choice among top-k from a Similo-style scorer; the repaired step then passes the whole-plan check and the step's postcondition | VON Similo 94.7%; VON Similo + GPT-4 95.0% at 1.9 s and $0.045 per localization; more than 10 candidates hurt GPT-4 | "Passed but wrong" heals: products validate by threshold or by later assertions, none by identity before the effect (§3.4) |
| **Extraction** | Elements whose text could hold the field; repeating groups for lists | Pick and copy (model picks, code copies, grounding by construction); completeness judge as a yes/no; page-type routing to the right extractor | Stagehand #2955 (36/75 with no LLM, 69/75 with LLM fallback); MinerU-HTML block labels are extractive; WCXB shows models trailing rs-trafilatura on non-article pages | Numbers, dates and counting are named Jev weaknesses: keep them in code; pages over the context limit |
| **Crawl** | Outlinks of a fetched page; or a subdomain | Score the fetched page and pass the score to its links; classify a subdomain sample | Craw4LLM, neural prioritisation, Data Scout: all score fetched content, all self-reported or simulated; no URL-only evidence | Cost per scored page at crawl scale; no benchmark for "prompt to crawl" products |
| **Login, challenge, page state** | The page as a whole | Rules first (`cf-mitigated: challenge`, CAPTCHA iframe hosts, password field, JSON-LD paywall markup), then a classifier for the rest | PhishIntention 95% with screenshot + layout vs 60.8% for an input-box rule; Stagehand's page-state question (unit tests only); #2994: content alone does not tell "loaded" | Feeds Q27's typed human-wait events, so a false "login wall" pauses a Session |
| **Vision** | Screenshot regions | Grounder proposes; a verifier or a decision model with image input judges; or OmniParser boxes become a candidate list for rung 2 | GUI-Actor-Verifier +3.1 points on ScreenSpot-Pro; Clef and decider-2b-vision accept images (no GUI benchmark found) | Grounders always answer; refusal near 0% |

### 7.4 Contract options for a decision-provider interface

What the sources converge on, written as options rather than a design:

- **Request:** decision kind (`pick-target`, `same-target`, `pick-value`, `page-state`, `link-score`, `complete?`); instruction; candidates with stable ids (engine refs, not model-visible selectors), role, accessible name, value, context lines; whether "none" is offered; the untrusted-content tag on page text; a budget reservation from the run's Limits (Q19).
- **Response:** `resolved { candidate_id, probabilities, confidence }` or `abstained { reason }` with typed reasons (`below-threshold`, `ambiguous-twins`, `none-chosen`, `invalid-answer`, `order-unstable`, `provider-error`, `budget`), plus provider, exact model version (not an alias), latency, tokens and cost. Stagehand's #3107 driver contract (`resolved | abstained(reason)`, a fallback chain is itself a driver) and #3102's validation (the choice must be an offered id; probabilities in range and consistent; the choice is the argmax) are the closest public precedents.
- **Thresholds per (provider, model version, decision kind)**, fitted on held-out labels and stored with the plan; re-fitted when the version changes. TypeSafe tells users to pin versions for this reason ([Models](https://docs.typesafe.ai/models)); #3102 says thresholds tuned on Jev do not transfer to other providers; ARGUS finds uncertainty rankings do not transfer across models ([arXiv 2606.25760](https://arxiv.org/abs/2606.25760)).
- **Cheap robustness checks the engine controls:** shuffle option order and re-ask on low margin (Jev's order bias); split long lists (decider: accuracy falls with long option lists and positional lookups); keep arithmetic, dates and counting in code (both vendors say so).
- **Journal and count every call** like any model call (Q17, Q19): request digest, candidate ids, answer, confidence, decision taken (act, escalate, abstain), and the oracle outcome when known. The journal is also the training and calibration set for a later local model (§3.3 on distillation; Jeff and decider are trained exactly this way, from teacher labels and outcome rewards).
- **Confidence is never proof.** The effect still goes through the target-identity check before and the postcondition after; a decision model's "done" or "complete" is a signal like an LLM judge (Q21).
- **Data leaving the machine.** Hosted providers receive instruction, candidate text and a page digest; Stagehand redacts `%var%` values in every request. A local provider removes that exposure; a hosted one needs the same redaction and the domain policy (Q28, Q29).

### 7.5 Local or hosted

| Option | For | Against |
| --- | --- | --- |
| Hosted System One (Jev, Clef on Workers AI, Decider, OpenAI Decisions, via OpenRouter) | No weights to ship; 40–540 ms; $0.04–0.24 per million input tokens | Network round trip on every decision; data leaves the machine; aliases move; rate limits "adjusting dynamically" (TypeSafe); OpenAI's API is beta and its own Stagehand test got a 403 |
| Local cross-encoder via `ort` | 22 MB int8 MiniLM, ~1 ms per pair on CPU, Apache-2.0, mature runtime | Scores, not distributions: needs a fitted "none" threshold; MS MARCO training is not web elements, and no fine-tune for element ranking was found |
| Local System One-style model (Jeff 0.8B, decider 0.8B/2B) via llama.cpp | Apache-2.0, GGUF, the only models found with published ECE and held-out splits | Very new (September–October 2026), single maintainers, self-reported; CPU latency unmeasured; GGUF fidelity to the readout unverified |
| Local vision grounder | Keeps screenshots on the machine | 2–9B VLM: seconds per screenshot on CPU; licences vary per size (Holo, Qwen2.5-VL-3B, KV-Ground) |

## 8. What a lab campaign would have to measure

The lab method ([rnd/lab/README.md](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/README.md)) already has the parts: decisions declared in `decisions/` before measuring, oracle gates (`oracle-exact`, `false-success-count`), paired blocks, typed errors, and an `ai` lane with `relabel`, `decoy-swap` and `price-change` variants ([D-ai-layer](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/decisions/D-ai-layer.json)). A decision-provider campaign would extend it rather than start over. Options for what to declare:

- **Question.** Does a decision rung between fingerprint and LLM reduce model cost and p95 latency per verified task, without adding false successes, on sites not used to fit its thresholds?
- **Options.** (a) no decision rung, LLM on the accessibility tree (baseline, today's `compiled-replay` with LLM repair); (b) local cross-encoder plus fitted threshold; (c) local System One-style model (Jeff or decider) through llama.cpp; (d) hosted System One (Jev and one open-weight hosted model, e.g. Clef-Flash); (e) for repair only, Similo-style scoring with no model.
- **Primary metric.** `model-usd-per-1k-verified` (already in the catalog), with `task-p95-ms-verified` as the latency counterpart; a non-inferiority rule on verified success.
- **Contract gates.** `oracle-exact` and `false-success-count = 0`; plus a new gate that no effect ran on a decoy.
- **New metrics the catalog would need** (from §3.5): abstention rate per decision kind; selective risk at fixed coverage and AURC; ECE and Brier on the held-out split; escalations per verified task; option-order instability rate; decision latency p50/p95 separate from settle and snapshot time (Stagehand found settle and snapshot dominate once the model is fast, [#2993](https://github.com/browserbase/stagehand/pull/2993), [#2994](https://github.com/browserbase/stagehand/pull/2994)).
- **Held-out design.** Fit thresholds and temperatures on one set of sites, report on others, and split like Mind2Web and WebLINX: same site / unseen site / unseen category. The Stagehand figures are in-sample by their own statement; the WebLINX and Mind2Web drops (10–25 points) are the size of effect to expect.
- **Variants that stress the rung:** twins (identical labels in different rows); relabel; decoy-swap; destructive look-alike (the lab's "Delete my account" trap); long candidate lists (over 40, over 255); prompt injection in an element label; non-English pages; late-loading targets and overlays (#2994's PerimeterX and consent cases); a page where the right answer is "none".
- **Uses beyond `act`:** a `same-target` check on cached replay (the case Stagehand left unevaluated); pick-and-copy extraction scored by the existing field oracle (`field-precision`, `field-recall`); page-state classification against fixture labels (login, challenge, error, soft-404) with rules as the baseline.
- **Host and cost.** Hosted providers need a cost cap per campaign like `ai-real-models`; local models need the host class and the model hash, runtime version and execution provider in the run record (§6).

## 9. Open points and caveats

- **Nearly every accuracy figure in this note is self-reported.** Independent re-runs exist mainly for 2024–25 vision grounders and for Online-Mind2Web agents. Nothing about Jev, Clef, Decider, decider or Jeff has been reproduced by a third party that I found.
- **The System One field is three to four weeks old.** Model versions moved weekly during this research (pplx-decider v1 → v1.1 in five days, decider v10 → v11). Any campaign must pin versions.
- **No benchmark of decision models on web element selection** exists outside the vendors' and Stagehand's own suites; Cloudflare's 43-benchmark Decision Index contains none.
- **Calibration is claimed more than measured.** TypeSafe publishes no ECE; Cloudflare's card reports none; decider publishes ECE and calls itself overconfident on hard held-out sets.
- **Unverified items** are marked inline. The main ones: Jev's release date (third-party), Clef and Decider hosted prices (third-party, sources disagree), GGUF fidelity of decision readouts, CPU latency of 0.8–2B decision models, and the Holo 3/4 grounding scores published only in images.

## 10. Method

- Read ticket [#92](https://github.com/Jbmanllr/automan/issues/92), the comments of [#72](https://github.com/Jbmanllr/automan/issues/72), the October 7 R&D notes named above, and skimmed the [AI browser frameworks survey](https://github.com/Jbmanllr/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md) to avoid repeating it.
- Stagehand: PR state, bodies and stack read through the GitHub API (`gh api repos/browserbase/stagehand/pulls/<n>`, `gh pr list --search`); the npm `latest` tarball of `@browserbasehq/stagehand` 4.2.0 unpacked and searched for `experimentalDecisions`.
- TypeSafe: documentation pages fetched from `docs.typesafe.ai` (index at [llms.txt](https://docs.typesafe.ai/llms.txt)), blog and enterprise pages, and the `typesafe-ai` GitHub organization.
- Other decision models: Hugging Face model cards and API (`/api/models/<id>`) for Clef, pplx-decider, decider and Jeff; vendor documentation for OpenAI Decisions and OpenRouter; third-party pages only where no primary page was found, and marked as such.
- Families in sections 3 to 6 were researched in parallel against primary sources (papers, model cards, official repositories, crates.io, vendor docs), then merged and cross-checked for contradictions (for example NuExtract 2.0 licences per size).
- No model was run and no figure was reproduced here.
