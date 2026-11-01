# Shared paths sweep of the open tickets (2026-11-01)

The one sweep of Goodfetch's open tickets against ADR 0140's fourteen Shared paths, as the `system-qualities` skill's `/to-tickets` row describes. The sources are ADR 0140 and the Quality register as on branch `claude/docs-quality-register-60d7a4` (`4774cc97b`), and ADR 0138 as on `docs/per-page-cost-structure`. The code was read at `85083c57b`, the tip of `milestone/m1-core`. **Status: applied.** The sweep's classes were approved by the owner and applied on 2026-11-01. D1–D14 were adopted the same day, and the tickets they decide were then updated: see "Decisions adopted".

## Scope and method

- **Swept:** the 1,107 open issues that are neither a spec (a `Spec:` title, #1577, #1749) nor among the 219 issues #1583's `Closes` lines name. The 73 issues merged in part on `milestone/m1-core` (`Part of` in #1583) stay in the sweep for their remaining work.
- **The path check:** for each ticket, does its work add counted work or a per-unit call to a Shared path, add a second entry point for a path's unit, or change a path's stage order? Eight subagents each read about 138 tickets and checked the code each non-untouched ticket names (`rg` and Read: rust-analyzer does not index this worktree, and a merge gate was running). I checked a sample myself: the code behind every finding about ADR 0140 and the register below, three rewrites (#225, #760, #762), and the 20 untouched tickets whose text names per-unit work most often. That sample moved #791 and #1486 to criteria.
- **The classes:** **untouched** (no Shared path); **criteria** (extends a path inside its owner module: its register rows and benchmark join its acceptance criteria, and the gate's benchmark ticket blocks it); **rewrite** (its design goes around an owner module); **ready-for-human** (needs an owner decision about a Cost contract or an owner module).

| Class | Tickets |
|---|---|
| untouched | 874 |
| criteria | 189 |
| rewrite | 9 |
| ready-for-human | 35 |

## Findings about ADR 0140 and the register itself

These are not tickets. They go to the owner of ADR 0140's branch, or to the gate's spec, before the register's first measure.

1. **`C5.fetch_messages_no_rule` has no Exception, yet fails today.** `pause_requests` (`crates/goodfetch-page/src/interception.rs:1084`, called at `:576`) enables `Fetch` with the `*` pattern for every page, whatever the rules. ADR 0140 says no row is set at a value the current code fails on its first gate. Decision D2 below.
2. **C2's context id cannot come from Chromium's context events as written.** Under ADR 0019 the page driver never calls `Runtime.enable` (`crates/goodfetch-page/src/driver/world.rs:3`), so `executionContextCreated` never arrives. The Shared paths spec has to say how the driver learns a new document's context id.
3. **X1 misses Foxhound parses outside the extractor.** The gate's use signals (`goodfetch-fetch-tier/src/gate.rs:389` into `use_signals.rs:154`, only when the document names `tdm-reservation` or RSL's type), each redirect hop's refresh stub (`refresh.rs:62`), and a document tier's served calls (`tiers.rs:2163`, `tiers/calls.rs:416`) each run a whole parse of their own.
4. **X6 misses two sites.** Each redirect hop reads the Scope inside a writer transaction (`goodfetch-fetch-tier/src/gate.rs:790-791`), and each sitemap read reads the Scope again (`goodfetch-crawl/src/work_items.rs:289`) and compiles its rules again (`goodfetch-frontier/src/sitemap.rs:551-562`).
5. **X13's citation is stale.** The spawn per refused request is now at `crates/goodfetch-page/src/interception/pipeline.rs:1731`, not `:1653`.
6. **Cost-contract counts with no register row,** which the register check will fail on ("a count in a Cost contract has no row"): F3's 0 transactions of its own; F4's 1 validation and 1 serialization per Record; C1's 1 encode and 1 send per command; C4's 1 Provenance entry per navigation; C5's 0 compilations and 1 CDP answer per paused request. Tickets also found counts that no contract names yet: Hook script compilations (#958), locale-form builds per page (#844), `cpu.stat` opens per governor sample (#1306), the Effect journal's sync per Effect (#1550, #779), prune passes per attempt (#694).

## Decisions adopted

The orchestrating session adopted D1–D14 on 2026-11-01 under the owner's words « fait tout ce que tu peux faire suel » (logged in `.claude/handoff/m1/decisions.md`). They stay reversible and open to the owner's review. The 35 tickets left `ready-for-human`: 34 moved to `ready-for-agent` and #1683 went back to `needs-triage`, its label before the sweep. Each gained a criterion and a "Shared paths decision" section. ADR 0140's own amendment belongs to the `qualities-setup` session, so a row it adds is named by what it counts, not by an ID.

Where what was applied differs from the text below:

- **D13 (#1783):** the adopted decision is a second GET sent only on the charset path, as the decisions log states. The recommendation below says the Chromium patch.
- **D8 (#684, #1649):** the mark's setting is off by default, as D8's recommendation says. The tickets' own lines below say on by default for compliance.
- **D1:** the path's cap is D1's, at most 2 transactions per attempt. The lines on #709 and #883 say at most 1.

**Waiting on #1577.** #1097, #1473, #1474 and #1514 are natively blocked by it. #767, #788, #715, #760, #762 and #994 are among #1577's own blockers, directly or through other tickets, so they can't wait on it: each lands with today's parse under Exception X1, and #1577 moves that work onto the Page analysis. The first version of the rewrites of #715, #760 and #762 and the criterion of #994 said otherwise, and those tickets were corrected.

## Sweep of #1805–#1822

These tickets opened after the first sweep's snapshot. The sweep used the same path check, from a reading of each ticket; the owner modules are the ones the first sweep checked in the code.

| Ticket | Class | Paths | Rows | Benchmark | Criterion or reason |
|---|---|---|---|---|---|
| #1805 | criteria | F1 | `F1.parses`, `F1.selector_compilations` | B1 | An item-scoped `extract` is a Page-analysis reader with its locators and filters compiled once, so it adds no parse and no selector compilation per page. Its readers wait on #1577. |
| #1806 | untouched | - | - | - | One choice over the matches the field read already makes. |
| #1807 | criteria | F1 | `F1.parses`, `F1.selector_compilations` | B1 | A structured-data field source is a Page-analysis reader over the one parse, and it runs only when a field declares it. Its readers wait on #1577. |
| #1808 | criteria | F3 | `F3.invariant_reads`, `F3.compilations` | B1 | The routing table is a Run invariant compiled when the Run starts, with no read or compilation per links batch. |
| #1809 | criteria | F3, F4 | `F3.payload_serializations`, `F3.validations`, `F4.deep_copies` | B1 | A carried field costs 1 validation and 1 serialization per URL through the Payload, and merging it into a Record makes no deep copy. |
| #1810 | criteria | F3, F2 | `F3.invariant_reads`, `F2.transactions` | B1 | The cap's count commits with the attempt, with no transaction of its own and no read of the rule per links batch. |
| #1811 | criteria | C2, C3 | `C2.commands`, `C2.world_creations`, `C3.script_sends`, `C3.commands` | B2 | Each scroll or click step is one page read and one guarded attempt, and a read that checks for new items creates no world. |
| #1812 | criteria | F1 | `F1.parses`, `F1.selector_compilations` | B1 | Previewing a Label runs through the Page analysis's readers, with no parse or page loop of its own. Its readers wait on #1577. |
| #1813 | criteria | C2 | `C2.commands`, `C2.world_creations` | B2 | A condition on a page fact is one page read through the driver, and a step with no condition costs none. |
| #1814 | untouched | - | - | - | Retries repeat existing attempts; the policy adds no counted work. |
| #1815 | criteria | C3 | `C3.script_sends`, `C3.commands` | B2 | An overlay rule is checked through the guard's kept handle, and with no rule it costs no read or command. |
| #1816 | untouched | - | - | - | A loop over a Run's input rows, not a Shared path's unit. |
| #1817 | criteria | F1, C5 | `F1.parses`, `F1.selector_compilations`, `C5.spawns`, `C5.file_opens` | B1, B2 | Embedded JSON is a Page-analysis reader, kept responses go through #778's observer with no spawn or file open per response, and each runs only when a field declares it. Its Page-analysis reader waits on #1577. |
| #1818 | untouched | - | - | - | Data forms of existing actions and waits, through their owners. |
| #1819 | untouched | - | - | - | Rules are checked once, at the Run's end. |
| #1820 | criteria | F2, F4 | `F2.transactions`, `F4.transactions` | B1 | A file output is staged with the attempt and commits in its outcome, and a Label declaring none costs nothing. Files written per declared output are a count no row covers yet. |
| #1821 | untouched | - | - | - | `daemon install` port handling; no per-unit work. |
| #1822 | untouched | - | - | - | Gate tooling and disk headroom; no runtime work. |

## Tickets opened after #1822, and #1683's triage

Checked on 2026-11-01. The children of #1836, #1837, #1838 and #1577 carry their path criteria from their own specs, so they aren't swept here.

- **#1900:** criteria on F1 and E3 (benchmark B1), read from its text, and blocked by #1846.
- **The Kennel map (#1823, #1827–#1835, #1915):** untouched. It is a separate local tool, and #1832 already asks the map's path question. #1824–#1826 are closed.
- **Rows named:** 32 tickets now name the rows of ADR 0140's amendment (`421b780b8` on its branch) by ID. A note in a criterion that a count has no row still stands where the register has none: that ticket's pull request adds the row, a tightening.
- **Gate blocks:** all 245 tickets on the list have a native block from a gate ticket under #1837. The tickets are linked by path. 39 of those links appeared from outside this session while this pass ran, and this pass added the ones for #1900 and #1695's C5.

**#1683 moved to `ready-for-agent`,** with behavioural acceptance criteria and blocked by #959, #721, #726 and #1863. These reversible choices are recorded on the ticket:

- Role and name come from the engine's own role engine (#721), not the `Accessibility` domain, so a pick's ladder can't drift from what replay checks.
- Firefox and WebKit are refused for now, as the ticket's "Chromium only is acceptable to start" allows.
- The right to read is the right to watch the Session's Live view.

## Owner decisions (ready-for-human)

35 tickets, grouped into 14 decisions. Each ticket gains the `ready-for-human` label and a comment stating its decision, and keeps it until the owner answers; the answer then makes it criteria or rewrite.

### D1. A Politeness permit and robots.txt match per request: a new Shared path owned by `politeness`?

Tickets: #239 (Politeness shared on one machine), #240 (Politeness store across machines), #709 (Proxy and cloud-browser limits shared between runs: a vendor's refusals set the ceiling), #883 (Every crawl request obeys robots.txt and the AI-use signals), #897 (Pacing: an adaptive delay and a per-domain rate), #1621 (Pacing on the browser tier and for redirect hops)

**Recommendation:** Yes. Amend ADR 0140 with a Politeness path: per attempt at most 2 transactions on the Politeness file (or 2 round trips to Valkey/Redis with scripts loaded once and called by `EVALSHA`), 0 opens, 0 statement preparations, robots.txt parsed once per origin per Run, and nothing when Politeness is off; measured on B1.

- #239: Decide which Shared path owns the Politeness permit, which costs every attempt two SQLite transactions on the Politeness file (`politeness.rs:502`, `:576`, statements prepared per call through `execute`), a per-attempt route ADR 0140 lists no contract or row for; recommend amending ADR 0140 to give it its own rows under `goodfetch-engine`'s `politeness` (at most 2 transactions, 0 syncs, 0 opens and 0 statement preparations per attempt, and nothing when Politeness is off, as ADR 0138 requires), measured on B1.
- #240: Same decision as #239 for the shared store: which Shared path owns a Politeness permit taken from Valkey or Redis per attempt; recommend the same Politeness rows with at most 2 store round trips per attempt, its Lua scripts loaded once per connection and called by `EVALSHA` (0 script sends per permit), and nothing when Politeness is off.
- #709: Decide who owns the Politeness permit taken per proxied navigation hop (a SQLite transaction and statement preparation in the per-user Politeness file, per unit, outside all fourteen paths): recommend a fifteenth Shared path, 'Politeness permit', owned by `goodfetch-engine`'s `politeness`, contract 0 opens, 0 statement preparations, at most 1 transaction per permit and nothing when no proxy limit or delay is set, benchmarked on B1 and B2.
- #883: Decide whether the per-request Politeness gate (robots.txt group matching, use signals and Permits, goodfetch-fetch-tier's gate over goodfetch-engine's politeness file) becomes a Shared path: ADR 0140 has none, and today each request reads the robots.txt copy in a Politeness-file transaction and parses it again (goodfetch-frontier robots_txt.rs SharedRobotsTxt::fetched). Recommend adding it, owned by the gate, with robots.txt parsed once per origin per Run into a bounded cache, 0 Politeness-file reads per request for robots.txt, at most 1 transaction per Permit taken and given back, and nothing consulted when Politeness and robots.txt are off, measured on B1.
- #897: Decide the same Politeness-gate Shared path as #883 before the token bucket lands: ADR 0140 owns no per-request Politeness Permit, and the rate adds GCRA state to each Permit's transaction in the Politeness file (goodfetch-engine politeness.rs take). Recommend the Permit's path with the bucket updated inside the Permit's one transaction, no extra transaction per request, and no Politeness work when delay is 0 with no rate or limit.
- #1621: Decide the Politeness-gate Shared path of #883 and #897 for browser-tier navigations and redirect hops too: each hop taking a rate token and recording latency is per-request Politeness work ADR 0140 does not own. Recommend one gate path for every tier and hop, the token and latency update inside the Permit's one transaction.

### D2. C5 says "with no rule: 0 `Fetch` messages", but every page pauses every request today for the egress check, the domain policy and the egress relay's 407

Tickets: #632 (Every kind of browser request judged: workers, WebSockets, WebRTC and service workers), #634 (The egress relay: every engine-launched browser Session reaches the network through it), #702 (One interception pipeline for every request of a page, and routes that fall through)

**Recommendation:** Amend C5 so the always-on egress check counts as a rule (the row becomes a ratchet), record an Exception now, and answer the relay's 407 on `Fetch.authRequired` with no request patterns where possible. Today `pause_requests` (`goodfetch-page/src/interception.rs:1084`) enables `Fetch` with `*` for every page, so `C5.fetch_messages_no_rule` fails on its first gate, against ADR 0140's own rule.

- #632: Decision: does ADR 0014's egress and domain-policy step count as a C5 rule, since every page pauses every request today (`goodfetch-page/src/interception.rs:576`) and no Exception covers `C5.fetch_messages_no_rule`? Recommend amending C5 so the egress step is a rule on Chromium (Fetch stays on, as ADR 0014 requires), the row measuring only a Session whose egress the relay enforces, with an Exception until then.
- #634: Decide whether the egress relay, on by default, may keep the Fetch domain pausing every request to answer its 407 at interception (today `interception.rs:576` and `:1084` pause every request whenever a relay answers authentication), which leaves `C5.fetch_messages_no_rule` unreachable by default; recommend no: with no route and no Filter list, answer the relay's credentials on `Fetch.authRequired` with no request patterns and let the relay judge addresses, so the row stays 0 on B2.
- #702: Decide whether C5's 'with no rule: 0 Fetch messages' (no Exception today) can hold when the interception pauses every request for the always-on egress check, domain policy and the egress relay's `Fetch.authRequired` credentials: recommend amending ADR 0140 so `C5.fetch_messages_no_rule` counts Fetch messages per request with no route, Filter list, placeholder or gate as a ratchet (today a pause and an answer per request, and a second pair at a document's response), adding an Exception now, since those checks are ADR 0017 security guarantees that cannot be switched off.

### D3. Does a document tier keep the Page analysis's one tree for a handler's served calls?

Tickets: #767 (One handler on every tier: serve what a tier serves exactly, escalate or refuse the rest), #1097 (AI extraction in a handler on the fetched document, no browser), #1473 (CrawlContext on the document tiers), #1474 (CrawlContext: Locators on the document tiers and the browser alike)

**Recommendation:** Yes: these calls read the tree kept for the attempt, scheduled by the demand plan from the declared use; amend ADR 0138, which ends the tree with the Page analysis. Wait on #1577.

- #767: Decision: does a document tier keep the page's one Page-analysis tree for its handler's served calls, or does ADR 0138 end the tree with the analysis so each served call parses again (today `tiers/calls.rs:415-416`, `tiers/calls/markup.rs:48`, `tiers.rs:2163`)? Recommend keeping one tree owned by the page's task for the handler's life, selectors from the engine's cache, 0 parses per served call; amend ADR 0138.
- #1097: Decide whether a handler's page reads on a document tier (here `document_observation`'s `ai` snapshot after cosmetic removal) are readers the demand plan schedules inside the Page analysis or may read a tree it keeps for the attempt, since ADR 0138 ends the tree with the analysis and F1's crate is open question 1; recommend readers of the Page analysis, scheduled from the Label's declared AI use, the snapshot kept in its immutable result.
- #1473: Decision: do a handler's calls on a document tier (`content`, `formats`, Locator reads, questions and waits) count in F1's per-page contract, or are they a new per-unit route? Today each served call parses the page again (`goodfetch-fetch-tier/src/tiers/calls.rs:415-416`, `tiers/calls/markup.rs:48`) and `formats` reruns the extractor (`goodfetch-page/src/pages/clean_up.rs:150`). Recommend: count them in F1; the Page analysis keeps the attempt's tree with its one owner on the CPU pool and answers served calls from it, selectors from the engine's compile cache, so F1.parses stays at most 1 and F1.selector_compilations 0 per page; B1's fixture gains a handler on a document tier; block on #1577, which places F1's crate (open question 1).
- #1474: Same decision as #1473 for Locator calls (`getByRole`, `locator`, their reads) on a document tier: recommend they read the Page analysis's tree kept for the attempt, with no parse or selector compilation per call (today `tiers/calls.rs:415-416` compiles a `Matcher` and parses the document per call), and wait on #1577.

### D4. Extractors that take a string: a second parse per page

Tickets: #788 (The `content.extractor` setting chooses dom-smoothie or trafilatura), #814 (An installed Extractor Plugin chooses the main content)

**Recommendation:** No Exception for the built-ins: both read the Page analysis's tree (dom-smoothie through `Readability::with_document`, a document entry patched into the trafilatura fork, #1634). An Extractor Plugin gets a counted allowance of 1 serialization and 1 parse, only when chosen.

- #788: Decision: F1 allows at most 1 HTML parse per page, but the Extractor interface takes the page as a string (`goodfetch-extractor/src/extractor.rs:17`) and rs-trafilatura 0.2.2 has no entry for a parsed tree, so choosing `trafilatura` costs a second parse; may a chosen built-in Extractor exceed `F1.parses`? Recommended: no; the Extractor takes the Page analysis's cleaned tree, dom-smoothie through `Readability::with_document` and rs-trafilatura through a document entry patched into the fork (#1634, it already builds on dom_query), so either built-in keeps 1 parse.
- #814: Decision: an Extractor Plugin receives the serialized document and answers HTML that the formats parse again, a second engine parse per page above F1's cap; does a chosen Extractor Plugin get its own counted allowance (1 serialization and 1 parse of its answer per page, 0 when none is chosen) in an amendment of ADR 0140's F1? Recommended: yes, as rows `F1.plugin_serializations` and `F1.plugin_answer_parses` at 1 each, since a process boundary cannot share the tree and the default keeps 1 parse.

### D5. ADR 0140 open question 3: Consent prompt detection in `recovery`'s one composed read

Tickets: #717 (Consent prompts refused in every frame, by default), #1003 (Decision providers between the Target fingerprint check and the model, and Consent prompts they answer)

**Recommendation:** Run the Consent detection inside `recovery`'s one composed read, keeping C4's cap at 1; only its clicks follow as guarded actions.

- #717: Answer ADR 0140 open question 3 for the Consent prompt runner: recommend that its detection run inside `recovery`'s one composed read per navigation and only its clicks and snippets follow as guarded actions (C3), keeping C4's cap at 1; approve a cap of 2 only if B2's first measure shows the runner's per-document start cannot join that read.
- #1003: Decision (ADR 0140 open question 3): does the Decision provider answering an unrecognised Consent prompt take its candidates from `recovery`'s one composed detection read, keeping C4's cap at 1? Recommend yes: the consent detector's function returns the candidate list in the same call as the Block markers and the Page check, and the provider's call adds no read per navigation.

### D6. ADR 0140 open question 2: C3's cap for human-like input

Tickets: #1018 (Human-like input: mouse paths, scrolling and press-and-hold as native events inside the guard's deadlines), #1022 (Human-like typing: slips are corrected before the guard re-checks that the field holds the requested value)

**Recommendation:** Human-like input is its own action kind, with a bound on events per Gesture or per character set from B2's first measure.

- #1018: Decide C3's cap per action kind for human-like input (ADR 0140 open question 2): a mouse path, scroll or press-and-hold sends many Input events per Gesture. Recommend a fixed bound on events per Gesture set by the model, measured as its own C3.commands figure per action kind in a B2 stealth cell, with stealth.human_input off leaving the standard figures unchanged.
- #1022: Decide C3's cap per action kind for human-like `fill` and `pressSequentially` (ADR 0140 open question 2), which send Input commands per keystroke and per corrected slip; recommend counting them as their own action kind with a cap per character typed, set from B2's first measure, so plain `fill`'s cap is not raised.

### D7. Hook stages: a new per-unit route with no owner

Tickets: #1481 (Hooks in code: declared on the Crawl, scoped by Label and named), #1482 (Hook returns and ending calls)

**Recommendation:** Amend ADR 0140 with a Hook-stage path, its chain built once per Run; a Record a Hook changed may be validated once more; add the missing `F4.validations` and `F4.serializations` rows.

- #1481: Decision: Hook stages (beforeRequest, afterResponse, navigation, pageReady, beforeRecordCommit) are a new per-unit route (one stage of one Crawl request) that ADR 0140 does not own; no Hook runner exists yet (`goodfetch-protocol/generated/foxhound_names.rs` HOOK_STAGES holds only `sample_crawled_page`; `goodfetch-page/src/interception.rs:10` assumes Hooks already applied). Recommend amending ADR 0140 with a Hook-stage Shared path: one Hook chain in `goodfetch-crawl`, built when the Run starts per stage and Label from settings-named Hooks, Script Hooks and Hooks in code, each stage called by its path's owner; contract per stage per request: 0 messages and 0 formatting with no Hook on that stage and Label, 1 request out and 1 typed decode per Hook called, 0 chain builds and 0 compilations; benchmark B1 with a Hook in its fixture.
- #1482: Decision: a Record a beforeRecordCommit Hook changes is re-checked against its schema, a second validation after `push`'s, while F4 caps validations at 1 per Record. Recommend amending F4 to at most 1 validation plus 1 per Record a Hook changed, counted per outcome (Records changed by a Hook), keeping 0 deep copies (the Hook's answer replaces the staged value), and adding the missing `F4.validations` and `F4.serializations` rows; depends on #1481's Hook-stage path.

### D8. The robots.txt "ignored" mark needs robots.txt read while `respect_robots_txt` is off, against ADR 0138

Tickets: #684 (Politeness and Site preferences values that press harder, recorded and lockable, and Records taken against robots.txt marked), #1649 (Mark a browser-tier Record taken against robots.txt in its Provenance)

**Recommendation:** Make the mark its own setting, off by default, so `respect_robots_txt: false` fetches, reads and stores nothing.

- #684: Decision: the `robots_txt: "ignored"` Provenance mark needs robots.txt fetched and matched per page while `site_preferences.respect_robots_txt` is off, which ADR 0138 forbids ("robots.txt is never fetched, read or stored" when off; merged at `goodfetch-crawl/src/delivery/attempt.rs:1078-1083`, `attempt/site_preferences.rs:265`). Recommend making the mark its own setting (on by default for compliance): with it off, `respect_robots_txt: false` reads no robots.txt (ADR 0138's scenario 1); with it on, one shared copy per origin is read and each page costs one match; amend ADR 0138 and ADR 0086 accordingly.
- #1649: Same decision as #684: this ticket reads robots.txt on the browser tier while `respect_robots_txt` is off, against ADR 0138's "never fetched, read or stored". Recommend the separate marking setting described on #684, this ticket reading the shared copy only when that setting is on.

### D9. Response cache, Response recordings and WARC: per-response work C5 and E5 do not count

Tickets: #793 (The Response cache reuses a fresh response and revalidates a stale one), #1291 (Response recordings: record once, replay with no network, and a failed write that can be written again), #1031 (A crawl writes WARC 1.1 that a recording replays), #1037 (WARC output from the browser tier, resumed to its committed length)

**Recommendation:** Amend C5 with a cache-hit cell (1 lookup and 1 body-file open per hit when `cache.mode` is not `off`) and a record-mode cell (1 body read per recorded response); a WARC record is a fourth Run-trace entry kind under E5.

- #793: Decision: C5 allows 0 files opened per paused request, but ADR 0046's content-addressed body files make every Response cache hit open one, and the index lookup per request has no row; amend ADR 0140 so that, only when `cache.mode` is not `off`, a request costs 1 lookup on the cache store opened once per engine with `prepare_cached` and a hit 1 body-file open (0 per miss, 0 with the cache off), as new rows `C5.cache_lookups` and `C5.cache_file_opens`? Recommended: yes, since body files keep the index small and stream large bodies, and the default (`off`) stays free.
- #1291: Decide how record mode fits C5 and E5: it needs per request a response-stage pause, a body read and a durable write, above C5's one CDP answer per request, and nothing owns a per-response run-state write. Recommend a C5 contract cell for record mode (at most 2 answers and 1 body read per request, off unless the Run names a recording), each recorded response sent as one entry to E5's Run-trace writer with no transaction of its own, and replay answering each request with one Fetch.fulfillRequest from an index built when the Run starts.
- #1031: Decision: is a WARC record (one archived response, shared with a Response recording's record mode) a new Shared path or a fourth Run-trace entry kind under E5's `run_trace`? Recommend a fourth E5 kind: one per-Run writer with one buffered handle per WARC file, flushed at F2's group commit with the committed length written in the outcome, 0 opens, spawns or syncs of its own per record.
- #1037: Decision: with `warc.content: all`, the browser tier must read every subresource's body (a response-stage pause or `Network.getResponseBody` per response), which C5's contract does not count; add a C5 row of 1 body read per recorded response with WARC on and 0 off? Recommend yes, read through the existing response observer (`page_events.rs:959`) without its per-response spawn, never a second interception loop.

### D10. New routes no path owns: Sink delivery, plugin-protocol messages, Playwright-driven Firefox and WebKit

Tickets: #1121 (Sink tracer: a Dataset's Records delivered to SQLite with delivery ID and epoch), #810 (Plugin handshake: `hello` negotiates the protocol and the approved manifest), #739 (Firefox and WebKit through Playwright's driver, with the same guard)

**Recommendation:** Sink delivery becomes a new Shared path owned by `goodfetch-sink`'s delivery driver; a plugin-protocol message is a unit of E1 (framed by `rpc`); the Playwright driver's connection becomes a second transport behind C1's seam.

- #1121: Decision: is Sink delivery (one batch of committed Records to one Sink) a new Shared path owned by `goodfetch-sink`'s delivery driver, since every destination plugs into it and it reads Records while the Run writes? Recommend yes: amend ADR 0140 with 0 store opens and 0 statement preparations per batch, reads only on the run file's read-only connections, 1 transaction per delivered batch for its position, benchmarked on B1 with the SQLite Sink.
- #810: Decision: is a plugin-protocol message (one call to or answer from a Plugin, per page read, navigation or Hook stage) a unit of E1 or a new Shared path ADR 0140 lacks? Recommend E1: `plugin_host` frames and decodes through `goodfetch-engine`'s `rpc` framing (`rpc/framing.rs` `decode`, which already reads replies) with 1 typed decode and 1 encode per message, and ADR 0140's E1 row and B3 are amended to name plugin messages.
- #739: Decide the owner of Firefox and WebKit commands through Playwright's driver, which cross no `goodfetch-cdp` connection and so no C1 to C5 counter: recommend that the Playwright driver's connection be C1's second transport behind the same seam and counts, with C2, C3 and C5 kept in `goodfetch-page`'s driver, guard and interception whatever the browser, and B2 gaining a scripted Playwright-driver end.

### D11. Page reads that need more than C2's 1 command

Tickets: #726 (Frames and closed shadow roots on Chromium), #1683 (Pick, hover and match reads on a Live view's Session, answered from the isolated world)

**Recommendation:** Separate outcomes with their own ratchet rows: a document holding a closed shadow root (cap 3), and person-driven Live view reads (hover, pick, match).

- #726: Decision: resolving closed shadow roots with `DOM.describeNode` (pierce) and `DOM.resolveNode` for each locator match, guard check and `ariaSnapshot` adds at least 2 CDP commands to a page read whose C2 contract is exactly 1 `Runtime.callFunctionOn`. Recommend amending C2 to count a read in a document holding a closed root as its own outcome, capped at 3 commands, keeping 1 for documents without one (the driver learning per document, in its context registry, whether closed roots exist), and requiring out-of-process frames' worlds to be registered once per frame as C2 already says.
- #1683: Decide whether Live view hover, pick and match reads are C2 page reads under its 1-command cap (the prototype's hover sends 3 DOM commands, a pick about 6, plus per-frame viewport metrics) or a new person-driven read kind; recommend amending ADR 0140's C2 with that kind and its own ratchet rows taken from B2, still 0 `Page.createIsolatedWorld` and the Session's registered world.

### D12. ADR 0140 open question 4: SDK framing per message

Tickets: #1558 (SDKs pass on the program's OpenTelemetry context in each request's `_meta`)

**Recommendation:** Let it land with the engine-side `_meta` decode counted in E1 and the SDK side under #1415's protocol count snapshots.

- #1558: Decide ADR 0140 open question 4 before SDKs add per-request work (an OpenTelemetry context read and a `_meta` write on every request): recommend letting this land with the engine-side `_meta` decode counted under `E1.deep_copies` and `E1.allocations` on B3 and the SDK cost covered by #1415's protocol count snapshots, making SDK framing a Shared path only once its CPU per message is measured.

### D13. Telling a cut document apart: a second request per page or a Chromium patch

Tickets: #1783 (Foxhound's browser tier reads a cut document whole when its document reader gives the browser the charset)

**Recommendation:** The Chromium patch: a second request adds a whole browser fetch per page.

- #1783: Decision: a chunked or coded document read through the charset path can be told cut only by a second request per page or a Chromium patch; a second request adds a whole browser-fetch (engine page, CDP commands, a charge) per page outside C5's and E3's counts. Recommend the Chromium patch, never a second request per page; the Content-Length check needs no decision.

### D14. `CrawlDeclaration` adds per-Label readers to every page (F1, open question 1)

Tickets: #1514 (`CrawlDeclaration`: a crawl as data, reviewed from a prompt and started)

**Recommendation:** Block it on #1577 and build its readers as Page-analysis readers.

- #1514: Decision: a `CrawlDeclaration` adds per-Label `follow` and `extract` readers to every page, extending F1, whose owner and crate #1577 places (open question 1). Recommend blocking #1514 on #1577 and building each Label's fields and follow rules as readers of the Page analysis, with follow patterns and validators compiled when the Run starts, rather than extending today's `goodfetch-crawl/src/code_free.rs` reader.

## Rewrites

Each section quotes the ticket's sentence, the owner module verified in the code, and the replacement paragraph to paste into the ticket's design. The gate's benchmark ticket also blocks every rewritten ticket.

### #1026 Run trace tracer: recorded on the CDP side, replayed on a Timeline

**Offending sentences** (Decisions it must honour):

> the `trace` module (ADR 0126) records frames every `trace.frame_interval` (1 s) and at each action, reusing the Live view's screencast when someone watches; `DOMSnapshot.captureSnapshot` at each action and Effect; the network log as HAR; console messages. It is stored as append-only segments in the run's folder, indexed in a trace table of run state; its threads run through `tasks`.

**Why it goes around an owner module.** The design sets up a second writer of the Run's trace files beside the Run-trace writer (E5), and its network log as HAR is a second entry per request beside the request log that writer already keeps. Nothing says how often a segment is opened, so each entry may open its file or start a task, as the request log does today (Exception X2). It also adds commands to every action (C3) and does not say they stop when the trace is switched off.

**Owner module it must go through.**
- E5: `goodfetch-engine`'s `run_trace`. Today the request log's `RequestLog::record` is at `crates/goodfetch-engine/src/run_trace.rs:80` and Provenance's `provenance` at `:126`. Both write `trace/` in the Run's folder (`FILE = "trace/requests.jsonl"`, `:34`). The CDP command log's writer is at `crates/goodfetch-page/src/trace.rs:91`.
- C3: the guard, `crates/goodfetch-page/src/guard.rs:205`, which calls `Turn::call` at `crates/goodfetch-page/src/driver.rs:1036`.

**Replacement design paragraph** (to paste in place of the sentences above):

> The Run trace's frames, DOM snapshots, network log and console messages are new kinds of entry for the Run's one Run-trace writer (`goodfetch-engine`'s `run_trace`, ADR 0140's E5). That writer opens each segment file once per Run and keeps one buffered handle for each file, so no entry creates a folder, opens a file or spawns a task. The network log comes from the request events the request log already records: one entry per request, formatted once. HAR is built from those entries when the trace is read or exported, not written as a second entry per request. The trace table of run state indexes a segment when the writer closes it, never once per action. The frame and `DOMSnapshot.captureSnapshot` at each action and Effect are a stage that the guard builds once per Session and calls once per action. With `trace.keep: never` the guard never calls that stage: it sends no command and nothing reaches the writer. The pull request records the commands this stage adds per action in `C3.commands`, with a `Quality-change:` trailer, measured on B2 with the trace on and off. It keeps `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures and is blocked by the gate's benchmark ticket.
### #867 macOS confinement: Seatbelt, rlimits and footprint sampling

**Offending sentence** (Decisions it must honour, second bullet):

> Resource limits are rlimits; because an unprivileged process can set no per-process memory limit on macOS, the engine also samples the instance's footprint and kills the instance with `plugin_limit_exceeded` past the limit.

As written, this adds a second loop that samples process memory beside the governor's sampler. That loop would be a second entry point for E4's unit (one governor sample): every sample would read memory again, outside E4's contract of "each process's memory read once" per sample.

**Owner module it must go through:** E4, `goodfetch-engine`'s `governor` sampler. `Sampler::run` (`crates/goodfetch-engine/src/governor.rs:297`) reads, at each `governor.sample_interval`, the processes registered with its shares (`self.shares.browsers().processes()`, `governor.rs:312`). It reads each process's footprint once through `System::footprint` (`governor.rs:318`, `crates/goodfetch-engine/src/governor/system.rs:92`) and publishes the measures (`measured`, `governor.rs:323`). Today the browser pool is its only registrant.

**Replacement design paragraph** (to replace the bullet):

> - Resource limits are rlimits. An unprivileged process can set no per-process memory limit on macOS, so the engine's Resource governor (ADR 0040) also measures each Plugin instance's footprint. `plugin_host` registers the instance's process with the governor's sampler when the instance starts and removes it when the instance stops. At each `governor.sample_interval` the sampler reads that process's footprint once, together with the browsers', and publishes it as its last measure. `plugin_host` reads that measure and kills the instance with `plugin_limit_exceeded` once it passes the manifest's memory limit. No Plugin instance runs its own sampling task, timer or memory read. The acceptance criterion adds that the instance's footprint is read once per governor sample. It keeps `E4.process_reads`, `E4.meminfo_opens` and `E4.pool_walks` at their register figures on B3, with B3's governor scenario gaining one registered Plugin process. It is blocked by the gate's benchmark ticket.
### #1400 The Daemon's HTTP listener: `POST /v1/rpc` with a token

**Offending sentences:**

> A new module `http_listener` of `goodfetch-engine` (area 3's crate; routes, token and `Host`/`Origin` verification, Server-Sent Events).

> `POST /v1/rpc` carries one JSON-RPC request per HTTP request with `hello`'s fields in the headers `Goodfetch-Protocol`, `Goodfetch-Product`, `Goodfetch-Form: catalog` and `Authorization: Bearer`

Together these place a transport for E1's unit (one JSON-RPC message in, one answer or event out) beside `rpc`. The listener would own its own Server-Sent Events writer, and nothing stops it decoding, dispatching and encoding each request itself. That makes it a second entry point for E1's unit.

**Owner module (E1, `goodfetch-engine`'s `rpc`: its framing and `composition`'s dispatch), checked at `85083c57b`:**

- `serve_carried` (`crates/goodfetch-engine/src/rpc/connection.rs:396`) serves a Connection over any `Carried { reading, writing, listener }` (`:388`). The Console already uses it for its two transports through `serve_console` (`:351`, `ConsoleConnection::{WebSocket, EventStream}` at `:338`).
- `rpc/event_stream.rs:66` (`opened`) is the existing transport of Server-Sent Events plus `POST`. The engine sends each JSON-RPC message as one SSE event, and `POST` bodies are framed by `rpc/framing.rs`.
- `rpc::Authorities` (`rpc/websocket.rs:37`, `refusal` at `:53`) already does the `Host` and `Origin` check.

**Replacement design paragraph:**

The new module `http_listener` of `goodfetch-engine` owns the listener's routes, its token and its `Host` and `Origin` checks, which reuse `rpc`'s `Authorities`. It decodes, dispatches and encodes nothing itself. Each `POST /v1/rpc` hands its body to `rpc` as one more carriage of a Connection, as the Console's event-stream transport does. The request is decoded once into typed params by `rpc`'s framing and dispatched through `composition`'s handlers, and its answer is encoded once into the HTTP response's body. `hello`'s fields come from the request's headers and are judged by `rpc`'s handshake, never by a check of the listener's own. `GET /v1/events` (#1401) streams through `rpc`'s event-stream writer, so an event is encoded once whichever transport carries it.

**Acceptance criterion to add:**

- [ ] Keeps `E1.deep_copies`, `E1.encodes`, `E1.instructions` and `E1.allocations` at their register figures on B3, with B3's input extended with a `POST /v1/rpc` request; blocked by the gate's benchmark ticket.
### #715 Live hiding and extraction-side removal: hidden on the page, lifted when a target is hidden

**Offending sentences:**

> Extraction-side removal walks shadow roots and runs in every tier, before dom-smoothie, htmd or any AI input; area 6 calls it, and it exposes the elements cosmetic rules matched so area 6's trap-link judgement leaves out links inside them (ADR 0035).

> **The removal flag (ADR 0047; shape):** a removal above 20 % of the main content's text is flagged in Provenance.

The first sentence makes extraction-side removal a function of its own that area 6 calls with the page. As built, it takes the page's HTML as a string, parses it again and serializes its result for the next step. That is the pattern ADR 0138 removes: one more parse per page beside the Page analysis, and a module on the page's path that takes HTML as a string. The second sentence, as built, measures the main content's text before and after the removal, which risks a second readability run per page.

**Owner module (F1, the Page analysis of ADR 0138, whose crate #1577's spec places), checked at `85083c57b`:**

- `goodfetch-clean-up-rules/src/removal.rs:130`: `pub fn remove(html: &str, url: &str, …) -> Removal` parses its own `Document` from the string (`:140`) and returns re-serialized HTML, and `remove_selected` (`:221`), which removes the prompts the Consent prompt runner hid, parses that HTML once more (`:234`). Its `Selectors` cache (`:72`) already keeps each parsed selector for the Run (#1765).
- `goodfetch-extractor/src/formats.rs:131`: `measured_formats` calls `clean_up.remove(document.html, document.url)` before it reads the request (ADR 0138's cause 1, Exception X1).
- `goodfetch-page/src/pages/clean_up.rs:73`: the browser tier's `CleanUp` impl calls `removal::remove` on the serialized DOM, shadow roots included, and records the share in Provenance.

**Replacement design paragraph:**

Extraction-side removal is the cosmetic clean-up stage of the Page analysis (ADR 0138), in every tier. The browser tier hands the Page analysis the DOM's serialization, shadow roots included. The clean-up runs after the readers that see the page untouched and removes its matched elements in place on the one tree, never on a copy or a re-serialized string. Its selectors come from the engine's bounded cache of compiled selectors, so a page compiles none. The demand plan schedules it only when a format, a field or learning asks for the cleaned page, so with none it does no work. The Page analysis's result lists the elements the cosmetic rules matched and the links inside them, so area 6's trap-link judgement reads that list and never runs the removal itself. The removal flag is measured in the same pass, without a second readability run: the clean-up counts the text it removed from within the cleaned page's main content, and the flag is that count over the main content's text plus that count. Live hiding on the page and the guard's lifting of a rule stay as described, and each lift adds its retry's CDP commands only to the action whose target was hidden. Keeps `F1.parses`, `F1.selector_compilations` and `F1.readability_runs` on B1, and `C3.commands` on B2, at their register figures. Blocked by #1577 and the gate's benchmark ticket.
### #760 The navigation gate: a handler's every navigation takes a permit and robots.txt is checked for it

**Offending sentence:**

> robots.txt, Crawl-delay and the signals already read are checked before the request is sent, a signal the response itself carries (aipref's `Content-Usage` header, TDMRep's headers and `<meta>`, RSL's `Link` header) when the response arrives, the engine then leaving the page on a blank document

The gate reads the response's `<meta>` and `<link>` signals by parsing the whole document again. The code does this on the owner's blocking pool: `crates/foxhound/goodfetch-fetch-tier/src/gate.rs:389` calls `UseSignals::with_markup`, which builds a `dom_query::Document` (`crates/foxhound/goodfetch-fetch-tier/src/use_signals.rs:154`). The document tiers parse each served document the same way again (`crates/foxhound/goodfetch-fetch-tier/src/tiers.rs:325`).

**Owner module:** F1, the Page analysis of ADR 0138: markup signals are a reader of the untouched page. Its crate is placed by #1577's spec (ADR 0140, open question 1). On the browser tier, the work goes through the interception pipeline's document reader, which reads each main-frame document before it renders (`crates/goodfetch-page/src/page_value.rs:23`, C5's owner `goodfetch-page`'s interception `pipeline`).

**Replacement design paragraph:**

The gate judges a response's own signals from what has already been read once. It reads the headers as they arrive. The markup signals (TDMRep's `<meta name="tdm-reservation">` and RSL's `<link rel="license">`) are readers of the Page analysis (ADR 0138), in its first group, which sees the page untouched. On the browser tier, the gate must judge before the page renders. There, the document reader reads these signals in one streaming lol_html pass over the response's bytes, with selectors from the engine's cache of compiled selectors, and hands the result to the gate, so no module parses the document again for its signals. Robots.txt and its Crawl-delay are judged from the Run's copy of each host's file, which is parsed once per host. Each permit is taken through the gate, so a navigation costs one match and no compilation. Keeps `F1.parses`, `F1.selector_compilations`, `C5.spawns` and `C5.file_opens` on B1 and B2. Blocked by the gate's benchmark ticket, and by #1577's spec for the Page analysis's crate.

### #762 Redirects and refresh stubs followed as the browser follows them, at most `fetch.redirect.max` times

**Offending sentences:**

> A `Refresh` header or a `<meta http-equiv="refresh">` outside `<noscript>`, to another URL, with a delay up to `fetch.refresh.max_delay` [...] is one more hop made at once on every tier

> The refresh-stub and `<meta>` parsers are fuzz targets with replaying `#[test]`s (ADR 0067).

These sentences build a `<meta>` parser of the ticket's own. The code parses each hop's whole HTML document to find the stub: `crates/foxhound/goodfetch-fetch-tier/src/refresh.rs:62` (`dom_query::Document::from` in `in_markup`), called per hop from `crates/foxhound/goodfetch-fetch-tier/src/chain.rs:269`. Each hop's Frontier check also opens a run-state write transaction and reads the Scope from the run file (`crates/foxhound/goodfetch-fetch-tier/src/gate.rs:790-791`).

**Owner modules:**

- F1, the Page analysis of ADR 0138, which lists `meta refresh` among the readers of the untouched page and its streaming lol_html pass. Its crate is placed by #1577's spec.
- F3, `goodfetch-frontier`'s admission, given the Run invariants built when the Run starts (the Scope with its compiled globs).
- F2, `goodfetch-engine`'s `run_state` writer, whose reads leave the writer.

**Replacement design paragraph:**

The Page analysis reads a refresh stub's `<meta http-equiv="refresh">` (ADR 0138), among the readers that see the page untouched. When nothing else needs a tree, its streaming lol_html pass reads it instead. The chain takes the stub from that result. A hop that is followed has its own document analysed in turn, so each document of a chain is parsed at most once. The `Refresh` header is read from the response's headers. The HTML standard's shared declarative refresh steps, which turn a header value or a `content` attribute into a delay and a URL, stay one pure function, and that function is the fuzz target. Each hop's Frontier check (scope, domain policy, duplicate) takes the Run invariants built when the Run starts. It reads only through the run file's read-only connections, and what it admits commits with the attempt's outcome, never in a transaction of its own. Keeps `F1.parses`, `F3.invariant_reads`, `F3.compilations`, `F2.transactions` and `F2.writer_reads` on B1. Blocked by the gate's benchmark ticket, and by #1577's spec for the Page analysis.

### #1695 The navigation gate judges a handler's documents by markup and host-declared use signals

**Offending sentence:**

> Both use the one parser of each signal `goodfetch-fetch-tier` already has (`honoured_purpose`), not a second one.

The "one parser" is `UseSignals::with_markup` (`crates/foxhound/goodfetch-fetch-tier/src/use_signals.rs:140-154`). It parses each document whole with `dom_query`, and it is called once per served document (`crates/foxhound/goodfetch-fetch-tier/src/tiers.rs:325`) and again per gated document (`crates/foxhound/goodfetch-fetch-tier/src/gate.rs:389`). `honoured_purpose` (`crates/foxhound/goodfetch-crawl/src/delivery/attempt/site_preferences.rs:158`) only resolves the Run's Purpose. The ticket would therefore make the gate depend on a parse that runs outside the Page analysis.

**Owner module:** F1, the Page analysis of ADR 0138: markup signals are a reader of the untouched page. Its crate is placed by #1577's spec. On the browser tier, the interception pipeline's document reader (C5's owner, `goodfetch-page`'s interception `pipeline`) reads them before the page renders.

**Replacement design paragraph:**

The gate takes a handler document's markup signals (TDMRep's `<meta>` and RSL's `<link>`) from the one read of that document. On a document tier, that read is the Page analysis's readers of the untouched page (ADR 0138). On the browser tier, it is the document reader's one streaming lol_html pass over the response, with selectors from the engine's cache of compiled selectors, before the page renders. `UseSignals` keeps only the judging of what those readers found, so no module parses a document for its signals. The host's signals (robots.txt's `Content-Signal` lines and TDMRep's well-known file) are read from the Run's copies, which are fetched and parsed once per host (`SignalFiles`), and judged before the navigation is sent. With `site_preferences.respect_use_signals` off, or with the Purpose `search`, the demand plan schedules no signal reader. Keeps `F1.parses`, `F1.selector_compilations`, `C5.spawns` and `C5.file_opens` on B1 and B2. Blocked by the gate's benchmark ticket, and by #1577's spec for the Page analysis.
### #225 Settings through five layers, with lock, expose and `config explain`

**Offending sentence (acceptance criteria):**

> A tightened cap in Operator policy reaches running runs at their next admission point

**Why it goes around the owner.** As written, and as built, every admission point reads the settings files again: `Settings::at_admission` (`crates/goodfetch-engine/src/settings.rs:253`) calls `self.settings.machine()` (`settings.rs:145`), which reads and parses the Operator policy and user configuration. It is called per request in `crates/foxhound/goodfetch-crawl/src/delivery.rs:302`, `:480`, `:1043` and `:1207`, and the admission module's own header (`crates/goodfetch-engine/src/admission.rs:7-9`) states the same rule. That is Exception X5's "settings read again at each admission". ADR 0140 sets the owner: one engine-wide snapshot of the machine's settings, resolved when the engine starts, refreshed when the engine writes its own configuration and resolved again when a Run is admitted. It also rejects re-reading the files on each change, by a stat or a watcher.

**Owner module:** `goodfetch-engine`'s `settings`, as ADR 0140's engine-scope settings snapshot. Today's file read per admission is at `settings.rs:145` and `:253`.

**Replacement paragraph:**

> A tightened cap in Operator policy reaches running Runs at their next admission point once the engine's settings snapshot holds it. The engine resolves the Operator policy and user configuration into one snapshot when it starts, again when it writes its own configuration (`config set`), and again when a Run is admitted (ADR 0140). Each admission point applies the snapshot's caps to the Run's values and reads no file. A hand edit of either file while the engine runs is seen at the next Run or the engine's next write. The test tightens a cap through the engine's own write and sees the running Run's next admission go under it. It also counts no configuration read per admission, keeping `F2.configuration_reads` on B1 and `E2.configuration_reads` and `E4.configuration_reads` on B3 at their register figures. Blocked by the gate's benchmark ticket.

### #234 Resource governor: one per engine, sized from measured load

**Offending sentence (acceptance criteria):**

> The memory allowance is the tightest of the process's own cgroup limit (v1 or v2, from its own path), a Windows Job object limit, `memory_allowance` and 50 % of physical memory, read again at every sample; tested in a cgroup-limited container and with a limit changed mid-run

**Why it goes around the owner.** "Read again at every sample" covers `memory_allowance`, which is a setting, and the cgroup's path. The sampler resolves both settings files on every loop: `Sampler::run` calls `self.machine()` at `crates/goodfetch-engine/src/governor.rs:302`, and `machine()` at `:435` reads `self.settings.machine()` and resolves the layers. It then reads the cgroup limit through `/proc`'s `self` files per sample (`crates/goodfetch-engine/src/governor/system.rs:155`, `linux_files`). That is Exception X17. E4's contract allows per sample 0 configuration reads, at most 1 `/proc/meminfo` open, 0 opens of `self/cgroup` or `self/mountinfo` (read when the engine starts), and each process's memory read once.

**Owner module:** `goodfetch-engine`'s `governor` sampler (`governor.rs:298`, `Sampler::run`), which reads settings from ADR 0140's engine settings snapshot.

**Replacement paragraph:**

> The memory allowance is the tightest of the process's own cgroup limit (v1 or v2), a Windows Job object limit, `memory_allowance` and 50 % of physical memory. The sampler takes `memory_allowance`, the pressure and critical thresholds and every other setting from the engine's settings snapshot (ADR 0140), never from the files. It finds the process's cgroup path from `self/cgroup` and `self/mountinfo` once, when the engine starts. At each sample it reads only that cgroup's limit file, the host pressure source, `/proc/meminfo` at most once and each process's memory once, so a limit changed mid-run is seen at the next sample. Tests run in a cgroup-limited container and change the limit mid-run. On B3 the sampler keeps `E4.configuration_reads`, `E4.meminfo_opens`, `E4.static_opens` and `E4.process_reads` at their register figures. Its one open per sample of the cgroup limit file and of the PSI file are counts the contract allows but no row covers yet: the gate's spec adds a row for each, at 1. Blocked by the gate's benchmark ticket.


## Criteria

Each line's criterion is added to the ticket's acceptance criteria, and the gate's benchmark ticket blocks the ticket.

| Ticket | Paths | Rows | Benchmark | Criterion to add |
|---|---|---|---|---|
| #214 Engine tracer: an Attached engine answers `hello` over stdio | E1 | E1.deep_copies, E1.encodes, E1.instructions, E1.allocations | B3 | Keeps `E1.deep_copies`, `E1.encodes`, `E1.instructions` and `E1.allocations` at their register figures on B3: each request decoded once straight into typed params in `rpc`'s framing, each answer and event encoded once into bytes, and the dispatch's `tracing` record per request formats nothing when its level is off; blocked by the gate's benchmark ticket. |
| #217 Leases and fencing: a handler past its lease can commit and send nothing | C1,F2 | C1.file_opens, C1.log_sends_off, F2.transactions, F2.writer_reads | B1,B2 | Keeps `C1.file_opens`, `C1.log_sends_off`, `F2.transactions` and `F2.writer_reads` at their register figures on B1 and B2: the page's lease check is an in-memory check of the `Clock` before each command, adding no counted work per CDP command, and the late-commit refusal is checked inside the attempt's outcome transaction, adding none of its own; blocked by the gate's benchmark ticket. |
| #227 Fair per-domain admission, rate cap, timeouts and safe points | F2,F3 | F2.transactions, F2.prepares, F2.configuration_reads, F3.invariant_reads, F3.compilations | B1 | Keeps `F2.transactions`, `F2.prepares`, `F2.configuration_reads`, `F3.invariant_reads` and `F3.compilations` at their register figures on B1: the per-domain queues' journal rides the attempt's claim and outcome transactions, the domain key comes from the Public Suffix List and Recipe override built when the Run starts, and the GCRA rate cap reads the Run's settings, never the files, at each admission; blocked by the gate's benchmark ticket. |
| #231 Usage ledger tracer: Limits across runs held by every engine of the user | E3 | E3.statements, E3.slice_transactions, E3.opens | B1,B3 | Keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B1 and B3: a charge the held Slices cover runs no SQL statement and opens nothing (the Limits held in memory between the ledger's own changes), and a Slice taken costs one transaction and one sync on the ledger's kept connection; blocked by the gate's benchmark ticket. |
| #232 Ledger guarantees: reserved costs, must-understand rows and who may raise a Limit | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | Keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B3: `reserve` goes through `ledger::charging`'s same `take` as a charge, a reserve the held Slices cover runs no ledger statement and a reserve that takes a Slice takes it in that Slice's one transaction; the reserve's journal entry in the Run is counted per paid call, not per charge; blocked by the gate's benchmark ticket. |
| #233 Ledger thresholds and "Needs you" items | E3 | E3.slice_transactions, E3.statements | B3 | Keeps `E3.slice_transactions` and `E3.statements` at their register figures on B3: a threshold crossing is recorded inside the transaction of the Slice that crosses it, and a charge within held Slices checks thresholds in memory with no statement; blocked by the gate's benchmark ticket. |
| #235 Browser pool and recycling | E4 | E4.pool_walks | B2 | Keeps `E4.pool_walks` at its register figure on B2: the recycle rule's warm-up and memory cap read the governor sampler's last measure of each browser, never walking a browser's processes per page opened; blocked by the gate's benchmark ticket. |
| #238 Hedged requests and coalescing | E5,E3 | E5.file_opens, E5.spawns, E5.formatting_off, E3.statements | B1 | Keeps `E5.file_opens`, `E5.spawns`, `E5.formatting_off` and `E3.statements` at their register figures on B1: a hedge and a shared response are Provenance entries through `run_trace`'s one writer, both copies' Usage charges go through `ledger::charging` within held Slices, and with hedging off no hedge timer, percentile or entry is kept per request; blocked by the gate's benchmark ticket. |
| #244 Routing Clients and placing runs in a group | E1 | E1.deep_copies, E1.encodes | B3 | Keeps `E1.deep_copies` and `E1.encodes` at their register figures on B3: forwarding is a stage of `rpc`'s dispatch that reads the cached group Run index, passes a forwarded request's bytes and its owner's answer through without decoding into a copy or encoding again, and costs a local call nothing; blocked by the gate's benchmark ticket. |
| #248 Partitioned crawl across Group members | F3 | F3.invariant_reads, F3.compilations, F3.validations, F3.payload_serializations | B1 | Keeps `F3.invariant_reads`, `F3.compilations`, `F3.validations` and `F3.payload_serializations` at their register figures on B1: each link's Partition is chosen in `goodfetch-frontier`'s admission from the partition map built once per map epoch, and a link forwarded from another Partition enters the receiving Run through that same admission with its Run invariants, never a second enqueue path; blocked by the gate's benchmark ticket. |
| #256 WebSocket transport for Exposed Daemons | E1 | E1.deep_copies, E1.encodes, E1.instructions, E1.allocations | B3 | Keeps `E1.deep_copies`, `E1.encodes`, `E1.instructions` and `E1.allocations` at their register figures on B3, with a WebSocket cell beside stdio: a frame's payload is decoded once straight into typed params and each answer and event is encoded once into the frame, through `rpc`'s one dispatch; blocked by the gate's benchmark ticket. |
| #258 Group-wide Limits, the group redirect row, erasures and an unreachable lead | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | Keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B3: a Group member's charge within its held Slices sends nothing to the Lead Daemon and runs no statement, and a Slice taken from the lead costs one transaction there and one peer round trip; blocked by the gate's benchmark ticket. |
| #631 Browser navigations and redirects of every kind re-checked, and the `file:` rule | C5 | C5.spawns, C5.file_opens | B2 | Keeps `C5.spawns` and `C5.file_opens` on B2: each redirect hop and scheme refusal is judged in the interception pipeline's egress step with no task spawned or file opened per refusal (today `pipeline.rs:1731` spawns one); blocked by the gate's benchmark ticket and #632's decision on `C5.fetch_messages_no_rule`. |
| #635 The relay's terminating mode on Chromium: exceptions for listed hosts with a name-constrained, in-memory authority | C5 | C5.fetch_messages_no_rule, C5.spawns, C5.file_opens | B2 | Keeps `C5.fetch_messages_no_rule`, `C5.spawns` and `C5.file_opens` at their register figures on B2: the cookie refusal for an exception host is a stage of the interception `pipeline` matching hosts compiled when the Session opens, with no Fetch message on a Session with no exception; blocked by the gate's benchmark ticket. |
| #643 Purpose and Site preferences recorded, and a sharing value that presses harder | E5 | E5.file_opens, E5.formatting_off | B1 | Keeps `E5.file_opens` and `E5.formatting_off` on B1: each Provenance entry's Purpose and Site preferences come from the Run's resolved settings and the per-host copies already read, through the Run-trace writer, never read again per result; blocked by the gate's benchmark ticket. |
| #655 Typing a placeholder only where the binding allows, and refusing shared or exception-bound pages | C3 | C3.commands, C3.script_sends | B2 | Keeps `C3.commands` and `C3.script_sends` at their register figures on B2 for `fill` and `press`: the two frame checks read the field's frame and its URL from the driver's frame registry and the guard's kept handle, adding no CDP command or script send per attempt; blocked by the gate's benchmark ticket. |
| #656 Placeholders in request headers and JSON strings, added after the page's scripts | C5 | C5.fetch_messages_no_rule, C5.spawns, C5.file_opens | B2 | Keeps `C5.fetch_messages_no_rule`, `C5.spawns` and `C5.file_opens` at their register figures on B2: placeholders are added in the interception `pipeline`'s one answer per paused request, checked against each hop's URL with the binding's domains built once per Session, and a Session with no header or JSON placeholder sends no Fetch message for them; blocked by the gate's benchmark ticket. |
| #657 `session.http_credentials` answered only from listed domains and only over HTTPS | C5 | C5.fetch_messages_no_rule | B2 | Keeps `C5.fetch_messages_no_rule` at its register figure on B2: credentials are answered on `Fetch.authRequired` in the interception `pipeline`, and a Session without `session.http_credentials` sends no Fetch message for them; blocked by the gate's benchmark ticket. |
| #659 A Client acting for a model observes nothing of a page during a Human pause | E1 | E1.encodes, E1.deep_copies | B3 | Keeps `E1.encodes` and `E1.deep_copies` at their register figures on B3: withholding a held page's events from a Client acting for a model is a check in `events`' delivery, and an event withheld is never encoded for that Client; blocked by the gate's benchmark ticket. |
| #671 One Audit log chain for a Daemon group, and the certificate store held by the Lead Daemon | E2 | E2.configuration_reads, E2.opens, E2.prepares, E2.transactions | B3 | Keeps `E2.configuration_reads`, `E2.opens`, `E2.prepares` and `E2.transactions` at their register figures on B3: entries arriving over the peer link and a cut-off member's merge go through `audit`'s one writer at the Lead Daemon, one transaction per entry; blocked by the gate's benchmark ticket. |
| #676 Redirect hops and Hook rewrites judged by the scope and the domain policy, each a `skipped` outcome | F3,F2 | F3.invariant_reads, F3.compilations, F2.writer_reads, F2.transactions | B1 | Each redirect hop is judged against the Run invariants built when the Run starts (the Scope and its compiled globs held in memory), with no run-file read or writer transaction per hop; keeps `F3.invariant_reads`, `F3.compilations`, `F2.writer_reads` and `F2.transactions` at their register figures on B1, whose fixture already has a redirect; blocked by the gate's benchmark ticket. Merged code reads and compiles the Scope through the writer per hop (`goodfetch-fetch-tier/src/gate.rs:785-791`), a site X6 and X4 do not list: add it to them. |
| #677 Sitemap entries counted and refused by the scope, with the sitemap's own fetches under the domain policy | F3 | F3.invariant_reads, F3.compilations | B1 | A sitemap's entries are one links batch judged against the Run invariants and the seed's rules compiled once when the seed is given, with no Scope read and no rule compilation per sitemap read; keeps `F3.invariant_reads` and `F3.compilations` at their register figures on B1, its fixture gaining a sitemap seed; blocked by the gate's benchmark ticket. Today each sitemap read reads the Scope (`goodfetch-crawl/src/work_items.rs:289`) and compiles its rules again (`goodfetch-frontier/src/sitemap.rs:551-562`), sites X6 does not list: add them. |
| #685 A Plugin's or Script Hook's rewrite held to its declared domains | F3,F2 | F3.invariant_reads, F3.compilations, F2.writer_reads, F2.transactions | B1 | A Plugin's or Script Hook's rewrite is judged against the Run invariants and each Hook's declared domains compiled once when the Run starts, with no run-file read, writer transaction or compilation per rewrite; keeps `F3.invariant_reads`, `F3.compilations`, `F2.writer_reads` and `F2.transactions` at their register figures on B1, its fixture gaining a rewriting Hook; blocked by the gate's benchmark ticket. Depends on #1481's Hook-stage path. |
| #694 Dataset retention applied by the engine, and an erasure across a run's stored data | F2,F4 | F2.transactions, F4.transactions | B1 | Retention is applied when the Run starts and then on a timer task of `tasks`, through `run_state`'s writer, never on a Record's commit or an attempt; keeps `F2.transactions` and `F4.transactions` at their register figures on B1; adds prune passes per attempt, which no row covers (add one as `E2.prunes` does for the Audit log); blocked by the gate's benchmark ticket. |
| #699 Redirect loops, cut documents, bad content codings and file navigations are typed | C2,C4 | C2.commands, C2.world_creations, C4.detection_reads | B2 | Reads each document's decoded encoding without a page read of its own (from the response's `charset` the browser reports, or inside the navigation's one composed detection read), so a navigation adds no `Page.createIsolatedWorld` and no extra `Runtime` call: keeps `C2.commands`, `C2.world_creations` and `C4.detection_reads` at their register figures on B2, its input extended with a Shift_JIS page; a separate encoding read per navigation is a new count no row covers; blocked by the gate's benchmark ticket. |
| #703 Page events, request and response bodies, and the Run's page events stream | E1,C1 | E1.encodes, E1.allocations, C1.file_opens, C1.log_sends_off | B2,B3 | Each page event is encoded once straight into its message's bytes through `goodfetch-engine`'s `rpc`, and only for a page with a listening `page.on` call; a body costs one `Network.getResponseBody` through `goodfetch-cdp`'s connection only when read; keeps `E1.encodes` and `E1.allocations` on B3 (gaining page events) and `C1.file_opens` and `C1.log_sends_off` on B2 at their register figures; adds events formatted for a page with no listener, which no row covers (add one, 0); blocked by the gate's benchmark ticket. |
| #704 Page functions: one JSON argument in, JSON out, nothing lost silently | C2 | C2.commands, C2.world_creations | B2 | A page function in the isolated world runs in the context the driver's registry keeps per frame and document, never a `Page.createIsolatedWorld` per call, and its result walker is installed once per context and called by handle: keeps `C2.commands` and `C2.world_creations` at their register figures on B2, counted per page function call as a page read, its input extended with an isolated `evaluate`; the walker's source sends per call are a new count no row covers; blocked by the gate's benchmark ticket. |
| #707 Service workers run under the interception on Chromium, or are blocked | C5,E5 | C5.spawns, C5.file_opens, C5.fetch_messages_no_rule, E5.file_opens, E5.spawns, E5.formatting_off | B2 | A service worker's requests pass the page's one interception `pipeline` as a Session-scoped target, with no loop of their own, and each is recorded through `run_trace`'s one writer per Run: keeps `C5.spawns`, `C5.file_opens`, `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B2, its input extended with a service worker that fetches; `block` sends no `Fetch` message for a worker that never runs; blocked by the gate's benchmark ticket. |
| #708 One proxy per Session, SOCKS5 credentials relayed locally, and a failing proxy told from a failing site | C5 | C5.spawns, C5.file_opens | B2 | The proxy-or-site rule judges each document response inside the interception `pipeline` from the response it already holds, with no task, file or extra CDP command per response, and a 407 is answered by the pipeline's one `Fetch.authRequired` handler: keeps `C5.spawns` and `C5.file_opens` at their register figures on B2, its input extended with a proxied Session; blocked by the gate's benchmark ticket. |
| #712 A Challenge on a program's Session ends in `challenge_unsolved`, and a demand for payment in `payment_required` | C4 | C4.detection_reads, C4.compilations | B2 | `payment_required` (402 or a paywall redirect) is classified by `recovery` from the response it judges, and the Challenge ladder's last answer is wired there, adding no detection read and no compilation per navigation: keeps `C4.detection_reads` and `C4.compilations` at their register figures on B2, its input extended with a 402 page; blocked by the gate's benchmark ticket. |
| #713 Filter lists in the interception: ads and trackers blocked, challenge scripts never | C5 | C5.fetch_messages_no_rule, C5.spawns, C5.file_opens | B2 | Filter-list matching is step 4 of the interception `pipeline`, against the one shared `adblock::Engine` loaded once per rules version, with 0 compilations per request and nothing when every list and `browser.block_resources` is off: keeps `C5.fetch_messages_no_rule`, `C5.spawns` and `C5.file_opens` at their register figures on B2, its input extended with blocked and stand-in requests; blocked by the gate's benchmark ticket. |
| #718 A consent wall is a Block cause, accepted only where a person approved | C4 | C4.detection_reads, C4.compilations | B2 | The wall test answers from the Consent prompt runner's pass on the document, through `recovery`'s composition, adding no detection read of its own: keeps `C4.detection_reads` and `C4.compilations` at their register figures on B2, its input extended with a consent-wall page; blocked by the owner's answer on #717 (ADR 0140 open question 3) and blocked by the gate's benchmark ticket. |
| #720 The page's navigation gate, lease check and response observer, empty on a program's page | C1,C4,C5,E5 | C1.file_opens, C1.log_sends_off, C4.detection_reads, C5.spawns, C5.file_opens, E5.file_opens, E5.spawns | B2 | The navigation gate is called at the interception `pipeline`'s step 6, the lease check is an in-memory check in the driver's send before each command reaches `goodfetch-cdp`'s connection, a refused navigation is recorded through `run_trace`'s one writer, and with no response observer no response body is read: keeps `C1.file_opens`, `C1.log_sends_off`, `C4.detection_reads`, `C5.spawns`, `C5.file_opens`, `E5.file_opens` and `E5.spawns` at their register figures on B2, its input extended with a recording gate that refuses one navigation; body reads per response with an observer are a new count no row covers; blocked by the gate's benchmark ticket. |
| #721 Locator constructors, selector rules and open shadow roots | C2,C3 | C2.commands, C2.world_creations, C3.script_sends, C3.commands | B2 | The `getBy*` constructors and XPath match inside the one page read of `goodfetch-page`'s driver (`read_in_frame`, one `Runtime.callFunctionOn` on the kept world) and the guard's `check.js` handle; a selector is validated once per text through the engine's compile cache, and `locator.test_id_attribute` is read from the Session's settings snapshot, never per call; keeps `C2.commands`, `C2.world_creations`, `C3.script_sends` and `C3.commands` at their register figures on B2, B2 gaining a `getByRole` read; adds selector compilations per page read, which no row covers (add one, 0); blocked by the gate's benchmark ticket. |
| #722 Every locator action, its target kinds, strict questions and `force` | C3 | C3.script_sends, C3.commands | B2 | Every added action (and `force`) runs through `goodfetch-page`'s guard calling `check.js` by its handle, its `target_unsupported` and strictness checks inside that same check call; keeps `C3.script_sends` at its register figure and starts `C3.commands` for each new action kind at its first measure on B2, B2 gaining each action kind; blocked by the gate's benchmark ticket. |
| #723 One input order per page: Gestures one at a time, reads beside them, input with no target | C3 | C3.commands, C3.script_sends | B2 | The page's input order and the Effect window hand-off cost no CDP command and no run-state write per action outside a declared Effect; the Effect's sent mark is written once per Effect through the `run_state` writer: keeps `C3.commands` and `C3.script_sends` at their register figures on B2, its input extended with two concurrent `fill`s; blocked by the gate's benchmark ticket. |
| #724 Reads in one round trip, and golden scenario 01: a dependent form read and typed when it fails | C2 | C2.commands, C2.world_creations | B2 | Every getter, question and `all*` read is one page read through `goodfetch-page`'s driver, whatever the number of matches; the one-round-trip criterion is checked on C2's counter, not the CDP command log, which ADR 0140 turns off by default; keeps `C2.commands` and `C2.world_creations` at their register figures on B2, B2 gaining a 200-cell table read; blocked by the gate's benchmark ticket. |
| #725 Locator handlers clear overlays, and a Gesture that waits on the program gives up its turn | C2,C3 | C2.commands, C2.world_creations, C3.commands, C3.script_sends | B2 | A locator handler's check before each attempt runs through the guard's kept `check.js` handle in the registered world, and clears nothing when no handler is registered: keeps `C2.commands`, `C2.world_creations`, `C3.commands` and `C3.script_sends` at their register figures on B2, its input extended with a registered handler and a page with none; blocked by the gate's benchmark ticket. |
| #733 Secrets typed and HTTP credentials answered only on their bound domains, never on a browser other code may read | C3,C5 | C3.commands, C3.script_sends, C5.fetch_messages_no_rule, C5.spawns, C5.file_opens | B2 | Secret placeholders resolve at the interception `pipeline`'s step 7 and HTTP credentials answer in its one `Fetch.authRequired` handler, with no compilation, task or file per request, a JSON body decoded only for a request that carries a placeholder, and a Secret typed through the guard's `fill`: keeps `C3.commands`, `C3.script_sends`, `C5.fetch_messages_no_rule`, `C5.spawns` and `C5.file_opens` at their register figures on B2, its input extended with a placeholder in a header and a body; body decodes per request with a placeholder are a new count no row covers; blocked by the gate's benchmark ticket. |
| #742 Borrow the person's running Chrome or Edge through Chrome's consented debugging | C5 | C5.spawns, C5.file_opens | B2 | On a Borrowed browser, step 3 of the interception `pipeline` resolves each request's host through the Session's `Lookups`, held in a bounded per-Session cache, with no task or file per request: keeps `C5.spawns` and `C5.file_opens` at their register figures on B2, its input extended with a Borrowed-browser Session; name resolutions per request are a new count no row covers; blocked by the gate's benchmark ticket. |
| #744 Collie works inside the program's own Playwright or Puppeteer: `run.borrow()` | C1,E1 | C1.file_opens, C1.log_sends_off, E1.deep_copies, E1.encodes | B2,B3 | The borrow relay is a reader and writer pair under `goodfetch-cdp`'s `Connection::new`, so each borrowed command still crosses `send_raw` once, and each CDP frame crosses the SDK's link as one JSON-RPC message carried as raw text, encoded once and never parsed into a `Value` twice: keeps `C1.file_opens`, `C1.log_sends_off`, `E1.deep_copies` and `E1.encodes` at their register figures on B2 and B3, B3's input extended with relay frames; blocked by the gate's benchmark ticket. |
| #745 A source's domains bound navigation, and a model's imports and saves wait for a person | C5,E2 | C5.spawns, C5.file_opens, E2.configuration_reads, E2.opens, E2.prepares, E2.transactions | B2,B3 | The source-domain check runs at the interception `pipeline`'s step 3 on the registrable domains parsed once when the Session opens, and each login import, borrow, save or deletion is one entry through `audit`: keeps `C5.spawns`, `C5.file_opens`, `E2.configuration_reads`, `E2.opens`, `E2.prepares` and `E2.transactions` at their register figures on B2 and B3, B2's input extended with a navigation to an unlisted domain; blocked by the gate's benchmark ticket. |
| #749 `page.lend()`: a library attaches to a page and sees only that page's Session | C1 | C1.file_opens, C1.log_sends_off | B2 | Every command a lent library sends reaches the browser through `goodfetch-cdp`'s connection (`send_raw`, given an owned method name for relayed commands), never a second socket or pipe to the browser, so the CDP command log and the counts see it: keeps `C1.file_opens` and `C1.log_sends_off` at their register figures on B2, its input extended with a lent page's commands; blocked by the gate's benchmark ticket. |
| #750 The Lent page's command table, its end of attach, and golden journey: lend a page to Playwright | C1,E5 | C1.file_opens, C1.log_sends_off, E5.file_opens, E5.spawns | B2 | The command table is data built once and looked up per command before it crosses `goodfetch-cdp`'s connection, and each attach and refused command is one entry through `run_trace`'s one writer per Run: keeps `C1.file_opens`, `C1.log_sends_off`, `E5.file_opens` and `E5.spawns` at their register figures on B2, its input extended with a refused command; blocked by the gate's benchmark ticket. |
| #751 `page.newCDPSession()`: the way down, over the program's own connection | C1,E1 | C1.file_opens, C1.log_sends_off, E1.deep_copies, E1.encodes | B2,B3 | A CDP session's command enters through `rpc`'s dispatch as one message, crosses `goodfetch-cdp`'s connection (`send_raw`, given an owned method name) with the in-memory lease check before it, and its answer is encoded once: keeps `C1.file_opens`, `C1.log_sends_off`, `E1.deep_copies` and `E1.encodes` at their register figures on B2 and B3, B3's input extended with a relayed raw command; blocked by the gate's benchmark ticket. |
| #753 `browser-host`: a Session driven through a Browser host beside its browser | C1,C2,C3,E1 | C1.file_opens, C1.log_sends_off, C2.commands, C2.world_creations, C3.script_sends, C3.commands, E1.deep_copies, E1.encodes | B2,B3 | The Browser host runs `goodfetch-page`'s driver, guard and interception and `goodfetch-cdp`'s connection unchanged (no second implementation of any), and each Session operation crosses the engine-to-host link as one JSON-RPC message through `rpc`'s framing: keeps `C1.file_opens`, `C1.log_sends_off`, `C2.commands`, `C2.world_creations`, `C3.script_sends`, `C3.commands`, `E1.deep_copies` and `E1.encodes` at their register figures on B2 and B3, B2's input also run through an in-memory Browser host link; blocked by the gate's benchmark ticket. |
| #754 A Browser host does the hosted browser's work: egress, Filter lists, governor and Live view, and golden journey: a remote browser | C4,C5,E4 | C4.detection_reads, C4.compilations, C5.spawns, C5.file_opens, C5.fetch_messages_no_rule, E4.configuration_reads, E4.meminfo_opens, E4.static_opens, E4.process_reads | B2,B3 | On the Browser host the egress check, Filter lists, live hiding and Consent prompt runner run inside the same interception `pipeline` and `recovery`, and its governor is `governor`'s sampler reading the host's settings snapshot: keeps `C4.detection_reads`, `C4.compilations`, `C5.spawns`, `C5.file_opens`, `C5.fetch_messages_no_rule`, `E4.configuration_reads`, `E4.meminfo_opens`, `E4.static_opens` and `E4.process_reads` at their register figures on B2 and B3 run through the host; pressure reports per sample are a new count no row covers; blocked by the gate's benchmark ticket. |
| #755 A Browser host and its engine settle on the lower protocol minor | E1 | E1.deep_copies, E1.encodes, E1.instructions | B3 | The settled minor and the per-field minors are read from a table built once per Browser host link, and the check walks no message beyond its one encode: keeps `E1.deep_copies`, `E1.encodes` and `E1.instructions` at their register figures on B3, its input extended with a Browser host link at a lower minor; blocked by the gate's benchmark ticket. |
| #757 The Wall allowance: read past a Wall only where a person approved | C4,F1 | C4.detection_reads, C4.compilations, F1.readability_runs, F1.parses | B1,B2 | The paywall detector runs inside `recovery`'s one composed detection read with its rules compiled when the rules version loads, and the withholds-content test reads the main content the Page analysis already produced, adding no readability run or parse: keeps `C4.detection_reads`, `C4.compilations`, `F1.readability_runs` and `F1.parses` at their register figures on B2 and B1, their inputs extended with a paywall page; blocked by #1577 and blocked by the gate's benchmark ticket. |
| #768 The tier choice: a sample fetched both ways, judged on what each attempt staged | F2,F4 | F2.transactions, F2.prepares, F4.deep_copies | B1 | Keeps `F2.transactions`, `F2.prepares` and `F4.deep_copies` on B1: a sample's learned choice commits in the committing attempt's outcome (today its own transaction, `delivery.rs:757`), and weighing compares staged Records without cloning their data (today `tier_choice/weighing.rs:77`); blocked by the gate's benchmark ticket. |
| #769 Browser-fetch keeps a Session's TLS fingerprint, and cookies agree across tiers | E4,C1 | E4.pool_walks, C1.file_opens, C1.log_sends_off | B2 | Keeps `E4.pool_walks`, `C1.file_opens` and `C1.log_sends_off` on B2; adds a count no row covers, engine pages opened per browser-fetch hop (today one opened and closed per hop, `goodfetch-page/src/pages.rs:929`, `sessions.rs:896`), which should fall to one per pool Session; blocked by the gate's benchmark ticket. |
| #775 A handler's lend: the gate sees a library's navigation once, and the lease cuts the attach | C5,E5 | C5.spawns, C5.file_opens, E5.file_opens, E5.spawns | B2 | Keeps `C5.spawns`, `C5.file_opens`, `E5.file_opens` and `E5.spawns` on B2: a Lent page's navigation takes the gate in the interception pipeline once, and an ungated request's Run-trace entry goes to the Run-trace writer with no task spawned per entry (not `pipeline.rs:1731`'s pattern); blocked by the gate's benchmark ticket. |
| #776 A Bearer token on a crawl request, only as a Secret placeholder | C5,E2 | C5.spawns, C5.file_opens, E2.configuration_reads, E2.opens, E2.prepares, E2.transactions | B1,B2 | Keeps `C5.spawns`, `C5.file_opens` and the E2 rows listed on B1 and B2: the Bearer placeholder is resolved once per Run through the Secret driver and reused per hop with its binding checked (`goodfetch-engine/src/secrets.rs`), one Audit entry per resolution, never per request unless `secrets.resolve_each_use` lists it; blocked by the gate's benchmark ticket. |
| #777 A route cannot step around the Frontier on a crawl request's own fetch | C5,F3 | C5.spawns, C5.file_opens, F3.invariant_reads, F3.compilations | B1,B2 | Keeps `C5.spawns`, `C5.file_opens`, `F3.invariant_reads` and `F3.compilations` on B1 and B2: a route's rewritten URL is judged against the Run invariants built at the start, never a Scope read from the run file (the pattern of `gate.rs:791`); blocked by the gate's benchmark ticket. |
| #778 Kept responses: a page's JSON read by the handler with no reload | C5 | C5.spawns, C5.file_opens | B2 | Keeps `C5.spawns` and `C5.file_opens` on B2, with the kept-response patterns compiled once per Session; adds a count no row covers, body reads per kept response (1 with a matching URL, 0 with none), read without a task spawned per response (today `page_events.rs:972`); blocked by the gate's benchmark ticket. |
| #779 Paid fetch tracer: a Cloudflare pay-per-crawl 402 paid under a person's Limit, in an identified Session | E2,E3,E5 | E2.configuration_reads, E2.opens, E2.prepares, E2.transactions, E3.statements, E3.opens, E3.slice_transactions, E5.file_opens | B1,B3 | Keeps the E2, E3 and E5 rows listed on B1 and B3: a Paid fetch's reservation is taken from held Slices, its Audit entry and Provenance mark go through `audit` and `run_trace`; adds a count no row covers, the Effect's journal commit per Paid fetch that ADR 0138 puts before the Effect; blocked by the gate's benchmark ticket. |
| #791 A Run's Key-value store holds typed JSON state across a restart | F2 | F2.transactions, F2.prepares, F2.writer_reads | B1 | Keeps `F2.transactions`, `F2.prepares` and `F2.writer_reads` on B1: Key-value writes are staged with the attempt and commit in its outcome, never in a transaction of their own, and a read of the attempt's own staged writes goes through the run file's read-only connections; blocked by the gate's benchmark ticket. |
| #794 Replay mode serves every stored response, POST included | C5 | C5.fetch_messages_no_rule, C5.file_opens, C5.spawns | B2 | The canonical body hash is taken once per request with a body, only under `replay`, inside the Response cache stage the interception pipeline calls: keeps `C5.fetch_messages_no_rule`, `C5.file_opens` and `C5.spawns` at their register figures on B2 with the cache off; the body hash per request has no row yet. Blocked by #793's decision and the gate's benchmark ticket. |
| #795 The cache never leaks a logged-in or geo-dependent page, and never replays what must be fetched | C5 | C5.file_opens, C5.spawns | B2 | The partition is built once per Session (again only when its proxy exit changes) and the never-store checks read the response's own outcome, with no lookup per request: keeps `C5.file_opens` and `C5.spawns` at their register figures on B2; blocked by #793's decision and the gate's benchmark ticket. |
| #796 The cache acts in browsers through the engine's interception | C5 | C5.fetch_messages_no_rule, C5.file_opens, C5.spawns | B2 | With `cache.mode: off` the cache arms no interception, and a hit is answered with one `Fetch.fulfillRequest` from the pipeline's cache stage: keeps `C5.fetch_messages_no_rule`, `C5.file_opens` and `C5.spawns` at their register figures on B2, B2's input extended with a cached subresource; blocked by #793's decision and the gate's benchmark ticket. |
| #797 The cache is bounded by size and age, and honours a short disk | C5 | C5.file_opens, C5.spawns | B2 | Records a hit's recency with no transaction of its own per request (grouped on the cache store's writer) and reads the storage level from its monitor's last measure, never per request: keeps `C5.file_opens` and `C5.spawns` at their register figures on B2; recency writes per hit have no row yet. Blocked by #793's decision and the gate's benchmark ticket. |
| #799 A Run answers every request from a WARC recording | C5 | C5.fetch_messages_no_rule, C5.file_opens, C5.spawns | B1,B2 | The WARC is read once when the Run starts into an index held for the Run, and bodies are read through one kept handle, so a replayed request opens no file and compiles nothing: keeps `C5.fetch_messages_no_rule` (with no recording), `C5.file_opens` and `C5.spawns` at their register figures on B2 and B1, B1's input extended with a replayed page; blocked by the gate's benchmark ticket. |
| #800 A Run records its responses into a WARC file | C5,F2 | F2.transactions, F2.prepares, C5.file_opens, C5.spawns | B1,B2 | Each recorded response is staged with its attempt's outcome in a crawl and grouped by the `run_state` writer in a Session, with no transaction of its own: keeps `F2.transactions`, `F2.prepares`, `C5.file_opens` and `C5.spawns` at their register figures on B1 and B2 with recording on; the body read and run-state write per recorded response have no row yet. Blocked by the gate's benchmark ticket. |
| #801 A replayed page sees the time and randomness it was recorded with | C5 | C5.fetch_messages_no_rule, C5.file_opens, C5.spawns | B2 | The `resolve` map is built once per Session and the replay stage answers before any DNS lookup with no compilation per request; the time and randomness script is registered once per page: keeps `C5.fetch_messages_no_rule`, `C5.file_opens` and `C5.spawns` at their register figures on B2; C5's 0 compilations per request has no row yet. Blocked by the gate's benchmark ticket. |
| #802 A recording shows in the Run report and in Provenance | E5 | E5.file_opens, E5.spawns, E5.formatting_off | B1,B2 | The recording mark is a field of the Provenance entry `run_trace` already writes: keeps `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B1 and B2; blocked by the gate's benchmark ticket. |
| #803 Cache hits in results, and what a person approved for one Run stays with it | E3,E5 | E3.statements, E3.opens, E5.file_opens, E5.spawns, E5.formatting_off | B1,B2,B3 | A hit's Usage is charged through `ledger::charging` from held Slices and its mark is a field of the Provenance entry `run_trace` already writes: keeps `E3.statements`, `E3.opens`, `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B1, B2 and B3; blocked by #793's decision and the gate's benchmark ticket. |
| #807 `locator.scrollCollect` reads an infinite or virtualized list whole | C2,C3 | C2.commands, C2.world_creations, C3.script_sends, C3.commands | B2 | Each step reads the rows as one page read through `goodfetch-page`'s driver and scrolls as one guarded attempt calling `check.js` by its kept handle: keeps `C2.commands`, `C2.world_creations`, `C3.script_sends` and `C3.commands` (a new `scroll_collect` action kind) at their register figures on B2, its input extended with a scroll collection; blocked by the gate's benchmark ticket. |
| #823 Kind registry and the Extractor kind: a Plugin replaces main-content extraction | C2,E3,E5 | C2.commands, C2.world_creations, E3.statements, E3.opens, E5.file_opens, E5.spawns, E5.formatting_off | B2,B3 | The kind driver charges each call's `plugin_calls` and `plugin_time` through `Run::charge` into `ledger::charging` from the held Slices, sends its Run-trace entry through `run_trace`'s one writer per Run (no file of its own), and the Extractor's input comes from one driver read of the page; keeps `C2.commands`, `C2.world_creations`, `E3.statements`, `E3.opens`, `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B2 and B3, B3 gaining a scenario of Plugin calls charged within a Slice; blocked by the gate's benchmark ticket. |
| #824 The `ai` accessibility snapshot, element references and `getByRef` | C2 | C2.commands, C2.world_creations | B2 | An `ai` snapshot is one engine read through the driver's per-frame registry (one `Runtime.callFunctionOn`, no `Page.createIsolatedWorld`), element references are kept in the page, and masking uses the Run's `Mask` built once; keeps `C2.commands` and `C2.world_creations` at their register figures on B2, B2 gaining an `ai`-mode read; blocked by the gate's benchmark ticket. |
| #825 The cache and recordings against declared Effects | C5 | C5.file_opens, C5.spawns | B2 | Whether an Effect's body runs is read by the cache stage from the Run's state held in memory, with no read per request: keeps `C5.file_opens` and `C5.spawns` at their register figures on B2; blocked by the gate's benchmark ticket. |
| #828 Frontier tracer: a crawl admits its seeds and enqueues, each answered with a reason | F3 | F3.invariant_reads, F3.compilations, F3.payload_serializations, F3.validations | B1 | Every seed and enqueue goes through goodfetch-frontier's admit, given the Run invariants built when the Run starts; keeps F3.invariant_reads, F3.compilations, F3.payload_serializations and F3.validations at their register figures on B1; blocked by the gate's benchmark ticket. |
| #829 `observe` through one model call: the Model reference, a declared provider and the first adapter | C2,E3 | C2.commands, C2.world_creations, E3.statements, E3.opens, E3.slice_transactions | B2,B3 | The Tower layer reserves each HTTP attempt through `Run::reserve` into `ledger::charging` from the held Slices, and the observation is one `ai` read through the driver; keeps `C2.commands`, `C2.world_creations`, `E3.statements`, `E3.opens` and `E3.slice_transactions` at their register figures on B2 and B3, B3 gaining a model attempt reserved within a Slice; blocked by the gate's benchmark ticket. |
| #833 `net.connect` streams and `proxy_tcp`: confined clients that only speak a proxy | E3,E5 | E3.statements, E3.opens, E3.slice_transactions, E5.file_opens, E5.spawns | B2,B3 | `plugin_stream_bytes` is charged through `Run::charge` into `ledger::charging` from the held Slices (never a ledger statement per frame), and each stream's destination reaches the Run trace through `run_trace`'s writer; keeps `E3.statements`, `E3.opens`, `E3.slice_transactions`, `E5.file_opens` and `E5.spawns` at their register figures on B2 and B3; blocked by the gate's benchmark ticket. |
| #834 `act` through the guard: one action, never sent twice | C3 | C3.script_sends, C3.commands | B2 | `act`'s one action runs through the guard's check by the kept `check.js` handle, adding no CDP command to the click, fill or press it performs; keeps `C3.script_sends` and `C3.commands` per action kind at their register figures on B2; blocked by the gate's benchmark ticket. |
| #840 Scope: a crawl never leaves the site it was given | F3 | F3.invariant_reads, F3.compilations | B1 | The Scope with its compiled include and exclude patterns is a Run invariant built when the Run starts, checked per enqueue, redirect and Hook rewrite without a read of the run file or a compilation; keeps F3.invariant_reads and F3.compilations at their register figures on B1, B1 crawling a redirect; blocked by the gate's benchmark ticket. |
| #841 `extract` in model mode: Grounding, missing fields and the bounded retry | C2,E5 | C2.commands, C2.world_creations, E5.file_opens, E5.spawns, E5.formatting_off | B2 | Grounding reads the page's text through one driver read per attempt, each Grounded value's Provenance goes through `run_trace`'s one writer per Run, and the schema's validator is built once per call, not per attempt; keeps `C2.commands`, `C2.world_creations`, `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B2; no row counts validator builds per page read (a new count to add); blocked by the gate's benchmark ticket. |
| #844 Number and currency Value transforms from CLDR data | F1 | F1.parses, F1.selector_compilations, F1.file_opens | B1 | goodfetch-value-transform's locale forms and CLDR data are built once when the crawl starts (as code_free::fields builds them today), each page's field only matched against them; keeps F1.parses, F1.selector_compilations and F1.file_opens at their register figures on B1, B1 extended with a code-free field under a number and a currency transform; no row counts locale-form builds per page yet; blocked by the gate's benchmark ticket. |
| #848 Date, date-time and relative-time Value transforms under a stated locale and zone | F1,E5 | F1.parses, F1.selector_compilations, F1.file_opens, E5.file_opens, E5.formatting_off | B1 | Date and relative-time forms are built once per crawl, and each field's Provenance travels in the page's one Provenance entry through the Run-trace writer; keeps F1.parses, F1.selector_compilations, F1.file_opens, E5.file_opens and E5.formatting_off at their register figures on B1, B1 extended with a relative_time field; blocked by the gate's benchmark ticket. |
| #849 Block causes as a typed outcome: a Challenge page is `blocked` with its cause, and the story file for the area | C4 | C4.detection_reads, C4.compilations | B2 | Block detection's document facts come from recovery's one composed detection read per navigation (goodfetch-page recovery), never a read of its own as pages/blocked.rs's block_of does today; keeps C4.detection_reads and C4.compilations at their register figures on B2, B2's navigations including a challenge page; blocked by the gate's benchmark ticket. |
| #853 `expect`: Success checks evaluated live by the engine | C2,E5 | C2.commands, C2.world_creations, E5.file_opens, E5.spawns, E5.formatting_off | B2 | Each poll of a Success check is one driver read, its regular expression compiled once per evaluation, never per poll, and each evaluation reaches the Run trace through `run_trace`'s writer; keeps `C2.commands`, `C2.world_creations`, `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B2, B2 gaining an `expect` that polls; blocked by the gate's benchmark ticket. |
| #857 Block detection rules for every cause, fired on evidence and seeded under their licences | C4 | C4.detection_reads, C4.compilations | B2 | Keeps `C4.detection_reads` and `C4.compilations` at their register figures on B2: the rules are compiled once into the engine's bounded cache and evaluated by the Block markers inside `recovery`'s one detection read per navigation, with a blocked document among B2's outcomes; blocked by the gate's benchmark ticket. |
| #863 `enqueueLinks` reports each matched element | F1,F3 | F1.parses, F1.selector_compilations, F3.invariant_reads, F3.compilations, F3.payload_serializations, F3.validations | B1 | enqueueLinks reads its locator's elements from the Page analysis with the selector from the engine's cache, and admits its batch under the Run invariants instead of Scope::read, PayloadRules::read and Fragments::read per batch (crawl enqueue/links.rs); keeps F1.parses, F1.selector_compilations and the F3 rows at their register figures on B1, B1's handler calling enqueueLinks; blocked by the gate's benchmark ticket. |
| #865 Plan tracer: `act` compiled once, promoted after a verified Run and replayed with no model call | C2,C3 | C2.commands, C2.world_creations, C3.commands, C3.script_sends | B2 | A replayed step resolves its whole locator ladder and checks its Target fingerprint in one driver read, then acts through the guard by handle; keeps `C2.commands`, `C2.world_creations`, `C3.commands` and `C3.script_sends` at their register figures on B2, B2 gaining a replayed step; blocked by the gate's benchmark ticket. |
| #869 Enqueues, seeds, Records and Key-value writes of an attempt commit together or not at all | F2,F3 | F2.transactions, F2.writer_reads, F3.invariant_reads, F3.compilations | B1 | An attempt's enqueues, seeds, Records and Key-value writes stage with it and commit in its one outcome transaction, n enqueues sharing the Run invariants; keeps F2.transactions, F2.writer_reads, F3.invariant_reads and F3.compilations at their register figures on B1; F3's 0 transactions of its own has no row yet; blocked by the gate's benchmark ticket. |
| #871 A proxy's refusal is never a Block cause, and a Block cause never enters the Response cache | C4 | C4.detection_reads, C4.compilations | B2 | The proxy-or-site judgment and the Response cache exclusion add no read: they use the response and recovery's one detection read; keeps C4.detection_reads and C4.compilations at their register figures on B2; blocked by the gate's benchmark ticket. |
| #878 A soft block no rule classifies goes to the Decision provider, which may abstain | C4 | C4.detection_reads, C4.compilations | B2 | The soft-block heuristic reads only the facts of recovery's one detection read per navigation, and the Decision provider is asked only when it fires; keeps C4.detection_reads and C4.compilations at their register figures on B2; blocked by the gate's benchmark ticket. |
| #882 Proxy pools: `proxy.provider`, kinds, country and one proxy per Session, through the Proxy-provider driver and a `list` provider | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | Proxy bytes are charged through ledger::charging by allowances reserved ahead (Run::reserve_part), never one ledger statement per request; keeps E3.statements, E3.slice_transactions and E3.opens at their register figures on B3, its charge scenario including proxy bytes; blocked by the gate's benchmark ticket. |
| #893 One Repair for a drifted step, one compile at a time, and Consent prompts that never read as Drift | C3,C4 | C3.commands, C3.script_sends, C4.detection_reads, C4.compilations | B2 | The Consent prompt answered before a replayed step's check is the existing consent runner's (`guard.rs:201`), and the live-hiding rule is lifted through `hiding`'s compiled rules, with no detection read or compilation of its own per step; keeps `C3.commands`, `C3.script_sends`, `C4.detection_reads` and `C4.compilations` at their register figures on B2; no row counts compilations per action (a new count to add); blocked by the gate's benchmark ticket. |
| #902 Compiled `extract`: locators the model wrote once, replayed with no model call | C2 | C2.commands, C2.world_creations | B2 | A compiled `extract` reads every field's ladder of a later page in one driver read, and the schema's validator is built once per schema hash, not per page; keeps `C2.commands` and `C2.world_creations` at their register figures on B2; no row counts validator builds per page read (a new count to add); blocked by the gate's benchmark ticket. |
| #914 Adding seeds to a running crawl, one at a time or as a stream | F3,F2 | F3.invariant_reads, F3.compilations, F3.payload_serializations, F3.validations, F2.transactions | B1 | add_seed and add_seeds admit through goodfetch-frontier's admit under the Run invariants, streamed seeds committing in the run_state writer's grouped transactions, never one transaction per seed; keeps the F3 rows and F2.transactions at their register figures on B1; F3's 0 transactions of its own has no row yet; blocked by the gate's benchmark ticket. |
| #920 The Session pool scores, rotates and retires a crawl's Sessions | F2 | F2.transactions, F2.prepares, F2.writer_reads | B1 | The Session pool's scores and retirements write inside the attempt's outcome transaction (Changes::write) with cached statements, the pool held in memory; keeps F2.transactions, F2.prepares and F2.writer_reads at their register figures on B1; blocked by the gate's benchmark ticket. |
| #926 One handler on every tier: the tier is learned per Label on a domain | F2,E5 | F2.transactions, F2.prepares, E5.file_opens | B1 | Keeps `F2.transactions`, `F2.prepares` and `E5.file_opens` at their register figures on B1: the tier choice's count of requests served and each learned choice commit inside the attempt's claim or outcome through `prepare_cached` statements, never in a run-state transaction of their own (today `delivery.rs:692` and `:757`), and Provenance's tier and forced mark go through the Run-trace writer; blocked by the gate's benchmark ticket. |
| #929 Confirmation for AI Effects in the five categories, and payments Collie never makes | C2,C3 | C2.commands, C3.commands, C3.script_sends | B2 | The classification's engine rules read the target's form facts in the guard's own check read (or one driver read), and their localized word lists are compiled once when the engine starts; keeps `C2.commands`, `C3.commands` and `C3.script_sends` at their register figures on B2; no row counts word-list compilations per action (a new count to add); blocked by the gate's benchmark ticket. |
| #932 Period Limits across a Schedule's runs: clamp, reserve and skip | E3 | E3.statements, E3.slice_transactions, E3.opens | B1,B3 | The `schedule` scope's Limits are held in memory and charged from Slices like any Limit's, with the clamp done once when the Run starts: keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B3 and B1, B3's input extended with a Run under a Schedule's period Limit; blocked by the gate's benchmark ticket. |
| #936 `discovered_api` and `light_browser` serve what they can | F1 | F1.parses, F1.selector_compilations | B1 | Keeps `F1.parses` and `F1.selector_compilations` on B1 for a `light_browser` page served from its document: served calls match from the page's one analysis, as #767's decision sets; adds a count no row covers, Lightpanda processes started per page (today one per run, `light_browser.rs:9`); blocked by the gate's benchmark ticket. |
| #940 A Plugin's spend: raw units, reserved against a currency Limit, capped | E3 | E3.statements, E3.opens, E3.slice_transactions | B3 | The reservation before each Plugin call and its settlement go through `Run::reserve`/`Run::charge` into `ledger::charging` against the held Slices, with the Limits held in memory; keeps `E3.statements`, `E3.opens` and `E3.slice_transactions` at their register figures on B3, B3 gaining a priced Plugin call reserved within a Slice; blocked by the gate's benchmark ticket. |
| #945 `fetch.tier_race`: browser-fetch first, a navigation after a delay | E3,E5 | E3.statements, E3.opens, E5.file_opens, E5.spawns | B1 | Keeps `E3.statements`, `E3.opens`, `E5.file_opens` and `E5.spawns` on B1 with both racers' charges covered by held Slices and the race recorded through the Run-trace writer; with `fetch.tier_race` off, no race task is spawned (today one per race, `delivery/attempt/race.rs:70`); blocked by the gate's benchmark ticket. |
| #947 Providers from outside: unknown names caught early, the program's own provider and a Model provider Plugin | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | A Plugin's or a program's model call reserves and settles through ledger::charging, the reservation covered by held Slices; keeps E3.statements, E3.slice_transactions and E3.opens at their register figures on B3; blocked by the gate's benchmark ticket. |
| #951 Usage and prices on every AI result and error, run as OpenTelemetry spans | E3,E5 | E3.statements, E3.slice_transactions, E3.opens, E5.file_opens, E5.spawns, E5.formatting_off | B3 | Each model call's reservation and settlement go through ledger::charging, and the provider-reported charge is one entry to the Run-trace writer; keeps E3.statements, E3.slice_transactions and E3.opens on B3 and E5.file_opens, E5.spawns and E5.formatting_off at their register figures, B3's charge scenario including a model call; blocked by the gate's benchmark ticket. |
| #954 Hooks named in settings: process-Plugin Hooks around each navigation of a Collie Run | C4 | C4.detection_reads, C4.compilations | B2 | The three page stages are stages that `pages`' navigation path calls in fixed order around `recovery`'s judgement (`pages.rs:1123` into `recovery::judge`), `recovery`'s one detection read running after `after_navigation`; with `hooks` empty, a navigation sends nothing and does no Hook work. Keeps `C4.detection_reads` and `C4.compilations` at their register figures on B2, B2 gaining a run with one Hook per stage; no row yet counts Hook work per navigation with `hooks` empty (a new count to add); blocked by the gate's benchmark ticket. |
| #956 Datasets: declare, push a checked Record, commit once, numbered from 1 | F4 | F4.declaration_reads, F4.validator_builds, F4.deep_copies, F4.transactions | B1 | push checks each Record with the validators goodfetch-dataset's Datasets built when the crawl starts and commits with the attempt's outcome; keeps F4.declaration_reads, F4.validator_builds, F4.deep_copies and F4.transactions at their register figures on B1; F4's 1 validation and 1 serialization per Record have no row yet; blocked by the gate's benchmark ticket. |
| #957 `page.agent`: a bounded, journaled loop to one objective | C2,C3,E1 | C2.commands, C2.world_creations, C3.commands, C3.script_sends, E1.encodes | B2,B3 | Each Agent step observes through one `ai` driver read, acts only through the catalog's page operations and the guard, and sends its step event encoded once; keeps `C2.commands`, `C2.world_creations`, `C3.commands`, `C3.script_sends` and `E1.encodes` at their register figures on B2 and B3; blocked by the gate's benchmark ticket. |
| #958 Script Hooks: a JavaScript function in a Recipe or settings, held to approved permissions | C4 | C4.detection_reads, C4.compilations | B2 | A Script Hook's function is compiled once when its scope opens and called by handle at each stage, never compiled per navigation; keeps `C4.detection_reads` and `C4.compilations` at their register figures on B2; no row counts script compilations per navigation (a new count to add); blocked by the gate's benchmark ticket. |
| #965 The Model allowance: stop model use at a set amount while the work goes on | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | The Model allowance is held in memory by ledger::charging beside the Run's Slices and checked with each reservation, a model call the held Slices cover running no SQL statement; keeps E3.statements, E3.slice_transactions and E3.opens at their register figures on B3, its charge scenario including an allowance; blocked by the gate's benchmark ticket. |
| #979 The Agent's bounds: steps, stalls, time, Limits and the sites it may visit | C5 | C5.spawns, C5.file_opens | B2 | `agent.domains` is compiled once when the task starts and checked at the interception pipeline's step 6, the navigation gate (`interception/pipeline.rs:906`), with one match per paused document; keeps `C5.spawns` and `C5.file_opens` at their register figures on B2; C5's contract of 0 compilations per paused request has no row yet (a new count to add); blocked by the gate's benchmark ticket. |
| #994 A code-free crawl turns a site into Markdown in one Dataset | F1,F3,F4 | F1.parses, F1.selector_compilations, F1.readability_runs, F3.invariant_reads, F3.compilations, F4.declaration_reads, F4.validator_builds, F4.deep_copies, F4.transactions | B1 | Keeps the F1, F3 and F4 rows listed on B1: a code-free page's formats, fields and `a[href]` links are readers of one Page analysis, links admitted with the Run invariants, and the `pages` Record validated by the validator built at the start (today formats, each field and the links each parse through served calls, `code_free.rs:343-357`; X1, X6); blocked by the gate's benchmark ticket and #1577. |
| #998 Vision as the last rung: coordinate actions bound to their screenshot | C2 | C2.commands, C2.world_creations | B2 | With `ai.vision` off an observation takes no screenshot and sends nothing more than its one driver read; keeps `C2.commands` and `C2.world_creations` at their register figures on B2, with B2's run of `ai.vision` off; blocked by the gate's benchmark ticket. |
| #1009 Live view time and a waiting pause count under the Limits | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | Keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B3, with each `live_view` second and a waiting pause's browser time and proxy bytes charged through `ledger::charging` out of the Slices the run holds (no SQL statement per charge, no store open); blocked by the gate's benchmark ticket. |
| #1015 Vendor safety signals journaled and the opt-in injection check | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | Each injection-check call is charged through ledger::charging, and with ai.injection_check.model unset no classification call or charge happens; keeps E3.statements, E3.slice_transactions and E3.opens at their register figures on B3; blocked by the gate's benchmark ticket. |
| #1021 `search.web` over interchangeable Search providers | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | A Search provider Plugin's units are reserved and settled through ledger::charging; keeps E3.statements, E3.slice_transactions and E3.opens at their register figures on B3; blocked by the gate's benchmark ticket. |
| #1024 A crawl replays a Response recording at full speed | F1 | F1.file_opens | B1 | The recording is read and indexed once when the Run starts and each replayed page is answered from that index with no file opened and no Politeness Permit; keeps F1.file_opens at its register figure on B1, with B1 run once replaying a recording; blocked by the gate's benchmark ticket. |
| #1029 SDK handlers: the Client's `hello` declares them and the engine routes a kind's calls to them | E1 | E1.deep_copies, E1.encodes, E1.allocations | B3 | A handler call is one engine request encoded once straight into bytes, and its reply is decoded into the kind's typed answer without the deep copy `reply_answer` makes today (`rpc/framing.rs:193`); keeps `E1.deep_copies`, `E1.encodes` and `E1.allocations` at their register figures on B3, B3 gaining a handler call and reply; no row counts encodes per engine-to-Client request (a new count to add); blocked by the gate's benchmark ticket. |
| #1030 Sites' WebMCP tools as catalog operations | C2 | C2.commands, C2.world_creations | B2 | page.site_tools() is one engine read through goodfetch-page's driver, its observer registered once per Session, and no script is injected where the browser has no interface; keeps C2.commands and C2.world_creations at their register figures on B2; blocked by the gate's benchmark ticket. |
| #1032 Calls in the form their Client wrote, Human pauses and failed handlers on the Timeline | E5,C5 | E5.file_opens, E5.spawns, E5.formatting_off, C5.fetch_messages_no_rule | B2 | Keeps `E5.file_opens`, `E5.spawns`, `E5.formatting_off` and `C5.fetch_messages_no_rule` at their register figures on B2: each new trace entry (a call's Client form, a Hook call, a refused command, a request left ungated, a service worker's request) is an entry of the Run's one Run-trace writer, a request left ungated is seen from the page's Network events with no `Fetch` message, and a kind switched off formats nothing; blocked by the gate's benchmark ticket. |
| #1033 A crawl run is given a Baseline: `change_tracking.baseline` and its digest table | F4,F2 | F4.deep_copies, F4.transactions, F2.writer_reads, F2.prepares | B1 | The Baseline's table is loaded once when the Run starts and held in memory; each committed Record's digest row is written by goodfetch-dataset's commit in the outcome transaction, its content digest hashing the Record's one serialization written in RFC 8785 form; keeps F4.deep_copies, F4.transactions, F2.writer_reads and F2.prepares at their register figures on B1, with B1 run once with a Baseline; F4's 1 serialization per Record has no row yet; blocked by the gate's benchmark ticket. |
| #1040 Records marked new, changed or unchanged, with the cause of a change | F4,F2 | F4.deep_copies, F4.transactions, F2.writer_reads | B1 | mark compares with the Baseline's table held in memory since the Run started, never reading run state per Record, inside the outcome transaction; keeps F4.deep_copies, F4.transactions and F2.writer_reads at their register figures on B1, with B1 run once with a Baseline; blocked by the gate's benchmark ticket. |
| #1058 The Challenge ladder: a state machine, the timed wait, `challenge_unsolved` with its steps, and the whole ladder bounded | C4,E5 | C4.detection_reads, C4.compilations, E5.file_opens, E5.spawns, E5.formatting_off | B2 | A navigation with no Challenge does no ladder work; the ladder re-judges only on a new document through recovery's one detection read with the Page check compiled when the Session opens, and records each step as one entry to the Run-trace writer; keeps C4.detection_reads, C4.compilations, E5.file_opens, E5.spawns and E5.formatting_off at their register figures on B2, B2 meeting one challenge page; C4's 1 Provenance entry per navigation has no row yet; blocked by the gate's benchmark ticket. |
| #1065 A challenge blocked by the Filter lists is retried once with blocking off, and the vendor is flagged | C4,C5,E5 | C4.detection_reads, C5.spawns, C5.file_opens, E5.file_opens | B2 | Keeps `C4.detection_reads`, `C5.spawns`, `C5.file_opens` and `E5.file_opens` at their register figures on B2: the interception `pipeline` marks a refused request of the document in memory, `recovery` reads that mark within its one detection read, and the vendor flag is a Provenance entry through `run_trace`'s one writer; blocked by the gate's benchmark ticket. |
| #1080 The Challenge-solver driver, with CapSolver through a token-service stand-in: token injected, spend reserved and counted | C3,E3,E5 | C3.script_sends, C3.commands, E3.statements, E3.slice_transactions, E3.opens, E5.file_opens, E5.spawns | B2,B3 | Token injection runs as guarded engine operations through goodfetch-page's guard, the price is reserved through ledger::charging's reserve and settled there, and each solve is one entry to the Run-trace writer; keeps C3.script_sends and C3.commands on B2 and E3.statements, E3.slice_transactions and E3.opens on B3 at their register figures, B3's charge scenario including a paid solve; blocked by the gate's benchmark ticket. |
| #1087 Discovery clicks find links that exist only in scripts | C3,C5 | C3.script_sends, C3.commands, C5.spawns, C5.file_opens | B2 | Keeps `C3.script_sends`, `C3.commands`, `C5.spawns` and `C5.file_opens` on B2: each discovery click goes through the guard calling `check.js` by its handle, and its navigations, popups and unsafe requests are recorded and aborted by the interception pipeline's own steps, with no task spawned or file opened per aborted request; blocked by the gate's benchmark ticket. |
| #1090 Scripted interactive solvers first, in `challenge.solver`'s order, and `needs_human` re-checked before a person is asked | C3 | C3.script_sends, C3.commands | B2 | Scripted interactive steps run as Gestures through goodfetch-page's guard, calling check.js by its kept handle; keeps C3.script_sends at its register figure and gives each new step kind a C3.commands figure on B2 under #1018's decision; blocked by the gate's benchmark ticket. |
| #1095 Vision-model solving through `challenge.ask_model`, counted against the run | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | Each challenge.ask_model call is charged and reserved through ledger::charging like any model call; keeps E3.statements, E3.slice_transactions and E3.opens at their register figures on B3; blocked by the gate's benchmark ticket. |
| #1098 Code-free crawls keep screenshots, PDFs and MHTML in the Key-value store | F2,F4 | F2.transactions, F4.deep_copies, F4.transactions | B1,B2 | Keeps `F2.transactions`, `F4.deep_copies` and `F4.transactions` on B1 and B2: each stored format's key-value write is staged with the attempt and commits in its outcome (`key_value_store.rs:440`), and the Record holds only key, size and digest; adds a count no row covers, `run_files` created per page per stored format (0 with the format off); blocked by the gate's benchmark ticket. |
| #1104 Acting verbs, the Agent and `expect` on a document tier: escalate under `auto`, `browser_required` when forced | F1 | F1.parses, F1.selector_compilations | B1 | Keeps `F1.parses` and `F1.selector_compilations` at their register figures on B1: `expect` and the escalation test read the document through the Page analysis as #1097's decision sets, with no parse or selector compilation of their own per page; blocked by the gate's benchmark ticket. |
| #1106 Change tracking: only new or changed Records, soft-delete after a complete Run | F4,F2 | F4.deep_copies, F4.transactions, F2.writer_reads | B1 | A keyless Record's digest hashes its one serialization, with the Baseline's digests held in memory from the Run's start; keeps F4.deep_copies, F4.transactions and F2.writer_reads at their register figures on B1, with B1 run once with a Baseline; blocked by the gate's benchmark ticket. |
| #1109 Learn once per page layout across a crawl: one compile, a Repair on Drift, failures that end one request | F1 | F1.parses, F1.selector_compilations, F1.file_opens | B1 | Keeps `F1.parses`, `F1.selector_compilations` and `F1.file_opens` at their register figures on B1: a page's layout key and its Plan entry's ladder are read from the Page analysis's result, the ladder's selectors compile once per Run through the engine's bounded cache, and the Plan file is opened once per Run (no row counts Plan lookups per page yet); blocked by the gate's benchmark ticket. |
| #1111 A Partitioned crawl: forwarded links land once, and a handoff loses nothing | F3 | F3.invariant_reads, F3.compilations | B1 | A forwarded links batch is admitted through goodfetch-frontier's admit under the owning Partition's Run invariants; keeps F3.invariant_reads and F3.compilations at their register figures on B1; blocked by the gate's benchmark ticket. |
| #1113 Requests that use a Secret or a logged-in identity are never carried by a third-party or program backend | E5 | E5.file_opens, E5.spawns, E5.formatting_off | B1 | Keeps `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B1: the mark for a request the engine sent in a backend's place is an entry of the Run's one Run-trace writer, and a crawl with no backend formats none; blocked by the gate's benchmark ticket. |
| #1117 Fetch backend spend, reads whatever the method, and no hedging of POSTs | E3 | E3.statements, E3.slice_transactions, E3.opens | B1,B3 | Keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B1 and B3: each `fetch.request`'s `plugin_units` are reserved through `ledger::charging` out of the Slices the run holds, and a crawl with no backend reserves nothing; blocked by the gate's benchmark ticket. |
| #1131 Accounts and the three roles | E1 | E1.deep_copies, E1.instructions, E1.allocations | B3 | Keeps `E1.deep_copies`, `E1.instructions` and `E1.allocations` at their register figures on B3: the account's role and a token's scope are resolved once when the Connection signs in and held on it, so the role check beside the `person_only` check in `rpc`'s dispatch reads no store per request (no row counts store reads per message yet); blocked by the gate's benchmark ticket. |
| #1137 Escalation by proxy tier: a 403 retires the Session with its proxy and the replacement comes from the next kind | F2 | F2.transactions, F2.writer_reads, F2.prepares | B1 | A domain's tier is held in memory for the Run and its change commits with the attempt's outcome through `run_state`'s writer, never read back through the writer per request nor in a transaction of its own; keeps `F2.transactions`, `F2.writer_reads` and `F2.prepares` at their register figures on B1, B1's fixture gaining a 403 that escalates; blocked by the gate's benchmark ticket. |
| #1140 A program's own Fetch backend and Sink, registered by name | E1 | E1.deep_copies, E1.encodes, E1.instructions, E1.allocations | B3 | Keeps `E1.deep_copies`, `E1.encodes`, `E1.instructions` and `E1.allocations` at their register figures on B3, with each `fetch.request` to a program's backend and each delivery to its Sink crossing `rpc` as one encode out and one decode in (the SDKs' own framing cost per message stays ADR 0140's open question 4, measured, not capped); blocked by the gate's benchmark ticket. |
| #1144 `before_request` Hooks: change a crawl request's URL and headers from a Plugin or Script Hook | F3 | F3.invariant_reads, F3.compilations | B1 | Keeps `F3.invariant_reads` and `F3.compilations` at their register figures on B1: the Hook chain per stage and Label is resolved once when the Run starts, a stage with no Hook builds no context, and a rewritten URL is judged against the Scope, domain policy and robots rules built at the Run's start, with no read of the run file and no compilation per attempt (no row counts Hook calls per attempt yet); blocked by the gate's benchmark ticket. |
| #1145 Conditions a site set never escalate, and what blocks cost is visible in Usage and the Run report | E3 | E3.statements, E3.opens, E3.slice_transactions | B1 | Proxy bytes per kind and paid solves are charged through `Run::charge` into `ledger::charging` from the held Slices, and the Block-cause counts are in-memory counters read once for the report; keeps `E3.statements`, `E3.opens` and `E3.slice_transactions` at their register figures on B1; blocked by the gate's benchmark ticket. |
| #1150 `after_response` Hooks: catch a soft block served with a 200 | F1 | F1.parses, F1.file_opens | B1 | Keeps `F1.parses` and `F1.file_opens` at their register figures on B1: a body an `after_response` Hook rewrites is the one the Page analysis parses (one parse per page, never the original too), and with no `after_response` Hook no response is encoded for one; blocked by the gate's benchmark ticket. |
| #1151 Named tokens for programs: scoped, expiring, revocable, shown once | E1 | E1.deep_copies, E1.instructions, E1.allocations | B3 | Keeps `E1.deep_copies`, `E1.instructions` and `E1.allocations` at their register figures on B3: a named token's scope, expiry and Limits are read once when its Connection opens and held on it, and a revocation reaches live Connections from the store's own write, so dispatch checks each request with no store read (no row counts store reads per message yet); blocked by the gate's benchmark ticket. |
| #1154 `before_record_commit` Hooks: clean, drop or reject a Record before it is committed | F4 | F4.validator_builds, F4.deep_copies, F4.transactions | B1 | Keeps `F4.validator_builds`, `F4.deep_copies` and `F4.transactions` at their register figures on B1: the Hook runs in `Datasets`' commit path with the validators built at the crawl's start, the schema check runs once, on the Hook's result, the Record is encoded for the Hook without a deep copy and commits in the outcome's transaction (no row counts validations per Record yet); blocked by the gate's benchmark ticket. |
| #1157 `crawl.trap_links.avoid` off, recorded and listed, with Site preferences still obeyed | F3 | F3.invariant_reads, F3.compilations | B1 | With `crawl.trap_links.avoid` off, `trap_links::judge` runs on no link of a links batch and nothing of it is written; keeps `F3.invariant_reads` and `F3.compilations` at their register figures on B1, B1 gaining a run with avoidance off; blocked by the gate's benchmark ticket. |
| #1158 `request_ended` Hooks: the failed-request handler for Plugins and Script Hooks | F2,E5 | F2.transactions, E5.file_opens, E5.spawns | B1 | Keeps `F2.transactions`, `E5.file_opens` and `E5.spawns` at their register figures on B1: the observe-only Hook runs after the outcome commits and adds no transaction, its `hook_failed` is an entry of the Run-trace writer, and a crawl with no `request_ended` Hook builds no outcome context; blocked by the gate's benchmark ticket. |
| #1164 Page-stage Hooks from settings on a crawl, with the request and its Label | F1 | F1.parses, F1.selector_compilations | B1 | Keeps `F1.parses` and `F1.selector_compilations` at their register figures on B1: `page_ready`'s page calls on a document tier are answered from the Page analysis as #1097's decision sets, and a crawl with no page Hook builds nothing for one; blocked by the gate's benchmark ticket. |
| #1167 Vector-store Sinks chunk and embed once, starting with pgvector | E3 | E3.statements, E3.opens, E3.slice_transactions | B3 | Keeps `E3.statements`, `E3.opens` and `E3.slice_transactions` on B3: each embedding call's currency charge is taken from held Slices with no SQL statement and its journal entry commits with the delivery's batch, not in a transaction per call; blocked by the gate's benchmark ticket and #1121's decision. |
| #1205 A managed fetching service through a Fetch backend: judged by block detection, counted in Usage, never given a credential | F1,E3 | F1.parses, F1.file_opens, E3.statements, E3.opens | B1 | A Fetch backend's response enters the same Page analysis as the engine's own, block detection reading it there with no parse of its own, and its spend is charged through `Run::charge` into `ledger::charging` from the held Slices; keeps `F1.parses`, `F1.file_opens`, `E3.statements` and `E3.opens` at their register figures on B1; blocked by the gate's benchmark ticket. |
| #1217 Full-text search in a run, and search across runs | F4,F2 | F4.transactions, F4.deep_copies, F2.prepares, F2.transactions | B1 | Keeps `F4.transactions`, `F4.deep_copies`, `F2.prepares` and `F2.transactions` at their register figures on B1: the FTS5 rows are written inside `records::commit`, in the outcome's transaction, through `prepare_cached` statements and from the Record's one serialization; indexing is a setting whose off state writes no index row (ADR 0138), and no row counts index writes per Record yet; blocked by the gate's benchmark ticket. |
| #1304 `page.newCDPSession()`: the way down, with the main path's protections | C1,E1 | C1.file_opens, C1.log_sends_off, E1.deep_copies, E1.encodes | B2,B3 | Every command a CDPSession sends goes through goodfetch-cdp's connection (send_raw), the C1 seam, never a socket of its own; keeps C1.file_opens and C1.log_sends_off on B2 and E1.deep_copies and E1.encodes on B3 at their register figures, a forwarded event costing one encode; C1's 1 encode and 1 send per command have no row yet; blocked by the gate's benchmark ticket. |
| #1306 `host_platform`: the detected target, supervision and a CPU-throttle warning | E4 | E4.configuration_reads, E4.static_opens, E4.meminfo_opens | B3 | The CPU-throttle check is a stage of `governor`'s sampler: `governor.cpu_throttle_warning` read from the settings snapshot, `/proc/self/cgroup` parsed once when the engine starts, and the cgroup's `cpu.stat` read at most once per sample (nothing with the setting at 0): keeps `E4.configuration_reads`, `E4.static_opens` and `E4.meminfo_opens` at their register figures on B3, its input extended with a throttling `cpu.stat`; `cpu.stat` opens per sample are a new count no row covers (add `E4.cpu_stat_opens`, at most 1); blocked by the gate's benchmark ticket. |
| #1375 Code mode: one tool for a model-written program in the Script sandbox | E1,E5 | E1.deep_copies, E1.encodes, E5.file_opens, E5.spawns, E5.formatting_off | B2,B3 | Each catalog call a Code mode program makes enters through `rpc`'s dispatch, with no dispatch of its own, and its Run-trace entry goes through `run_trace`'s one writer per Run: keeps `E1.deep_copies`, `E1.encodes`, `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B3 and B2, B2's input extended with a Code mode program; blocked by the gate's benchmark ticket. |
| #1386 Prefactor: a ledger driver above the local, Lead Daemon and caller backings | E3 | E3.statements, E3.slice_transactions, E3.opens | B1,B3 | Keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their figures on B1 and B3: `ledger::charging` stays the per-charge seam, covering charges from held Slices with no SQL statement, and calls the driver only when a Slice is taken; blocked by the gate's benchmark ticket. |
| #1390 Caller-held Usage ledger tracer: Limit slices from the caller over the run's connection | E3,E1 | E3.statements, E3.opens, E1.encodes | B3 | Keeps `E3.statements`, `E3.opens` and `E1.encodes` on B3: under `usage.ledger: caller` a charge the held Slices cover sends no message; adds a count no row covers, `$/slice` and `$/usage_report` messages per Slice taken (1 each), with 0 local transactions per Slice; blocked by the gate's benchmark ticket. |
| #1396 Limit thresholds and cookie-expiry warnings | E3 | E3.statements, E3.slice_transactions, E3.opens | B1,B3 | Detects a Limit threshold crossing from the Limits and Slices `ledger::charging` already holds, writing the crossing only in a Slice-take transaction: keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B3 and B1, B3's input extended with a charge that crosses 50 %; blocked by the gate's benchmark ticket. |
| #1401 `GET /v1/events`: follow a Run as Server-Sent Events from a position | E1 | E1.encodes, E1.allocations | B3 | Streams each event through `rpc`'s event-stream writer and credits (`rpc/event_stream.rs`), encoded once whichever transport carries it, with no event writer of the listener's own: keeps `E1.encodes` and `E1.allocations` at their register figures on B3, its input extended with a Run followed over `GET /v1/events`; blocked by #1400's rewrite and the gate's benchmark ticket. |
| #1441 Foxhound's values in the caller-held ledger: Paid fetches and the Partitioned crawl scope | E3 | E3.statements, E3.opens, E3.slice_transactions | B1,B3 | Whether a Paid fetch may pay is answered by `ledger::charging` from the rows and setters the held Slices carry, never a read of the Limits per charge, and the must-understand flag is checked when a Slice is taken: keeps `E3.statements`, `E3.opens` and `E3.slice_transactions` at their register figures on B3 and B1, B3's input extended with a `paid_fetch` charge; blocked by the gate's benchmark ticket. |
| #1448 Partition forwarding through the caller: assign, forward and deliver with epochs and credits | F3,E1 | F3.invariant_reads, F3.compilations, F3.payload_serializations, F3.validations, E1.encodes, E1.deep_copies | B1,B3 | A delivered batch is admitted by `goodfetch-frontier`'s admission as one links batch with the Run invariants built when the Partition's Run started, and the partition map is held in memory; each forward and delivery is one `$/` message encoded once: keeps `F3.invariant_reads`, `F3.compilations`, `F3.payload_serializations`, `F3.validations`, `E1.encodes` and `E1.deep_copies` at their register figures on B1 and B3, B1's input extended with a forwarded batch; blocked by the gate's benchmark ticket. |
| #1453 A crawl's Limit and Model allowance span its Partitions | E3 | E3.statements, E3.opens, E3.slice_transactions | B1,B3 | Each model call reserves against the allowance slice held in memory in the same `ledger::charging` step as its Limits, with no SQL statement or open per call the held slice covers, and one `$/allowance_slice` per slice taken: keeps `E3.statements`, `E3.opens` and `E3.slice_transactions` at their register figures on B3 and B1, B3's input extended with model charges under an allowance; allowance slices taken per model call are a new count no row covers; blocked by the gate's benchmark ticket. |
| #1475 Escalation to the browser and `browser_required` | C3,C5,F3 | C3.script_sends, C3.commands, C5.fetch_messages_no_rule, C5.spawns, C5.file_opens, F3.invariant_reads, F3.compilations | B1,B2 | `enqueueLinksByClickingElements` clicks through `goodfetch-page`'s guard (`guard.rs`) with `check.js` called by its handle, aborts unsafe requests through the interception `pipeline` armed only for the call's duration, and admits its links as one links batch through `goodfetch-frontier`'s admission with the Run invariants; keeps `C3.script_sends`, `C3.commands`, `C5.fetch_messages_no_rule`, `C5.spawns`, `C5.file_opens` on B2 and `F3.invariant_reads`, `F3.compilations` on B1 at their register figures, B2 gaining the click-to-enqueue action kind; blocked by the gate's benchmark ticket. |
| #1476 Content types and the responses a page kept | C5 | C5.fetch_messages_no_rule, C5.spawns, C5.file_opens | B2 | Responses a Label keeps (`fetch.kept_responses.urls`) are kept by the interception `pipeline` (or the page's events reader) with the URL patterns compiled once per Session, so a Label with no kept URLs sends no `Fetch` message and a kept body costs one `getResponseBody` and no file open; keeps `C5.fetch_messages_no_rule`, `C5.spawns` and `C5.file_opens` at their register figures on B2; adds compilations per paused request, which C5's contract caps at 0 but no row covers (add `C5.compilations`); blocked by the gate's benchmark ticket. |
| #1484 Fetch backends written as code | E1,E5 | E1.encodes, E1.deep_copies, E1.allocations, E5.file_opens, E5.spawns, E5.formatting_off | B1,B3 | Each Fetch-backend call is one handler request through `goodfetch-engine`'s `rpc` (body bytes encoded once, the answer decoded straight into typed params) and each engine-sent request's mark goes through `run_trace`'s one writer per Run; with no backend registered nothing is sent or formatted; keeps `E1.encodes`, `E1.deep_copies`, `E1.allocations` on B3 (gaining a backend request and answer with a body as large params) and `E5.file_opens`, `E5.spawns`, `E5.formatting_off` on B1 at their register figures; blocked by the gate's benchmark ticket. |
| #1486 Datasets, `push` and Records that commit once with the attempt | F4 | F4.validator_builds, F4.deep_copies, F4.transactions | B1 | Keeps `F4.validator_builds`, `F4.deep_copies` and `F4.transactions` on B1, and at most 1 engine validation and 1 serialization per pushed Record (counts with no register row yet): an SDK's native schema check never makes the engine validate twice; blocked by the gate's benchmark ticket. |
| #1491 Sinks written as code, and the Rust call streams | E1,F2 | E1.encodes, E1.deep_copies, E1.allocations, F2.writer_reads | B1,B3 | Each Sink-handler delivery is one handler request through `goodfetch-engine`'s `rpc`, its Records read from committed state through the run file's read connections (never the writer) and encoded once into the message's bytes; keeps `E1.encodes`, `E1.deep_copies`, `E1.allocations` on B3 (gaining a `SinkDelivery` of Records as large params) and `F2.writer_reads` on B1 at their register figures; blocked by the gate's benchmark ticket. Sink delivery itself has no Shared path in ADR 0140 (area 7's tickets). |
| #1503 Replay a whole crawl from a Response recording, with no network and no model | F1 | F1.file_opens | B1 | A replay Run opens and indexes its Response recording once when the Run starts and serves each request from that held index, opening no file per request; keeps `F1.file_opens` at its register figure on B1 run in replay mode; blocked by the gate's benchmark ticket. |
| #1525 The record-range trigger and the listener's Records route | E1,F2 | E1.encodes, E1.deep_copies, E1.allocations, F2.writer_reads | B3 | The listener's Records route answers through `composition`'s dispatch of `run.records` (one dispatch, Records encoded once straight into the answer's bytes) and reads through the run file's read connections, never a store opened per HTTP request nor the writer; keeps `E1.encodes`, `E1.deep_copies`, `E1.allocations` and `F2.writer_reads` at their register figures on B3, gaining a 1,000-Record answer as large params; blocked by the gate's benchmark ticket. |
| #1541 Politeness waits under ADR 0118: decorrelated jitter and a bound on a site's `Retry-After` | E5 | E5.file_opens, E5.spawns, E5.formatting_off | B1 | Keeps `E5.file_opens`, `E5.spawns` and `E5.formatting_off` at their register figures on B1: the backoff draw and the `Retry-After` wait are written in the Permit's give-back transaction the attempt already takes, adding none, and an unparsable `Retry-After` is noted through `run_trace`'s one writer; the Politeness file's own counts await #239's decision; blocked by the gate's benchmark ticket. |
| #1546 Hooks in the engine under ADR 0104: the order across forms, `timeout.hook`, retries, a failed Record-stage commit and the `registered` origin | F2,F4 | F2.transactions, F4.transactions, F4.deep_copies, F2.configuration_reads | B1 | Keeps `F2.transactions`, `F2.configuration_reads`, `F4.transactions` and `F4.deep_copies` at their register figures on B1: each stage's Hook chain is built once when the Run starts from its resolved settings, a Record-stage Hook reads the Record without copying it and its failure discards the attempt's staged outputs inside the outcome transaction, adding none; blocked by the gate's benchmark ticket. |
| #1547 Run progress with the reasons a Run waits, and reads of a Run the program does not hold | E1,F2 | E1.encodes, E1.allocations, F2.writer_reads | B1,B3 | Keeps `E1.encodes`, `E1.allocations` and `F2.writer_reads` at their register figures on B1 and B3: progress is built from the engine's in-memory counts and encoded once per value sent, coalesced under load, and the reads of a Run no program holds go through the read-only connections, never the writer; blocked by the gate's benchmark ticket. |
| #1550 Declared Effects under ADR 0106: `effect.open` and `effect.close`, the Effect journal's states and reconciliation | F2 | F2.transactions, F2.syncs | B1 | Keeps `F2.transactions` and `F2.syncs` at their register figures on B1 for attempts with no Effect; the Effect journal's sent mark is the synced commit ADR 0138 keeps per Effect, a count per Effect that no row covers yet; blocked by the gate's benchmark ticket. |
| #1551 Web Bot Auth signing keys (ADR 0045): an `identified` Session signs its requests with the operator's key | C5 | C5.fetch_messages_no_rule, C5.spawns, C5.file_opens | B2 | Signing is a step of goodfetch-page's interception pipeline, on only in an identified Session, its key resolved once when the Session opens; a Session that is not identified sends no Fetch message for it; keeps C5.fetch_messages_no_rule, C5.spawns and C5.file_opens at their register figures on B2, with B2 run once with an identified Session; C5's 0 compilations and 1 CDP answer per paused request have no row yet; blocked by the gate's benchmark ticket. |
| #1566 Storage levels, the sampler, the low and critical stops and the Daemon log rotation (ADR 0122) | E4 | E4.configuration_reads, E4.static_opens | B3 | Keeps `E4.configuration_reads` and `E4.static_opens` at their register figures on B3: the storage sample takes `storage.*` from the engine's settings snapshot, never the files, and reads each filesystem once per sample from the folder list built when each Run starts; blocked by the gate's benchmark ticket. |
| #1596 A Lent page's request the interception no longer sees goes ungated, and the Run trace says so | E5 | E5.file_opens, E5.spawns, E5.formatting_off | B2 | Keeps `E5.file_opens`, `E5.spawns` and `E5.formatting_off` on B2: an ungated request's Run-trace entry goes through the Run-trace writer's kept handle with no task spawned per entry; blocked by the gate's benchmark ticket. |
| #1602 The built-in `fetch.tier: auto` takes the engine's tier choice, and crawl tests reach a site | F2 | F2.transactions, F2.prepares | B1 | Keeps `F2.transactions` and `F2.prepares` on B1 now that `auto` puts the tier choice on every default crawl: its per-attempt count commits in the attempt's claim or outcome, never in its own transaction (today `delivery.rs:692`); blocked by the gate's benchmark ticket. |
| #1604 The browser tier follows a crawl request's redirects and refresh stubs as the `http` tier does | F1,F3,C5 | F1.parses, F3.invariant_reads, F3.compilations, C5.spawns | B1,B2 | Keeps `F1.parses`, `F3.invariant_reads`, `F3.compilations` and `C5.spawns` on B1 and B2: the browser tier takes a refresh stub from the document's one read (see #762's rewrite) and checks each hop with the Run invariants; blocked by the gate's benchmark ticket and #762. |
| #1611 The run file connection's statement cache fits the Frontier's statements | F2 | F2.prepares | B1 | Keeps `F2.prepares` at 0 per page on B1: the run file connection's statement cache is sized where `run_state` opens it to hold every statement the Frontier's admission and `rules::learn` run, removing that part of X5; blocked by the gate's benchmark ticket. |
| #1612 The guard's check is called by reference, not sent as source on every check | C3 | C3.script_sends, C3.commands | B2 | Brings `C3.script_sends` to 0 per check on B2 and keeps `C3.commands` at its register figure: `check.js` is evaluated once per execution context and called by its kept handle, made again only when Chromium reports the context created, destroyed or cleared; removes X11 and writes the row's figure; blocked by the gate's benchmark ticket. |
| #1626 Retire crawl_request.handle's answer records in favour of crawl_request.push | F4,E1 | F4.declaration_reads, F4.validator_builds, F4.deep_copies, F4.transactions, E1.deep_copies, E1.encodes | B1,B3 | Records reach goodfetch-dataset's Datasets through crawl_request.push only, one entry point; keeps the F4 rows on B1 and E1.deep_copies and E1.encodes on B3 at their register figures, B3's large-params scenario being a crawl_request.push; blocked by the gate's benchmark ticket. |
| #1629 Record a dedicated or service worker's domain-policy refusal in the Run trace | C5,E5 | C5.spawns, C5.file_opens, E5.file_opens, E5.spawns | B2 | Keeps `C5.spawns`, `C5.file_opens`, `E5.file_opens` and `E5.spawns` on B2: a worker's domain-policy refusal is recorded through the Run-trace writer's kept handle with no task spawned per refusal (not `pipeline.rs:1731`'s spawn); blocked by the gate's benchmark ticket. |
| #1637 The sample comparison's blocking-off sample keeps blocking off for a Label that needs it | F2,F4 | F2.transactions, F4.deep_copies | B1 | Keeps `F2.transactions` and `F4.deep_copies` on B1: the learned blocking-off flag commits with the committing attempt's outcome, not in its own transaction, and the removal report feeds weighing without cloning Records; blocked by the gate's benchmark ticket and #1636. |
| #1641 A Daemon accepts Clients through a relay it connects out to | E1 | E1.deep_copies, E1.encodes, E1.instructions, E1.allocations | B3 | Keeps `E1.deep_copies`, `E1.encodes`, `E1.instructions` and `E1.allocations` at their register figures on B3: a relayed message enters `rpc`'s one framing and dispatch as a carriage of the Connection (one decode into typed params, one encode per answer or event), its end-to-end encryption adds one seal and one open per message, counted beside them (no row yet), and with the relay off nothing of it runs; blocked by the gate's benchmark ticket. |
| #1642 An engine checks a person's passkey on any Connection, relayed or from another origin | E1 | E1.deep_copies, E1.instructions, E1.allocations | B3 | Keeps `E1.deep_copies`, `E1.instructions` and `E1.allocations` at their register figures on B3: the passkey assertion is checked once per session key, and each call's signature is verified in `rpc`'s framing against the key held on the Connection, with no store read and no second decode (no row counts signature checks per message yet); blocked by the gate's benchmark ticket. |
| #1651 Browser time is a Usage kind on the wire, with a stated way to count it | E3 | E3.statements, E3.slice_transactions, E3.opens | B3 | Keeps `E3.statements`, `E3.slice_transactions` and `E3.opens` at their register figures on B3: browser time is charged through `ledger::charging` out of the Slices the run holds (no SQL statement or store open per charge), at most one charge per browser per second; blocked by the gate's benchmark ticket. |
| #1652 Bytes a run moves without a proxy count in Usage | E3,C5 | E3.statements, E3.slice_transactions, E3.opens, C5.fetch_messages_no_rule | B1,B2,B3 | Keeps `E3.statements`, `E3.slice_transactions`, `E3.opens` and `C5.fetch_messages_no_rule` at their register figures on B1, B2 and B3: direct bytes are charged through `ledger::charging` out of held Slices as proxy bytes are, and a browser's direct bytes are read from its Network events, adding no `Fetch` message to a page with no route or Filter list; blocked by the gate's benchmark ticket. |
| #1653 The light-browser tier serves a live Lightpanda page | C1,C2,C5 | C1.file_opens, C1.log_sends_off, C2.commands, C2.world_creations, C5.spawns, C5.file_opens | B2 | Keeps the C1, C2 and C5 rows listed on B2: Lightpanda's CDP server is driven through `goodfetch-cdp`'s connection and `goodfetch-page`'s driver and interception, never a second CDP client; adds a count no row covers, Lightpanda processes per page, which should become one `lightpanda serve` per Session (today one per run, `light_browser.rs:9`); blocked by the gate's benchmark ticket. |
| #1666 A Recipe's manifest declares the sites, Secret references and Plugins it reaches, and the engine refuses the rest | C5,C4,F3 | C5.fetch_messages_no_rule, C5.spawns, C5.file_opens, C4.compilations, F3.invariant_reads, F3.compilations | B1,B2 | Keeps `C5.fetch_messages_no_rule`, `C5.spawns`, `C5.file_opens`, `C4.compilations`, `F3.invariant_reads` and `F3.compilations` at their register figures on B1 and B2: the manifest's declared domains are compiled once when the Run starts into the domain policy that interception's step 3 and the crawl's admission already apply, with no read of the manifest and no compilation per request, navigation or links batch, and a Run with no declaration adds nothing; blocked by the gate's benchmark ticket. |
| #1674 A Client follows an engine's Run index and Usage ledger from a cursor | E3,E1 | E3.statements, E3.opens, E1.encodes | B3 | Keeps `E3.statements`, `E3.opens` and `E1.encodes` at their register figures on B3: the feed publishes Usage ledger changes from the ledger's own Slice transactions and Run index writes, never a read per charge, and each change is encoded once as an event; blocked by the gate's benchmark ticket. |
| #1692 Refuse a Record field named #wall, or rename the Wall marker's field | F4 | F4.deep_copies, F4.validator_builds | B1 | The check of a top-level field starting with # runs in push's one validation, with no copy of the Record's data; keeps F4.deep_copies and F4.validator_builds at their register figures on B1; blocked by the gate's benchmark ticket. |
| #1693 A host can gate every request an engine sends to a site | C5,F2 | C5.fetch_messages_no_rule, C5.spawns, C5.file_opens, F2.transactions | B1,B2 | Keeps `C5.fetch_messages_no_rule`, `C5.spawns`, `C5.file_opens` and `F2.transactions` at their register figures on B1 and B2: the host's gate is a stage interception's `pipeline` and the crawl attempt call after Politeness, built once per Run, waits with no task spawned and reports outcomes on the existing request events; with no gate supplied no `Fetch` message and no call is made (no row counts gate calls per request yet); blocked by the gate's benchmark ticket. |
| #1694 A host can refuse a site's requests with its own reason while the run goes on | C5,E5 | C5.spawns, C5.file_opens, E5.file_opens, E5.spawns | B1,B2 | Keeps `C5.spawns`, `C5.file_opens`, `E5.file_opens` and `E5.spawns` at their register figures on B1 and B2: a host's refusal of a page's request goes through the interception pipeline's refusal path (0 tasks spawned, 0 files opened, 1 entry to the Run-trace writer), and a crawl request's ends `skipped` in its outcome's transaction; blocked by the gate's benchmark ticket. |
| #1715 Smaller page views for cheap models: `observe` scoped to one element and `find` over the `ai` snapshot | C2 | C2.commands, C2.world_creations | B2 | Keeps `C2.commands` and `C2.world_creations` at their register figures on B2: a scoped `observe` is one read through the driver's `read_in_frame` (one `Runtime.callFunctionOn` in the registered world), and `find` searches the snapshot that read returns, sending no command of its own; blocked by the gate's benchmark ticket. |
| #1747 A race's navigation can be charged in Usage and never sent | F2,E3 | F2.transactions, F2.writer_reads, E3.statements, E3.opens | B1 | The race's reservation and its settlement go through `ledger::charging` on the held Slices and journal with the attempt's claim and outcome transactions, so the fix adds no run-state transaction of its own and a settle at 0 writes nothing extra: keeps `F2.transactions`, `F2.writer_reads`, `E3.statements` and `E3.opens` at their register figures on B1, its input extended with a tier race; blocked by the gate's benchmark ticket. |
| #1775 An HTTP run that escalates after its Page check still teaches the Frontier | F2,F3 | F2.transactions, F2.writer_reads, F3.invariant_reads | B1 | Keeps `F2.transactions`, `F2.writer_reads` and `F3.invariant_reads` on B1: the page's lesson is staged with the attempt and commits in its outcome, so a discarded run teaches nothing (today its own transaction before the handler, `delivery/attempt/learning.rs:41`); blocked by the gate's benchmark ticket. |
| #1782 Keep the event stream's last lossless events serialized, not as JSON values | E1 | E1.encodes, E1.allocations | B3 | Keeps `E1.encodes` and `E1.allocations` at their register figures on B3: the kept window holds each event's one encoding, the bytes each subscriber is sent, with no intermediate `Value` and never parsed back to build an envelope; blocked by the gate's benchmark ticket. |
| #1784 Foxhound crawl: reuse loaded filter lists across Runs, and cut per-URL cosmetic-rule and per-admission lease costs | F1,F2 | F1.selector_compilations, F1.file_opens, F2.transactions, F2.configuration_reads | B1 | Keeps `F1.selector_compilations`, `F1.file_opens`, `F2.transactions` and `F2.configuration_reads` at their register figures on B1: loaded Filter lists are held at the engine's scope, keyed by Filter list version within a documented bound, the clean-up reads a URL's cosmetic resources once per page, and leasing reads no settings per admission; blocked by the gate's benchmark ticket. |

## Untouched

| Ticket | Reason |
|---|---|
| #138 `xtask release` prepare: one version, changelog and release pull request | Release tooling; no runtime work. |
| #139 `xtask release` branches, backports and promotion check | Release branch tooling; no runtime work. |
| #140 `xtask release` publish plan against local registry stand-ins | Publish plan tooling; no runtime work. |
| #141 Build every target once and rehearse publishing locally | Build and publish rehearsal; no runtime work. |
| #142 Platform signing jobs and the Platform signature gate | Signing jobs in CI; no runtime work. |
| #143 Nightly channel published end to end | Nightly publishing pipeline; no runtime work. |
| #144 Stable, release-candidate and patch releases | Release channels pipeline; no runtime work. |
| #146 Nightly test tier and the named-egress jobs | Nightly test tier in CI. |
| #147 Release-candidate benchmark job on the reference host | Release-candidate benchmark job; measures, adds no runtime work. |
| #148 API breakage checks against the previous release | API breakage checks in CI. |
| #149 Docs site on Vercel with versions and agent formats | Docs site build and hosting. |
| #150 Generated reference pages and tested examples | Generated reference pages and tested examples. |
| #168 Proving ground online copy behind real challenge vendors, with real consent vendors' prompts | Proving ground online deployment. |
| #171 Conformance target: a process Plugin | Conformance harness target. |
| #173 `bench/` tracer: the lab's campaign runner, judged by the Proving ground's oracle | bench/ campaign runner. |
| #174 `bench/`: statistics, verdicts, claims and ledger moved from the lab | bench/ statistics. |
| #175 `bench/`: three views, counts and the cost model | bench/ reporting views. |
| #176 `bench/`: the release-gate decision | bench/ release-gate decision. |
| #177 `bench/`: the `reference` host class and published runs | bench/ host class gates. |
| #178 `bench/`: load runs and the concurrency search | bench/ load runs and concurrency search. |
| #179 `bench/`: publication of results | bench/ publication. |
| #180 Competitor adapters: Crawlee JavaScript and Python, Scrapy, Scrapling, Firecrawl self-hosted | Competitor adapters. |
| #181 Competitor adapters: Spider and Colly | Competitor adapters. |
| #182 Competitor adapters: Playwright, Stagehand v4, Vercel `agent-browser` | Competitor adapters. |
| #183 Competitor adapters: Crawl4AI, Browser Use, ScrapeGraphAI, Skyvern, changedetection.io | Competitor adapters. |
| #184 Goodfetch adapter for `bench/`, with default and matched rows | Goodfetch bench adapter through public surfaces. |
| #185 Benchmark campaigns: HTTP crawl, rendered crawl and Extraction | Benchmark campaigns. |
| #186 Benchmark campaigns: scripted actions, remote browser and the guard's measurements | Benchmark campaigns. |
| #187 Benchmark campaigns: AI use cases across model and browser locality | Benchmark campaigns. |
| #188 Benchmark campaigns: runtime families and the cost of each control | Benchmark campaigns. |
| #189 Benchmark campaigns: the Access family on both rungs | Benchmark campaigns. |
| #202 Five OCI image tags, the Helm chart artifact and the seccomp profile | Image tags, Helm chart and seccomp profile publishing. |
| #203 Release-profile builds, debug symbols and the ceilings before every publish | Release builds, symbols and size ceilings. |
| #204 Size increase check in the merge queue and the pull-request estimate | Size increase check in the merge queue. |
| #205 Nightly Python index on `goodfetch.dev` | Nightly Python index hosting. |
| #206 Deployment-target smoke runs in the merge queue | Deployment smoke runs in the merge queue. |
| #207 Clean-up rules in every release | Clean-up rules packaging per release. |
| #208 Clean-up rules daily-channel job | Clean-up rules daily channel job in CI. |
| #209 Release-candidate Cloud Run deploy | Release-candidate Cloud Run deploy. |
| #210 V8 built from source and the V8 currency gate | V8 build and currency gate in CI. |
| #211 Conformance: gate mode for the Clean-up rules' daily channel | Conformance gate mode. |
| #212 `plugins test` runs the suites from each Product's installed executable | `plugins test` command and embedded suites. |
| #218 Checkpoints, resume on another machine and run epochs | Checkpoints and run epochs are per Run or interval, not per unit. |
| #220 Daemon on the machine: discovery, token, idle stop and per-major isolation | Daemon discovery, pick and idle stop are per Daemon or connection. |
| #222 Attached engines die with their Client, and `workers=N` share one run | Process lifetime and private endpoint; work items lease through existing delivery. |
| #226 Contract versions, `upgrade-files` and the age notice | Version markers checked on shared stores' transactions, none per page or message. |
| #229 Checkpoints on deployed hosts: object storage, filesystem refusal, ephemeral targets | Checkpoint destinations and filesystem refusal are per Run. |
| #230 Ops listener, metrics and logging | Ops listener and probes are per scrape; metric increments are not counted work. |
| #237 Governor default ceilings from the concurrency search | Sets default ceilings from a measurement; no per-unit work. |
| #241 Politeness store outages: granted shares and bounded degraded admission | Heartbeats per engine; degraded shares checked in memory, no per-request store work. |
| #242 Daemon group formation and the peer link | Group formation and peer link are per Group member. |
| #243 Group control state: replicated stores, group-wide Limits and an unreachable lead | Replicated stores push per change; settings stay cached per member. |
| #245 Scale-down, drain and `runs adopt` in a group | Scale-down and adoption are per run handoff. |
| #246 Group backups, promotion and lead fencing | Group backups and promotion are per interval or operator action. |
| #247 Daemon groups on Kubernetes | Kubernetes join and lead confirmation are per pod. |
| #249 Partition handoff when Group members change | Domain handoff is per membership change, not per URL. |
| #250 Script sandbox tracer: a program runs in a confined V8 worker | Script worker pools start once per Run; per program, not a Shared path. |
| #251 Script sandbox limits, failures and quarantine | Script limits and quarantine are per script call. |
| #252 What a script can use: language, globals and SES Compartments | Script Hook Compartments are built once per Run. |
| #253 Measure V8's sandbox cost and the musl slowdown on the reference host | Measurement campaign only. |
| #254 Raw CDP attaches bound to their work item's lease | Raw CDP attach is per attach; cuts audited rarely. |
| #259 Browser cache: pinned, digest-checked builds, `browsers install`, `browsers clean` and the offline path | Browser cache checks run per launch or download. |
| #289 M1 owner test: Foxhound's crawls and extraction, with both Products' Daemon, Console and settings | Owner's hands-on milestone test; no code. |
| #290 M2 owner test: the TypeScript, Python and Rust SDKs, MCP and skills | Owner's hands-on milestone test; no code. |
| #291 M3 owner test: browser automation | Owner's hands-on milestone test; no code. |
| #313 `bench/`: the Performance series on the owner's machine | Nightly Performance series. |
| #320 Private fuzzing repository and the private gates App | Private fuzzing repository and gates App. |
| #328 Proving ground: probe pages, codec samples, font pages and a Google-host stand-in for the Goodfetch Chromium's tests | Proving ground pages. |
| #344 LazyFS setup: a real executable power-cut on Linux | Test infrastructure; no product per-unit work. |
| #346 Journey variants from runner listings: the one-test-per-variant rule, `xtask journeys select` and `just journeys` | Test infrastructure; no product per-unit work. |
| #354 Feature report: tested, untested by cause, excepted and planned, per area and milestone | Test reporting; no product per-unit work. |
| #355 `xtask journeys report` and the owner test's generated "Journeys to try" | Test reporting; no product per-unit work. |
| #356 Fault plans: the format, the shrinking generator and a replay command | Test infrastructure; no product per-unit work. |
| #357 The remaining invariant checkers: one lead, Partitions, the Audit log chain and bounded liveness | Test invariant checkers; no product per-unit work. |
| #358 Toxiproxy setup: cut off one engine or partition one Group member on every OS | Test infrastructure; no product per-unit work. |
| #359 Simulation harness: turmoil's adapter of `transport`, the seed streams and the determinism check | Simulation harness, test-only crate; no product code. |
| #360 TLA+ trace writer and trace validation of the Simulation's traces | Test-only trace writer for Simulation; no product path. |
| #392 Owner confirmations and decisions recorded by the test infrastructure spec | Owner decisions on test infrastructure. |
| #393 `xtask chromium surfaces check` and `protocol check`: no untested patch, no unknown CDP method | xtask Chromium checks, build time. |
| #394 Python in-process player: `play_script` over an in-memory transport | Test infrastructure. |
| #395 Launch-argument codegen: the gate's WPT runs the browser as the engine does | Launch-argument codegen for WPT, build time. |
| #397 `xtask chromium expectations check` and the upstream exclusions' rules | xtask Chromium expectations check, CI. |
| #398 `xtask chromium package`: each suite's runtime deps packaged by digest | Chromium suite packaging, CI. |
| #399 pytest case-log writer for Client conformance cases | Test infrastructure. |
| #400 Chromium build gate workflow: tree hashes, verdict attestations and the fast path | Chromium build gate workflow, CI. |
| #401 Python async helpers: `anyio_backend` over asyncio and trio, three-way cases and the free-threaded check | Test infrastructure. |
| #402 Goodfetch Chromium build: the gate's build-and-test path and the first measured build | Chromium build-and-test path, CI. |
| #403 TypeScript Journey helper for `sdk-typescript` variants | Test infrastructure. |
| #404 Goodfetch Chromium test helpers: identity consistency, codec and font samples, `NetLog::read` | Test-support helpers for Chromium surfaces. |
| #405 Python Journey helper for `sdk-python` variants | Test infrastructure. |
| #406 `goodfetch-scripted-model`: the engine's Scripted model at the Model provider interface | Test-only Scripted model crate. |
| #407 Model recordings replayed by `httpmock`: `Recordings::serve`, stream re-cuts and synthetic faults | Test-only Model recordings replay. |
| #408 Install harness, npm side: packed packages from a local Verdaccio through five managers | Install harness; test infrastructure. |
| #409 `xtask recordings check`: no leaked header or key, no missing matrix cell | xtask recordings check, CI. |
| #410 Install harness, PyPI and crates.io sides: fresh environments for wheels and a temporary crate consumer | Install harness; test infrastructure. |
| #411 Model manifest and `xtask models check`: warn 30 days before a model's retirement | Model manifest and retirement check, CI. |
| #412 Runtime matrix: the cells file, the job steps and the smoke-script runner | CI runtime matrix. |
| #413 Live AI jobs' credentials and caps: per-job keys, a currency Limit, the worst case first, disabled by default | Live AI job caps, CI. |
| #414 MCP conformance: the fixture server, `just mcp-conformance` per revision and `xtask mcp check` | MCP conformance harness. |
| #415 Recorder workflow: recordings change only through a reviewed pull request | Recorder workflow, CI. |
| #416 MCP client matrix: TypeScript v1 and v2, Python and `rmcp` over stdio and Streamable HTTP | MCP client matrix. |
| #417 Provider check: a nightly Live check of provider response shapes against the recordings | Nightly provider check, CI. |
| #418 MCP Inspector runs: boot smoke, `--strict` on profiles and verified skills | MCP Inspector runs. |
| #419 Feature generators for `mcp-profile:` and `skill:` | Feature generators. |
| #420 Evaluation sizes in `bench/`: `eval-pr`, `eval-nightly` and `eval-weekly` with 3 trials per task | Evaluation campaigns in `bench/`. |
| #421 Scripted engine as a Daemon's HTTP listener behind a token on port 0 | Scripted engine test mode. |
| #422 `goodfetch-test-env --daemon --http-listener`: a Daemon with its HTTP listener, a token and port 0 | Test environment option. |
| #423 Skill Evaluations: promptfoo runs Claude Code and Codex with and without the skills | Skill Evaluations campaign. |
| #424 Adapter harness: `BindToolsChatModel` and the lowest-version runs | Adapter test harness. |
| #425 Enable the live AI jobs within the approved budget | Enabling live AI jobs, CI configuration. |
| #426 Integration harness: Pipedream's `$`, pinned n8n and Activepieces images | Integration test harness. |
| #427 Local models in the Release suite: `LocalModel::start` for llama.cpp server and Ollama | Local model servers in the Release suite. |
| #428 Real-account job behind a Cloudflare Tunnel, present and disabled | Disabled CI job. |
| #429 `xtask probes generate` and `check`: one probe per confinement rule, positive control first | Confinement probe generation and check, CI. |
| #430 Feature generators for `plugin-kind:`, `sink:`, `adapter:` and `integration:` | Feature generators. |
| #431 Watch jobs: IANA special-purpose registries and comparable projects' advisories | Nightly watch jobs that run none of the product. |
| #432 `serveStatic`: the built docs site served under `/docs/` on port 0 | Docs test server. |
| #433 `xtask docs-samples`: every runnable block extracted with a manifest, and the Proving ground's origin replaced | Docs sample extraction. |
| #434 `xtask artifacts fetch` and `verify`: released artifacts by digest and verified attestation | Release artifact fetch and verification, CI. |
| #435 Docs build plugins: `language-check`, `twoslash-check` and `xtask docs-samples check` | Docs build plugins. |
| #436 `GroupUpgrade::run`: a three-member Daemon group upgraded one member at a time, in both orders | Group upgrade test harness. |
| #437 Docs samples run by every surface's tests against a local Proving ground | Docs sample runners. |
| #438 `just helm-upgrade`: chart upgrades tested from a release's chart by digest | Helm chart upgrade test. |
| #439 `xtask compat jobs`: the Release suite's skew and upgrade jobs generated from `compat/releases.toml` | Generated skew and upgrade job matrix, CI. |
| #440 Commit the approved resident-memory ceilings | Resident-memory ceilings, a Performance target. |
| #442 Owner accounts for Integration tests: Zapier, Make, Pipedream, Power Apps and the Cloudflare Tunnel | Owner accounts for Integration tests. |
| #445 Decide the free Chromium build gate | ADR deciding the free Chromium build gate. |
| #546 Decide macOS x64 support once Rosetta 2 and GitHub's Intel runners end | Platform support decision; no runtime work. |
| #616 OS keychain and encrypted local file as Secret providers | Secret providers resolve per reference, not per unit. |
| #617 Run state sealed under a per-run data key, wrapped by a key the caller holds | Field sealing of Session secrets; not on a Shared path. |
| #618 Discarding a run destroys its key; field encryption and `secure_delete` switchable per run | Key destruction on discard; per-run switches. |
| #625 TLS verified everywhere on the engine's HTTP client, with per-domain trusted authorities | TLS verification per connection; no Shared path unit. |
| #627 Operator-configured endpoints as their own egress class | Operator endpoint egress class; per configured endpoint. |
| #636 The relay shows the site the browser's own fingerprint on every pinned browser and OS | Relay fingerprint per connection; not a Shared path. |
| #641 Crash reports stay local and `doctor` builds a redacted bug-report file | `doctor` and crash reports are one-off. |
| #642 Chromium's sandbox on by default, probed at startup, refused with its fix | Sandbox probe at startup and per launch. |
| #644 Response limits and hostile input at Collie's layers | Body and JSON limits on engine reads; no counted work. |
| #649 Secret canaries searched in every output of Collie, raw and encoded | Test harness for Secret canaries. |
| #650 One regression test per bypass, citing its advisory | Regression tests and fuzz targets; tests only. |
| #651 Fuzzing Live check holds the next minor's promotion while a crash is open | Fuzzing Live check in release; CI only. |
| #652 The caller as a Secret provider: `secrets.resolve` answered by the Client | Secret provider kind; reuse per placeholder is the driver's. |
| #653 A certificate exception refused where it cannot be honoured, served by a Browser host's own relay | Refusal per Session source. |
| #654 Secret placeholders bound to Secret references and domains | Bindings resolved per connection, Run or Session. |
| #658 One-time codes: TOTP from a seed behind a reference, and codes a provider makes | TOTP computed per typed code; no Shared path count. |
| #660 Raw CDP and Lent pages: a token per attach, audited, and no secrets typed | Tokens per attach, audited per attach. |
| #661 Raw CDP and Lent page command refusals that keep contexts, extensions and the engine's browser safe | Refusal table on the Lent page endpoint, built once. |
| #662 File-reading CDP commands refused on a Daemon with accounts | Refusal table on the Lent page endpoint, built once. |
| #663 Session export bundles encrypted with `age`, and their import audited | Session export bundles per export or import. |
| #664 Journey: log in with a Secret placeholder bound to its domain, then find the password nowhere | Journey definition only. |
| #665 Journey: drive one page and see nothing leave the machine beyond its site and pinned downloads | Journey definition only. |
| #666 Standard Webhooks signing and the vectors every SDK's verification shares | Webhook signing per delivery; not a Shared path. |
| #667 An Exposed Daemon terminates TLS itself, or opts in to TLS terminated upstream | Exposed Daemon TLS per connection. |
| #668 An Exposed Daemon refuses everything with `setup_required` until its one-time setup link is used | One-time setup link. |
| #669 Program tokens declared in configuration, their values as Secret references | Token declarations read with configuration. |
| #670 The peer link refuses another group's authority, a revoked Group member and an expired certificate | Peer link certificate checks per connection. |
| #672 A Daemon group refuses secret providers it cannot share, with the fix | Refusal of local secret providers at group setup. |
| #673 Personal-data notice shown once per run for fields marked personal | Notice once per Run from the schema. |
| #674 Journey: expose a Daemon over TLS with a program token declared in configuration | Journey definition only. |
| #678 The HTTP and discovered-API tiers check the address of every connection, per hop and per address tried | Connector address check per connection; no Shared path unit. |
| #681 Response caps on the HTTP tiers: bombs, endless bodies and deep JSON end `failed` | Body caps checked while reading; no counted work. |
| #682 A crawl sent from a chosen local address, and a replacement Session in an unblocked block | Session local address chosen per Session. |
| #683 Exports carry no session secrets and refuse file names that leave their folder | Export content and file-name checks; one-off. |
| #686 SSRF case table at Foxhound's Frontier and fetch-tier layers | SSRF case table completeness check; tests. |
| #687 Journeys: a crawl that sends nothing beyond its targets, and an offline crawl with a missing artifact refused | Journeys. |
| #688 SDK variants of the certificate and robots.txt Journeys | Journey variants. |
| #689 Secrets and HTTP credentials set on a Label, each Label with its own Session pool | Per-Label Session pools built once; bindings at Session. |
| #690 A before-request Hook's header values may be Secret placeholders, added after the last Hook | Secrets resolved once and held per Run (`secrets.rs`). |
| #691 A Plugin, a Script Hook and a Fetch backend never receive a secret's value | Placeholders only; engine inserts held secrets. |
| #692 Sinks and Sink webhooks as operator-configured endpoints, signed under Standard Webhooks | Sink endpoint policy and signing; Sinks are area 7's. |
| #693 A Fetch backend Plugin's connection kept out of private ranges unless first-party or allowed by policy | Fetch backend connection egress check. |
| #696 Chromium launch: headless and headful present one browser, `resolve`, the sandbox probe and unthrottled background pages | Launch flags and focus emulation are once per browser or page opened. |
| #697 Sessions: `run.newSession`, one browser context each, and a Session that closes cleanly | Session open and close happen once per Session. |
| #714 Clean-up rules: one version per Run, from the release set, the daily channel or upstream | Rules version chosen once per Run, recorded in run state. |
| #727 Uploads from the program, downloads kept with the Run, `saveAs` whole or not at all | Uploads and downloads are per file, on no Shared path. |
| #728 Session settings: device emulation, permissions and the clipboard's refusal | Emulation applied once per Session and page opened. |
| #729 Screenshots and PDFs, with Secrets masked and the Session's colour scheme | Screenshots and PDFs are per call; masking is area 4's. |
| #730 A prerendered page passes the waits, the Page check and the gate, or prerendering is off | Prerender switch is a launch flag; no per-unit work added. |
| #732 The Human pause: wait for a person with every clock stopped | Human pause holds calls; adds no counted work per unit. |
| #734 Two-factor codes read from a source the program configured | Code sources are read once per login, not per unit. |
| #735 Run-only imports: a `state_file` or a `cookie_header` brings a login into one Run | Run-only import runs once when its Session opens. |
| #736 Saved identities: keep a login encrypted in the install's store and reuse it | Saved identity store is used once per save or reuse. |
| #737 A Profile directory used in place: its lock, its versions and its registered workers | Profile directory checks run once per launch. |
| #738 Import a login from an installed Chrome, Edge or Brave | Login import runs once per import, over its own browser. |
| #740 Import a login from an installed Firefox profile | Firefox import runs once per import on a temporary copy. |
| #741 Import a login from Safari after Full Disk Access | Safari import runs once per import after Full Disk Access. |
| #743 A `cdp_url` Session, and one error code per remedy when it cannot open | cdp_url probe runs once when its Session opens. |
| #746 Cookies and storage on a Session, in the SDKs only | SDK-only cookie and storage operations, per call. |
| #747 A lost login: the recovery ladder, one typed `logged_out`, and golden scenario 04 | Recovery ladder runs only on a lost login, not per unit. |
| #748 An imported login that expires is re-imported, and golden journey: bring a Firefox profile's login in | Re-import runs only when a login expired. |
| #752 Two drivers on one page take turns, and a navigation the verb did not start is seen | Main-frame watch subscribes to events; sends no command per unit. |
| #756 A person's own Browser host connects out to the engine, and the Run waits while it sleeps | Offline host handling is per link loss, not per unit. |
| #758 Global Privacy Control only when the presented browser would send it | GPC chosen once per Session from the presented browser. |
| #763 Cut bodies fail their attempt, and only a request the site never processed is sent again | Transport-level cut detection and resends; no counted work added. |
| #766 The responses that mean slow down: the site's, never the proxy's | Status classification for backoff; no counted work per unit. |
| #772 HTTP-only `stealth` Sessions on `wreq`, with the Identity profile's fingerprints | Transport choice per Session; no per-unit work. |
| #773 Session pools per Label, and a blocked Session retired and replaced | Session pools per Label; per-Session work only. |
| #780 Paid fetches through x402: `exact`, `upto` and `batch-settlement` on EVM and Solana, and the Journey | Payment schemes inside #779's Paid fetch; no new per-unit count. |
| #781 An interrupted payment proved settled, still able to settle, or unable to settle | Proof of interrupted payments on resume; no per-unit work. |
| #782 Pages left at `payment_required` are listed and the Run's Completeness is `unverified` | Run report listing and Completeness; no per-unit work. |
| #790 A Download is kept with its Run and written whole | Downloads kept per Download, off every Shared path. |
| #798 See and empty the cache: `cache stats`, `clear` and `verify`, and deletion with the Run | Cache stats, clear and verify operations, once per call. |
| #804 Print and screen are kept apart, and a page can be saved as one MHTML file | `page.mhtml` is one CDP call per request, off every path. |
| #805 A large Download streams under credits, and one no longer needed is cancelled | Download streaming and cancel, per Download. |
| #806 Downloads a Lent page's library starts stay the Run's | Lent page Downloads, per Download. |
| #808 Journey: develop against the Response cache, then test offline from a recording | Journey definition and variants. |
| #809 Journey: read a product page and a scrolling list, save its files | Journey definition and variants. |
| #812 The Key-value store in Code mode and Script Hooks | Code mode reaches the existing Key-value store operations. |
| #813 Linux confinement: a Plugin at network level `none` can create no socket | Confinement applied once per instance start; no per-unit work. |
| #815 Area 9's Collie stories as Features and the six Collie Journeys in the Journey catalog | Story and Journey inventory files; no behaviour. |
| #816 Refusing where a host cannot confine, resource limits and the shared startup probe | Startup probe and kernel limits, once per engine or instance. |
| #818 Re-measure the extractor and the Markdown converter on WebMainBench | One-off WebMainBench measurement on the owner's machine. |
| #819 A run's process Plugin instance: confined start, `hello` check and the instance's states | Instance lifecycle per Run; per-message framing is decided on #810. |
| #820 `goodfetch-model-provider`: the Model provider interface in its own crate, and the model contracts in `goodfetch-protocol` | Model provider interface crate and protocol types; no behaviour. |
| #821 Cache entries deleted on a Daemon group's erasure broadcast | Erasure broadcast applied per erasure. |
| #827 Plugin calls wait for credits and meet their deadlines | Credits and deadlines per Plugin call; no Shared path unit. |
| #830 The Console and the Daemon's life: Attached engine, visible tab, socket activation, first-launch Limit and links | Console and Daemon lifecycle; no Shared path. |
| #831 A Plugin's calls back: accepted only inside a call, `net.fetch` through the egress check | Call-back gate and egress class are pure decisions per Plugin call. |
| #837 Crash, restart, quarantine and heartbeat of a Plugin instance | Crash, restart and heartbeat per instance; no Shared path unit. |
| #838 Variables and Secret placeholders in `act`: values never reach the model | Variable and Secret placeholder checks per verb call. |
| #842 Secrets by reference in a Plugin's `net.fetch`: inserted by the engine, scrubbed from responses | Secret insertion and scrub on Plugin net.fetch, outside the paths. |
| #845 Effects, `page_script` and navigation scope for a Plugin's page and network calls | Permission rules per Plugin call; pure decisions, no path stage. |
| #850 Logged-in Sessions: third parties reach an account only through `session_credentials` | Session credential rules per Plugin; no per-unit path work. |
| #854 Schedule store and the `schedule` operations: create, get, list and delete on the Daemon | Schedule store and operations, off every Shared path. |
| #855 `direct` network level for the kinds that cannot use a proxy | Network level checked once at instance start. |
| #858 The Run's Verdict from AI steps: Success checks, Grounding and what stays `needs-review` | Verdict computed once per Run from recorded outcomes. |
| #859 Consent walls and Walls: offer the domain, and answer the pause for this run or always | Console item and settings answer; no path. |
| #861 A Plugin's settings: one JSON Schema serves every surface | Settings validated at Plugin start and on change only. |
| #862 A refetch stopped without stored data, and a certificate whose key changed | Console item answers; no path. |
| #864 Triggers: cron with DST rules, anchored intervals, jitter and `schedule preview` | Schedule triggers, per occurrence or preview call. |
| #870 A Run names a Plan file: its content in at the start, merged versions out at the end, and the Plan report | Plan file content in and out once per Run. |
| #872 Schedule settings and Operator policy caps: count, minimum interval and period Limits | Schedule settings and caps, checked per definition change. |
| #873 Windows confinement: AppContainer, restricted token, Job object, and blocked executables | Windows confinement applied once per instance start. |
| #875 The Plan file as a registered contract format: stable JSON, refusals and failed writes | Plan file format read and written once per Run. |
| #876 A Schedule fires: occurrences recorded, runs started under derived IDs and labels, crash-safe | Scheduler firing, per occurrence; Runs start through `run.start`. |
| #879 Windows filter broker: persistent filters for `proxy_tcp` and `direct` | Windows filter broker per instance lifecycle; no path unit. |
| #881 Verbs refuse to send a page to a model when its AI-use signal says no, and replay still runs | AI-use signal check per verb before any model call. |
| #884 Ubuntu 23.10 and later: the `goodfetch-sandbox` helper and `plugin setup` | Linux helper install and start per instance. |
| #885 Pending Plan versions: review before promotion, merge by entry and the person-only answers | Pending Plan review operations; once per answer. |
| #886 Pause, resume, edit and run now: a Schedule's lifecycle | Schedule lifecycle operations, per call. |
| #889 `plugin add`: install a Plugin's OCI artifact by digest without running anything | Install-time command; nothing per unit. |
| #890 Who may act on a Schedule: roles, named tokens, the creator and the Audit log | Schedule roles; Audit entries per change through `audit`. |
| #894 Overlap policies: skip, queue one, allow and replace | Overlap policy, per occurrence. |
| #895 `plugin add` fetches the Plugin's language runtime by digest, and offline never downloads later | Runtime fetched at install; checked at start. |
| #896 The content origin: scraped pages shown from a second origin, by signed address | Console content origin; UI and HTTP serving only. |
| #898 A registered Proxy provider becomes `proxy.provider`, and Recipes and models cannot route a person's traffic | Proxy provider registration and settings refusals; once per Session. |
| #899 `plan.frozen`: replay only, no model call, nothing written | plan.frozen switches model calls off; no added per-unit work. |
| #900 Approvals: a third-party Plugin loads only as Operator policy allows, permission by permission | Approvals read at Plugin start; no per-unit path work. |
| #901 What a Schedule may target: Recipes, code-free runs and the refusals of unattended targets | Schedule target checks, per definition change. |
| #904 Access modes: `standard`, `identified` and `stealth` resolved when a Session opens, with every refusal | Access mode resolved once when a Session opens. |
| #905 A scheduled Plan replay reads its Plan file and promotes only after a verified run | Plan file read per scheduled occurrence. |
| #907 `plugin add` shows what is granted and the spend prices before anything is approved | Install-time display of grants and prices. |
| #908 Notifications and "Needs you" items of Schedules | Schedule Notifications and Needs you items, per event. |
| #910 Locale-dependent Value transforms in a Plan: variants per language, Drift on a changed format, `map` tables for a person | Plan variant keys per Plan entry; not a Shared path. |
| #911 The Plugin index: a signed, fresh, expiring set with a revocation list | Index check at install and refresh, not per unit. |
| #913 The Fingerprint method driver: `none`, `js_injection` and `basic`, each with its warning, Provenance mark and Audit record | Fingerprint method set up once per Session; Runtime stays off. |
| #915 Missed occurrences, catch-up and wake detection | Scheduler wake detection, per tick. |
| #916 Provenance, signer identity and look-alike names | Provenance check at install time. |
| #917 The verbs on a Borrowed page: turn-taking with another driver | Borrowed page turn-taking reuses the main-frame watch. |
| #918 Accessibility: the keyboard walk, announcements and Enter during composition | Console accessibility tests; no runtime path. |
| #919 `plugin search` and `plugin index import` for an air-gapped host | CLI read and import commands; no path. |
| #921 A Lent page keeps its Identity profile after a library detaches | Re-apply runs once per ended attach. |
| #922 A Schedule pauses itself after repeated failures or when its creator loses the right | Auto-pause counter, per occurrence. |
| #923 AI safety in the engine: domain policy on AI requests, page text marked untrusted, no credentials in observations | Applies the existing navigation gate and Mask to AI calls. |
| #924 `plugin update`: a permission diff and the signer, approved by a person | Update command with permission diff; no path. |
| #925 The Goodfetch Chromium: fingerprint-chromium's patch series on ungoogled-chromium, built from `chromium/` on every platform | Browser build pipeline. |
| #927 A Human pause in a scheduled run times out at the next occurrence | Pause timeout, per Run; a progress field changes no counts. |
| #928 `plugin list` and `plugin remove`: one Plugin store for both Products | Store list and remove commands; no path. |
| #933 Every patch has a surface row and every expectation its category: Identity consistency and version truth tests | Browser build gate tests. |
| #934 Built-in Model providers: OpenAI, Anthropic, Gemini, the Vercel AI Gateway, OpenRouter and four local servers | Model provider adapters; model HTTP calls are not a Shared path. |
| #935 A Recipe names the third-party Plugins it needs by digest | Recipe pin validated once when the run starts. |
| #938 Provider failures: one Error code per remedy, classified at each adapter, and `timeout.model` | Provider failure classification per model call; no Shared path. |
| #941 A Schedule never escapes its creator's Limits, and a raise needs a person | Creator's Limit scopes placed once per Run. |
| #942 Codecs, fonts and the quiet browser: background pages show the timers and visibility of one in front | Launch flags and focus emulation once per launch. |
| #943 Declared capabilities and the structured-output strategy ladder | Capability registry and structured-output ladder; per model call only. |
| #946 Baseline: the run a Schedule compares with, and pinning one | Baseline selection over Run index rows, per call. |
| #948 The run report states which Plugins ran, how they were confined and what came from them | Run report content, built once per Run. |
| #949 No background traffic and the variations seed: Google sites receive `X-Client-Data` with no call at run time | Browser build-time traffic settings. |
| #952 Watch: start the target only when chosen pages change | Watch runs reuse existing page operations, per occurrence. |
| #953 A new Stable version opens one Chromium pull request; a major ships only through the gate, and a held major never weakens it | Scheduled CI job. |
| #959 Live view tracer: the channel, the driver and a Session watched over the screencast tier | Live view frames are no Shared path; CDP commands cross C1 unchanged. |
| #960 Watch on a field, held to the Clean-up rules version of its comparison point | Field Watch, per occurrence. |
| #962 Live checks: Chromium patch preview, build currency and the `upstream-flaky` tracking issues | Live checks; CI only. |
| #963 After-a-run triggers and the refusal of cycles | After-a-run triggers and cycle check, per definition. |
| #966 Observe-only lifecycle Hooks: a run's life reported to the operator's own systems | Lifecycle Hooks fire once per Run event, not per unit. |
| #967 Human pauses in "Needs you" and the Live view: Done, Decline, and a page nobody closes | Pause answers are per person action, not per unit. |
| #968 Retention: keep what a Schedule needs and discard the rest | Retention after each Run and daily. |
| #969 The Goodfetch Chromium fetched by digest into the Browser cache on first use, with a missing build named | Browser download per first use. |
| #971 The Agent's Success check and Grounded output | Success check per done reuses the expect evaluation. |
| #973 Browser backend kind: a vendor browser, a Browser host or a third-party CDP endpoint | Browser backend chosen once per Session. |
| #974 The control lock: take, request, force and idle release, one holder at a time | Control lock lives in live_view's actor; per person action only. |
| #975 Schedule files and `schedule apply` | Schedule files applied per `apply` call. |
| #976 An attended Agent can ask a person; a closed page stops it | Human pause offer per Agent task. |
| #977 `engine` Fingerprint method: a stealth Session runs the Goodfetch Chromium in a process of its own | Browser process choice once per Session. |
| #978 Taking control of a paused Session: hand back, the consent prompt runner and the governor | Governor already skips held_by_person (ladder.rs:61); consent only suppressed. |
| #981 A browser a Plugin declares: downloaded by digest, allowed per digest | Declared browser downloaded and checked per launch. |
| #982 Third-party stealth binaries only by digest under Operator policy | Build admission by digest once per Session. |
| #983 The active major fires: marker, `schedule activate` and the service offer | Active-major marker read per occurrence. |
| #984 Export as CSV, XLSX, HTML table and Markdown with every field seen | Export runs once per call over committed Records. |
| #985 Model provider kind: a model format beyond the built-in three | Model provider calls use the kind driver; no path stage. |
| #986 Agent tools: the developer's own functions in the loop | Agent tools use the existing engine-to-Client call path. |
| #987 Typing in the Live view: keys in any layout, composition, touch and the escape chord | Person's keys sent as CDP input; adds no work per command. |
| #988 A licensed browser of the user's own under `engine` through a Browser backend Plugin | Browser backend Plugin per launch. |
| #990 An Agent's task as one Plan entry, replayed whole with no model call | Agent task replay reuses Plan replay; no new path stage. |
| #991 A Schedule that needs Foxhound: refused to create, missed on a Collie Daemon | Product check per Schedule definition. |
| #992 Decision provider kind: one option from a fixed list, or a typed abstention | Decision provider calls use the kind driver; no path stage. |
| #993 Identity profiles: a pure `generate` from a seed, per engine, narrowed by `stealth.identity.*` | Identity profile generated once per Session from its seed. |
| #995 The Session's tabs, dialogs, files and clipboard in the Live view | Tabs, dialogs, files and clipboard are per person action. |
| #996 Secret provider kind: a vault Goodfetch does not ship | Secret resolution per reference; no Shared path. |
| #997 A generated identity matched to the proxy exit, holding across launches, frames, workers and headers; locale locks | Identity matched once per Session and launch. |
| #999 Live view over WebRTC for a remote viewer: VP8 from a new crate, falling back to JPEG | New WebRTC crate for viewers; no Shared path unit. |
| #1000 Schedule formats registered: store, file and marker, with the lowest-minor rule | Schedule formats' registration and compatibility sets. |
| #1001 Challenge solver kind: a solver that acts only through the engine | Challenge solver per challenge; reuses Page check and driver. |
| #1004 Live view of a virtual display at about 25 fps, and the tier setting | Virtual display capture tier; no Shared path unit. |
| #1006 Firefox stealth is Camoufox: Firefox identities only, service workers refused through Firefox's own preference | Session launch choice; the Firefox command path is another ticket's. |
| #1007 Proxy provider kind: each Session gets its proxy when it opens | Proxy acquired once per Session open. |
| #1008 Only the Lead Daemon fires: failover without a double occurrence | Lead Daemon firing and failover, per occurrence. |
| #1011 `goodfetch-decision-provider`: the in-engine Decision provider with weights in the Browser cache | Model runner and weights loaded once; runs through tasks. |
| #1012 Headful `stealth` Sessions on a virtual display, with `capability_unavailable` when Xvfb is missing | One Xvfb display per headful browser process. |
| #1013 Search provider kind: a search API Goodfetch does not list | Search provider calls; no Shared path unit. |
| #1014 Embedding the Live view: short-lived grants | Grants are per viewer connection; no per-unit work. |
| #1017 Schedule conformance suite on the Stub gateway, and `schedule.list` and `info` answering the Console | Schedule conformance suite. |
| #1019 Code source kind: unattended MFA logins | Code source polled within an MFA step; no path. |
| #1020 `<goodfetch-live-view>` and its React wrapper, published at the Product version | Web component and React wrapper packages; no engine path. |
| #1023 Notification channel kind: a channel Goodfetch does not ship, never repeated across a resume | Notification sends per event; no Shared path. |
| #1025 Schedule a crawl: a Schedule that needs Foxhound runs on Foxhound's Daemon | Schedule plumbing; once per occurrence. |
| #1027 Browser search: DuckDuckGo in a Session of its own, opt-in and honest | Browser search in its own Session; uses existing page operations. |
| #1028 The browser version check: grades from a shipped release table with no network call | Version graded once before a Session uses a build. |
| #1034 `browser versions` and the opt-in online release lookup | CLI listing and opt-in lookup; not per unit. |
| #1035 Research synthesis: search, read, answer with Grounded citations | Research task uses existing search and reads. |
| #1036 A run records its Plugins; a resume refuses a changed third-party Plugin | Plugins recorded in run state once per Run. |
| #1038 Trace retention, the size cap and a full disk | Retention and size cap act per segment and Run, not per unit. |
| #1039 Code mode: a model-written program calling only the Operation catalog | Code mode dispatches through the catalog; not a JSON-RPC message path. |
| #1041 Journey: crawl two kinds of page into a typed, keyed Dataset and read why each failed or was skipped | Journey test over existing crawl paths; adds no per-unit work. |
| #1042 A Saved identity keeps its seed, Access mode and proxy binding, and refuses a contradicting reuse | Saved identity fields per Session save or reuse. |
| #1043 Register your own handler of any Collie kind by name | SDK registration options; no engine path. |
| #1044 Masking: Secret values and typed input blanked in the Live view, traces and the Console | Masking rides the trace's existing per-entry Mask; frames are no path. |
| #1045 Missing Records and soft-delete only after a complete run | Missing keys computed once when the Run ends. |
| #1046 Code mode replay by position: a retried program never repeats an order | Effect journal matching by position; not a Shared path. |
| #1047 Warm-up browsing and the expiry of unused Saved identities | Warm-up per Session open; daily sweep. |
| #1048 The trace format is a registered contract, and `run.trace()` returns the same trace | Registers the trace format and compat corpus; no per-unit work. |
| #1049 The Run report states the changes; a lost Baseline marks Records unknown | Run report section; once per Run. |
| #1050 The recorder: a Console recording becomes a Plan and matching code | Recorder captures a Live view session; not per unit. |
| #1051 `stealth` on an import or a Profile directory used in place keeps the login's identity binding | Launch choice per import. |
| #1052 Journey: crawl static, script-rendered and scrolling pages with one handler | Journey test over existing crawl paths; adds no per-unit work. |
| #1053 'Records changed' Notification, Sinks that deliver only what moved, and the retention veto | Notification and Sink modes; once per run or delivery. |
| #1054 Rust handlers: declared in `ConnectOptions`, read as an `EventStream` | Rust SDK handler streams; no engine path. |
| #1055 Spike: can a Collie trace open in Playwright's trace viewer? | Spike exporting traces to Playwright's viewer; no engine path. |
| #1056 Adapters replayed from Model recordings at their base URL | Replay tests of adapters; test infrastructure. |
| #1057 A Run that uses a handler refuses to detach and refuses a resume without it | Detach and resume refusals, once per Run. |
| #1059 Failures grouped by cause, from a pattern to one example | Failure grouping is a query over finished runs. |
| #1060 A Partitioned crawl Schedule compares as one crawl | Per-Record marking is #1040's; aggregation runs once per crawl. |
| #1061 Pinned llama.cpp server and Ollama in the Release suite | Release-suite local model servers; test infrastructure. |
| #1062 Plugins and handlers on remote engines: a tenant's isolation, the Browser host, run-layer values | Remote placement of Plugins and handlers; no path unit. |
| #1063 Journey: kill a crawl, resume it, retry its failures (J-005's SDK variants) | Journey test of resume and retry; adds no per-unit work. |
| #1064 Reconcile an uncertain Effect: "It happened", "It did not happen" or "Leave uncertain" | Reconciliation is a person's answer per occurrence. |
| #1066 Request snapshots for every AI operation, `ai.system_prompt`, and the Quick-job exceptions | Request snapshots and a system-prompt setting; no per-unit work. |
| #1067 A Human pause when a person can answer, and `challenge_unsolved` at once for an unattended program | Human pause step; once per unsolved Challenge. |
| #1068 A crawl request after "It did not happen": re-admitted like `retryFailed` | One re-admission per answer, as retryFailed already does. |
| #1069 A Daemon group compares every member against the Lead Daemon's last Plugin index | Daemon group index comparison per placement. |
| #1071 The step that cleared a Challenge is learned per domain, and the ladder never presses past a site's condition | Learned step written once per cleared Challenge. |
| #1072 Journey: archive a crawl as WARC and replay it as a recording | Journey test of WARC replay; WARC path decided on #1031. |
| #1073 Live checks: the provider check and the recorder job through the Vercel AI Gateway's free tier | Live checks through the gateway; CI only. |
| #1074 Submitting a Plugin to the official index by pull request | CI validation of index entries. |
| #1075 Choosing a Session source where a run starts, and guarding a Saved identity's export and domains | Session source picker and audited export; per person action. |
| #1076 A Challenge met by an Agent's page follows the same ladder | Agent reuses the ladder; no new per-unit work. |
| #1077 A linked Rust engine finds or downloads its `plugin-shim` executable | Shim found or downloaded once per engine. |
| #1078 Evaluation tasks for Plans, the verbs, extraction and the Agent | Evaluation tasks; test infrastructure. |
| #1079 Approval pauses before an agent or Recipe imports, borrows or changes a login | Approval pauses before identity changes; per person action. |
| #1081 Account rotation over several Saved identities, and a pool per Label | Pool per Label and source rotation; per Session, writes ride #920's. |
| #1082 Older first-party Plugins keep passing their kind's suite against the current engine | Compatibility test matrix; no runtime work. |
| #1084 A user's own browser is never rotated or retired | Exemption from rotation; no per-unit work. |
| #1085 Evaluation tasks for Code mode, vision, Decision provider calibration and AI safety | Evaluation tasks; test infrastructure. |
| #1086 CapMonster, 2Captcha and Hyper Solutions adapters, each reading its service's failures in one table | Adapters fill SolverDriver primitives; no new per-unit work. |
| #1088 Reference pages for the plugin protocol's method groups | Generated reference documentation. |
| #1091 The served-call table's layout and gesture rows | Escalation rows of the served-call table; tests only. |
| #1092 Journey: find a Plugin, install it, approve it, run with it, then update it | Journey test only. |
| #1093 Area 9's Foxhound stories as Features and the two Foxhound Journeys in the Journey catalog | Feature inventory and Journey files only. |
| #1094 Moving Schedules to a newer major, on purpose | Schedule major activation prompt; Console and Daemon start only. |
| #1096 Confirm or decline an Effect that waits for a confirmation | Confirmation answers per Effect; no per-unit path. |
| #1099 Hermetic stealth probes on the Proving ground, and the Journey that opens a `stealth` Session and reads its grade | Hermetic test fixtures and a Journey; no product per-unit work. |
| #1100 Journey: carry a Script Hook in a Recipe that sets a cookie before each navigation | Journey test only. |
| #1101 Review a pending Plan version: old and new steps side by side | Plan diff review screen; no per-unit path. |
| #1102 A Model allowance reached skips the request and marks the Run incomplete | One more skip reason in the outcome; no new counted work. |
| #1103 Fetch backend kind: a process Plugin carries the HTTP tiers' requests | Opt-in transport behind the fetch driver; unset does no work. |
| #1105 Record a Plan by acting in a Live view | Recorder in the Live view; per person action. |
| #1107 Published stealth wording: every page, message and setting description passes `xtask wording`, and none leads with access | Wording checks. |
| #1108 A third-party Fetch backend connects only to its call's target, and only if allowed | Plugin net.connect limits; per backend connection. |
| #1110 AI verb steps and Agent tasks on the Timeline | Timeline display of recorded AI steps. |
| #1112 Every kind of "Needs you" appears and clears, and a Schedule that needs another Product | Remaining Needs-you kinds; listing and clearing only. |
| #1114 Access results per Fingerprint method against stock controls, and Camoufox's missing service workers measured | Access measurements and reporting. |
| #1115 The crawl's Model allowance: stop model use at a set amount and finish the crawl | Allowance checked per model call, not per page. |
| #1116 A Partitioned crawl's Datasets are read and exported as one | Partitioned reads and exports; once per read. |
| #1118 Online conformance on every Chrome-major patch: relayed Sessions against the pinned Chrome on HTTP/3 hosts | Online conformance scheduling; test infrastructure. |
| #1119 The Model allowance counted once across a Partitioned crawl | Allowance across Partitions; per Slice from the lead. |
| #1120 A period Limit raised by a file change waits for a person | Pending Limit raises held for a person; per edit. |
| #1122 Challenge solvers as Plugins or SDK handlers, chosen by `challenge.solver` like a built-in | Solver Plugins and handlers under the existing driver. |
| #1123 Testing AI handlers offline: `scriptedExtraction` and a crawl replayed with no request and no model call | Testing kit and frozen replay; test path. |
| #1124 Fetch backend capabilities proved one at a time | Capability suite against the Proving ground; test infrastructure. |
| #1125 An Exposed Daemon's first operator: the one-time setup link and a passkey | First-operator setup and passkey enrolment; once per Daemon. |
| #1126 Goodfetch's own solving logic as a confined process Plugin, and the ladder without it for self-hosters | Confined Plugin loading; once per Plugin load. |
| #1127 `run.crawl_from_prompt`: a crawl declaration to review, never a crawl started | Prompt-to-crawl compile; once per call. |
| #1128 Probes and Guarantee classes: a destination's name never promises more than it proves | Probe runs at configuration and Run start, once per Sink. |
| #1129 Sink kind: a confined Sink Plugin speaks the `sink.*` group | Sink deliveries read committed Records in batches, off F4. |
| #1132 A crawl seeded from search results, only where the provider lets them be kept | Search-result seeds checked once before the crawl starts. |
| #1133 Write modes and keys: the same product collected twice is one row | Write modes inside the Sink delivery driver; path pending #1121's decision. |
| #1134 Bridge Sinks: Airbyte destinations and Singer targets in a rootless container | Bridge Sink container launcher; per Sink start. |
| #1135 Invite people, change roles, remove accounts and recover a passkey | Account management operations; per person action. |
| #1136 Schema drift never loses data silently | Schema evolution inside the Sink delivery driver; path pending #1121. |
| #1138 Step-up: a fresh passkey for every write that approves | Step-up challenge on listed writes only; per person action. |
| #1139 A bad Record never blocks a Sink: batches are split and dead-lettered | Batch splitting inside the Sink delivery driver; path pending #1121. |
| #1141 Rotation within a provider, the escalation budget and the periodic trial on the cheaper kind | Periodic trial per domain and Session rotation, not per page. |
| #1142 Delivery retries, pauses and statuses | Retry schedule inside the Sink delivery driver; path pending #1121. |
| #1143 Widening trust is deliberate: step-up, the operator role and a plain warning | Step-up and warnings on trust-widening writes; per person action. |
| #1146 Each Sink's class, dead letters and late deliveries in `sink list` and the report | Reporting per Sink and Run; no per-unit work. |
| #1147 OIDC single sign-on, groups mapped to roles | OIDC sign-in; once per sign-in, not per message. |
| #1149 Collection goes on when a Sink fails, and delivery outlives the Run | Delivery policy and redelivery commands; path pending #1121's decision. |
| #1152 Files mode: numbered Export parts to an object store | Files mode inside the Sink delivery driver; path pending #1121. |
| #1156 Google Cloud Storage and Azure Blob Sinks | Destination Sinks filling the driver's primitives; path pending #1121. |
| #1159 SFTP and FTP Sinks | Destination Sinks filling the driver's primitives; path pending #1121. |
| #1161 Each Sink is rate-limited, batched and adaptive to its destination | Rate limiting inside the Sink delivery driver; path pending #1121. |
| #1162 PostgreSQL and Supabase Sinks | Destination Sinks filling the driver's primitives; path pending #1121. |
| #1163 MySQL and MariaDB, DuckDB and SMTP Sinks | Destination Sinks filling the driver's primitives; path pending #1121. |
| #1165 A Secret placeholder in a header a `before_request` Hook returns | Placeholder insertion per hop of a Hook's request; no Shared path. |
| #1166 Webhook Sinks signed under Standard Webhooks | Destination Sink filling the driver's primitives; path pending #1121. |
| #1168 Collie refuses Foxhound's kinds and stages with `not_in_product` | Refusal of Foxhound names at Run start. |
| #1169 A Sink written in the program serves its `sinks.<name>` entry | Program Sink fills the driver's primitives; path pending #1121. |
| #1170 Each Goodfetch Chromium digest ships its own HTTP profiles, captured by launching it | CI job capturing HTTP profiles per Chromium digest. |
| #1171 One project file serves both Products: `sinks.*` is Foxhound's | Settings refusal per Product at start; no per-unit work. |
| #1173 Sink delivery under crashes, a zombie copy and a compare-and-set destination | Simulation scenarios for Sink delivery; tests only. |
| #1175 A `stealth` Session that cannot move to the browser sends the browser's TLS, HTTP/2 and headers through wreq | HTTP profile applied when a stealth Session's client is built. |
| #1176 Sink processes: one executable per destination, starting with MongoDB | Sink process pattern; path pending #1121's decision. |
| #1177 Live view and event streams follow a Group member's redirect | Redirect token per channel open; no per-unit work. |
| #1178 A `stealth` Session that may move to the browser fetches through browser-fetch from its first request, and the version check grades HTTP profiles | Tier choice per Session; version grading per check. |
| #1179 Database and search Sinks: SQL Server, Elasticsearch, OpenSearch, Redis and ClickHouse | Destination Sink processes; path pending #1121's decision. |
| #1181 Warehouse Sinks: Snowflake, BigQuery, Redshift, Databricks, Iceberg and MotherDuck | Destination Sink processes; path pending #1121's decision. |
| #1182 The ladder's later steps run for a Session pool's Session, and the step that worked is shared by the Run's pools | Reuses Collie's ladder; learned step held per Run. |
| #1184 Vector-database Sinks: Qdrant, Pinecone, Weaviate, Milvus, Chroma and LanceDB | Destination Sink processes reusing #1167's embedding stage. |
| #1185 Accounts, tokens and the Console key held by the Lead Daemon, cached on every member | Accounts replicated to Group members; cache refresh, not per unit. |
| #1186 Escalating into `stealth` only when the run switches it on, and never for an `identified` Session | Escalation setting checked per Session replacement. |
| #1187 Tables, files and notes Sinks: Drive, Sheets, Dropbox, OneDrive, Box, Airtable and Notion | Destination Sink processes; path pending #1121's decision. |
| #1188 Exit codes: one row per class and code, and a Run that stopped, a Verdict that is not `verified` and an interrupt | CLI exit-code table, once per command; no Shared path. |
| #1189 A request whose ladder ends unpassed is `blocked` with `challenge_unsolved` among its attempts, and `retryFailed({ blocked: true })` re-admits it | Blocked outcome recorded in the existing attempt outcome. |
| #1190 Cloud queue and stream Sinks: SQS, SNS, Kinesis, Firehose, Pub/Sub, Service Bus and Event Hubs | Destination Sink processes; path pending #1121's decision. |
| #1192 Broker Sinks: Kafka, RabbitMQ, NATS and Redis Streams | Destination Sink processes; path pending #1121's decision. |
| #1194 A Saved identity in a crawl's pool keeps its binding while the pool retires Sessions by age and use; Access mode settings reach every pool Session | Saved identity binding per pool Session. |
| #1196 E-mail service Sinks: SES, Resend, Postmark, SendGrid and Mailgun | Destination Sink processes; path pending #1121's decision. |
| #1197 The hermetic Access rung on Foxhound's executable: HTTP and browser on one seed compared server-side | Hermetic test rung in CI. |
| #1198 Bridge Sinks run an Airbyte destination image or a Singer target by digest | Bridge Sink images; path pending #1121's decision. |
| #1199 A Foxhound command is refused with `not_in_product` and exit 10, and one shared file serves both executables | Product refusal and config listing, once per call or start. |
| #1200 A Sink's OAuth token is refreshed and written back | Token refresh per Sink, near expiry; no per-unit work. |
| #1201 Crawl families of the Access benchmark per Fingerprint method, tier, Access mode and build age | Benchmark families reported; no runtime work. |
| #1202 Foxhound's Console build: Collie's screens unchanged, plus Foxhound's for every run that needs Foxhound | Console build. |
| #1203 `daemon`, `browser install\|clean` and `upgrade-files` from the CLI | CLI daemon, browser and upgrade commands, once per call. |
| #1204 Journey: deliver a crawl's Dataset to SQLite, kill the Run mid-delivery, find every Record once | Journey test of Sink delivery; tests only. |
| #1206 `usage`, `config explain`, `doctor` and `--follow` on a Run | CLI reports; `--follow` prints existing progress events Client-side. |
| #1207 Person-only answers are prompts on a terminal and `person_required` anywhere else | Person-only CLI prompts, once per person's answer. |
| #1209 `recipe start`: a Recipe run from a shell or a CI step | CLI Recipe start, once per Run. |
| #1210 Person-only prompts for certificates and egress approvals, and the generated-surface check | Person-only prompts and a build-time generated-surface check. |
| #1212 Person-only prompts for widened domains, `ask` answers and a clear-text `storageState` export | Person-only prompts and Console items; Audit entries go through `audit`. |
| #1213 Person-only prompts for Schedules: `schedule activate --major` and a period-Limit raise | Person-only Schedule prompts, once per answer. |
| #1214 `cache stats\|clear\|verify`, `run reindex` and `--recording` on a start | CLI cache commands; recording file read once per Run (ADR 0116). |
| #1218 Python SDK tracer: `AsyncCollie` on AnyIO and a sync `Collie` on a private thread | Python SDK tracer; client-side only. |
| #1220 Export a Dataset from the Console, and erase it everywhere | Export and erase are per person action on a Dataset. |
| #1221 Rust client crate: `goodfetch-runtime`, the `transport` module on an in-memory network, and `Call<T>` | Rust client crate and transport; client-side only. |
| #1223 Rust SDK tracer: `goodfetch-collie`, the linked engine, `Collie::scope` and the `blocking` module | Rust SDK tracer; client-side only. |
| #1225 One duration type per language, nested timeouts and phase defaults held as settings | SDK duration types and deadlines; no engine per-unit work. |
| #1227 Limits in code: typed per Usage kind, lowered and never raised | SDK Limit types; enforcement stays the ledger's. |
| #1230 `collie.start(options)`: a Run held by the program, a fixed `runId` that resumes, tags and its own timeout | SDK start options and Run lifecycle; once per Run. |
| #1231 Spelling by one rule per language, the closed list of initialisms and the names codegen refuses | Codegen naming rules; build time only. |
| #1232 The Console conformance suite against Foxhound's Daemon and the Stub gateway: routing by Product | Console conformance suite. |
| #1233 One form per declared kind in each SDK: round trips, reads, queries, builders, sub-objects and openers | Generated SDK forms; build time only. |
| #1234 The program's side of file saving: whole-or-not-at-all writes with typed failures | Program-side file writer; once per saved file, no Shared path. |
| #1235 Foxhound's Console build: under 1 MB with its own first-screen ceiling | Console size budget. |
| #1236 Page tracer: `run.newPage()`, `goto` returning a `Response`, and a body that decodes | SDK Page API spelling over the engine's existing navigation path. |
| #1237 Locators by what a user sees, actions that wait for one actionable element, and one obvious way per task | SDK locators and typing; the guard already raises target_not_unique. |
| #1238 Crawl request failures join the failure groups by Label | Adds crawl outcomes to a read-time grouping query. |
| #1239 Schemas in the language's own types: Zod, Pydantic and `schemars`, strict inputs and results that keep unknown fields | SDK schema adapters; client-side only. |
| #1240 Sinks: delivery status, dead letters, and a Sink that needs attention in "Needs you" | Sink status read when shown; no per-Record work. |
| #1241 One generated error class per code under seven groups, with the fields every error carries | Generated SDK error classes; client-side only. |
| #1243 A call raises or returns, never both: outcomes in values, honest AI results and partial data | SDK result and error shapes; client-side only. |
| #1245 `settings` in code: one typed option on every value that opens a scope | Typed settings option in SDKs; client-side only. |
| #1246 `act`, `observe` and `extract` as Plan steps with a typed result | SDK typing of the AI verbs; no engine per-unit work. |
| #1247 Records marked new, changed or gone against a Schedule's Baseline | Baseline comparison computed when shown, off the Record path. |
| #1248 Plan files on the program's machine: replay with no model and a file a person reviews | Plan file read and written once per Run by the SDK. |
| #1249 Paid fetches: allow a domain that asked for payment, and a page paid for but not received | Needs-you items and settings writes for paid fetches. |
| #1250 `--plan <file>` on a start | CLI `--plan` flag; Plan file read once per Run. |
| #1251 `catalog-run start`: typed data from a page with no program | CLI code-free Run start, once per Run. |
| #1252 Datasets as a named token's scope | Adds a scope value to #1151's check; no new per-unit work. |
| #1253 The manifest of hand-written members and the Quick check that keeps the SDKs honest | SDK manifest Quick check; test infrastructure only. |
| #1254 `call(operation, params?, options?)`: the way down to an operation the SDK does not generate | SDK raw call escape hatch; dispatch unchanged. |
| #1256 Deprecated and Experimental markers in each language's own way | Deprecation markers in SDKs; client-side only. |
| #1257 `Connection`: the type both Products' connections implement, `openRun` and the client package as a peer dependency | Connection type in SDKs; client-side only. |
| #1259 `connect()` finds the engine: `auto`, `attached`, `linked`, `daemon` or `{ url, token }`, and says which it reached | Engine discovery at connect(); once per connection. |
| #1260 The seccomp profile: Docker's default plus exactly the user-namespace rules | Seccomp profile generation; deployment files only. |
| #1261 The `auto` fallback: a Daemon that refuses by Product or version never changes where the program runs unseen | Auto fallback at connect(); once per connection. |
| #1262 `init --deploy compose`: the deployment template set from one source | Deployment template rendering; CLI, once per init. |
| #1263 `not_in_product` and the provisioning errors name the Product and the command that fixes them | Error codes and fixes; raised once, no per-unit work. |
| #1264 Two Products on one machine: one home, one Daemon per major, and nothing collides | Two Products on one machine; install and connect only. |
| #1265 The Compose deployment runs safely and passes its smoke check | Compose deployment smoke; deployment files only. |
| #1266 Closing: the tree Connection, Run, Session, Page, scoped forms and idempotent `close()` | SDK close semantics; once per value. |
| #1267 Signals, forks and an engine that ended: no handler, `connection_forked`, `engine_exited` | SDK fork and engine-exit handling; client-side only. |
| #1268 A small `/dev/shm` switches Chromium to `--disable-dev-shm-usage` and says so | Once per browser launch; reuse engine-start mountinfo read. |
| #1269 Logs through the host's logger, and the engine's log level follows what the logger records | SDK log levels; engine's $/log events already exist. |
| #1270 Unknown values from a newer engine are kept, never raised | SDKs keep unknown values; client-side only. |
| #1271 Container cases: PID 1 reaping, a mid-run `docker stop`, the sandbox on, no silent downloads | Container lifecycle cases; once per stop or start. |
| #1272 Messages read in the caller's spelling while `operation` and `limit` stay canonical | Message rendering per error; no per-unit path work. |
| #1273 Block causes as wire values, `retryAfter` as two types, and the details every error code completes | Error details and wire spellings; contract shapes only. |
| #1274 Cancellation reaches the engine and `call_cancelled` means someone else stopped the call | Cancellation via existing $/cancel; client-side only. |
| #1275 Python: a value from another loop or thread fails at once, sync Ctrl+C cancels, and Windows works under either loop | Python loop and thread checks; client-side only. |
| #1276 Helm chart tracer: one pod that passes the `restricted` Pod Security level | Helm chart tracer; deployment files only. |
| #1277 Events by meaning: listeners, iterators and Rust's `EventStream`, with no queue in the SDK | SDK event shapes; engine events unchanged. |
| #1278 Configuration-only keys, `explainSettings()` and refusals that say how to fix | Settings refusals and explainSettings; once per scope. |
| #1279 The chart installs from the published artifact in `kind`, and a deleted pod resumes its runs | Chart install and resume in kind; test infrastructure only. |
| #1280 The `env` map, the closed `GOODFETCH_` list and the project file | SDK env map and project file; once per connect(). |
| #1281 `resolveSecret` on `connect()`: the program answers `caller:` references at the moment of use | Secret resolution at moment of use; not a Shared path unit. |
| #1282 `finish()`, `stop()` and `progress()`: how a Run ended and what it waits for | Run end and progress subscription; once per Run. |
| #1283 Seccomp modes: the Security Profiles Operator, GKE Autopilot and `hostUsers: false` | Seccomp modes in chart; deployment files only. |
| #1284 `openRun(runId)`, `RunView`, the Run's Key-value store and the Checkpoint and trace copies | Reads of an ended Run and its Key-value store; no Shared path. |
| #1285 The chart as a Daemon group: any number of pods, a named Lead Daemon, refusals at render | Chart Daemon group values; render-time refusals only. |
| #1286 `startRecipe` and `startCatalogRun`: work that outlives the program is explicit | Handler-less Run starts; once per Run. |
| #1287 `run.effect`: declare an Effect once and a restart gets its value back | Effect journal per declared Effect; not a Shared path unit. |
| #1288 The chart's Operator policy, Secret references and declared tokens | Chart Operator policy and Secrets; deployment files only. |
| #1289 `TestEngine`: a real engine in a temporary home that never touches the network | Testing kit's engine start; once per test. |
| #1290 Chart exposure: Services, Ingress or HTTPRoute, TLS, NetworkPolicy and per-pod addresses | Chart exposure resources; deployment files only. |
| #1292 Group lifecycle in the chart: a backup before a lead change, and the `run adopt` Job | Chart backup and adopt Jobs; deployment files only. |
| #1293 Fakes for a program's own unit tests and `reconcile` for a person's answer | Testing-kit fakes and reconcile; no engine per-unit work. |
| #1294 Testing guide and API-stability checks for the kits | Testing guide and API-stability checks. |
| #1295 A Browser host from Collie's image on Compose and Kubernetes, and the remote-engine guide | Browser host deployment files and guide; no engine work. |
| #1296 Getters in one round trip and `evaluate` with one explicit JSON argument | SDK getters and evaluate over the driver's existing evaluate (both worlds). |
| #1297 Routes, dialogs and handlers by the written rules, and page events under Playwright's names | SDK route and handler rules over existing compiled routes. |
| #1298 Chart upgrades: rolling by drain, major upgrades through `upgrade-files`, `helm test` and autoscaling | Chart upgrades and autoscaling; deployment only. |
| #1299 Waiters armed when called, the wait list and no fixed sleep | SDK waiter arming; no Shared path unit gains work. |
| #1300 URL globs and regular expressions that run in the engine, and predicates that run in the program | SDK pattern translation; engine compiles UrlPattern once at arming. |
| #1301 Uploads, Downloads, `screenshot()`, `pdf()` and `mhtml()` against a remote engine | Uploads, downloads and captures per call; no Shared path. |
| #1302 The systemd system unit, the AppArmor profile and the Ubuntu smoke run | systemd unit and AppArmor profile; deployment only. |
| #1303 `page.request` through the page's Session | page.request options; page.request is not a Shared path unit. |
| #1305 Browser setup as typed settings where a Session opens | Session-open settings typed in SDKs; once per Session. |
| #1307 Frames reached only through frame locators, and the excluded calls fail the type checker | SDK frame locator surface and excluded calls; typing only. |
| #1308 `page.formats` and `locator.scrollCollect` in each language | SDK typing; formats and scrollCollect engine work is area 6's. |
| #1309 AWS Lambda: a container image, a handler per language and runs that outlive the 15-minute limit | Lambda image and handlers; deployment only. |
| #1310 `run.newSession({ source, login, settings })` and a Session that closes with its pages | Session open and close; once per Session. |
| #1311 `saveIdentity` and `identities`: an identity is kept without leaking | Saved identity management; not per unit. |
| #1312 Google Cloud Run service: an Exposed Daemon that fits Cloud Run's contract | Cloud Run service deployment; deployment only. |
| #1313 The Human pause: wait for a person without a sleep, with every clock stopped | Human pause per call; not a Shared path unit. |
| #1314 `session.grantLiveView` and `revokeLiveView`: a person finishes a step in the Live view | Live view grants; once per grant. |
| #1315 `secret("name")` placeholders typed to the positions that take them | SDK placeholder typing; refusals happen before sending. |
| #1316 Google Cloud Run job: a Recipe per execution started by Cloud Scheduler | Cloud Run job deployment; deployment only. |
| #1317 Cookies and page storage in the SDKs only | Cookie and storage calls; SDK-only operations, not per unit. |
| #1318 The scripted `login` step, `session.page_check` and `logged_out` with its recovery steps | SDK spelling of the engine's existing Page check and login step. |
| #1319 The edge rule: `edge_handlers_unsupported` and the edge runtime detection | Edge rule in the client package at connect(). |
| #1320 `blocked` and `challenge_unsolved`: a block is a typed fact and the Session stays open | SDK typing of blocked and challenge_unsolved errors. |
| #1321 Edge Client conformance on workerd, Vercel's edge runtime and Deno | Edge conformance cases; test infrastructure only. |
| #1322 One proxy per Session and basic-auth credentials as settings | Proxy and credentials settings; once per Session. |
| #1323 `page.lend()`: a CDP endpoint that shows only the page's Session | SDK lend surface; the engine relay is area 5's ticket. |
| #1324 Deployment guides generated from the template set, with the target table | Generated deployment guides; docs only. |
| #1325 TypeScript lending helpers: `lendToPlaywright` and `lendToPuppeteer` | TypeScript helpers over page.lend; SDK only. |
| #1326 Python lending helpers: `lend_to_playwright` and `lend_to_browser_use` | Python helpers over page.lend; SDK only. |
| #1327 Doc text that opens with the rule a user missed | Doc text check; docs only. |
| #1328 Lending docs: the turn-taking rule and what a Lent page keeps and loses | Documentation and one error code; no per-unit work. |
| #1329 `run.borrow(page)`: a crawler's Playwright or Puppeteer page becomes a Collie `Page` | SDK borrow surface; once per borrow. |
| #1330 The Operations guide: probes, metrics, drain, backups, upgrades and rollback | Operations guide; docs only. |
| #1331 Prove the Python sync `borrow` relay before it is published | Python sync relay prototype; SDK only. |
| #1332 `expect`: a Success check the engine evaluates, never an `AssertionError` | SDK expect over the engine's Success check; no new per-unit work. |
| #1333 `ariaSnapshot({ mode: "ai" })` bounded with Element references, and `getByRef` | SDK typing; the snapshot read itself is area 9's. |
| #1334 Remote access: Caddy, nginx, Tailscale through Headscale, and a Cloudflare Quick Tunnel | Remote access guide; docs only. |
| #1335 `extract.locale` and `extract.timezone_id` typed in `settings` | Settings typing in SDKs. |
| #1336 `site_preferences_refused`: use signals are honoured where content leaves for a model | One check per AI verb before sending; not a Shared path. |
| #1337 Foxhound's deployment templates: names, a crawl starter and the Compose Valkey example | Deployment templates. |
| #1338 Native schema validators join the extraction retry loop | Native validators per extraction attempt; no Shared path. |
| #1339 `page.agent(task, options)`: one call reaches a goal with a verdict | SDK Agent task surface; Agent steps are not a Shared path. |
| #1340 Foxhound's chart from the one source, the rendering check and the Browser host on Collie's image | Helm chart and rendering check. |
| #1341 The Success check guards the task, not the answer after it | Success check callback in the program; SDK only. |
| #1342 Agent tools: the program's own functions the model may call | Agent tools run in the program; SDK only. |
| #1343 Foxhound's smoke run on Compose, `kind`, systemd and the Cloud Run contract, and `-slim` for HTTP-only crawls | Deployment smoke runs. |
| #1344 Watch a task and graduate it to code: `steps()` with every action as code | Agent step stream; SDK only. |
| #1345 Model references, provider settings and typed provider failures | Model reference and provider settings typing; per scope. |
| #1346 SDK handlers of each Plugin kind: one option per kind on `connect()`, registered and chosen by name | SDK handler registration on connect; once per connection. |
| #1347 Foxhound on Lambda: a crawl that outlives the 15-minute limit across invocations | Lambda handler template. |
| #1348 The program's own Model provider: no provider SDK needed | Program's Model provider handler; SDK side of a model call. |
| #1349 `ModelUsage` on every AI result and error, and `run.usage()` | Usage typing on results; reads only. |
| #1350 Foxhound on Cloud Run: a crawl Recipe as a Cloud Run job | Cloud Run job template. |
| #1351 The Model allowance: a Run that finishes without the model | Allowance settings typing in SDKs; engine side is #965. |
| #1352 `scriptedModel` and `scriptedExtraction`: AI code tested with no live model | Testing-kit scripted model; test values only. |
| #1353 Crawl handlers on an edge runtime fail at once with `edge_handlers_unsupported` | SDK edge runtime refusal. |
| #1354 Rust handler streams: declared names, one reader and `fail(BoxError)` | Rust handler streams; SDK only. |
| #1355 Start a crawl Recipe from an edge function and watch it to its report | Edge function Journey. |
| #1356 WebSocket to an Exposed Daemon, reconnect with backoff and edge runtimes | SDK WebSocket transport and reconnect; client-side only. |
| #1357 The Helm upgrade: every released minor's Foxhound chart under a Partitioned crawl | Helm upgrade test. |
| #1358 Daemon groups from the client: `run_moved`, `groupMember` and `lead_unreachable` | SDK follows run_moved; once per start. |
| #1359 MCP server tracer: `goodfetch-collie mcp` over stdio serves a tool and returns an Error code as a result | MCP server is a Client; calls enter the engine through `rpc`. |
| #1360 The MCP projection: tool names, annotations and one description per operation | MCP tool projection generated at build time. |
| #1361 Skills: one source rendered per Product, `skill install` and a valid Claude Code plugin | Skills generated at build time and installed by a command. |
| #1362 Foxhound's deployment guide parts: Partitioned crawls over pods, the Politeness store and scale-down | Deployment guide parts. |
| #1363 Profiles: `browser`, `agent`, `run`, `code` and `full`, identical in both executables' servers | MCP profiles chosen once per server start. |
| #1364 The AI and site tools join their MCP profiles | Adds existing operations to MCP profiles; Client side. |
| #1365 Streamable HTTP: loopback, `Host` and `Origin` checks and a per-launch token | MCP Streamable HTTP transport, a Client of the engine. |
| #1366 WARC output under the storage levels | Storage metric label for the WARC folder. |
| #1367 A remote MCP server: TLS and OAuth 2.1 with the audience checked and no passthrough | MCP server's TLS and OAuth, outside the engine's paths. |
| #1369 The four MCP revisions with their expected-failures files | MCP revision negotiation and conformance runs, Client side. |
| #1370 Cloud-readiness conformance suite tracer: the gateway target kind and routing cases on the Stub gateway | Conformance suite target kind and routing cases; tests only. |
| #1371 Docs and skills served over MCP | MCP resources served from embedded docs, Client side. |
| #1372 Observations that keep diffs and send a screenshot only when the page changed | Snapshot diffs kept in the MCP server, a Client, per session. |
| #1373 The Agent on the command line and over MCP: `page agent` and `page_agent` | CLI and MCP surfaces of the existing Agent operation. |
| #1374 Engine target tracer: an engine driven exactly as a gateway drives one | Engine target harness; tests only. |
| #1376 The remaining skill topics: page, AI, agent, sessions, runs and testing | Skill topic content and generator checks only. |
| #1377 `engine.shared_host`: a per-run engine knows it shares its machine | Operator-policy key read once per engine or Run. |
| #1378 Framework adapter tracer: `@goodfetch/collie-ai-sdk` for the Vercel AI SDK | Framework adapter over the TypeScript SDK, Client side. |
| #1379 Detaching behind a gateway, start by `runId`, labels and the Clean-up rules version | Start idempotence and labels, once per Run. |
| #1380 LangChain and LangGraph adapters in TypeScript and Python | LangChain adapters over the SDKs, Client side. |
| #1381 The Mastra adapter | Mastra adapter over the SDK, Client side. |
| #1382 Adapter peer ranges, stated and tested | Adapter peer ranges and Release-suite cells. |
| #1383 Checkpoint, migrate and resume on another engine, fenced by the run epoch | Checkpoint and migration, per Run; no per-unit work. |
| #1384 Frameworks reached only through MCP: a model-free example each | Docs examples of frameworks connecting through MCP. |
| #1385 The Notification channel driver and the `webhook` channel | Notification delivery per Run event; no Shared path unit. |
| #1387 Slack, Teams, Discord and Telegram channels | Notification channels' rendering and failure translation. |
| #1388 Email channel: rendering, SMTP and SES | Email channel providers, per Notification. |
| #1389 Email channel: Resend, Postmark, SendGrid and Mailgun | Email channel providers, per Notification. |
| #1391 Webhook verification helpers in the SDKs | Engine-free SDK webhook verification helpers. |
| #1392 Event names and the attention set | Event names and default filter, per Run event. |
| #1393 The caller fails closed: no slice, an unreadable row, an unanswered request | Fail-closed handling per Slice request; no new count. |
| #1394 A Human pause Notification with a link to the Live view | One Notification per Human pause. |
| #1395 Schedule failures notify once per period or wake | Schedule Notifications keyed per period or wake. |
| #1397 Prefactor: an identity-store driver above the local, Lead Daemon and caller backings | Identity-store driver prefactor; per-Session work only. |
| #1398 Notifications in a Daemon group: one event is one message | Notification deduplication at the Lead Daemon, per event. |
| #1399 Caller-held Saved identities: load, a write lease and a sealed write-back | Caller-held identities load per Session; no per-unit work. |
| #1402 `caller:` Secret references answered by the program or the gateway | Secret provider kind; reuse per placeholder is the driver's. |
| #1403 Integrations core: actions, a Notification trigger, subscriptions and the rules of a Client acting for a model | Integrations are Clients over the HTTP listener. |
| #1404 Forwarding fidelity: the run index, positions per engine, `resync`, files by handle, the Live view and `path: null` | Stub gateway forwarding behaviour; gateway side. |
| #1405 n8n node `@goodfetch/n8n-nodes-collie` | n8n node package, a Client. |
| #1406 Zapier app "Goodfetch Collie" | Zapier app, a Client. |
| #1407 `usage.*` answered by the gateway's ledger, shared with the caller-held slices | Stub gateway's in-memory ledger; gateway side. |
| #1408 Make app "Goodfetch Collie" | Make app definition, a Client. |
| #1409 Pipedream components and Activepieces pieces `goodfetch-collie` | Pipedream and Activepieces packages, Clients. |
| #1410 Gateway sign-in, step-up, tokens and person-only operations | Gateway sign-in and tokens; no engine per-unit work. |
| #1411 Power Automate custom connector "Goodfetch Collie" | Power Automate connector, a Client. |
| #1412 The gateway's Saved identity store and the approval rule | Gateway's identity store; gateway side. |
| #1413 Integration docs: which setup reaches which tool | Integration docs; docs only. |
| #1414 A person's Browser host behind the gateway, by the `run` it serves | Browser host relay through gateway; per-Session work only. |
| #1415 SDK weight: import cost, bundle size, startup with the Daemon probe, protocol counts and no telemetry | Measures SDK weight and protocol counts; adds no per-message work. |
| #1416 Proxy limits across per-run engines through the Politeness store | Gateway configures shared Politeness store; configuration only. |
| #1417 npm packages install from a local registry through five managers | npm install harness; release infrastructure only. |
| #1418 `gateway_unsupported` and unclaimed duties | Unsupported-family refusal at the gateway; no per-unit work. |
| #1419 Wheels and crates install into fresh environments | Wheel and crate install checks; release infrastructure only. |
| #1420 The Console conformance suite on the Stub gateway: routing and unclaimed duties | Console conformance on the Stub gateway; tests only. |
| #1421 Smoke scripts on Node, Bun, Deno, `workerd` and Vercel's edge runtime | Runtime smoke scripts; test infrastructure only. |
| #1422 `goodfetch-collie init`: the `browser` starter in three languages | `init` scaffolding command, once per call. |
| #1423 Client conformance by address: the same SDK cases against a gateway | Client conformance by address; tests only. |
| #1424 The `extract` and `agent` starters, using the Fake model in their tests | Starter projects and their tests. |
| #1425 Every starter carries a Dockerfile, Compose file, CI workflow, AGENTS.md, skills and an MCP host config | Starter project files. |
| #1426 Journey: the same SDK program on an Attached engine, then an Exposed Daemon by two variables | Journey test; tests only. |
| #1427 The `recipe` starter and the Limit offer | Recipe starter and the Limit offer, once per `init`. |
| #1428 Package READMEs and install pages that are true and tested | READMEs and install pages; docs only. |
| #1429 Journey: carry a Saved identity to another engine by session export | Journey test; tests only. |
| #1430 Rust install docs: CMake, a compiler, libclang, git and V8 from source | Rust install docs; docs only. |
| #1431 Docs for agents: `llms.txt` content, the `docs` command and the embedded docs | Docs for agents and docs command; docs only. |
| #1432 Self-hosting parity: the engine cases also run against an Exposed Daemon | Engine cases against an Exposed Daemon; tests only. |
| #1433 Migration guide from Playwright | Migration guide; docs only. |
| #1434 Migration guide from Stagehand v4, with a part for v3 | Migration guide; docs only. |
| #1435 Foxhound's cloud cases: crawls land on Foxhound's engine behind a gateway | Gateway routing cases; once per Run start. |
| #1436 Migration guide from Browser Use | Migration guide; docs only. |
| #1437 Migration guides that cannot go stale: "after" samples run, "before" files type-check against a pin | Migration guide checks; test infrastructure only. |
| #1438 A crawl behind a gateway: cursors, retries, a keep-alive crawl and migration by Checkpoint | Cursor reads through gateway; reads leave the writer already. |
| #1439 Live check `migration-guides-current` | Live check of competitor versions; no engine work. |
| #1440 Crawler guides with tested examples and the Selenium note | Crawler guides; docs only. |
| #1442 Lending and crawler Live checks, and the release notes' supported ranges | Live checks and release notes. |
| #1443 Sinks, Exports and declined duties through a gateway | Sinks and Exports through gateway; no Shared path touched. |
| #1444 "Use with other tools" pages and "When you need many pages" | Docs pages; docs only. |
| #1445 Wording rules over what Collie ships | Wording check; build-time only. |
| #1446 Prefactor: a Partition-relay driver above the peer link and a caller transport | Behaviour-preserving move of the partition relay; no work added. |
| #1447 Journey: drive a page from a program in each language, end it cleanly and see a failed call as a typed error | Journey. |
| #1449 Journey: open a page, read it and take its screenshot from the command line, in a script under `--json` and its exit codes | Journey. |
| #1450 A Partitioned crawl on the Stub gateway: one engine per Partition, aggregated as one crawl | Gateway runs one engine per Partition; per-Partition work only. |
| #1451 Add or remove a Partition while the crawl runs | Partition add or remove; per rebalance, not per unit. |
| #1452 Journey: connect a coding agent to the MCP server, have it read a page and finish a Run, then see the Run in the Console | Journey. |
| #1454 Journey: install the skills for a coding agent and find them over MCP | Journey. |
| #1455 Per-domain Politeness across per-run engines | Proves existing shared Politeness store; nothing new built. |
| #1456 Journey: lend a page to a Playwright program and borrow a Playwright page into a Run | Journey. |
| #1457 The Console shows a crawl's screens only for a run on Foxhound's engine | Console screens per Run engine; UI only. |
| #1458 Journey: give a framework's agent the browser tools and run a read, an Effect and a failing call | Journey. |
| #1459 Journey: start a Recipe run and read its report from a workflow tool | Journey. |
| #1460 Journey: start a run and follow its events over the HTTP listener with a token | Journey. |
| #1461 Journey: receive a signed Notification when a Run needs a person | Journey. |
| #1462 Journey: scaffold a project with `init`, run its starter and its test | Journey. |
| #1463 Journey covers for the flagged stories of Runs, Sessions, AI and Models | Journey covers. |
| #1464 Foxhound's SDK packages connect in TypeScript, Python and Rust | SDK packages and `hello`; once per connection, no Shared path. |
| #1465 Engine choice, connect options and configuration-only keys | Connect options and setting refusals; once per connection. |
| #1466 Foxhound's widened types and Rust constants | Generated SDK types and constants; no engine per-unit work. |
| #1467 Crawl tracer in TypeScript: declare, serve and finish a crawl on the event loop | TypeScript runtime over existing `crawl_request.handle` delivery and push. |
| #1468 Crawl in Python: AsyncCrawl, Crawl, decorators and Pydantic Payloads | Python runtime over existing delivery; SDK framing unchanged. |
| #1469 Crawl in Rust: handlers, `serve`, panics and the blocking twin | Rust runtime over existing delivery; wiring checks once per Run. |
| #1470 Typed Payloads checked at enqueue | SDK Payload types; engine `PayloadRules::check` already exists unchanged. |
| #1471 Seeds, sitemap seeds and enqueue answers that are values | SDK seed and enqueue values over existing Frontier admission. |
| #1472 Login steps and the settings a Label may change | Label settings checked at Run start; Session pools built once. |
| #1477 How an attempt ends: stale contexts, skip, retry and handler failures | Attempt endings and errors; existing delivery, no added counted work. |
| #1478 Status rules, redirects and truncated bodies reach a handler honestly | Status-rule endings surfaced in SDKs; engine rules already exist. |
| #1479 Site preferences and the lent page inside a handler | Navigation gate refusal surfaced; existing gate, no new counted work. |
| #1480 Model allowance and Effects inside a handler | Model allowance and Effects through existing ledger and Run paths. |
| #1483 Hook failures, cost and the absence of a failed-request handler | Hook failure endings and per-call deadline; counted work is #1481's. |
| #1485 Rust: Hook and Fetch-backend calls as event streams, and `for_each` | Rust event-stream API over the same handler requests. |
| #1487 Reading Records live, rejected Records and schema drift | SDK readers over existing `run.records` and `run.rejected_records`. |
| #1488 Exports that reach the program's machine whole | Export once per call; no per-unit path. |
| #1489 Read a Run you did not start: `openCrawlRun` | Read-only view opened once; reads outside the attempt path. |
| #1490 Sinks as typed settings | Sink settings typed and checked at Run start. |
| #1492 `startCrawl` returns a CrawlRun and `finish()` returns a report that never reads as more than it is | Start, finish and report; once per Run. |
| #1493 The Run ID names a crawl for good: resume, ended and conflict | Resume comparisons once per start, before any work. |
| #1494 Seeds of a running crawl: `seeds()`, `addSeed`, `addSeeds` and keep-alive crawls | SDK streams seeds into existing `run.add_seeds` admission. |
| #1495 Outcomes: a log of endings, current failed requests and `retryFailed` | Outcome reads and `retryFailed` over existing operations. |
| #1496 `progress()` says why a slow crawl is slow | Coalesced progress telemetry; no per-unit counted work. |
| #1497 `workers`: one crawl across processes | Extra processes join through existing `crawl.serve`. |
| #1498 Runs that keep handlers in the program stay attached | Detach refusals at start; no per-unit work. |
| #1499 Politeness settings in code: safe by default and visible when pressed | Pressed-harder values recorded once per Run. |
| #1500 Crawl-space settings and `explainSettings()` for Foxhound's keys | Settings types and existing URL caps (`bounds.rs`). |
| #1501 Testing a crawl: `TestEngine` and `runHandler` on a saved document | Testing kit's one-attempt Run; outside production paths. |
| #1502 Testing a Hook by name: `runHook` | Testing a Hook by name; test-only. |
| #1504 A crawl inside a service: one connection, a Run per call, and nothing outlives the caller | Connection and Run lifecycle in a service. |
| #1505 The library's logs go to the program's logger and never to standard output | SDK logging channel; no engine per-unit work. |
| #1506 Notebooks, pre-fork servers and event-loop mismatches | SDK event-loop and fork guards. |
| #1507 Journey: run a crawl inside a web service (golden scenario 12) | Journey. |
| #1508 Journey: test a crawl offline (golden scenario 10) | Journey. |
| #1509 Journey variants: J-002, J-003 and J-011 on the three SDKs | Journey variants. |
| #1510 Journey: re-run only a crawl's failed requests | Journey. |
| #1511 Journey variants: J-005 and J-012 on the three SDKs | Journey variants. |
| #1513 `crawl start`, `run export` and `config explain politeness` | CLI commands over existing operations; one-off per call. |
| #1515 CLI: Sink commands, `auth <provider>` and the person-only `run erase` | CLI Sink commands, auth and erase; one-off. |
| #1516 MCP profiles `crawl` and `data` | MCP profiles project existing operations. |
| #1517 Journey variants: J-002, J-003 and J-011 over MCP | Journey variants. |
| #1518 Foxhound's docs, READMEs and agent formats rendered from one source | Docs rendering. |
| #1519 Foxhound's skills and docs resources over MCP | Skills and docs resources. |
| #1520 `goodfetch-foxhound init`: the `crawl` and `extract` starters | Project starters. |
| #1521 Docs rules for the changed and the trapped | Docs rules and checks. |
| #1522 Foxhound's Framework adapters | Framework adapter packages. |
| #1523 Dataset loaders: LangChain and LlamaIndex read a crawl's Records | Dataset loaders read through `openCrawlRun`. |
| #1524 Foxhound's Integrations: start a crawl, get Records, export a Dataset | Integration apps over existing operations. |
| #1526 The Airbyte source for a Dataset | Airbyte manifest outside the engine. |
| #1527 Notification events for Sinks and Records | Notification events on Sink state changes; not per unit. |
| #1528 Collie's skills, profiles, adapter tools and starters live unchanged in Foxhound's executable | Containment check of generated outputs. |
| #1529 Migration guides from Crawlee, Scrapy, Spider and Crawlee with Stagehand | Migration guides. |
| #1530 Coding agents write the crawl golden scenarios from the docs and skills alone | Agent evaluations of docs and skills. |
| #1531 Counted figures: protocol counts, import set and bundle sizes | Counted figures' snapshots; measurement only. |
| #1532 Partitioned crawls: the `partitions` start option | Partitions option, errors and typed values only. |
| #1533 Change tracking in code: `changes`, `missingRecords` and the Baseline setting | SDK change-tracking reads and setting types. |
| #1534 Plugin index workflows: sign and publish the index, and validate entries by pull request | Plugin index CI workflows. |
| #1535 Proving ground: a dependent form, an expiring seeded login and a Firefox profile seed for the golden Journeys | Proving ground fixtures. |
| #1537 Proving ground: pay-per-crawl and x402 pages settled on a local chain | Proving ground payment pages. |
| #1539 Proving ground: solver-service stand-ins and a challenge page that needs a service worker | Proving ground solver stand-ins. |
| #1540 Proving ground: WebMCP, search and language pages, a daylight-saving page and the Fake model's image answers | Proving ground pages and Fake model. |
| #1544 Run start and resume under ADR 0102: the idempotent start, the comparison of inputs and the keys fixed for a Run's life | Run start and resume are per Run. |
| #1548 The engine's and a Run's transition tables (ADR 0128): every invalid transition is refused | Pure transition functions per lifecycle event. |
| #1549 Journey helpers for the `mcp-stdio`, `mcp-http`, `skill`, `adapter` and `integration` variants | Journey helpers; test infrastructure. |
| #1562 Owner step: register the Mac as the self-hosted runner, with its throwaway Linux VM (ADR 0108 checklist, #545) | Owner step registering a self-hosted runner. |
| #1574 `just compare-spider`: a one-off wall-time and memory comparison on the owner's machine | One-off comparison recipe. |
| #1575 `just targets`: the Performance targets report for each owner test | Performance targets report, owner's machine. |
| #1576 Resident-memory ceilings for native macOS Intel and Windows arm64 | Resident-memory ceilings for two more OSes. |
| #1586 Linux CI container boots systemd so the Daemon's service installs run in CI | CI container setup only. |
| #1590 Test infrastructure, part 1: the real-run and per-OS acceptance criteria not yet proven | Real-run proofs of test infrastructure; no code. |
| #1594 Mark a Setting's Session scope in the catalog | Catalog scope mark; Session settings checked once per open. |
| #1603 Under several handlers at once, a page a handler opens takes its own attempt's navigation gate and lease | Attributes each opened page to its attempt; no counted work. |
| #1605 Cookies across a crawl request's redirect hops on the HTTP tier | In-memory cookie jar across hops; no counted work. |
| #1606 `Authorization` on a crawl request is dropped at a cross-origin hop, end to end | Conformance case for a header drop; tests only. |
| #1607 `page.reload`, `page.go_back` and `page.go_forward` take their own `timeout` | Adds a timeout parameter to three navigation operations. |
| #1609 Chain `https://` upstream proxies through the egress relay | Relay chaining per connection. |
| #1610 The admit_seeds benchmark admits inside a transaction, as the crawl does | Benchmark scenario fix outside B1-B3; no path work changes. |
| #1615 A Label's pool Sessions open from every Session source kind, and keep their cookies across a resume | Session sources per Label pool; per-Session work only. |
| #1616 Bind a local-address interface to its device on Linux, macOS and illumos | Socket device binding per connection setup; no counted work. |
| #1617 The group-member settings scope for machine-specific settings | Adds a settings scope resolved per Group member. |
| #1618 Chromium's renderers sandboxed on macOS and Windows: the OS-specific check | OS-specific sandbox test. |
| #1619 Recognize EKS on Fargate as a platform with no browser sandbox | Platform detection at startup. |
| #1620 The relay presents client certificates in terminating mode | Relay client certificate per connection. |
| #1622 Browser-fetch's conformance gate decodes `br` and `zstd` over HTTPS | Conformance gate over HTTPS; tests only. |
| #1623 A movable Session's robots.txt fetches for other origins go through browser-fetch | Robots.txt fetch transport, once per origin. |
| #1624 The relay's mirrored ClientHello carries the browser's ML-DSA signature algorithms | Relay ClientHello mirroring per connection. |
| #1625 egress.local_addresses at a Session's own scope | Local addresses per Session; per-Session work only. |
| #1627 A handler page waiting for the Resource governor's admission is named in its lease's end | Names the governor admission wait; no counted work per sample. |
| #1628 Rebinding cases end to end on Chromium in a DnsNamespace | Rebinding test runner; tests only. |
| #1630 A tier race's navigation waits in Chrome's HTTP cache lock behind browser-fetch | Header choice for a tier race's two requests; no counted work. |
| #1631 A test run refuses a test tool built from an older tree | Test tool staleness check; tests only. |
| #1632 Browser-fetch hands a response's head over before its body, so a refused type is never read whole | Reads less of a refused body; uses existing streamed read. |
| #1634 Vendor the rs-trafilatura fork once it needs a patch | Vendoring a dependency, no runtime change. |
| #1635 test-support: catch a test that turns off the engine's JSON log its exercise record needs | Test-support audit check; tests only. |
| #1636 browser.block_resources does not hold on a crawl's browser runs under fetch.tier: auto | Bug fix so blocking holds on sample runs; no new work. |
| #1638 A forced tier still takes one browser sample for the wall test | One browser sample per Label on a domain; not per page. |
| #1639 A declared total that stops matching samples the Label again | Calls invalidate on a total mismatch; no per-unit work. |
| #1640 The HTTP tier leaves through the Run's proxy | Proxy taken once per pool Session; HTTP tier client per Session. |
| #1643 A person-attended Client other than the Console gives person-only answers under step-up | Person-only answers from other Clients; per person action. |
| #1644 A Client newer than a Daemon settles on the Daemon's minor | Version settling at hello; once per Connection. |
| #1645 A `surface` value for third-party interfaces | A surface value at hello; once per Connection. |
| #1646 Live view for a remote viewer whose signalling passes through a relay | WebRTC signalling per viewing; no per-unit path. |
| #1647 A browser Client served from another origin reaches an engine directly | Origin-checked browser WebSocket; checked once per Connection. |
| #1648 An Operator-configured endpoint presents its own client certificate | Client certificate per operator endpoint connection. |
| #1650 Goodfetch Cloud will not serve the Console: update the glossary and ADRs 0096 and 0097 | Glossary and ADR wording only. |
| #1654 The CLI signs in to a remote engine with a code confirmed in a browser, bound to a key it keeps | CLI device sign-in; once per sign-in. |
| #1655 A Notification sent through the relay carries only the event's kind and IDs | A Notification channel per event; no Shared path. |
| #1656 Forward a live run's stop and checkpoint to the engine that owns it | Forwards stop and checkpoint like abort; per person action. |
| #1657 The light-browser bridge sends from the Session's local address and applies its TLS settings | Bridge connection settings per Session; no per-unit work. |
| #1658 Cloud Secret providers read ambient credentials (instance, container and metadata sources) | Cloud credential sources per Secret resolve. |
| #1659 discovered_api answers network waits and kept responses from its replayed calls | Handler waits answered in memory from replayed calls. |
| #1661 Test support: Playwright integration tests fail when target/ is a symlink outside the repository | Test project location fix; tests only. |
| #1662 Publish each release's Operation catalog as a signed release file | Release pipeline publishes a signed catalog file. |
| #1663 One spec owns the Recipe format: its manifest, what a package holds and the runtimes it may name | Recipe format spec; no per-unit work. |
| #1664 A Recipe's digest is computed one way by every tool | Recipe digest definition; computed per package. |
| #1665 A Recipe declares its own typed inputs and its output, with example values apart from defaults | Recipe inputs checked once before a run starts. |
| #1667 Push, install and list Recipes by digest, on a Daemon or a gateway | Recipe install and list operations; per person action. |
| #1668 J-002 and J-011's cli variants pass on a code-free crawl | Journey variants for the code-free crawl; tests only. |
| #1669 The CLI and the MCP server reach a remote engine through `GOODFETCH_URL`, as the SDKs do | CLI and MCP transport choice; once per Connection. |
| #1670 A caller-held Limit carries its holder's own name and fix, and every Client shows it | Display name and fix on a Limit row; no per-unit work. |
| #1671 A host adds its own notes to a run, including that the host caused its failure | Host notes on a Run; no per-unit work. |
| #1672 Every retryable refusal can say when to try again | retryAfter field on errors; no per-unit work. |
| #1673 The Cloud-readiness suite tests `schedule.*` against a gateway | Conformance suite target; test infrastructure. |
| #1675 Catalog operations for an engine's accounts and named tokens, with granting left to a person | Account and token catalog operations; per person action. |
| #1676 A crawl through a forward proxy learns the proxy's ceiling from its own 429, and no navigation hangs on a paused request | Bug fix for proxy 429 and a stuck request; no new work. |
| #1677 Vitest visual screenshots taller than the 800 px viewport are clipped white | Console visual-test screenshot fix; test infrastructure only. |
| #1678 XLSX Export refuses a Dataset past Excel's sheet limits | XLSX Export limits, once per Export. |
| #1679 Read XLSX Exports back with a spreadsheet library | Test reading XLSX Exports; tests only. |
| #1680 Generate the crawl's progress types for the Console and the SDKs | Codegen reaches a progress type; no runtime change. |
| #1681 Redo a run on your own machine: fetch its Recipe version by digest and start a local run from its recorded settings | Fetch a Recipe and redo a run; once per run. |
| #1682 A person saves named inputs and run settings for a Recipe, and starts runs and Schedules from them | Saved Recipe inputs; checked once per start. |
| #1684 The recorder reachable by any person-attended Client, returning each step as a target as it happens | Recorder operations opened to other Clients; per person action. |
| #1685 A step list: an ordered, code-free automation a Recipe can name as its entry, replayed with no model | Step-list format replayed through the existing verbs. |
| #1686 Render a whole program from a step list or a CrawlDeclaration, and a Recipe made from it | Code rendering from data; no runtime path. |
| #1687 A Recipe carries its sample as a Response recording, replayed as its test without reaching the site | Recipe sample test replayed from a recording. |
| #1688 recipe pull by digest, and a push that records its parent and warns before replacing a newer version | Recipe pull and push parent check; per person action. |
| #1689 Working copies of Recipes kept on the engine and edited live by several people | Working copies on the engine; per edit, no Shared path. |
| #1690 Give the egress relay its column of the endpoint table | Endpoint table column; test only. |
| #1691 Cloud-readiness: Recipe routing and the Recipe-worded not_in_product | Recipe routing at the Stub gateway; once per start. |
| #1696 An attended Session where a person and an agent take turns, with take control at any time | Attended Session turns; per person or agent turn. |
| #1697 A building conversation on the engine: an AI that edits a Recipe's working copy with the person, using the page verbs | Building conversation loop over existing page verbs. |
| #1698 AI actions returned as step-list steps, with a test status | Shapes AI results as step-list steps; no per-unit work. |
| #1699 Recorded steps classed as Effects by the engine's rules | Classes recorded steps as Effects; per recorded step. |
| #1700 A run option that stops before any Effect, for previews | Stop-before-Effect option uses existing Effect classing. |
| #1701 Model spend per conversation, with a threshold that asks before going on | Model spend per conversation; per model call. |
| #1704 Collie's hand-written command groups move into goodfetch-cli, one module per group | CLI module move. |
| #1705 `xtask clients check` requires each cli case on both executables unless the case is Foxhound's | xtask clients check rule; tooling only. |
| #1706 Protocol-count snapshots carry bytes after the shared redactions | Snapshot redaction in tests. |
| #1707 An outside agent joins an attended Session and proposes changes to a Recipe's working copy | Outside agent joins a Session; per turn. |
| #1708 Person-only answers reached from an agent surface by a link, never through the agent's host | Person-only answers by link; per person action. |
| #1709 A ChatGPT-plan Model provider through Sign in with ChatGPT's open-source flow | New Model provider sign-in; per model call. |
| #1710 The provider layer reaches current models: OpenAI's Responses format, models that refuse a forced tool call, and a local server's context | Provider format adapters; per model call. |
| #1711 Gateway calls never fall back to the gateway's own credentials, and can ask for zero data retention | Gateway options per model call; no Shared path. |
| #1712 Evaluations for the building conversation: one campaign per job, cheapest model first | Evaluation campaigns; test infrastructure. |
| #1713 Limit flags on the CLI's starts (`run start`, `crawl start`) | CLI Limit flags. |
| #1714 Exit 21 through the binary once the engine can give a crawl a Verdict other than `verified` | CLI exit-code test. |
| #1716 A ranked candidate shortlist with "none of these" as the check model's input | Shortlist built from observe's candidates; per model call. |
| #1717 A crawl drafted from a prompt is surveyed, sampled and checked by the engine, and the model only chooses | Prompt-to-crawl survey and samples; once per draft. |
| #1718 The provider layer reaches hosted decision APIs: OpenAI's Decisions shape and the System One shape, with recordings and prices | Decision provider codecs; per model call. |
| #1719 Automation issues go to the spec that owns their test, not to the current milestone | Issue routing for automation; process only. |
| #1726 Incremental gate hardening left out of #1725: signed records, variable refusals, planted-break tests | Merge gate hardening; no runtime work. |
| #1730 fetch-tier: a Headers type for the driver's ordered header list | Type refactor of header lists; no counted work. |
| #1733 The Daemon soak's placement and time | Daemon soak placement and timing. |
| #1739 page.request reads a body bounded by the Run's limits on the browser path | Bounds page.request and route.fetch bodies; no new per-unit count. |
| #1742 Tests that count a hostile table's cases fail when a case is added, before checking any case | Test-table hygiene; test code only. |
| #1745 Test the CLI's wait for a stopping Daemon | Test seam for a stopping Daemon; test code. |
| #1746 Snapshot daemon install's output, human and --json | CLI snapshot filters for `daemon install`. |
| #1750 No executable carries or opens a Console | Removes the Console; adds no per-unit work. |
| #1751 The engine serves no Console | Removes the Console; adds no per-unit work. |
| #1752 Sign-ins, the Console key and the accounts store leave the engine | Removes the Console; adds no per-unit work. |
| #1753 The Console packages leave the workspace | Removes the Console; adds no per-unit work. |
| #1754 Gates, budgets and the Feature inventory stop checking a Console | Removes the Console; adds no per-unit work. |
| #1755 Journeys lose their console surface | Removes the Console; adds no per-unit work. |
| #1756 Documents and ADR notes describe no Console | Removes the Console; adds no per-unit work. |
| #1757 Sweep: nothing of the served Console is left | Removes the Console; adds no per-unit work. |
| #1761 .gitignore's target/ pattern hides source folders named target | .gitignore pattern and a lint check. |
| #1762 A crawl held by its rate cap says why in run.progress | Adds a waiting reason to the progress value; no counted work. |
| #1769 Export the Daemon's live heap as the goodfetch.daemon.heap metric | Ops metric gauge per scrape; not a Shared path. |
| #1772 Politeness treats a site's 429 on a direct HTTP-tier fetch as the proxy's when the Run sets a proxy | Politeness judged by the attempt's route; no counted work. |
| #1776 A Proving ground case for the light browser's bridge refusing a script's cross-host fetch | Proving ground test case only. |
| #1778 A browser page's navigation to a refused address answers no egress_refused code through NavigationGate | NavigationGate refusal kind; no counted work. |
| #1786 The gate's stress step can't list its base from a checkout: the checkout has no Console build | Gate stress step setup; no runtime work. |
| #1797 Foxhound CLI tests leave engine processes running for hours after they end | Test teardown of leaked engine processes. |
| #1799 Drift details a Client can show: what the step kept, what the engine found now, and what the Repair proposes | Drift details in the Run report; per drifted step. |
| #1800 Check a Plan target against a page snapshot in a Run trace | Offline match against a trace snapshot; per person action. |
| #1801 Plan version history: list, diff, revert, and apply an entry to another Plan | Plan version history operations; per person action. |
| #1802 A setting that holds every new Plan version for a person, read-only ones included | Setting holding Plan versions pending; per version. |
| #1803 Fold a Plan's repaired entries into a step-list working copy | Folds Plan entries into a step list; per person action. |
| #1804 Start a building session replayed up to a given step, stopping before any Effect | Replay up to a step; per building session. |

## Tickets the gate's benchmark ticket must block

245 tickets: the criteria, rewrite and decided tickets and those swept later, each blocked by its path's gate ticket under #1837. The same list is in the comment on #1577.

#214 #217 #225 #227 #231 #232 #233 #234 #235 #238 #239 #240 #244 #248 #256 #258 #631 #632 #634 #635 #643 #655 #656 #657 #659 #671 #676 #677 #684 #685 #694 #699 #702 #703 #704 #707 #708 #709 #712 #713 #715 #717 #718 #720 #721 #722 #723 #724 #725 #726 #733 #739 #742 #744 #745 #749 #750 #751 #753 #754 #755 #757 #760 #762 #767 #768 #769 #775 #776 #777 #778 #779 #788 #791 #793 #794 #795 #796 #797 #799 #800 #801 #802 #803 #807 #810 #814 #823 #824 #825 #828 #829 #833 #834 #840 #841 #844 #848 #849 #853 #857 #863 #865 #867 #869 #871 #878 #882 #883 #893 #897 #902 #914 #920 #926 #929 #932 #936 #940 #945 #947 #951 #954 #956 #957 #958 #965 #979 #994 #998 #1003 #1009 #1015 #1018 #1021 #1022 #1024 #1026 #1029 #1030 #1031 #1032 #1033 #1037 #1040 #1058 #1065 #1080 #1087 #1090 #1095 #1097 #1098 #1104 #1106 #1109 #1111 #1113 #1117 #1121 #1131 #1137 #1140 #1144 #1145 #1150 #1151 #1154 #1157 #1158 #1164 #1167 #1205 #1217 #1291 #1304 #1306 #1375 #1386 #1390 #1396 #1400 #1401 #1441 #1448 #1453 #1473 #1474 #1475 #1476 #1481 #1482 #1484 #1486 #1491 #1503 #1514 #1525 #1541 #1546 #1547 #1550 #1551 #1558 #1566 #1596 #1602 #1604 #1611 #1612 #1621 #1626 #1629 #1637 #1641 #1642 #1649 #1651 #1652 #1653 #1666 #1674 #1683 #1692 #1693 #1694 #1695 #1715 #1747 #1775 #1782 #1783 #1784 #1805 #1807 #1808 #1809 #1810 #1811 #1812 #1813 #1815 #1817 #1820 #1900
