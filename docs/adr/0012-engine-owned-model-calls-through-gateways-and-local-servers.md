# The engine owns its model calls, speaks three formats, and reaches the rest by URL

Every model call the engine needs is made by the engine, so it can be journaled, counted in Usage, held to Limits and kept free of secrets; an SDK may register a model callback that the engine still journals, counts and caps, and an outside agent's own model calls stay outside the engine. The provider layer is Automan's own interface over three formats: the OpenAI-compatible format through the async-openai crate, with a Tower layer on every HTTP attempt and its default retries replaced by Automan's; and in-house native adapters for Anthropic Messages and Gemini. Cloud models are reached through a gateway the user picks (Vercel AI Gateway, OpenRouter, any OpenAI-compatible gateway, which also carry the user's own Azure, Bedrock and Vertex keys) or a direct key; local models (Ollama, LM Studio, llama.cpp server, vLLM) through the same OpenAI-compatible path. Each AI operation picks its own model. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).

Evidence: [Rust model-provider libraries](https://github.com/Jbmanllr/automan/blob/research/rust-llm-libraries/rnd/research/2026-10-10-rust-llm-libraries.md).

## Considered Options

- **Rig**: rejected; the widest coverage, but every minor release breaks, about every two weeks.
- **genai**: kept as the fallback; clean, but no hooks and no way to add a provider from outside.
- **aisdk**: rejected; text-only input, no computer use or provider search, hooks that cannot refuse a call, no release since February 2026.
- **Forking a library**: rejected; Automan would own all its code.
- **The OpenAI format alone**: rejected. Anthropic calls its OpenAI-compatible endpoint a testing aid, and through it structured output, prompt caching and thinking are lost; Gemini's compatible endpoint lacks search grounding and computer use.
- **Gateways only**: rejected. Many users want no third party or work offline; Vercel's BYOK falls back to its own credentials when a key fails.

## Consequences

- JSON repair and a bounded retry that feeds the validation error back to the model live above the three formats, since no Rust library has them; each release publishes a tested list of local models.
- Money Limits: the engine reserves each call's worst case before sending it (maximum output tokens at the model's price, from a shipped, overridable price table) and refuses a call that would cross the Limit; a stopped run ends incomplete and resumable. These are third-party prices, so ADR 0001 holds.
