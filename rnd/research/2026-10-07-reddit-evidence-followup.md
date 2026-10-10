# Reddit: evidence and limits for Stagehand, Crawlee and Spider-rs

Survey carried out on October 7, 2026. This document checks citable Reddit testimonies; it does not rank the frameworks and does not scale back Automan's Rust ambition.

## Result

No direct and sufficiently documented Reddit comparison between **Stagehand, Crawlee and Spider-rs**, nor any common protocol allowing them to be ranked, was found in this sweep. This describes the results of this limited search, not proof that no discussion exists anywhere on Reddit. Reddit or vendor home pages cannot support a "Reddit consensus".

There are individual reports on Stagehand versus Browser Use, but they are contradictory, versions are generally missing and the use case is often poorly described. The thread with the most detailed figures compares complete stacks, including **Browserbase + Stagehand**, not Stagehand in isolation. Crawlee appears in an official launch, an old discussion about its errors and a benchmark announced by the author of another Rust crawler. That crawler is **Crawlyx, not Spider-rs**. The Skyvern comments found here include posts by people presenting their own product.

Permissible conclusion: these threads help build validation scenarios — authenticated sessions, recovery after page drift, browser memory, cost of model calls and proof of success. They demonstrate neither a general superiority of Stagehand nor an inferiority of Crawlee or Spider-rs.

## Method and provenance

The Exa Search skill and its references `searching.md` and `source-quality.md` were used to separate stated experience, promotion, verifiable metrics and opinion. Five Exa queries covered: direct comparison of the trio; use of Stagehand versus Browser Use/Skyvern; experience with Crawlee/Spider-rs at scale; Crawl4AI/Scrapling comparison; explicit restriction to Reddit.

| Exa query | `numResults` requested | Useful result for Reddit |
| --- | ---: | --- |
| Direct comparison Stagehand/Crawlee/Spider-rs | 10 | Documentation and comparisons outside Reddit |
| Stagehand versus Browser Use/Skyvern, real experience | 10 | Editorial comparisons and vendor pages |
| Crawlee/Spider-rs, production and metrics | 10 | GitHub, documentation, blogs, one Hacker News thread |
| Crawl4AI/Scrapling and neighboring frameworks | 10 | GitHub benchmarks and articles outside Reddit |
| `site:reddit.com` with the tools concerned | 15 | No results |

**`sources_reviewed: 55`**, according to the Exa convention: sum of the `numResults` requested, including the empty query. This counter does not mean that 55 sources were read. Exa returned 40 URL entries before deduplication, no Reddit URL. Exa retrieval of six known threads failed with `SOURCE_NOT_AVAILABLE`.

The supplementary web search favored the `reddit.com` domain, then opening the exact threads. Ten retained threads were opened, with six comment permalinks opened separately. The authors below come from the opened pages, not from a guess based on the title. Attempts at public JSON retrieval of two threads returned HTTP 403; no timestamp or hidden field was obtained from them.

The absolute dates come either from the thread's search result or from the Reddit footer when it gives the day of the post. For the threads of May 22 and August 13, 2026, they come from the earlier search survey preserved in [stagehand-usage.md](stagehand-usage.md). The current opening provides only relative dates for these two threads. Relative displays vary between pages or cached versions; they were not used to compute false exact dates. The date of a thread is not automatically that of each of its comments.

## Testimonies actually read

### 1. Stagehand versus Browser Use: favorable report, case insufficiently described

- **Source:** [Stagehand vs Browser Use.. which one actually works for production agents?](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/), r/AI_Agents, **April 15, 2026** in the search; **u/Mammoth_Disk_6803**.
- **Stated experience:** two weeks of Browser Use clicking on nonexistent elements, then more completed sessions with Stagehand. Documentation criticized; the author acknowledges that their case may not generalize.
- **Versions / context:** no model, site, scenario, number of runs or session settings.
- **Metrics:** no success rate, trace or corpus. The two weeks do not measure comparative reliability.
- **Quality:** stated personal trial, with no visible promotion in the post; professional identity not verified.

In [this comment, which was read](https://www.reddit.com/r/AI_Agents/comments/1slvuta/comment/ogboy1n/), **u/opentabs-dev** says they abandoned both tools because of authentication difficulties and promotes their OpenTabs extension/MCP. No configuration provided demonstrates that reusing authentication is impossible. **Quality:** stated experience, visible promotional interest; non-independent comparison.

### 2. Browserbase + Stagehand: stated operational figures, confounded effect

- **Source:** [We rebuilt our scraping stack 4 times in 18 months. Is this common?](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/we_rebuilt_our_scraping_stack_4_times_in_18/), r/WebScrapingInsider; **u/yuuliiy**; **May 22, 2026** according to the earlier survey, only a relative age in the current opening.
- **Stated experience:** price-monitoring SaaS, **400 sites, 2 million pages/day**, four stacks in 18 months: requests/BeautifulSoup, self-hosted Playwright, ScrapingBee, Browserbase + Stagehand.
- **Stated figures:** four weeks on the latest stack: **1.2% failures**, **p99 ~6 s**, **monthly cost -45%** compared with the previous combination.
- **Reproducibility:** no corpus, raw data, definition of success, billing, version, code or machine. SDK, hosting and operations change together; the Stagehand effect is impossible to isolate.
- **Quality:** claimed production account, unidentified company, non-auditable figures; no commercial affiliation established.

The replies read include **u/0xMassii**, promoting their Webclaw repository, and **u/CapMonster1**, offering a CAPTCHA solver. [u/DaKheera47](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/comment/onapp63/) asks for the failures to be classified. None of these comments independently validates the figures.

### 3. Cost of Stagehand primitives: old experience and newsletter

- **Source:** [Stagehand - Node package to control browser with natural language](https://www.reddit.com/r/node/comments/1jtdwpz/stagehand_node_package_to_control_browser_with/), r/node; post of **April 7, 2025**, day confirmed by the Reddit footer of the [author's comment](https://www.reddit.com/r/node/comments/1jtdwpz/comment/mltjyi6/); author **u/opensourcecolumbus**.
- **Stated experience:** search for a building block for an agent after trying Computer Use/Operator; preference for atomic primitives or autonomous execution depending on the need.
- **Announced metrics:** about **7,000 tokens** for an automation in three atomic steps, versus **more than 500,000** for executing the same goal in a single request.
- **Versions / reproducibility:** no Stagehand version, detailed task, token log or protocol provided in the comment. `gpt-4o-mini` appears as an example model, not as proof of the model used for these two measurements. The old statement about the lack of support for open-source models is not a description of the current version.
- **Quality:** stated personal experience with an announced measured contrast, but explicit promotion of the OpenSourceDiscovery newsletter and an external article; no independent validation of the figures in the thread.

### 4. Less favorable report on Stagehand

- **Source:** [exact comment by u/joPrincess1](https://www.reddit.com/r/AI_Agents/comments/1uhvp85/comment/oubl84a/) in [How are you using browser agents?](https://www.reddit.com/r/AI_Agents/comments/1uhvp85/how_are_you_using_browser_agents/), post by **u/HagiBoi** dated **June 28, 2026** in the search.
- **Stated experience:** Browser Use considered unpredictable, Stagehand requiring too much supervision, then a preference for Opera Browser CLI. The author describes personal and professional uses: WhatsApp messages, Reddit monitoring, flight options and summarizing team documents.
- **Versions / metrics:** not specified; no number of failed tasks, delay or cost. The tasks for the different solutions are not matched.
- **Quality:** concrete stated experiences and tasks, an unfavorable opinion that prevents presenting the sample as unanimous; the text then centers on promoting another solution, affiliation undeclared and unverified.

### 5. Local models, sessions and Skyvern: a mix of trials and vendors

- **Source:** [Stagehand / BrowserUse / Logged In experiences for agents](https://www.reddit.com/r/LocalLLaMA/comments/1i0toui/stagehand_browseruse_logged_in_experiences_for/), r/LocalLLaMA; **January 14, 2025**; author **u/VigilOnTheVerge**.
- **Experience:** the post asks for a comparison for authenticated sites; it does not publish the results of the benchmark it is considering.
- **User comment read:** [u/alphakue](https://www.reddit.com/r/LocalLLaMA/comments/1i0toui/comment/m72ypjd/) says they did not succeed with local models below 14B, apart from a partial result with `llama3-groq-tool-use:8b-q4_K_M`, and mentions a broken Bedrock integration at that time. No commit, version or complete run.
- **Commercial posts read:** **u/Financial_Radio_5036** introduces themselves as the creator of Browser Use, mentions their `qwen2.5:latest 7b` tests and announces their cloud. **u/do_all_the_awesome** presents the Skyvern/Bitwarden integration and offers Skyvern Cloud. These are claims by people presenting their own solution, not independent testimonies from Skyvern customers.
- **Quality:** specific local trial for alphakue, with no reproducible metric; vendor interest explicitly declared or visible for the other two. Old, model-dependent information: unusable as a judgment on the October 2026 versions.

### 6. Browser Use versus Crawl4AI: the title goes beyond the actual comparison

- **Source:** [Browser-use vs Crawl4ai](https://www.reddit.com/r/AI_Agents/comments/1iyw8l6/browseruse_vs_crawl4ai/), r/AI_Agents; **February 26, 2025**; author **u/Effective-Street1634**.
- **Stated experience:** Browser Use drifts off-task in planner mode, sometimes skips extraction and fails on complex tasks. The author wants to read messages, reply to customers and send a summary; they then ask whether Crawl4AI might be suitable.
- **Scope:** this post does not show that the author tried Crawl4AI on these same tasks. The visible replies include a promotion of rtrvr.ai and technical comments; several comments are deleted.
- **Quality:** concrete user problem, but only one solution tested and no version or metric. The "vs" title does not turn the question into a direct comparison.

### 7. Scrapling and Crawl4AI in a self-hosted research agent

- **Source:** [Self-hosted research agent: which tools do you run?](https://www.reddit.com/r/AI_Agents/comments/1wejc1o/selfhosted_research_agent_which_tools_do_you_run/), r/AI_Agents; **September 12, 2026** in the search and an age of 25 days on the opened permalink; author **u/F1nch74**.
- **Stated experience:** the author uses SearXNG and Scrapling on their VMs and runs into bot/CAPTCHA detection; they are looking for a solution without a paid API or cloud service.
- **[Comment actually read](https://www.reddit.com/r/AI_Agents/comments/1wejc1o/comment/p9e8w89/):** **u/NationalCry9555** says they have been using Crawl4AI for a few months, like its JavaScript rendering and find self-hosted Firecrawl heavy for their needs.
- **Versions / metrics:** none. No common test demonstrating that Crawl4AI solves the blocking encountered with Scrapling. Several posts with the same title found in other subreddits are reposts of the same message, not additional independent users.
- **Quality:** stated personal use of the tools, with a use case and an economic constraint; sites and settings missing, experience not audited.

### 8. Rust crawler versus Crawlee/Crawl4AI: Crawlyx, not Spider-rs

- **Source:** [Built a Rust web crawler after getting fed up with Firecrawl's pricing and crawl4ai's RAM usage](https://www.reddit.com/r/SideProject/comments/1uc1gfo/built_a_rust_web_crawler_after_getting_fed_up/), r/SideProject; **June 21, 2026** in the search; **u/SameerVers3**.
- **Stated experience:** crawler built for a client agent; Crawlee format unsuitable for their pipeline.
- **Announced metrics:** same site/machine, 200 pages: Crawlyx **53 s**, Crawlee **110 s**, Firecrawl **210 s**, Crawl4AI **328 s**; stable memory claimed for Crawlyx, more than 1 GB for Crawl4AI. Crawlyx **v1**, without proxy rotation or anti-bot.
- **Reproducibility:** [Crawlyx repository](https://github.com/SameerVers3/crawlyx) given, not audited here; no site, command, competitor versions, raw data, repetitions or equivalent outputs.
- **Reply read:** **u/SnooMuffins9844**, stating that they work at Firecrawl, asks whether the pages are static or SPAs; question unresolved in the opened view.
- **Quality:** benchmark by the tool's author, direct product interest, non-auditable result. These numbers do not concern Spider-rs.

### 9. Crawlee: launch and a criticism tied precisely to an old version

- **Source:** [Crawlee - the web scraping and browser automation library](https://www.reddit.com/r/webscraping/comments/wvk5za/crawlee_the_web_scraping_and_browser_automation/), r/webscraping; launch of **August 23, 2022**; author **u/B4nan**, text speaking on behalf of the Apify team.
- **Nature of the post:** announcement of the framework and its features. Congratulations or intentions to use it are not production reports.
- **Comment read:** **u/mdaniel** criticizes the error context provided by the code and gives several links to `http-crawler.ts` at tag **v3.0.4**. **u/B4nan** replies that intermediate errors are abbreviated before retry, that the last failure keeps the full trace and that `CRAWLEE_VERBOSE_LOG` makes it possible to get more.
- **Scope:** an old technical discussion associated with a specific version, not a test of the current version. No comparison with Stagehand or Spider-rs and no success rate.
- **Quality:** vendor announcement; criticism grounded in links to versioned code and a maintainer's reply. Useful for identifying an observability contract to verify, without extrapolating a current defect.

### 10. SDK, CLI and cloud: integration confusion, not a benchmark

- **Source:** [confused about browserbase product lines.. stagehand sdk or browse cli](https://www.reddit.com/r/LLMDevs/comments/1vn530v/confused_about_browserbase_product_lines/), r/LLMDevs; author **u/CloudTheoryqa**; **August 13, 2026** according to the earlier survey, absolute date not exposed by the current opening.
- **Stated experience:** integrating a browser into a local Hermes agent; confusion between Stagehand SDK, Browse CLI, Browse.sh recipes and Browserbase hosting. About 40 minutes spent on the wrong quickstart according to the author.
- **Scope:** need for ergonomics and a clear separation of layers. Versions missing; no comparison of success, anti-bot or memory consumption between frameworks.
- **Quality:** detailed personal integration problem, no comparative protocol. Several replies are deleted; the remaining thread proves neither a relative popularity of the products nor a consensus on their reliability.

## Search results not turned into evidence

An excerpt from [Thoughts on best ai web crawlers or scraping apis?](https://www.reddit.com/r/aiagents/comments/1nl6skn/thoughts_on_best_ai_web_crawlers_or_scraping_apis/) mentions a consent-button problem with Crawl4AI, as well as recommendations of services/proxies. This thread was not opened separately in this survey: search excerpt only, excluded from the verified testimonies.

The Exa results include vendor comparisons and the benchmarks from the Spider-rs repository. They may point to code or a methodology to audit, but they are not independent Reddit comments. The expressions "very fast Rust", "production-ready" or "Reddit consensus" found in syntheses do not constitute proof of comparative performance.

## What remains to be demonstrated for Automan

The needs raised by the testimonies must be verified on Automan's exact versions, models and tasks: expiring authenticated state, dynamic navigation, schema-based extraction, recovery after a crash, DOM change, explicit blocking, classified error and proof of the final state. The measurements must distinguish rendering/HTTP, browser provider, proxies, orchestration and LLM decisions.

A ranking would require a common corpus, equivalent outputs, frozen versions and configurations, several repetitions, preserved failures and complete costs. None of the retained threads provides this set for Stagehand/Crawlee/Spider-rs. **The validation work remains open; these testimonies determine neither the final Rust architecture nor Automan's capacity ceiling.**
