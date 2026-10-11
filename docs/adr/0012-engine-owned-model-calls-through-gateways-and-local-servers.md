# The engine owns its model calls, speaks three formats, and reaches the rest by URL

Every model call the engine needs is made by the engine, so it can be journaled, counted in Usage, held to Limits and kept free of secrets; an SDK may register a model callback that the engine still journals, counts and caps (the SDK-handler form of the Model provider Plugin kind, ADR 0033), and an outside agent's own model calls stay outside the engine. The provider layer is Goodfetch's own interface over three formats: the OpenAI-compatible format through the async-openai crate, with a Tower layer on every HTTP attempt and its default retries replaced by Goodfetch's; and in-house native adapters for Anthropic Messages and Gemini. Cloud models are reached through a gateway the user picks (Vercel AI Gateway and OpenRouter, which both accept the user's own Azure, Bedrock and Vertex keys, or any OpenAI-compatible gateway) or a direct key; local models (Ollama, LM Studio, llama.cpp server, vLLM) through the same OpenAI-compatible path. Each AI operation picks its own provider and model. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).

Evidence: [Rust model-provider libraries](https://github.com/Jbmanllr/automan/blob/research/rust-llm-libraries/rnd/research/2026-10-10-rust-llm-libraries.md); vendor pages for [Anthropic's OpenAI compatibility](https://platform.claude.com/docs/en/api/openai-sdk), [Gemini's OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai), [Vercel AI Gateway BYOK](https://vercel.com/docs/ai-gateway/authentication-and-byok/byok) and [OpenRouter BYOK](https://openrouter.ai/docs/guides/overview/auth/byok).

## Considered Options

- **Rig**: rejected; the widest coverage, but every minor release breaks, about every two weeks.
- **genai**: kept as the fallback; clean, but no hooks and no way to add a provider from outside.
- **aisdk**: rejected; text-only input, no computer use or provider search, hooks that cannot refuse a call, no release since February 2026.
- **Forking a library**: rejected; Goodfetch would own all its code.
- **The OpenAI format alone**: rejected. Anthropic calls its OpenAI-compatible endpoint a testing aid, and through it structured output, prompt caching and thinking are lost; Gemini's compatible endpoint is in beta and lacks search grounding for chat and computer use.
- **Gateways only**: rejected. Many users want no third party or work offline, and Vercel's BYOK falls back to Vercel's own credentials when a user's key fails.

## Consequences

- JSON repair and a bounded retry that feeds the validation error back to the model live above the three formats, since no Rust library has them; each release publishes a tested list of local models.
- Limits apply to tokens and calls natively. A currency Limit uses a price table shipped with each release and overridable: the engine reserves each call's worst case before sending it (maximum output tokens at the model's price) and refuses a call that would cross the Limit; a stopped run ends incomplete and resumable with a higher Limit, which only a person can grant (ADR 0041). These are third-party prices, so ADR 0001 holds.
- The price table may also declare fixed rates between currencies, such as USDC to USD, for Paid fetches; the engine never fetches exchange rates (ADR 0045).
