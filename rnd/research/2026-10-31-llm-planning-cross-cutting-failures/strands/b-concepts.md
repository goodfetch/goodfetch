# Strand B: what established engineering calls this failure class

Research strand for `2026-10-31-llm-planning-cross-cutting-failures`. Accessed 2026-10-31 for every source.

**The failure under study.** Each feature slice (trap detection, canonical URLs, politeness, durability, ledger, robots.txt, datasets, cosmetic clean-up) added its own layer to every crawled page's path. Each layer redid base work: its own HTML parse, its own database commit, its own SQL prepares. Every slice was correct, tested and reviewed on its own. Together they made the system about 100x slower than a competitor. No architecture decision owned the shared per-page pipeline or its total cost. Speed was left for a later "optimization pass". Per-change budgets compared each change with the one before, so slow accretion got through. The absolute ceilings were informative and did not block.

**Verification marks.** VERIFIED means I read the quoted words at the primary source, or in a faithful reproduction of it, which is named. UNVERIFIED means I saw the wording only in secondary sources or search snippets. Those entries are paraphrased, or the quote is labelled as secondary. Where only part of an entry was checked, the entry says which part. Some PDFs could not be read by the fetch tool; I extracted their text locally from the bytes it had already downloaded, and the entry says so.

---

## 1. Quality attributes and architecturally significant requirements

### 1.1 ISO/IEC 25010, performance efficiency

- **Source:** ISO/IEC 25010 product quality model. The reproduction at iso25000.com lists nine characteristics, including "Interaction Capability", which matches the 2023 edition. https://iso25000.com/en/iso-25000-standards/iso-25010
- **Definition:** performance efficiency is "the degree to which a product performs its functions within specified time and throughput" and uses resources efficiently. Its sub-characteristics are time behaviour, resource utilization and capacity.
- **Status:** VERIFIED at the reproduction. The ISO text itself is paywalled and was not read.
- **Transfer to agent workflows:** The standard gives the spec a name for the missing requirement: "performance efficiency: time behaviour of the per-page path". It belongs to the product, not to a feature, so it cannot sit in one ticket. A spec template that lists the 25010 characteristics and makes the author mark each one in scope or out of scope stops a quality being left out just because nobody named it.
- **Limits:** A taxonomy says nothing about the target, the owner or the test. Without a measured value it becomes a checklist that agents tick off.

### 1.2 Quality attribute scenarios (Bass, Clements, Kazman, *Software Architecture in Practice*)

- **Source:** Bass, Clements, Kazman, *Software Architecture in Practice*. Read in a course-hosted copy of the 2nd edition (Addison-Wesley 2003, ISBN 0-321-15495-9), section 4.3. https://people.ece.ubc.ca/matei/EECE417/BASS/ch04lev1sec3.html. The 4th edition is from 2021.
- **Definition:** "A quality attribute scenario is a quality-attribute-specific requirement. It consists of six parts": source of stimulus, stimulus, environment, artifact, response, response measure.
- **Status:** VERIFIED for the 2nd-edition text. The 4th-edition wording is UNVERIFIED.
- **Transfer:** This is the format a spec needs for whole-system performance. For example: "a crawl of N pages (stimulus) from the crawler (source), on reference hardware with all features enabled (environment), against the per-page pipeline (artifact), completes (response) at ≥ X pages/s (response measure)". The artifact is the whole pipeline with every feature on, never one slice. The response measure can be turned directly into a gate.
- **Limits:** Scenarios are written once, near the start. Nothing in the format re-checks them as features accumulate; that takes a gate (section 2). Agents can also write scenarios scoped to their own slice, which brings the original failure back one level up.

### 1.3 ATAM and the utility tree (SEI)

- **Source:** Rick Kazman, Mark Klein, Paul Clements, *ATAM: Method for Architecture Evaluation*, CMU/SEI-2000-TR-004, August 2000. https://www.sei.cmu.edu/library/atam-method-for-architecture-evaluation/ (PDF: https://www.sei.cmu.edu/library/file_redirect/2000_005_001_13706.pdf/)
- **Definitions, VERIFIED in the report text:**
  - "Utility trees provide a top-down mechanism for directly and efficiently translating the business drivers of a system into concrete quality attribute scenarios." The tree "contains utility as the root node. This is an expression of the overall 'goodness' of the system."
  - "Sensitivity points are parameters in the architecture to which some measurable quality attribute response is highly correlated."
  - "A tradeoff point is found in the architecture when a parameter of an architectural construct is host to more than one sensitivity point".
  - "Risks are architecturally important decisions that have not been made … or decisions that have been made but whose consequences are not fully understood".
- **Transfer:** In ATAM terms, the shared per-page pipeline (parse, commit, prepare) was a **sensitivity point** for throughput. That nobody owned it was an ATAM **risk**: an architecturally important decision that had not been made. An agent workflow can run a cheap utility-tree step before writing tickets. The output is a ranked list of quality scenarios plus the sensitivity points each feature touches. Every ticket that touches a sensitivity point is then routed to the point's owner.
- **Limits:** ATAM is an evaluation of an architecture that already exists, run as a facilitated, multi-day workshop. An LLM imitating it has no stakeholders to argue with and tends to agree with its own plan. It is useful as a structured prompt, not as assurance.

### 1.4 Quality Attribute Workshop (SEI)

- **Source:** Mario Barbacci, Robert Ellison, Anthony Lattanze, Judith Stafford, Charles Weinstock, William Wood, *Quality Attribute Workshops (QAWs), Third Edition*, CMU/SEI-2003-TR-016, October 2003. https://www.sei.cmu.edu/library/quality-attribute-workshops-qaws-third-edition/
- **Definition:** "The Quality Attribute Workshop (QAW) is a facilitated method that engages system stakeholders early in the life cycle … to discover the driving quality attributes of a software-intensive system."
- **Status:** VERIFIED (abstract).
- **Transfer:** The key word is *early*: quality drivers are found before the architecture exists. That is the opposite of a later "optimization pass". In an agent pipeline, the QAW is the step before the spec that answers: "which qualities, measured how, decide whether this product beats its competitor?". For a crawler, throughput against a named competitor is a driving quality attribute, not an extra.
- **Limits:** It needs real stakeholders. Without the owner, an agent will invent priorities.

### 1.5 Architecturally significant requirements (ASRs)

- **Source:** Lianping Chen, Muhammad Ali Babar, Bashar Nuseibeh, "Characterizing Architecturally Significant Requirements", *IEEE Software* 30(2):38–45, 2013, DOI 10.1109/MS.2012.174. https://eprints.lancs.ac.uk/id/eprint/64499
- **Definition:** The abstract is VERIFIED: "This article presents a framework for characterizing architecturally significant requirements (ASRs) on the basis of an empirical study using grounded theory", based on 90 practitioners. The usual one-line definition, "requirements that have a measurable impact on the architecture of the software system", is UNVERIFIED: I found it only in secondary sources, and no full text was available.
- **Transfer:** "Every page goes through the same pipeline at competitive speed" is an ASR. A ticket-level reviewer cannot judge it, because its effect is on the structure, not on one slice. Agent planners need an explicit "ASR sweep" that pulls such requirements out of the spec before decomposition. Each ASR is then owned by an architecture decision (an ADR), not by a ticket.
- **Limits:** "Architecturally significant" is judged after the fact more easily than before. LLM planners are poor at spotting which requirement is significant unless prompted with heuristics such as wide effect, strict constraint or hard to achieve.

---

## 2. Fitness functions, runway and budgets

### 2.1 Architectural fitness functions (Ford, Parsons, Kua)

- **Source:** Thoughtworks Technology Radar, "Architectural fitness function", Trial, November 2017 and May 2018. https://www.thoughtworks.com/radar/techniques/architectural-fitness-function. Book: Neal Ford, Rebecca Parsons, Patrick Kua, *Building Evolutionary Architectures*, O'Reilly, 1st ed. 2017, 2nd ed. 2023 (with Pramod Sadalage). https://www.oreilly.com/library/view/building-evolutionary-architectures/9781492097532/
- **Definition, VERIFIED at the Radar:** "Borrowed from evolutionary computing, a fitness function is used to summarize how close a given design solution is" to its aims. An architectural fitness function "provides an objective integrity assessment of some architectural characteristics."
- **Taxonomy, UNVERIFIED:** The O'Reilly page returned 403. Search snippets of the 2nd edition's table of contents give "Atomic Versus Holistic" and "Triggered Versus Continual Versus Temporal", so the 2nd edition says "continual", not "continuous". Secondary summaries describe them this way. Atomic checks one characteristic in isolation. Holistic checks several together, where their combination matters. Triggered runs on an event (commit, build). Continual runs all the time, as in production monitoring. Temporal runs on a schedule.
- **Transfer:** This is the mechanism that was missing. The per-page cost is a **holistic** property: only the pipeline with every feature enabled shows it. A per-diff atomic test shows each slice as fine. The fix is a **triggered, holistic, absolute** fitness function: "full-feature crawl of the reference corpus at ≥ X pages/s". It runs on every merge and blocks it, and each ticket's done criteria cite it. A **continual or temporal** one, for example a nightly comparison against the competitor, catches drift that triggered checks absorb.
- **Limits:** Holistic fitness functions are slow and noisy. Teams demote them to informative, which is exactly what happened here (see 5.1). They also measure only what they encode, and agents optimise toward the encoded check (Goodhart, 5.2).

### 2.2 Architectural runway, enablers, intentional architecture vs. emergent design (SAFe)

- **Sources, VERIFIED:** Scaled Agile Framework:
  - https://framework.scaledagile.com/architectural-runway
  - https://framework.scaledagile.com/enablers
  - https://framework.scaledagile.com/nonfunctional-requirements
- **Definitions:**
  - The runway "consists of the existing code, components, and technical infrastructure" "needed to implement near-term features with minimal redesign and delay."
  - Emergent design means "defining and extending the architecture only as necessary to deliver the next increment of functionality". SAFe says emergent design alone "cannot handle the complexity of large-scale Solution development" and must be balanced "with intentional architecture, which requires some centralized planning" and "cross-team coordination."
  - "Enablers are backlog items that extend the architectural runway"; one type, Architecture, is "used to build the Architectural Runway."
  - "Nonfunctional Requirements (NFRs) are system qualities that guide the design of the solution" and "often serve as constraints across the relevant backlogs". They are "persistent qualities and constraints typically revisited as part of the definition of done (DoD)".
- **Transfer:** SAFe gives the ticket model the parts that were missing:
  1. An **architectural enabler ticket** that builds the shared per-page pipeline (parse once, one commit per batch, prepared statements cached) *before* the feature slices.
  2. Feature slices that must **consume the runway** instead of building their own.
  3. The performance NFR as a constraint in **every** slice's definition of done.

  "Emergent design only" describes the run under study, and SAFe names the failure mode it predicts.
- **Limits:** SAFe is heavy and process-centric. "Intentional architecture" needs someone who has the authority to say no to a slice, and an agent pipeline has to give that authority to a role explicitly. Runway also decays. SAFe itself lists "poor solution performance" among the risks of relying on emergent design alone.

### 2.3 Performance budgets (Kadlec; web.dev)

- **Sources:**
  - Tim Kadlec, "Setting a performance budget", 28 January 2013, http://timkadlec.com/2013/01/setting-a-performance-budget/. URL and date VERIFIED via Jeremy Keith's link post https://adactio.com/links/6007, which calls it "setting a page-weight limit". The original page now returns 404, so its text is UNVERIFIED.
  - Tim Kadlec, "Performance Budgets That Stick", 7 March 2019, https://timkadlec.com/remembers/2019/03/performance-budgets-that-stick/. VERIFIED.
  - Milica Mihajlija, "Performance budgets 101", web.dev, 2018-11-05, https://web.dev/articles/performance-budgets-101. VERIFIED.
- **Definitions:**
  - Kadlec 2019: "A performance budget is a clearly defined limit on one or more performance metrics that the team agrees not to exceed", "used to guide design and development". It "has to be concrete, meaningful, integrated, and enforced". "It's not just a number, but a hard line."
  - web.dev: "A performance budget is a set of limits imposed on metrics that affect site performance". When something crosses a threshold, the article's options are to optimise the asset or feature, remove it, or not add it.
- **Status:** I did not fetch an Addy Osmani primary.
- **Transfer:** A budget is **absolute**: a hard line against a fixed target, not a delta against the last commit. Its rule for new features fits this case exactly. A slice that would exceed the per-page budget must pay for itself: optimise the shared path, remove cost elsewhere, or not ship. In an agent pipeline, the budget belongs in the spec. Each ticket gets an allocation, such as "≤ N µs and 0 extra parses per page", and the gate enforces the total.
- **Limits:**
  - Kadlec's own conditions ("integrated, and enforced") were exactly what was dropped here. An informative budget is not a budget in his sense.
  - Allocating a budget per ticket needs a cost model of the pipeline, and agents cannot invent a sound one.
  - Web budgets mainly cover page weight and timings. A server pipeline needs its own metrics, such as parses, commits and statement prepares per page.

---

## 3. Diffuse slowness: commons, cuts and flat profiles

### 3.1 Knuth's full "premature optimization" passage, and the 3% premise

- **Source:** Donald E. Knuth, "Structured Programming with go to Statements", *ACM Computing Surveys* 6(4):261–301, December 1974, DOI 10.1145/356635.356640, p. 268. Text extracted from the scan at https://gwern.net/doc/cs/algorithm/1974-knuth.pdf.
- **Exact text, VERIFIED:**
  - "Programmers waste enormous amounts of time thinking about, or worrying about, the speed of noncritical parts of their programs … We should forget about small efficiencies, say about 97% of the time: premature optimization is the root of all evil. Yet we should not pass up our opportunities in that critical 3%. A good programmer will not be lulled into complacency by such reasoning, he will be wise to look carefully at the critical code; but only after that code has been identified."
  - On the same page: "In established engineering disciplines a 12% improvement, easily obtained, is never considered marginal".
  - On p. 267: "most of the running time in non-IO-bound programs is concentrated in about 3% of the source text."
- **Transfer:** Deferring speed to a later pass rests on Knuth's empirical premise that cost concentrates in a small critical region a profiler can find. A per-page pipeline that every feature extends is that critical region. The crawler's inner loop *is* the per-page path. So by Knuth's own rule it was the 3% that deserved care from the start, not the 97% to forget. Agent specs should name the critical path explicitly ("this is the 3%") so that "don't optimise prematurely" never applies to it.
- **Limits:** Knuth's premise fails when the cost is spread out, as in 3.2. Once every slice adds its own parse and commit, no 3% hotspot exists to find later. That is why the later pass is so expensive.

### 3.2 "Peanut-butter" problems and "death by a thousand cuts"

- **Source A, VERIFIED:** Joe Duffy, "The 'premature optimization is evil' myth", 6 September 2010. http://joeduffyblog.com/2010/09/06/the-premature-optimization-is-evil-myth/
  - "These kinds of 'peanut butter' problems add up in a hard to identify way."
  - "Your performance profiler may not obviously point out the effect of such a bad choice".
  - "Rather than making one routine 1000% slower, you may have made your entire program 3% slower."
  - Results come from "many 'performance in the small' decisions adding up."
  - Knuth's quip is "not a license to ignore performance altogether."
- **Source B, VERIFIED:** Alex Rousskov, squid-dev mailing list, "Death by a thousand cuts: v3.0 vs v3.2", 17 November 2010. https://list-archive.squid-cache.org//mail-archive/squid-dev/201011/0122.html
  - "I am referring to this phenomenon as 'death by a thousand cuts'". Changes that each had "negligible overhead in most locations" "add up to measurable 3-10% performance degradation".
  - "most of the observed slowdown can be attributed to code reorganization and various new features added after v3.0."
- **"Flat profile":** a standard profiling term for time spread evenly with no dominant method. I found no single coining source.
- **Not found:**
  - "Uniformly slow code" as a named term from a credible primary source.
  - Any primary usage of these terms by Russ Cox, Chandler Carruth or Dan Luu. These attributions stay UNVERIFIED and should not be cited.
- **Transfer:** The case is a peanut-butter failure made by the process: each slice spread a thin layer (parse, commit, prepare) over the same hot path. Rousskov's thread is close to the same story at 3–10%: necessary features, each negligible locally, together measurable. Per-diff review and per-diff benchmarks cannot see this by construction. Detection has to be holistic (2.1) and absolute (2.3). Prevention has to be structural: one owned pipeline that features plug into (sections 4 and 8).
- **Limits:** These are practitioner terms, not formal concepts. They describe the symptom, not the governance cause.

### 3.3 Tragedy of the commons on a shared hot path

- **Source:** Garrett Hardin, "The Tragedy of the Commons", *Science* 162:1243–1248, 13 December 1968. Reproduction: https://www.garretthardinsociety.org/articles/art_tragedy_of_the_commons.html
- **Exact text, VERIFIED at the reproduction:**
  - The herdsman's "positive utility is nearly +1", while "the negative utility for any particular decision-making herdsman is only a fraction of -1".
  - "Each man is locked into a system that compels him to increase his herd without limit--in a world that is limited."
  - "Ruin is the destination toward which all men rush."
- **Transfer:** The per-page time budget is a commons. Each slice gets the whole benefit of its feature and bears only a fraction of the shared cost. A per-slice reviewer, measuring that slice against the previous commit, sees +1 and a small fraction of −1 every time. Hardin's way out is "mutual coercion mutually agreed upon": an owner with authority over the shared resource and an enforced limit. That is the absolute, blocking budget plus a named owner of the pipeline.
- **Limits:** This is an analogy. Agents are not self-interested. The structure of the incentive comes from the scope of review, not from motive. Ostrom's later work on commons governed without a central owner exists, but it assumes communicating participants that ticket-scoped agents are not.

---

## 4. Decomposition and organisation: Conway, Parnas, Team Topologies

### 4.1 Conway's law (1968)

- **Source:** Melvin E. Conway, "How Do Committees Invent?", *Datamation*, April 1968. https://www.melconway.com/Home/Committees_Paper.html
- **Exact text, VERIFIED:**
  - Organisations that design systems are "constrained to produce designs which are copies of the communication structures of these organizations."
  - "the very act of organizing a design team means that certain design decisions have already been made".
  - Once delegation starts, "the class of design alternatives which can be effectively pursued is also narrowed."
- **Transfer:** In an agent workflow, the "organisation" is the ticket graph. Agents talk only through the spec and the diff. A decomposition into one ticket per feature, with no ticket for the shared pipeline, produces one layer per feature on the page path. That is Conway's law with tickets as teams. The act of slicing *was* the design decision Conway describes, taken before anyone had considered the pipeline.
- **Limits:** Conway describes communication structure, and agents can read the whole repository, so the parallel is not exact. In practice, an agent's effective communication is limited to its brief and its context window, so the law holds strongly.

### 4.2 Inverse Conway maneuver

- **Source:** Thoughtworks Technology Radar, "Inverse Conway Maneuver", Trial, July 2014, updated January 2015. https://www.thoughtworks.com/radar/techniques/inverse-conway-maneuver
- **Definition, VERIFIED:** the maneuver "recommends evolving your team and organizational structure" to promote the desired architecture. "Ideally your technology architecture will display isomorphism with your business architecture."
- **Attribution:** the coinage is commonly credited to Jonny LeRoy and Matt Simons (2010). That credit is UNVERIFIED and comes from memory, not from a source read here.
- **Transfer:** Design the ticket graph to match the architecture you want. If the target is "one pipeline, many plug-in analyses", the first ticket owns the pipeline and its cost contract. Each feature ticket is then scoped as a plug-in against that contract. The decomposition is the most powerful architectural lever an agent planner has.
- **Limits:** The maneuver needs the target architecture first, which is exactly what was missing here. It cannot help a planner that never asked what the architecture should be.

### 4.3 Parnas (1972): decomposition by processing step vs. information hiding

- **Source:** David L. Parnas, "On the Criteria To Be Used in Decomposing Systems into Modules", *Communications of the ACM* 15(12):1053–1058, December 1972.
- **Status:** VERIFIED via reproductions. The available PDFs are image scans the tools could not read. The wording below is as quoted by Adrian Colyer (https://blog.acolyer.org/2016/09/05/on-the-criteria-to-be-used-in-decomposing-systems-into-modules/) and confirmed by search results that quote the paper. The 1978 "work assignment" definition is as quoted by H. Conrad Cunningham (https://john.cs.olemiss.edu/~hcc/researchMethods/notes/ClassicParnas/ACMannotated/ClassicParnasRevisionAnnotated.pdf).
- **Exact text:**
  - "it is almost always incorrect to begin the decomposition of a system into modules on the basis of a flowchart. We propose instead that one begins with a list of difficult design decisions or design decisions which are likely to change. Each module is then designed to hide such a decision from the others."
  - On the first, conventional decomposition, the formats shared between modules mean "The development of those formats will be a major part of the module development and that part must be a joint effort among the several development groups."
  - "Every module in the second decomposition is characterized by its knowledge of a design decision which it hides from all others."
  - Parnas 1978 defines a module as "a work assignment given to a programmer or group of programmers".
- **Transfer:** This is the most precise fit. A module is a work assignment, so a ticket is a module in Parnas's sense. Slicing by feature, where each slice does fetch, parse, analyse and commit for its feature, is decomposition by processing step. Each slice re-makes the shared, hard design decisions: how a page is parsed, when state commits, how statements are prepared. Under information hiding, "how a page is analysed once" and "how run state commits" become modules with owners, and features consume them. A planner should list the hard shared decisions *before* cutting slices, and give each one a ticket and an owner.
- **Limits:**
  - Parnas warns that the information-hiding decomposition can be less efficient if each interface call is costly: "If we are not careful, the second decomposition will prove to be much less efficient". So hiding alone does not guarantee speed. The hidden module must own the cost too.
  - Vertical slicing has real benefits, such as feedback and independent delivery. Parnas argues against *module* boundaries by step, not against delivering by feature.

### 4.4 Team Topologies: platform and enabling teams, cognitive load

- **Source:** Matthew Skelton and Manuel Pais, *Team Topologies* (IT Revolution, 2019; 2nd ed. 2025). Key concepts: https://teamtopologies.com/key-concepts. Talk report: Helen Beal, InfoQ, 9 July 2019, https://www.infoq.com/news/2019/07/team-topologies-cognitive-loa/
- **Definitions, VERIFIED on the site:**
  - Stream-aligned team: "aligned to a flow of work from (usually) a segment of the business domain".
  - Enabling team: "helps a Stream-aligned team to overcome obstacles. Also detects missing capabilities."
  - Platform: "a grouping of other team types that provide a compelling internal product to accelerate delivery by Stream-aligned teams".
  - "Teams can only handle so much complexity before breaking down."
  - InfoQ reports the principle that each service must be "fully owned by a team with sufficient cognitive capacity to build and operate it."
- **Status:** The book's own definition of cognitive load is UNVERIFIED.
- **Transfer:** Each feature ticket is a stream-aligned agent. The per-page pipeline should be a **platform** with an owner, offering "one parse, one commit, one prepared-statement cache" as an internal product. Feature agents consume it instead of rebuilding it. An **enabling** role, a reviewer or architect agent that "detects missing capabilities", would have noticed that the third slice to write its own parser meant a platform was missing. An agent's context window is its cognitive load. Agents that cannot hold the whole pipeline in context should not be the ones deciding its shape.
- **Limits:** Team Topologies is about long-lived human teams. Agents are short-lived and have no memory between tickets. The platform's owner has to be durable in artifacts: an ADR, a module with a contract, a gate. A session cannot be the owner.

---

## 5. Signals that do not block, metrics and drift detection

### 5.1 Normalization of deviance (Vaughan 1996)

- **Source:** Diane Vaughan, *The Challenger Launch Decision: Risky Technology, Culture, and Deviance at NASA*, University of Chicago Press, 1996; enlarged edition 2016. https://press.uchicago.edu/ucp/books/book/chicago/C/bo22781921.html
- **Definition:** The publisher's description is VERIFIED: NASA insiders, "when repeatedly faced with evidence that something was wrong, normalized the deviance so that it became acceptable to them." The common short form, "the gradual process through which unacceptable practice or standards become acceptable", is UNVERIFIED and comes from a secondary source.
- **Transfer:** An absolute ceiling made informative is a warning that fires, gets noted and gets merged anyway. Each merge past it makes the next one look normal. Agents speed this up: they read "informational, not blocking" literally and record the breach as a known item. The rule that follows: a quality ceiling either blocks, or carries a dated, owned waiver that expires. "Informative" is not a stable state.
- **Limits:** Vaughan studied organisational culture under production pressure. Agents have no culture, but the workflow's rules play that role. The mechanism is the same, but it is written into the configuration rather than learned.

### 5.2 Goodhart's law (Goodhart 1975; Strathern 1997)

- **Sources:**
  - Charles Goodhart, "Problems of Monetary Management: The U.K. Experience", 1975. Not accessed (paywalled).
  - Marilyn Strathern, "'Improving ratings': audit in the British University system", *European Review* 5(3):305–321, 1997. https://www.cambridge.org/core/journals/european-review/article/improving-ratings-audit-in-the-british-university-system/FC2EE640C0C44E3DB87C29FB666E9AAB
- **Wording:**
  - Strathern's abstract is VERIFIED: "audit does more than monitor—it has a life of its own that jeopardizes the life it audits."
  - The popular form, "When a measure becomes a target, it ceases to be a good measure", is UNVERIFIED in the text. Secondary sources attribute it to Strathern's paper, p. 308.
  - Goodhart's 1975 wording, "any observed statistical regularity will tend to collapse once pressure is placed upon it for control purposes", is UNVERIFIED (secondary only).
- **Transfer:** Agents optimise hard for the gates they are given. A "≤ 5% slower than the last commit" check is satisfied at every step while the absolute metric collapses. A narrow micro-benchmark gets tuned while the real crawl does not improve. Fitness functions should therefore measure the real target (the full-feature end-to-end crawl against a reference) and keep the gate's metric close to the user's.
- **Limits:** This is a caution, not a design method. It argues for several metrics and for occasional end-to-end checks against a competitor, not for dropping gates.

### 5.3 The ratchet pattern

- **Source:** I found no canonical coining paper. The best primary for the mechanism is Betterer's documentation (Craig Spence): https://phenomnomnominal.github.io/betterer/docs/introduction
- **Definition, VERIFIED:** Betterer "will help you make incremental improvements to your codebase", "built upon an idea popularised by snapshot testing"; "If it gets worse, your test will fail".
- **Transfer:** A ratchet locks in improvements, so the baseline only moves in the good direction. Paired with an absolute budget, it stops "each change vs. the previous one" from allowing slow decline. The comparison is against the best recorded value, not against the parent commit.
- **Limits:**
  - A ratchet on a noisy performance metric needs tolerance bands. Too tight and it fails at random. Too loose and it lets slow decline through in steps smaller than the tolerance.
  - A ratchet alone has no target. It stops getting worse but does not demand being competitive. Pair it with an absolute ceiling.

### 5.4 How large perf projects separate per-change regressions from slow drift

Only the naming is confirmed here; another strand surveys the tools.

- **Chromium, VERIFIED:** "It's all too easy for performance regressions to creep in." "If after one business day the regression has not been resolved, the patch is reverted." "the longer we allow the regression to remain in the code base, the harder it is to fix." https://chromium.googlesource.com/chromium/src/+/main/docs/benchmark_performance_regressions.md. Perf sheriffs triage automatically detected and bisected regressions: https://chromium.googlesource.com/chromium/src/+/2c0f4a7f43a85ae0448647bd87d4e180ba3e3373/docs/speed/perf_regression_sheriffing.md
- **Mozilla, VERIFIED:** Sheriffs "review the alerts generated on Perfherder on a daily basis". "Any time a test exceeds the threshold set for its framework, one or more alerts are generated", with a default of 2% (5% for Dromaeo, ≥ 0.25% for AWSY). For noisy graphs, sheriffs "zoom out, choose a larger timeframe (30 days, 60 days, etc.)". The page describes no automated mechanism for cumulative change below the thresholds. https://firefox-source-docs.mozilla.org/testing/perfdocs/perf-sheriffing.html
- **Rust compiler, VERIFIED:** weekly perf triage reviews "the effects of merged PRs on rustc's speed and memory usage" and is published in This Week in Rust. https://github.com/rust-lang/rustc-perf/blob/master/triage/README.md
- **Names used:** *perf sheriffing*, *perf regression*, *alert threshold*, *bisect*, *perf triage*. I found no single standard name for sub-threshold cumulative drift. Practitioners use "creep", "slow drift" or "death by a thousand cuts".
- **Transfer:** Even the best per-change systems use relative thresholds of about 2% per push. A run of 1.9% changes passes every one of them. Mature projects make up for it with human sheriffs who look at long windows, and with competitive or absolute benchmarks. Chromium's revert policy covers Speedometer, JetStream and MotionMark. An agent workflow needs the same two layers: a per-change relative gate and a periodic **absolute** check against a fixed reference or competitor, with authority to revert.
- **Limits:** Sheriffing is human work done on a rota. An agent sheriff needs a stable benchmark environment and noise control, or it produces false alarms that people learn to ignore (5.1 again).

---

## 6. Erosion, drift, piecemeal growth

### 6.1 Architectural erosion and architectural drift (Perry and Wolf 1992)

- **Source:** Dewayne E. Perry and Alexander L. Wolf, "Foundations for the Study of Software Architecture", *ACM SIGSOFT Software Engineering Notes* 17(4):40–52, October 1992. https://webhome.csc.uvic.ca/~hausi/480/papers/perry.pdf (text extracted locally from the fetched PDF).
- **Exact text, VERIFIED:**
  - Evolution brings "an increasing brittleness of the system". "This is due in part to two architectural problems: architectural erosion and architectural drift. Architectural erosion is due to violations of the architecture. … Architectural drift is due to insensitivity about the architecture. This insensitivity leads more to inadaptability than to disasters and results in a lack of coherence and clarity of form, which in turn makes it much easier to violate the architecture that has now become more obscured."
  - The paper also says: "we are still at the stage of recreating every design element for each new architecture."
- **Transfer:** The case is **drift**, not erosion. There was no stated pipeline architecture to violate. Each slice was simply insensitive to the shared path, and the result was "a lack of coherence and clarity of form". Erosion follows drift, as Perry and Wolf predict. Once the pipeline is obscured, later slices violate even the parts that existed. For agents the remedy is to make the architecture explicit and visible in every brief: an ADR for the per-page pipeline, linked from each ticket. Insensitivity is then a review failure, not the default.
- **Limits:** Drift is a description, not a detector. Measuring it needs fitness functions or conformance checks.

### 6.2 Lehman's laws: increasing complexity and declining quality

- **Source:** M. M. Lehman, "Laws of Software Evolution Revisited", in C. Montangero (ed.), *Software Process Technology* (EWSPT 1996), LNCS 1149, Springer, 1996, pp. 108–124, DOI 10.1007/BFb0017737. https://www.cs.kent.edu/~jmaletic/cs63902/Papers/Lehman96.pdf and https://gwern.net/doc/cs/1996-lehman.pdf (text extracted locally).
- **Exact text, VERIFIED:**
  - "II -- Increasing Complexity: As a program is evolved its complexity increases unless work is done to maintain or reduce it." It "results from the imposition of change upon change upon change".
  - "VII -- Declining Quality: E-type programs will be perceived as of declining quality unless rigorously maintained and adapted to a changing operational environment."
  - "VIII -- Feedback System: E-type Programming Processes constitute Multi-loop, Multi-level Feedback systems and must be treated as such to be successfully modified or improved."
- **Transfer:** "Unless work is done" is the key clause. A workflow where every unit of work is a feature slice does no anti-complexity work by construction. The "later optimization pass" is a promise of that work with no trigger. Law VIII says the fix is a feedback loop, not a one-off pass. An agent workflow should schedule recurring, gated "maintain or reduce" work driven by the absolute fitness function. For example, a holistic budget breach automatically creates a pipeline-consolidation ticket that blocks further feature work.
- **Limits:** The laws are empirical, drawn from large long-lived systems such as OS/360. They are not mechanistic and do not say how much work is enough.

### 6.3 Big Ball of Mud and piecemeal growth (Foote and Yoder 1997)

- **Source:** Brian Foote and Joseph Yoder, "Big Ball of Mud", PLoP '97; in *Pattern Languages of Program Design 4*, 2000. http://www.laputan.org/mud/mud.html
- **Exact text, VERIFIED:**
  - "A BIG BALL OF MUD is a casually, even haphazardly, structured system."
  - "Architecture is often seen as a luxury or a frill." "Therefore, focus first on features and functionality, then focus on architecture and performance."
  - On Piecemeal Growth: "The biggest risk associated with PIECEMEAL GROWTH is that it will gradually erode the overall structure of the system." Its remedy: "Allow opportunities for growth to be exploited locally, as they occur. Refactor unrelentingly."
  - "With software, alas, there is seldom any corresponding agency to police such squalor."
- **Transfer:** The case is piecemeal growth with the counterweight ("Refactor unrelentingly") removed. Each slice grew locally, and nobody was charged with consolidating the shared path. "Focus first on features … then … performance" is the sequence the run followed, and Foote and Yoder present it as the force that *produces* mud, not as advice. The "agency to police" is what an agent workflow must add: a role and a gate.
- **Limits:** Foote and Yoder are partly sympathetic to mud as a rational response to pressure. It is not always the wrong outcome for throwaway code, but it is for a product whose selling point is throughput.

### 6.4 Accidental architecture (Booch 2006)

- **Source:** Grady Booch, "The Accidental Architecture", *IEEE Software* 23(3):9–11, May/June 2006, DOI 10.1109/MS.2006.86. https://computer.org/publications/tech-news/on-architecture/the-accidental-architecture
- **Status:** Bibliographic details VERIFIED. The wording "Every interesting software-intensive system has an architecture. While some of these architectures are intentional, most appear to be accidental." is UNVERIFIED (seen only in search snippets). Booch's point, also from snippets, is that accidental architecture is not bad in itself if its decisions are visible and understood.
- **Transfer:** The per-page pipeline existed. It was accidental: the sum of eight slices' local choices, never named or reviewed as a whole. Booch's condition (visible, understood decisions) is the cheap test for agents. Before a feature is accepted, can the reviewer state the current pipeline and its per-page cost? If not, the architecture is accidental and opaque.
- **Limits:** This is a column, not a method.

### 6.5 Conceptual integrity and the second-system effect (Brooks)

- **Source:** Frederick P. Brooks, Jr., *The Mythical Man-Month*, Addison-Wesley 1975; Anniversary Edition 1995. Text read in the archive.org scan of the Anniversary Edition: https://archive.org/stream/MythicalManMonth/ (full path in the source list).
- **Exact text, VERIFIED:**
  - Chapter 4: "I will contend that conceptual integrity is the most important consideration in system design." "Conceptual integrity in turn dictates that the design must proceed from one mind, or from a very small number of agreeing resonant minds." "Architecture must be carefully distinguished from implementation."
  - Chapter 13: conceptual integrity "not only makes it easier to use" but "also makes it easier to build and less subject to bugs."
  - Chapter 5: "This second is the most dangerous system a man ever designs."
  - Chapter 5's epigraph is Ovid, "Adde parvum parvo magnus acervus erit" ("add little to little and there will be a big heap"). I saw it on the O'Reilly chapter page in a search result.
- **Transfer:**
  - **Conceptual integrity** is the core fit. Eight agents, each with a fresh context and one slice, are the opposite of "one mind". Brooks's remedy is an architect role that holds the whole design and separates architecture (the pipeline contract) from implementation (the slices). In an agent workflow that means one architect pass, a single long-context owner, that writes the pipeline decision and reviews every slice against it.
  - The **second-system effect** proper, over-design from pent-up ideas, does not fit this case. Chapter 5's epigraph, however, names exactly the accretion seen here.
- **Limits:** "One mind" fits a model poorly, since each session is a new mind. Integrity must live in artifacts (ADRs, the glossary, contracts) that every session loads. Brooks also assumes the architect has authority over implementers, which the workflow has to grant explicitly.

---

## 7. Cross-cutting concerns (Kiczales et al. 1997)

- **Source:** Gregor Kiczales, John Lamping, Anurag Mendhekar, Chris Maeda, Cristina Lopes, Jean-Marc Loingtier, John Irwin, "Aspect-Oriented Programming", ECOOP '97, LNCS 1241, pp. 220–242, 1997. https://www.cs.ubc.ca/~gregor/papers/kiczales-ECOOP1997-AOP.pdf (text extracted locally from the fetched PDF).
- **Exact text, VERIFIED:**
  - Some design decisions get "scattered throughout the code, resulting in 'tangled' code". "We call the properties these decisions address aspects, and show that the reason they have been hard to capture is that they cross-cut the system's basic functionality."
  - "Aspects tend not to be units of the system's functional decomposition, but rather to be properties that affect the performance or semantics of the components in systemic ways."
  - The paper's motivating example is close to this case. When each image filter loops over its inputs separately, "Output images are created frequently, often existing only briefly before they are consumed by some other loop. This results in excessively frequent memory references and storage allocation … and terrible performance." The fix needs "a more global perspective of the program". Loop fusion "composes" differently from the functionality, and "This cross-cutting phenomena is directly responsible for the tangling in the code."
- **Transfer:** Per-page cost is an **aspect** in Kiczales's exact sense: a property affecting "the performance … of the components in systemic ways", not a unit of the functional (feature) decomposition. Each feature filter re-parsing the page and committing on its own is the image-filter example with HTML instead of images. A fusion step (parse once, batch commits) needs a global view that no single filter owns. Ticketing by feature is a functional decomposition. It cannot hold cross-cutting concerns unless they are lifted out as their own owned modules or contracts.
- **Limits:** AOP the *technique* (weaving) is not the remedy proposed here. The *analysis* is what transfers. Lifting a concern into a framework adds indirection costs; compare Parnas's efficiency warning in 4.3.

---

## 8. Timing of decisions: last responsible moment, "make it work, right, fast", fix-it-later, walking skeleton

### 8.1 Last responsible moment (Poppendieck) and its critique

- **Source:** Mary and Tom Poppendieck, *Lean Software Development: An Agile Toolkit*, Addison-Wesley, 2003. Quoted in Rebecca Wirfs-Brock, "Agile Architecture Myths #2: Architecture Decisions Should Be Made at the Last Responsible Moment", 18 January 2011: https://wirfs-brock.com/rebecca/blog/2011/01/18/agile-architecture-myths-2-architecture-decisions-should-be-made-at-the-last-responsible-moment/
- **Definition, VERIFIED via that reproduction:** "Concurrent development makes it possible to delay commitment until the last responsible moment", "the moment at which failing to make a decision eliminates an important alternative." The book itself was not read.
- **Critique, VERIFIED (paraphrased):** Wirfs-Brock prefers the "most responsible moment". The last moment is hard to pinpoint, and decisions with wide impact need making early, because waiting leaves collaborators blocked.
- **Disputed origin, UNVERIFIED:** A 2015 SPA conference deck attributes the concept's origin to Ballard and Zabelle (Lean Construction Institute, 2000). Cockburn's critique that it is "not a single instant" was also seen only in search results.
- **Transfer:** By the Poppendiecks' own definition, the last responsible moment for the per-page pipeline was **before the second slice**. After that, "one shared parse" stopped being a cheap alternative, because each further slice made it costlier to retrofit. In an agent run, slices land within hours, so the window closes very fast. Decisions that many slices depend on should be taken at spec time.
- **Limits:** The principle is sound for local, reversible choices. It turns into "never decide" when nobody is watching for the moment.

### 8.2 "Make it work, make it right, make it fast" (attributed to Kent Beck)

- **Source:** Quoted and explained in Foote and Yoder, "Big Ball of Mud", 1997: http://www.laputan.org/mud/mud.html
- **Exact text, VERIFIED there:** "Kent Beck has observed that the way to build software is to: Make it work. Make it right. Make it fast." The authors gloss "make it fast" as being "concerned about optimizing performance only after we've learned how to solve the problem".
- **Origin, UNVERIFIED:** Some sources trace the phrase to Johnson and Kernighan, "The C Language and Models for Systems Programming", *BYTE* 8(8), 1983. I did not find a Beck primary.
- **Transfer:** The aphorism has conditions. "Make it right" (structure) comes *before* "make it fast", and "fast" is deferred only until "we've learned how to solve the problem". Here the problem (a crawler's per-page pipeline) was well understood from the start and a competitor showed the target. The "right" step, a coherent pipeline, was skipped, so there was no structure for "fast" to tune. Agents quoting the aphorism should be held to its order.
- **Limits:** This is folklore with an unclear origin. Cite Foote and Yoder's rendering, not a Beck primary.

### 8.3 "Fix-it-later" (Connie U. Smith, software performance engineering)

- **Source:** Connie U. Smith, "Software Performance Engineering Then and Now: A Position Paper", WOSP-C '15 (ICPE 2015 companion). https://research.spec.org/icpe_proceedings/2015/wospc/p1.pdf (text extracted locally). Smith's book: *Performance Engineering of Software Systems*, Addison-Wesley, 1990 (not read).
- **Exact text, VERIFIED:**
  - SPE "is a proactive approach that uses quantitative techniques to predict the performance of software early in design to identify viable options and eliminate unsatisfactory ones before implementation begins."
  - "Systems without critical performance requirements adopted the 'fix-it-later' method." Its premises: "Performance problems are rare", "Hardware is fast and inexpensive", "It is too expensive to build responsive systems", "You can tune software later, if necessary", "Efficiency implies 'tricky code'".
  - "In the late 1970s all of these premises became false."
- **Transfer:** This is the established performance-engineering name for "speed deferred to a later optimization pass": the **fix-it-later** approach. Smith's alternative is a cheap early model of the per-page cost, for example "parses per page × cost + commits per page × cost". An agent workflow should require that model in the spec, update it per ticket, and check it against the absolute gate.
- **Limits:** SPE modelling was historically expensive and specialist. For agents, the useful form is a lightweight cost model plus a measured holistic benchmark, not queueing-network models.

### 8.4 Walking skeleton (Cockburn) and tracer bullets (Hunt and Thomas)

- **Sources:**
  - Alistair Cockburn, bio page: "(1996) Walking Skeleton: A thinly connected functioning architecture." https://alistaircockburn.com/Bio. VERIFIED.
  - Cockburn's fuller definition, attributed to *Crystal Clear* (2005, pp. 49–53): "a tiny implementation of the system that performs a small end-to-end function. It need not use the final architecture, but it should link together the main architectural components. The architecture and the functionality can then evolve in parallel." UNVERIFIED, secondary only.
  - Andy Hunt and Dave Thomas, interviewed by Bill Venners, "Tracer Bullets and Prototypes", Artima, 21 April 2003. https://www.artima.com/articles/tracer-bullets-and-prototypes. VERIFIED: "one thin line of execution goes end to end", "end to end, skeletally thin", "enough of an application to show people how it's going to hang together"; "Prototypes are designed to be thrown away." Book: *The Pragmatic Programmer*, 1999; 20th anniversary ed. 2019 (not read).
- **Transfer:** Both put the *architecture* first, as a thin end-to-end path through the real components that later features attach to. For a crawler, the walking skeleton is fetch → parse once → analysis hook → batched commit, *with its throughput measured from day one*. Feature slices then attach to the skeleton's hooks. The first ticket of a spec should be the skeleton plus its holistic fitness function. That turns "vertical slices" from "each slice builds its own path" into "each slice adds flesh to one path".
- **Limits:** A skeleton fixes the shape, not the cost. Cockburn says it "need not use the final architecture". Without a budget attached, slices can still add per-page work to its hooks. The skeleton must carry the gate.

---

## 9. Other concepts found that fit

- **ATAM risk ("architecturally important decisions that have not been made")**, VERIFIED (1.3). The most exact SEI phrase for "no architecture decision owned the per-page pipeline".
- **Software performance engineering, fix-it-later**, VERIFIED (8.3). The standard name for deferring speed.
- **Parnas's work assignment = module**, VERIFIED via reproduction (4.3). It lets the Conway and Parnas arguments apply directly to tickets.
- **Kiczales's image-filter example**, VERIFIED (7). A published, peer-reviewed example of exactly "each component redoes base work on the shared data → terrible performance → fix needs a global view".

---

## Best name for this failure class

**Proposed compound name:**

> **Architectural drift by piecemeal growth around an unowned cross-cutting quality attribute, under a fix-it-later stance with normalized (non-blocking) budgets.**

Short form: **unowned cross-cutting quality drift**.

**Why each part fits:**

1. **Architectural drift** (Perry and Wolf 1992). The process: "due to insensitivity about the architecture", producing "a lack of coherence and clarity of form". It fits better than *erosion*, which needs an architecture to violate. Here there was none for the pipeline. Drift then makes later erosion easy, which matches how slices came to violate even the pieces that existed.
2. **By piecemeal growth** (Foote and Yoder 1997). The mechanism: growth "exploited locally, as they occur", minus "Refactor unrelentingly", with no "agency to police" the shared structure. It also covers "focus first on features … then … performance".
3. **Unowned** (ATAM's risk; Brooks's conceptual integrity; Team Topologies). The governance cause. The per-page pipeline was an ATAM sensitivity point and an unmade "architecturally important decision", with no architect "mind" and no platform owner.
4. **Cross-cutting quality attribute** (ISO 25010 performance efficiency; Kiczales's aspect; Parnas's shared design decision). What drifted is a property that "cross-cut[s] the system's basic functionality" and affects "the performance … of the components in systemic ways". By its nature it cannot belong to any one feature slice. Parnas explains why feature-by-step decomposition duplicates it.
5. **Fix-it-later stance** (Connie Smith). The timing error: "You can tune software later, if necessary". It is also a misreading of Knuth, whose "critical 3%" *was* the per-page path.
6. **Normalized (non-blocking) budgets** (Vaughan; Kadlec; Hardin). Why detection failed. Relative per-change gates let a commons be grazed one small step at a time. Absolute ceilings made informative are signals that were normalised, not budgets in Kadlec's sense ("a hard line", "enforced").

**Best established term on its own:** **architectural drift** (Perry and Wolf 1992). It names the process, it is defined in a foundational primary source, and its definition carries the causes: nobody attended to the architecture, there was no violation of a stated design, and the result was a loss of coherence. If a reader allows two words, **fix-it-later drift** adds the performance-engineering timing error. For the *root cause* rather than the process, the closest single established phrase is ATAM's **risk**: an "architecturally important decision" that was never made. It is generic, though, so pair it with "per-page pipeline".

**Distinctive features of the agent case**, which none of the classical terms fully captures:

1. Conway's law applied to tickets: the ticket graph *is* the organisation.
2. Every unit of work is reviewed only against its own diff and its parent commit. That makes Hardin's +1 / fraction-of-−1 asymmetry structural rather than motivational.
3. Slices land in hours, so the last responsible moment for shared decisions passes almost at once.

---

## Source list

All accessed 2026-10-31.

1. ISO/IEC 25010 reproduction, iso25000.com: https://iso25000.com/en/iso-25000-standards/iso-25010
2. Bass, Clements, Kazman, *Software Architecture in Practice*, 2nd ed., §4.3 (course-hosted copy): https://people.ece.ubc.ca/matei/EECE417/BASS/ch04lev1sec3.html
3. Kazman, Klein, Clements, *ATAM: Method for Architecture Evaluation*, CMU/SEI-2000-TR-004, 2000: https://www.sei.cmu.edu/library/atam-method-for-architecture-evaluation/ ; PDF https://www.sei.cmu.edu/library/file_redirect/2000_005_001_13706.pdf/
4. Barbacci et al., *Quality Attribute Workshops (QAWs), Third Edition*, CMU/SEI-2003-TR-016, 2003: https://www.sei.cmu.edu/library/quality-attribute-workshops-qaws-third-edition/
5. Chen, Ali Babar, Nuseibeh, "Characterizing Architecturally Significant Requirements", *IEEE Software* 30(2), 2013: https://eprints.lancs.ac.uk/id/eprint/64499
6. Thoughtworks Technology Radar, "Architectural fitness function", 2017: https://www.thoughtworks.com/radar/techniques/architectural-fitness-function
7. Ford, Parsons, Kua, Sadalage, *Building Evolutionary Architectures*, 2nd ed., O'Reilly, 2023: https://www.oreilly.com/library/view/building-evolutionary-architectures/9781492097532/ (403; taxonomy UNVERIFIED)
8. Paula Paul and Rosemary Wang, "Fitness function-driven development", Thoughtworks, 2019: https://www.thoughtworks.com/insights/articles/fitness-function-driven-development
9. SAFe, Architectural Runway: https://framework.scaledagile.com/architectural-runway
10. SAFe, Enablers: https://framework.scaledagile.com/enablers
11. SAFe, Nonfunctional Requirements: https://framework.scaledagile.com/nonfunctional-requirements
12. Tim Kadlec, "Setting a performance budget", 2013: http://timkadlec.com/2013/01/setting-a-performance-budget/ (404; date via https://adactio.com/links/6007)
13. Tim Kadlec, "Performance Budgets That Stick", 2019: https://timkadlec.com/remembers/2019/03/performance-budgets-that-stick/
14. Milica Mihajlija, "Performance budgets 101", web.dev, 2018: https://web.dev/articles/performance-budgets-101
15. Donald E. Knuth, "Structured Programming with go to Statements", *ACM Computing Surveys* 6(4), 1974: https://gwern.net/doc/cs/algorithm/1974-knuth.pdf
16. Joe Duffy, "The 'premature optimization is evil' myth", 2010: http://joeduffyblog.com/2010/09/06/the-premature-optimization-is-evil-myth/
17. Alex Rousskov, "Death by a thousand cuts: v3.0 vs v3.2", squid-dev, 2010: https://list-archive.squid-cache.org//mail-archive/squid-dev/201011/0122.html
18. Garrett Hardin, "The Tragedy of the Commons", *Science*, 1968 (reproduction): https://www.garretthardinsociety.org/articles/art_tragedy_of_the_commons.html
19. Melvin E. Conway, "How Do Committees Invent?", *Datamation*, 1968: https://www.melconway.com/Home/Committees_Paper.html
20. Thoughtworks Technology Radar, "Inverse Conway Maneuver", 2014: https://www.thoughtworks.com/radar/techniques/inverse-conway-maneuver
21. David L. Parnas, "On the Criteria To Be Used in Decomposing Systems into Modules", *CACM* 15(12), 1972, via Adrian Colyer: https://blog.acolyer.org/2016/09/05/on-the-criteria-to-be-used-in-decomposing-systems-into-modules/ ; and H. C. Cunningham: https://john.cs.olemiss.edu/~hcc/researchMethods/notes/ClassicParnas/ACMannotated/ClassicParnasRevisionAnnotated.pdf
22. Team Topologies, Key Concepts: https://teamtopologies.com/key-concepts
23. Helen Beal, "Team Topologies … cognitive load", InfoQ, 2019: https://www.infoq.com/news/2019/07/team-topologies-cognitive-loa/
24. University of Chicago Press, Vaughan, *The Challenger Launch Decision*: https://press.uchicago.edu/ucp/books/book/chicago/C/bo22781921.html
25. Marilyn Strathern, "'Improving ratings'", *European Review* 5(3), 1997: https://www.cambridge.org/core/journals/european-review/article/improving-ratings-audit-in-the-british-university-system/FC2EE640C0C44E3DB87C29FB666E9AAB
26. Betterer documentation: https://phenomnomnominal.github.io/betterer/docs/introduction
27. Chromium, "Regressions to Competitive Benchmarks": https://chromium.googlesource.com/chromium/src/+/main/docs/benchmark_performance_regressions.md
28. Chromium, "Perf Regression Sheriffing" (snapshot): https://chromium.googlesource.com/chromium/src/+/2c0f4a7f43a85ae0448647bd87d4e180ba3e3373/docs/speed/perf_regression_sheriffing.md
29. Mozilla, "Performance Sheriffing": https://firefox-source-docs.mozilla.org/testing/perfdocs/perf-sheriffing.html
30. rustc-perf, triage README: https://github.com/rust-lang/rustc-perf/blob/master/triage/README.md
31. Perry and Wolf, "Foundations for the Study of Software Architecture", *SIGSOFT SEN* 17(4), 1992: https://webhome.csc.uvic.ca/~hausi/480/papers/perry.pdf
32. M. M. Lehman, "Laws of Software Evolution Revisited", EWSPT 1996, LNCS 1149: https://www.cs.kent.edu/~jmaletic/cs63902/Papers/Lehman96.pdf ; https://gwern.net/doc/cs/1996-lehman.pdf
33. Foote and Yoder, "Big Ball of Mud", 1997: http://www.laputan.org/mud/mud.html
34. Grady Booch, "The Accidental Architecture", *IEEE Software* 23(3), 2006: https://computer.org/publications/tech-news/on-architecture/the-accidental-architecture
35. Frederick P. Brooks, Jr., *The Mythical Man-Month*, Anniversary Ed. (archive.org scan): https://archive.org/stream/MythicalManMonth/Brooks%201974%20The%20Mythical%20Man-Month-%20Essays%20on%20Software%20Engineering%2C%20Anniversary%20Edition%20%282nd%20Edition%29_djvu.txt ; Chapter 5 page: https://www.oreilly.com/library/view/mythical-man-month-the/0201835959/ch05.xhtml
36. Kiczales et al., "Aspect-Oriented Programming", ECOOP '97: https://www.cs.ubc.ca/~gregor/papers/kiczales-ECOOP1997-AOP.pdf
37. Rebecca Wirfs-Brock, "Agile Architecture Myths #2 …", 2011: https://wirfs-brock.com/rebecca/blog/2011/01/18/agile-architecture-myths-2-architecture-decisions-should-be-made-at-the-last-responsible-moment/
38. Connie U. Smith, "Software Performance Engineering Then and Now: A Position Paper", WOSP-C 2015: https://research.spec.org/icpe_proceedings/2015/wospc/p1.pdf
39. Alistair Cockburn, bio: https://alistaircockburn.com/Bio
40. Bill Venners, "Tracer Bullets and Prototypes" (Hunt and Thomas), Artima, 2003: https://www.artima.com/articles/tracer-bullets-and-prototypes

**Not found or not verified:**

- "Uniformly slow code" as a primary-sourced term.
- Russ Cox, Chandler Carruth or Dan Luu primary usage of "flat profile" or "death by a thousand cuts".
- Addy Osmani's performance-budget post (not fetched).
- Goodhart's 1975 text.
- The text of Kadlec's 2013 post.
- The 2nd-edition fitness-function taxonomy wording.
- The full definitions from Cockburn's *Crystal Clear* and Booch's column.
