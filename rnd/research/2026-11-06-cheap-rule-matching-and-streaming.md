# Cheap cosmetic rule matching and streaming parsing per page

Research for [#2154](https://github.com/goodfetch/goodfetch/issues/2154), a child of the map [#2141](https://github.com/goodfetch/goodfetch/issues/2141). Read on 2026-11-06 from primary sources at the versions and commits named in each section. R&D material: nothing here is an adopted decision.

## Question

How do fast extractors and ad blockers keep cosmetic and clean-up rule matching cheap per page, and how do streaming HTML parsers run on a body's chunks as they arrive? What could Foxhound adopt without breaking ADR 0047 (clean-up for formats), ADR 0119 (decoding) or ADR 0138 (one Page analysis, one parse)?

## Answer

1. **Nobody tests every rule against every element.** Every engine that matches thousands of selectors buckets each rule by a key that an element must hold for the rule to match: Blink's `RuleSet` and Stylo's `SelectorMap` key by the rightmost compound's ID, class, tag or attribute name, so an element tests only the buckets for what it holds. adblock-rust and uBlock Origin do the page-level half of it: they send a page only the generic rules keyed by a class or ID the page holds, and leave per-element matching to the browser's CSS engine, which buckets.
2. **Foxhound does the page-level half and skips the per-element half.** It asks adblock-rust for the page's rules the way Brave does, then runs every rule on every element (`removal.rs`: `css.iter().find(|rule| node.is_match(&rule.kind))`). It has no browser CSS engine, so it needs its own index. About 600 rules apply to every page, the generic rules adblock-rust cannot key by class or ID, and about 360 of those are `a[href^="…"]`. A tag and attribute index means a `<p>` tests none of them.
3. **The per-page rule list is mostly Run- and host-invariant work redone per page.** adblock-rust's `hostname_cosmetic_resources` copies every generic selector it cannot key into a fresh `HashSet<String>` on every call. Foxhound does this once per list per page, then rebuilds a `BTreeMap` and parses each procedural rule's JSON twice. uBlock Origin precomputes that generic set once as one string. A 16-entry MRU cache keyed by the host's exception set means a page whose exceptions touch no generic rule reuses the precomputed string. uBlock Origin also keeps a per-hostname selector cache of up to 50 hosts. ADR 0138 already requires Run invariants to be built once. A per-host cache of the compiled index extends that rule to host invariants.
4. **lol_html streams with bounded memory, but it cannot do Foxhound's clean-up.** It needs an ASCII-compatible encoding. Its selectors reject `+`, `~`, `:has()` and every non-tree-structural pseudo-class. It decides at the start tag, so it cannot run `:has-text`, `:upward` or `:min-text-length`. It builds no tree, so readability would need a second parse, which ADR 0138 forbids. Keep it for the no-tree path ADR 0138 already gives it.
5. **The tree can be built while the body arrives without lol_html.** html5ever's parser accepts chunks (`TendrilSink::process`), and `dom_query::Document` is its `TreeSink`. ADR 0119's encoding order allows this once the encoding is known: BOM, setting and header at once, the `<meta>` prescan by the end of the head. Its last step, UTF-8 for a valid UTF-8 body, works when UTF-8 is validated as it streams: an invalid byte means escalation, and the partial tree is dropped, the same outcome as today. It saves latency, not memory or instructions. Low priority.
6. **Readability's cost is its retry loop.** dom_smoothie's `parse()` clones the whole `Document` for each of up to 4 attempts while the text found is under 500 characters. Readability.js resets `innerHTML` from a cached string for each attempt. trafilatura runs readability-lxml and jusText as well unless `fast=True`. Foxhound calls `Readability::new(html)`, a parse from a string, then `parse()`. `Readability::with_document` (ADR 0138) and a single-attempt policy behind a setting are the levers.

## 1. Where Foxhound's clean-up spends (code at `b360ab6b1`)

From `.claude/handoff/wiki/research-spider-gap-20261106.md`, section "Extraction": clean-up is 4.73 M instructions of a page's 8.15 M. Within it, `Matcher::match_element` (7,968 calls) costs 2.51 M, `rules_selector` cached lookups (613 calls) 0.81 M, and `PageRules::of` with `css_rules` 1.10 M. The benchmark pages are small: 7,968 ÷ 613 ≈ 13 elements tested against every rule.

What the code does, `crates/goodfetch-clean-up-rules/src/`:

- `cosmetic.rs` `PageRules::of` calls `engine.url_cosmetic_resources(url)` for each list that is on, once per page.
- `css_rules` merges `hide_selectors`, `hidden_class_id_selectors(classes, ids, exceptions)` and the procedural rules that are plain CSS into a `BTreeMap<String, (String, List)>`, cloning each selector. `procedural()` runs `serde_json::from_str` on every procedural rule, once from `css_rules` and again from `PageRules::procedural`.
- `removal.rs` `remove` parses the page, collects every class and ID into two `BTreeSet<String>`, gets the page's CSS rules and looks each one up in `Selectors` (a `Mutex<HashMap<String, Option<Matcher>>>`, compiled once per Run since #1765). Then, for each element of each root, it tests rules in order until one matches. An element that matches nothing tests all of them.
- dom_query 0.28's `Matcher::match_element` builds a fresh `SelectorCaches` and `MatchingContext` per call with no ancestor Bloom filter (`dom_query-0.28.0/src/matcher.rs` 36–50, 323–333). `match_element_with_caches` lets a caller reuse one. `Matcher`'s `selector_list` is private, so an index has to derive each rule's key from its text.

Census of the embedded lists. This is a lexical approximation by a Python grep, not adblock-rust's parser, so it is **unverified at the parser level**: `crates/goodfetch-clean-up-rules/rules/`, `published_at` 2026-10-22.

| List | Generic `##` rules | Keyed by a leading `.`/`#` | Not keyable (misc generic) | Host-specific rules | Host rules with a procedural operator or `:has` |
|---|---|---|---|---|---|
| EasyList | 13,634 | 13,204 | 430 (260 `a…`, 92 `[attr]`, 42 `div…`) | 10,733 | 971 |
| Fanboy Annoyance | 12,024 | 11,842 | 182 (98 `a…`, 31 `div…`, 26 `[attr]`) | 9,760 | 419 |
| EasyPrivacy | 2 | 1 | 0 | 35 | 0 |

The 430 + 182 = 612 misc generic selectors match ADR 0138's "about 614 CSS selectors per page" and the register's 613 (TESTING.md, X1): these are the rules every page receives whatever its classes. Of the 612, about 360 are `a[href^="https://…"]` affiliate-link rules.

## 2. adblock-rust 0.13.3: the cosmetic filter cache

Source: crates.io `adblock` 0.13.3, the version the workspace pins (`Cargo.toml`: `adblock = { version = "0.13.3", … }`), `src/cosmetic_filter_cache_builder.rs`, `src/cosmetic_filter_cache.rs`, `src/engine.rs`.

- **Build time** (`CosmeticFilterCacheBuilder::add_generic_filter`): a generic plain-CSS rule starting with `.` goes to `simple_class_rules` when it is just `.class`, else to `complex_class_rules[class]`. `key_from_selector` keys it by its leading class. The same holds for `#` with IDs. Everything else goes to `misc_generic_selectors`. Procedural rules can't be generic and are dropped silently. Host-specific rules go to `hostname_hide`, `hostname_inject_script` and `HostnameSpecificRules` (unhide, procedural actions and their exceptions), keyed by host and entity hashes. All of it is serialized into a FlatBuffer.
- **There is no tag index.** "Generic" rules that start with a tag or an attribute (`a[href^=…]`, `[data-…]`, `div[id^=…]`) are all in `misc_generic_selectors`.
- **Per page, `url_cosmetic_resources(url)`**: one `check_generic_hide` network match on the URL. Then `hostname_cosmetic_resources(hostname, generichide)` looks up each of the host's hashes and applies unhide exceptions. Unless `generichide` is set, it copies every misc generic selector not excepted into a new `HashSet<String>` ("Calculate the intersection of the two sets"). That copy runs per call. It is the part of `PageRules::of` that depends on neither the page nor, for most hosts, the host.
- **Per class and ID, `hidden_class_id_selectors(classes, ids, exceptions)`**: probes FlatBuffer hash views for each class and ID and returns `.class` / `#id` and the complex selectors keyed by them, minus exceptions. The doc comment states the intended use: "we only inject rules based on classes and ids that actually appear on the page (in practice, a `MutationObserver` is used to identify those elements)". It also says that "the exceptions, along with the set of already-seen classes and ids, must be cached externally as the cosmetic filtering subsystem here is designed to be stateless with regard to active page sessions".
- **Keying is by the leftmost class or ID, not the rightmost.** `.a-class div#ads > .advertisement` is keyed by `a-class`. That decides whether a rule can apply to a page. It is not a per-element bucket: the element the rule hides is `.advertisement`, not the `.a-class` holder.
- The engine's own doc: cosmetic filtering "is *not* designed to provide a 100% accurate report of what will be blocked on any particular page". Its job is a safe superset to inject as CSS.

### How Brave applies it per page (brave-core `5dbaed752a884f3528c0ac32624c82a0dc11b9ab`)

- `components/cosmetic_filters/renderer/cosmetic_filters_js_handler.cc`: on navigation, `ProcessURL` asks the browser process for `UrlCosmeticResources(url)`, async or sync, and `ApplyRules` injects `hide_selectors` as a stylesheet and sets `generichide` for the content script.
- `components/cosmetic_filters/resources/data/content_cosmetic.ts`: the content script keeps `queriedIds` and `queriedClasses` sets for the frame's life. It reads new IDs and classes from a `MutationObserver`, or by polling with `[id]:not(html):not(body),[class]:not(html):not(body)`. It throttles calls to `cf_worker.hiddenClassIdSelectors({classes, ids})` with only the not-yet-queried names. Returned selectors go into one constructed `CSSStyleSheet`, so the browser's CSS engine does the per-element matching.
- Brave's first-party check (`hide1pContent` off) is evaluated with three queues. Each "pump" joins at most 60 selectors (`maxWorkSize = 60`) into one combined selector for a single `document.querySelectorAll`. Only for a matched element whose subtree is first party, or which shows significant text, does it run `matches(selector)` per selector to find which one. This batches 60 rules into one tree walk, the cheap trick that works without an index.
- **Not checked:** whether Brave's browser process caches `UrlCosmeticResources` per host. Only the renderer side was read.

## 3. uBlock Origin 1.75.4: generic families, MRU and a per-host selector cache

Source: tag `1.75.4` (latest release on 2026-11-06), `src/js/cosmetic-filtering.js`, `src/js/contentscript.js`.

- **Families** (comment above `CosmeticFilteringEngine`): "Low generic simple: class or id only", "Low generic complex: class or id + extra stuff after", "High generic" (high-low `[alt=…]`/`[title=…]`, high-medium `[href^=…]`, high-high everything else), and "Specific hostname / entity".
- **Lowly generic** rules sit in `lowlyGeneric: Map<hash, "sel,\nsel">`, keyed by a djb2-style `hashFromStr(type, s)` of the class or ID, the hash mirrored in the content script. The content script's surveyor hashes each new class and ID, skips hashes already in `queriedHashes`, and sends only new ones. `retrieveGenericSelectors` returns the matching buckets minus exceptions.
- **Highly generic** rules are split into `simple` (`/^(?:[a-z]*\[[^\]]+\]|\S+)$/`) and `complex` sets. Each is frozen once into a single joined string, `highlyGeneric.*.str = Array.from(dict).join(',\n')`. Per page, `retrieveSpecificSelectors` looks up a 16-entry `MRUCache` keyed by the page's exception set (`exceptionFilters.join()`). Only when an exception hits a generic selector does it copy the set and re-join. Otherwise the precomputed string is reused as is ("A most-recent-used cache is used to skip computing the resulting set of high generics for a given set of exceptions").
- **Per-host cache**: `selectorCache: Map<hostname, SelectorCacheEntry>` holds the lowly generic selectors the surveyor found on that host (`addCosmetic`) and a capped set of network-derived selectors (`net`, kept at 20–30 by recency). The next page on the host gets them with its specific rules before the surveyor runs. The cache prunes to the 40 most recently used hosts once it exceeds 50 (`selectorCacheCountMin = 40`, `selectorCacheCountMax = 50`, prune delay 10 minutes).
- Matching itself is again the browser's CSS engine: uBlock Origin injects CSS and runs only procedural filters in JS.

## 4. How browsers match many selectors per element

- **Stylo** (`servo/stylo` `7f0189de128e6dbd2b504acd3aebee3befa78358`, `style/selector_map.rs`): `SelectorMap` has `id_hash`, `class_hash`, `local_name_hash`, `attribute_hash`, `namespace_hash`, `rare_pseudo_classes`, `root` and `other`. Each rule is filed under its rightmost compound's most specific key, in the order root > ID > class > attribute > local name > namespace > universal. "The union of the rules keyed on each of element's classes, ID, element name, etc. will contain the Selectors that actually match that element."
- **Blink** (`chromium/chromium` `dfd92739409ec4c23ed7c7fedcc36aa709038ad1`, `third_party/blink/renderer/core/css/rule_set.h`): `IdRules`, `ClassRules`, `AttrRules` and `TagRules` buckets, compacted into contiguous extents (`RuleMap`).

This is the per-element index Foxhound lacks. adblock-rust and uBlock Origin don't need one because they hand their selectors to these engines.

## 5. lol_html 3.0.2: chunked rewriting

Source: crates.io `lol_html` 3.0.2, the workspace pin.

- **Chunks:** `HtmlRewriter::write(&[u8])` parses what it can and buffers only the unconsumed tail, a token cut by the chunk boundary (`transform_stream/mod.rs` `write`). `end()` flushes.
- **Memory bounds** (`rewriter/settings.rs` `MemorySettings`): `preallocated_parsing_buffer_size` defaults to 1,024 bytes. `max_allowed_memory_usage` defaults to `usize::MAX`, with errors from `write`/`end` past it, and the measure is "the best approximation by measuring the memory consumed by internal buffers that grow depending on the input". `graceful_bail_out_on_memory_limit_exceeded` (default false) flushes the unprocessed input as is. The selector VM keeps a stack of open elements, so memory grows with nesting depth and with text a handler asks to buffer, not with document size.
- **Selectors** (`selectors_vm/parser.rs`): `*`, type, class, ID, attribute operators, `:nth-child`, `:first-child`, `:nth-of-type`, `:first-of-type`, `:not(compound)`, and descendant and child combinators. It rejects `+` and `~` (`UnsupportedCombinator`) and any non-tree-structural pseudo-class (`NonTSPseudoClassStub` is empty), which includes `:has()`. By the lexical census above, about 1,390 of the 20,493 host-specific rules in EasyList and Fanboy carry a procedural operator or `:has`, and about 37 a sibling combinator.
- **Decisions at the start tag:** a handler can `remove()` an element when its start tag matches. It cannot decide from content that comes later. `:has-text`, `:min-text-length` and `:upward`, which ADR 0047 evaluates for extracted content, are out of reach.
- **Strict mode** (`with_strict`, default true) bails out on markup whose parsing context needs tree backtracking: lol_html simulates the tree builder and builds no tree.
- **Charset:** `with_encoding(AsciiCompatibleEncoding)` refuses UTF-16BE/LE, ISO-2022-JP and `replacement`. `with_adjust_charset_on_meta_tag(true)` switches encoding at any `<meta charset>` or `http-equiv` it meets (`rewriter/mod.rs` `handler_adjust_charset_on_meta_tag`). That has no 1,024-byte/head-only limit and no UTF-16→UTF-8 mapping, so it is not ADR 0119's Blink prescan.
- **Spider 2.53.9** (`spider-2.53.9/src/page.rs` around 4890–5010): takes the header charset if it names an ASCII-compatible encoding, else UTF-8 with `adjust_charset_on_meta_tag = true`. It feeds each chunk to a `lol_html::send::HtmlRewriter` with a no-op sink to read links, and still collects the whole body into `collected_bytes` for the page content. Spider's streaming saves a second pass for links. It doesn't stream clean-up or readability, and its decoding is not ADR 0119's.
- **Interaction with a later tree:** the rewriter's output is bytes. A tree built after it parses those bytes again. Foxhound's existing lol_html reader (`goodfetch-extractor/src/references.rs`) runs on the already-decoded `&str` (UTF-8), which avoids the charset issue entirely.

## 6. Building the tree while the body arrives

- html5ever 0.36.1 `driver.rs`: `impl TendrilSink for Parser<Sink>`, where `process(t)` pushes a chunk and feeds the tokenizer. Its comment reads "FIXME: Properly support </script> somehow", which is harmless with scripting off. dom_query 0.28 implements `TreeSink for Document` and builds documents with `parse_document(Document::default(), opts).one(html)`, so `.process(chunk)` per chunk then `.finish()` is the same parser fed in pieces.
- ADR 0119's order (`goodfetch-fetch-tier/src/charset.rs` `read`) against arrival:
  1. A BOM is known from the first bytes.
  2. `fetch.encoding` is known before the fetch.
  3. The `Content-Type` charset is known from the head.
  4. The `<meta>` prescan ends at the first non-head tag after 1,024 bytes, and its output is the same on a prefix as on the whole body.
  5. "UTF-8 when the whole body is valid UTF-8" can only be confirmed at the end. A non-browser tier escalates otherwise. Decoding optimistically as UTF-8 with `encoding_rs`'s streaming `Decoder` and dropping the partial tree on the first invalid byte gives the same outcome, escalation, without decoding twice.
- **Costs, unverified by measurement:**
  - A `dom_query::Document` is `Send` only with its `atomic` feature (ADR 0138's amendment D3), which is needed to move a half-built parser between the receive task and the CPU pool.
  - ADR 0118's cut body and ADR 0067's limits must drop the partial tree.
  - The gain is overlap of parse with network time on large or slow bodies. It doesn't reduce instructions or peak memory, since readability needs the whole tree.
  - On the benchmark's 462-byte pages it is nil.

## 7. Readability cost per page

- **dom_smoothie 0.18.2** (workspace pin), `src/grab.rs` `grab_article`: `loop { let doc = self.doc.clone(); attempt_grab_article(...) ... }`. It starts with `StripUnlikelys | WeightClasses | CleanConditionally` and drops one flag per failed attempt. That is up to 4 full `Document` clones while the text is under `char_threshold` (default 500, `config.rs`), keeping the best attempt. `parse_with_policy(ParsePolicy)` does exactly one attempt (`readability.rs` 449–455), and its doc says it "uses less memory because it does not need to keep the best attempt".
  - Its matchers are process-wide statics (`once_cell::Lazy`). With the optional `aho-corasick` feature, class and ID scoring uses Aho-Corasick automata (`ac_automat.rs`). Without it, a byte pre-check skips patterns (`helpers.rs` `BytePatternCheck`).
  - The workspace builds it with `default-features = false`, so without `aho-corasick`.
  - It depends on dom_query 0.28, the workspace's version, so `Readability::with_document` can take Foxhound's tree with no version split.
- **Foxhound today** (`goodfetch-extractor/src/extractor.rs` 62–67) calls `Readability::new(html, Some(url), None)`, a parse from a string, and on a URL error parses again with `None`. Then it calls `parse()`, the multi-attempt loop.
- **Readability.js 0.6.0** (`mozilla/readability`, tag `0.6.0`): `var pageCacheHtml = page.innerHTML;`, then per failed attempt `page.innerHTML = pageCacheHtml;` (lines 1043, 1549), which re-parses the body. The threshold is `DEFAULT_CHAR_THRESHOLD: 500`, and `maxElemsToParse` defaults to 0 (no cap).
- **trafilatura v2.3.1** (`adbar/trafilatura`, tag `v2.3.1`, `trafilatura/core.py`, `external.py`): it runs its own extraction on `copy(tree)`. Unless `fast` (formerly `no_fallback`) is set, `compare_extraction` also runs a readability-lxml fork on the raw tree, and `justext_rescue` runs jusText. Each works on another `copy`. Its XPath expressions are compiled once at import (`xpaths.py`, module-level `XPath(...)`).
- **Pattern:** every readability family pays for retries and fallbacks with full-tree copies or re-parses. Each offers a single-pass mode: dom_smoothie `parse_with_policy`, trafilatura `fast`.

## 8. Extractors that build rules once per host

- **uBlock Origin**'s `selectorCache` (above) is the direct precedent: a bounded per-hostname cache of what earlier pages on that host needed.
- **Postlight Parser** (formerly Mercury, `postlight/parser` `e8ba7ece291efa4d915d50dd4deeec17d54359f2`, last commit 2023-03-15, so the project is inactive): `src/extractors/get-extractor.js` picks an extractor by `hostname`, then by base domain (last two labels), then by HTML detection, then falls back to the generic one. There are 145 entries under `src/extractors/custom/`. These are static per-host rules chosen by one map lookup, not learned.
- **adblock-rust** itself is stateless per page by design and leaves the per-page and per-host caching to the caller (its doc comment above).
- **Foxhound's own** learned rules (`frontier.learning`, ADR 0138) are the in-house case of per-host work and are not reviewed here.
- **Not found:** an open-source main-content extractor that compiles its rule set once per host as a cache. The per-host precedent is in ad blocking.

## 9. What Foxhound could adopt

Each item is checked against ADRs 0047, 0119 and 0138. Expected gains are reasoned from the counts above, **not measured**.

1. **A per-element rule index, built once per Run.** It removes most of the 2.51 M in `match_element`.
   - **Build:** when a selector is first compiled into `Selectors`, also derive its rightmost compound's key: ID, then a class, then an attribute name, then the tag, else universal, as Stylo orders them.
   - **Match:** walk the tree once. For each element, gather the rules in the buckets of its ID, its classes, each attribute name it holds and its tag, plus universal, and test only those. Take the earliest candidate in the order `css` holds today (selector order), so each element is credited to the same rule as now. Reuse one `SelectorCaches` across the walk (`match_element_with_caches`).
   - **Key derivation:** `Matcher` hides its selector list, so take the key from the selector's text with a small parser that falls back to universal when unsure. A wrong key would miss a match, so the table test is that the index and the linear scan remove the same elements on the corpus.
   - **ADRs:** keeps 0047 (same rules, same removals, same Provenance credit) and 0138 (in place on the one tree, no new parse).
2. **Host-invariant rule lists, cached per host, bounded.** This removes most of `PageRules::of` + `css_rules` (1.10 M).
   - **The generic part:** the misc generic set minus exceptions is the same for every host whose exceptions touch none of it. Precompute it per Run and per list combination, compiled and indexed. Copy it only for a host whose unhide rules hit a generic selector, as uBlock Origin's MRU does.
   - **The host part:** `hide_selectors`, the procedural rules (JSON parsed once) and `exceptions` depend on the hostname and the `generichide` bit. Cache them in an LRU keyed by `(hostname, generichide)`, with uBlock Origin's 40–50 hosts as a starting bound.
   - **The class/ID part:** `hidden_class_id_selectors` stays per page, but its results are cheap once compiled matchers are cached by selector text, as today.
   - **Exactness caveat:** `generichide` comes from a network match on the full URL, and adblock-rust 0.13.3 does not expose `check_generic_hide` on `Engine`. In the embedded lists at `published_at` 2026-10-22, every `$ghide`/`$generichide` exception is host-level (`@@||host^$ghide`, `@@||host.*$ghide`, `@@*$ghide,domain=…`; lexical check). A per-host key is exact for them. With `filter_lists.extra` a user list could add a path-scoped one. Either check the lists at Run start and fall back to per-URL resources when one exists, or keep calling `url_cosmetic_resources` per page and cache only the downstream compiled index by a digest of its result.
   - **ADRs:** keeps 0047, since the Run still holds to one rules version. It follows 0138's "Work that depends only on a Run's inputs is built when the Run starts". A per-host cache is a new bounded structure on a Shared path (ADR 0140), so it needs a register row.
3. **Optional, after 1: collapse the about 360 `a[href^="…"]` rules into one prefix automaton over `href`.** That is uBlock Origin's "high-medium generic" family made into a matcher. With the index in place, only `<a>` elements reach those rules. Worth it only if a profile still shows them.
4. **Readability:**
   - Use `Readability::with_document` on the cleaned tree, which ADR 0138 already decides and the code doesn't do yet.
   - Expose dom_smoothie's single-attempt `ParsePolicy` as a setting, keeping today's multi-attempt default. That follows 0138's rule that a costly feature is a setting, and changes output only when chosen.
   - The `aho-corasick` feature is a dependency speed-up, not a capability. Its gain is unmeasured.
5. **Streaming the tree build (section 6):** compatible with 0119 and 0138 as described. Low priority, because it cuts latency, not the per-page instructions this ticket measures.
6. **Don't adopt:**
   - **lol_html for clean-up:** it can't run 0047's procedural rules or `:has`/sibling rules, can't record removed text and links the way `Removed` does, and would add a second tokenization before the tree (0138).
   - **lol_html's `adjust_charset_on_meta_tag`:** it is not 0119's prescan. Decode first and feed UTF-8, as `references.rs` already does.
   - **Brave's 60-selector batching:** with an index it adds nothing.

## Unverified

- The speed-ups in section 9 are reasoned from call counts. No build or profile was run.
- The list census and the count of rules lol_html would reject are lexical greps, not adblock-rust's or lol_html's parsers.
- Whether Brave's browser process caches `UrlCosmeticResources` per host was not checked.
- Feeding html5ever chunk by chunk into a `dom_query::Document` across the receive task and the CPU pool (the `atomic` feature) was not tried.
- Exa search tools were not available in this session. Sources were read directly from the crates.io registry sources the workspace pins and from GitHub at the commits named. No paid service was used.
