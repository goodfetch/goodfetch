# One Operation catalog serves every agent surface

The engine exposes one typed Operation catalog, each operation classed read, idempotent write or Effect, and generates every agent surface from it: an MCP server (stdio and Streamable HTTP), the SDK methods, thin tool adapters for the Vercel AI SDK, LangChain/LangGraph (TypeScript and Python) and Mastra, the CLI with `--json`, and agent skills. Coding agents get the CLI plus skills, agent apps get MCP, and both can use code mode: a short model-written program that runs in a JavaScript sandbox inside the engine with no network or files, only catalog calls (journaled), and time and memory limits. Profiles trim the catalog so tool definitions stay short. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).

Evidence: lab [interface arena](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/code-mode-journal-2026-10-09.md) and [AI-in-the-loop browser frameworks](https://github.com/Jbmanllr/automan/blob/research/ai-browser-frameworks/rnd/research/2026-10-10-ai-browser-frameworks.md) §6.

## Considered Options

- **Hand-written wrappers per agent framework**: rejected. MCP reaches all 15 frameworks surveyed and dedicated wrappers see little use (Firecrawl's MCP server has about 117k weekly downloads, framework wrappers tens to thousands); generated adapters cost a few dozen lines each.
- **The CLI for humans only**, the R&D's position: rejected. Playwright now steers coding agents to its CLI plus skills and Midscene dropped MCP; the lab's model-dependent CLI result (Haiku left 18 of 45 tasks unfinished, without skills) becomes a lab campaign and a documented warning.
- **Code mode without a sandbox or journal**: rejected. Raw code mode gave false successes in the lab, and eval of model output led to remote code execution in LaVague.

## Consequences

- Other frameworks (OpenAI Agents SDK, Pydantic AI, CrewAI, Google ADK, LlamaIndex, Strands, Claude Agent SDK) connect through their MCP clients, each with a tested docs page.
- Observations carry stable element references, send diffs after the first one, and include a screenshot only when the page changed.
