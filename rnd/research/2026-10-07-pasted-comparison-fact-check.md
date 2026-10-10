# Fact-check of the pasted comparison: Crawlee, Stagehand and agent-browser

Checked on **October 7, 2026**. This note corrects the claims in the text provided by the user, based on official documentation, published manifests and versioned code. The examples below illustrate documented contracts; no crawl, LLM call or comparative benchmark was run.

## The version difference is decisive

The current npm package **`@crawlee/stagehand` 3.18.2** declares a peer dependency **`@browserbasehq/stagehand: ^3.0.0`** and uses **3.0.7** for its development. It therefore does not declare Stagehand v4 compatibility. Its integration enhances Playwright pages and connects via CDP to the browser launched by Stagehand. [Versioned manifest](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json), [current npm registry](https://registry.npmjs.org/@crawlee/stagehand/latest), [versioned plugin](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-plugin.ts).

The **Stagehand v4** documentation describes a different API: driving Chromium via CDP without a Playwright dependency, no direct interoperability with a Playwright `Page`, and removal of the `agent()` orchestrator. Whoever uses the SDK controls the loop, possibly with an external agent. Confusing the Crawlee v3 examples with the Stagehand v4 documentation leads to incorrect code. [Playwright migration](https://docs.stagehand.dev/v4/migrations/playwright), [Stagehand v3 to v4 migration](https://docs.stagehand.dev/v4/migrations/v3).

## The integration exists; the projects have not merged

The addition of **StagehandCrawler in JavaScript** was announced with **Crawlee 3.16.0 on February 6, 2026**. It is an official composition of libraries. It adds AI functions to the crawler's capabilities; it does not replace the other crawlers and does not constitute a merger of the projects. [Crawlee announcement](https://crawlee.dev/blog/crawlee-v3-16), [package changelog](https://crawlee.dev/js/api/stagehand-crawler/changelog.md).

The **Python** equivalent also exists, added in **Crawlee Python 1.7.0 on May 12, 2026**. Its contract is distinct: installation `crawlee[stagehand]`, configuration `stagehand_options=StagehandOptions(...)`, methods `act`, `extract`, `observe`, `execute`. In **Crawlee Python 1.10.4**, the extra depends on the Python client `stagehand>=3.19.5,<4.0.0`. These Python SDK numbers must not be equated with the npm versions. [Versioned Python changelog](https://github.com/apify/crawlee-python/blob/v1.10.4/CHANGELOG.md), [Python manifest](https://github.com/apify/crawlee-python/blob/v1.10.4/pyproject.toml), [Python guide](https://crawlee.dev/python/docs/guides/stagehand-crawler).

## The configuration block provided mixes options

For `@crawlee/stagehand` 3.18.2:

| Pasted proposal | Verdict within the scope of the published package |
| --- | --- |
| `stagehandOptions` | Yes: a real option of `StagehandCrawlerOptions`. |
| `modelName: "gpt-4o-mini"` | Not a property of its `StagehandOptions` interface. The expected property is `model`, notably with a provider-prefixed string. |
| `domToParse: "accessibility"` | Not an option of the published interface, and no middleware of that name is provided or shown in the excerpt. Nothing justifies claiming that this line forces an accessibility tree. |
| "Disable vision" with this excerpt | The code contains no documented vision switch. The choice of a model and that of the page representation are two different decisions. |

Source governing the contract: [`StagehandOptions` and `StagehandCrawlerOptions` interfaces](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-crawler.ts). The launcher notably sets a default `model`; the plugin builds its `V3Options` using `stagehandOptions.model`. In untyped JavaScript, passing an unknown property does not establish that it is interpreted. [Launcher](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-launcher.ts), [model construction in the plugin](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-plugin.ts).

Minimal form consistent with the published JS integration:

```ts
import { StagehandCrawler } from '@crawlee/stagehand';

const crawler = new StagehandCrawler({
    stagehandOptions: {
        env: 'LOCAL',
        model: 'openai/gpt-4.1-mini',
        apiKey: process.env.OPENAI_API_KEY,
    },
    async requestHandler({ page }) {
        await page.act('Click the Documentation link');
    },
});
```

The model is the one used as the example and default in the inspected sources, with no claim that it is the best current economic choice. The excerpt is an API illustration, not an executed example or a validated configuration for disabling vision. [JS guide](https://crawlee.dev/js/docs/guides/stagehand-crawler-guide).

In Python, the real parameters are notably `model` and `model_api_key`, with separate fields for Browserbase. The REST SDK and the Playwright wrapper are not configured by copying the JavaScript object. [`StagehandOptions` dataclass](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/browsers/_stagehand_types.py).

## The accessibility tree is not exclusive to agent-browser

Stagehand v4 exposes `page.snapshot()` with a formatted accessibility tree (`formattedTree`) and a mapping to XPaths (`xpathMap`). Its documentation presents snapshots and screenshots as distinct tools to expose to an agent. It also recommends deterministic operations for stable selectors and AI primitives only when the task requires judgment. The portrayal "Stagehand always sends a full DOM or an image" is therefore inaccurate. [v3 to v4 migration](https://docs.stagehand.dev/v4/migrations/v3).

**Playwright MCP also uses structured accessibility snapshots**, without requiring a vision model. Its README now explains the value of CLI + skills for limiting verbose schemas and trees in the context of coding agents. The comparison "agent-browser lightweight tree versus Playwright MCP heavy screenshot" is therefore not a correct description of the standard modes. [Official Playwright MCP README](https://github.com/microsoft/playwright-mcp).

The current agent-browser README describes a **Rust CLI and daemon**, with direct CDP. It documents filtered snapshots and optional screenshots; the text provided retains the old Node.js daemon architecture. [agent-browser README](https://github.com/vercel-labs/agent-browser#architecture).

## The quantitative advantages and guarantees are exaggerated

The "15× to 32× fewer tokens" ratios were not traced, in this check, to a reproducible primary benchmark comparing the same tasks, models, results and current modes of Stagehand/Playwright MCP/agent-browser. The generic Reddit, YouTube or LinkedIn links in the text do not allow these claims to be audited. This finding does not prove that no external benchmark exists; it forbids presenting this ratio as a general result established here.

The official benchmarks in the agent-browser repository compare **its old Node.js daemon and its Rust daemon**, not Stagehand's tokens against Playwright MCP. The benchmark README specifies that per-command delays depend mostly on Chrome and CDP exchanges; the main benefits of the Rust daemon concern startup, daemon memory and distribution. [agent-browser benchmarks](https://github.com/vercel-labs/agent-browser/tree/main/benchmarks).

The Crawlee guide explicitly mentions the limits of its Stagehand adapter: Chromium only and reduced fingerprinting control. One cannot automatically attribute to it all the launch/fingerprinting possibilities of `PlaywrightCrawler`, let alone a general guarantee of bypassing protections or of correct repair. [JS limits](https://crawlee.dev/js/docs/guides/stagehand-crawler-guide), [Python limits](https://crawlee.dev/python/docs/guides/stagehand-crawler).

## Proposed consequence for Automan

Adopting a compact page representation by default, a targeted scope and optional vision is consistent. This can be independent of the CDP driver, the queue and the CLI/MCP/SDK choice. Differentiation must be measured on real flows: observations avoided, deterministic actions reused, validated repair and correct results, rather than on the presence of an accessibility tree already shared by several tools.

For the detailed contracts and limits already inspected, see [the Crawlee/Spider study](2026-10-07-crawlee-spider-contracts-followup.md) and [the study of Stagehand alternatives](2026-10-07-stagehand-alternatives-followup.md). No new experimental result is added by this note.
