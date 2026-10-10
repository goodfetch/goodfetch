# One Operation catalog serves every agent surface

The engine exposes one typed Operation catalog, each operation classed read, idempotent write or Effect, and generates every agent surface from it: an MCP server (stdio and Streamable HTTP), the SDK methods, the CLI with `--json`, agent skills, and thin tool adapters for the Vercel AI SDK, LangChain/LangGraph (TypeScript and Python) and Mastra; the Claude Agent SDK uses the MCP server directly. Coding agents get the CLI plus skills, agent apps get MCP, and both can use code mode: a short model-written program that runs in a JavaScript sandbox inside the engine with no network or files, only catalog calls (journaled), and time and memory limits; it is opt-in in MCP profiles. Profiles trim the catalog so tool definitions stay short. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).

Evidence: lab [code-mode report](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/code-mode-journal-2026-10-09.md) and [AI-in-the-loop browser frameworks](https://github.com/Jbmanllr/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md) §6.

## Considered Options

- **Hand-written wrappers per agent framework**: rejected. MCP reaches all 15 frameworks surveyed and dedicated wrappers see little use (Firecrawl's MCP server has about 117k weekly downloads, framework wrappers tens to thousands); generated adapters cost a few dozen lines each.
- **The CLI for humans only**, the R&D's position: rejected. Playwright now steers coding agents to its CLI plus skills and Midscene dropped MCP; the lab's model-dependent CLI result (Haiku left 18 of 45 tasks unfinished, without skills) becomes a lab campaign and a documented warning.
- **Code mode without a sandbox or journal**: rejected. Raw code mode gave false successes in the lab, and an open LaVague report shows prompt injection reaching code execution through eval of model output.

## Consequences

- Other frameworks (OpenAI Agents SDK, Pydantic AI, CrewAI, Google ADK, LlamaIndex, Strands) connect through their MCP clients, each with a tested docs page.
- Observations carry stable element references, send diffs after the first one, and include a screenshot only when the page changed.
- One web search operation takes interchangeable providers: search APIs (Brave, Exa, Tavily, Perplexity, Mojeek, Kagi) and SearXNG with the user's key or URL, each flagged for whether its results may be kept in the run; a model provider's built-in search, offered only as an option of AI operations and never as a source of links to crawl, as Google's grounding terms require; and opt-in browser search, DuckDuckGo by default, which stops at the first challenge and never masks itself. Evidence: [web search sources research](https://github.com/Jbmanllr/automan/blob/research/web-search-sources/rnd/research/2026-10-10-web-search-sources.md).
- Sites' own WebMCP tools join the catalog as Effects unless marked read-only; a crawl configured from a prompt compiles into a configuration reviewed before it runs.
