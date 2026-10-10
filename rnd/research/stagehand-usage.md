# How Stagehand is used

Research consulted on October 7, 2026. This section describes Stagehand **v4** based on official documentation. The documentation examples demonstrate possible uses; they do not measure how often customers actually use them.

## What the developer installs and runs

Stagehand provides TypeScript/JavaScript, Python and Go SDKs. The quickstart installs the package for the chosen language, creates an `index.ts`, `main.py` or `main.go` file, launches Chrome locally, initializes Stagehand, then calls `act`, `extract` and `observe`. The script remains an ordinary program run with Node/tsx, Python or Go. [Quickstart v4](https://docs.stagehand.dev/v4/first-steps/quickstart)

The developer can mix precise commands, such as `page.goto()` and `page.locator().fill()`, with natural-language instructions and schema-validated extraction. They choose which passages use a model. [Introduction v4](https://docs.stagehand.dev/v4/first-steps/introduction)

The browser can run on the developer's machine, in a Browserbase session, or in an already running Chromium to which the SDK connects via CDP. Using the SDK therefore does not require using the Browserbase cloud service. The local browser requires a Chrome installation and a model provided by the developer for the AI primitives; Browserbase notably adds the Model Gateway and the server-side cache. [Quickstart v4](https://docs.stagehand.dev/v4/first-steps/quickstart), [Browser configuration](https://docs.stagehand.dev/v4/configuration/browser)

## The main modes of use

| Mode | Where Stagehand comes in | Who decides what happens next? |
| --- | --- | --- |
| Standalone script | A program launches a browser and performs a sequence of operations. | The script's code. [Quickstart](https://docs.stagehand.dev/v4/first-steps/quickstart) |
| Application or workflow | The application uses the SDK to automate a service's web workflow, then retrieves the results. | The application's code. [Integrations: Applications](https://docs.stagehand.dev/v4/integrations/overview) |
| Backend, webhook or job | An automation function is invoked via API from a backend, a worker, a webhook or a scheduled task. | The handler written by the developer. Browserbase Functions is one deployment option. [Functions](https://docs.browserbase.com/platform/functions/overview) |
| LLM agent developed in an application | The agent's framework receives browser tools and keeps a session between calls. | The model and the tool loop managed by the agent's framework. [Agent Frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview) |
| Existing development agent | Claude Code, Codex, fx or Pi receives tools to inspect an application under development or browse the web. | The development agent. [CLI Agents](https://docs.stagehand.dev/v4/integrations/cli-agents/overview) |

**Interpretation for the product:** an application can offer a button or a chat that triggers Stagehand in its backend/worker. The end user sees the feature and the result; it is the application developer who integrates the SDK. This button → backend → Stagehand → browser → result flow is an illustrative integration example, consistent with the documented Applications and Functions modes. [Integrations](https://docs.stagehand.dev/v4/integrations/overview), [Functions](https://docs.browserbase.com/platform/functions/overview)

## Five concrete official examples

1. **Small local script:** open `stagehand.dev`, extract the value proposition into a typed object, click on Evals, then discover the available actions. The program writes these three steps explicitly. This is the getting-started example, not a customer testimonial. [Quickstart v4](https://docs.stagehand.dev/v4/first-steps/quickstart)
2. **Automation called via API:** the `page-title` Function receives a URL, connects Stagehand to the Browserbase session created for this invocation, navigates, extracts a title, then returns JSON. The same mechanism allows triggering from a backend, a webhook, a queue or a job. This is an official deployment example. [Functions](https://docs.browserbase.com/platform/functions/overview), [Invocation](https://docs.browserbase.com/platform/functions/invoke)
3. **Agent in an application:** the Vercel AI SDK example opens an MCP client once, loads `run`, `snapshot` and `screenshot`, then lets its tool loop respond to a request such as opening a site and reporting its title. The browser keeps authentication and state between calls. The integration is experimental and is built from the repository; it is not published as an independent adapter. [Vercel AI SDK](https://docs.stagehand.dev/v4/integrations/vercel-ai-sdk)
4. **Development agent:** configure the Stagehand MCP server in Codex or Claude Code, then ask the agent to open a page and report its title. The docs also mention inspecting a running application. The agent uses its own model and authentication; Stagehand provides the browser tools. The shared adapter is experimental and is built from the repository. [CLI Agents](https://docs.stagehand.dev/v4/integrations/cli-agents/overview)
5. **Integrated business workflow:** the Stagehand 4.1.0 + Stripe Link example prepares a flower delivery on 1-800-Flowers, reviews the cart, requests payment approval via Link and can fill in the checkout. Stagehand manipulates the site; Link handles the approval request and the payment method; the application orchestrates the workflow. This is a complete official example, not proof of measured adoption. [Stagehand + Link SDK](https://docs.browserbase.com/integrations/stripe/stagehand), [Stripe overview](https://docs.browserbase.com/integrations/stripe/introduction)

## The key point of v4: the engine is not the agent loop

`stagehand.agent()` and `agent.execute()` are gone in v4. There is no one-to-one replacement. Two uses are recommended: an assistant writes a script that is then executed, or a model stays in a tool loop that the application owns. `act`, `extract` and `observe` remain available for steps that benefit from a natural-language instruction. [Migration v3 → v4](https://docs.stagehand.dev/v4/migrations/v3)

In an agent integration, the external framework manages the model, the instructions and the loop. Stagehand keeps the browser and executes the operations. The repository's integrations share three tools: `run` to execute JavaScript or element actions, `snapshot` to read an accessibility representation, and `screenshot` for a capture. They exist as MCP/stdio or as native tools depending on the framework: Anthropic SDK, Eve, Deep Agents, CrewAI, Mastra and Vercel AI SDK. [Agent Frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview), [Integrations README](https://github.com/browserbase/stagehand/blob/main/packages/integrations/README.md)

Deterministic operations such as navigation, snapshot and screenshot do not require an additional Stagehand model. If the agent also calls Stagehand AI methods, their model can have a configuration and credentials distinct from those of the model directing the agent. [Agent Frameworks: Configure models](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)

## Where each part runs

The SDK code and the script/agent run in the host process of the developer or the application. The core of Stagehand v4 — target management, frames, execution contexts and CDP commands — runs in a browser extension. The SDKs are clients that talk to this core via RPC over the CDP connection. When a developer provides a custom model function, this function stays in their process and the extension requests generations from it over the same channel. [Official v4 architecture](https://www.browserbase.com/blog/stagehand-v4)

In MCP integrations, the server keeps a persistent browser session. The JavaScript sent to `run` executes in the Stagehand extension's service worker; it can control the browser. The agent that chooses this call runs in its own environment. [Agent Frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)

Browserbase Functions is a complementary hosting product: the handler's TypeScript code runs close to the cloud browser, and the application invokes the function via API. The Stagehand SDK, the Browserbase hosted browser and Functions code hosting must be distinguished. [Functions](https://docs.browserbase.com/platform/functions/overview)

## Consequence for a similar framework

**Design inference:** the consumption interfaces to plan for are first an SDK usable in a script/backend, then tools for agents/MCP. A graphical app specific to the framework can be added, but the documented uses do not require it. The Rust choice concerns the implementation of the core and its distribution; the consuming developer mainly needs a simple installation, a natural API in their language and suitable deployment. This conclusion is deduced from the modes of use above; it is not an official Stagehand recommendation. [Integrations](https://docs.stagehand.dev/v4/integrations/overview)

## Caution about versions

Some marketing pages or indexed results still talk about four primitives and `agent()`. To answer questions about v4, the versioned documentation and the v3 → v4 migration are authoritative: three AI primitives, browser APIs and external orchestration. Do not take the old `new Stagehand()`, `init()` or `agent.execute()` examples as the v4 path. [Migration v3 → v4](https://docs.stagehand.dev/v4/migrations/v3)

## User feedback on Reddit

The following discussions are public statements by their authors. Company identities, cost figures and success rates have not been independently verified. They illustrate uses and integration questions; they measure neither the popularity of each use nor the performance of v4. The dates below come from the thread search results, and the contents were opened and reread.

| Post date | Use described | Scope of the testimony |
| --- | --- | --- |
| May 22, 2026 | The author says they maintain a price monitoring pipeline for a SaaS. They describe Stagehand as the control SDK and Browserbase as the browser service, with results feeding dashboards. | Self-reported example of integration into a product and of recurring collection. Post predating v4; its snippets and figures do not serve as a description or benchmark of the current version. [WebScrapingInsider thread](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/we_rebuilt_our_scraping_stack_4_times_in_18/) |
| April 7, 2025 | An author explains that they are looking for a browser automation building block for an agent they are building and appreciates the ability to control individual steps. | Old testimony mixed with promotion of their newsletter. Useful for identifying the role of a tool within an agent, not for evaluating v4 or reusing the token figures put forward. [r/node thread](https://www.reddit.com/r/node/comments/1jtdwpz/stagehand_node_package_to_control_browser_with/) |
| August 13, 2026 | A user wants to give their local Hermes agent a browser and hesitates between the Stagehand SDK, Browse CLI and Browse.sh recipes. | Example of a need to integrate with an existing agent and of confusion between the product's layers. The installation choices and features are verified in the official docs, not inferred from the comments. [LLMDevs thread](https://www.reddit.com/r/LLMDevs/comments/1vn530v/confused_about_browserbase_product_lines/) |
| April 15, 2026 | A user compares Stagehand and Browser Use browser agent sessions. Others comment on difficulties with authentication and element localization. | Individual feedback, sparsely detailed use case predating v4. No reliability ranking can be deduced from it. [AI_Agents thread](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/) |

## Practical answer to "an app or an agent?"

Both: Stagehand is a library that the developer consumes, and the program that consumes it can be a script, a backend/worker or an agent. In a script, the code chooses the sequence; in an agent, the agent's loop chooses the tools and Stagehand executes the browser operations. This distinction is directly documented in the [v4 quickstart](https://docs.stagehand.dev/v4/first-steps/quickstart) and the [agent integrations](https://docs.stagehand.dev/v4/integrations/agent-frameworks).

Illustrative application flow: the user requests an export in the interface; the backend triggers a worker; the worker uses Stagehand to open the site and retrieve the data; the application stores and displays the result. This flow is a design illustration, not an identified customer or a universal export feature provided by Stagehand. The code hosting can be the developer's own or a complementary offering such as [Browserbase Functions](https://docs.browserbase.com/platform/functions/overview).

For the envisaged framework, the consumer interface to define first is therefore a library API with session management, browser operations and structured results. An adapter for agents/MCP is a second surface on the same engine. This proposal does not yet assume a visual builder, a marketplace or a SaaS.
