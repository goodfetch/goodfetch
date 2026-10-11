# Payment Required is a block cause, and paid fetches are opt-in, capped and journaled

A site that asks a crawler to pay, with HTTP 402 or a paywall redirect, gets the block cause `payment_required`. Goodfetch never escalates, rotates or retries past it, because changing identity to avoid a payment gate is the masking the legal research rates medium to high risk. A run can instead pay, from the user's own Cloudflare account or wallet, through Cloudflare's pay-per-crawl headers or x402. Every payment goes to a domain the run lists, under a price cap and a currency Limit reserved before sending, through the Effect journal so that a resume never pays twice. Goodfetch itself never charges anyone or holds money. This answers pain #6 (`compliance-site-owner-politeness`): site owners who want to be paid rather than merely block. Decided in [Network defaults](https://github.com/Jbmanllr/automan/issues/118).

Cloudflare Pay per crawl, in private beta since July 2025 ([blog](https://blog.cloudflare.com/introducing-pay-per-crawl/)), answers an unpaid crawler with a 402 and a `crawler-price` header. The crawler retries with `crawler-exact-price`, or sends `crawler-max-price` up front, inside a Web Bot Auth signature that covers the header. A 200 then carries `crawler-charged`, and Cloudflare bills the crawler's registered account as merchant of record ([docs](https://developers.cloudflare.com/ai-crawl-control/features/pay-per-crawl/use-pay-per-crawl-as-ai-owner/crawl-pages/)). x402 has been a Linux Foundation project since 2 April 2026, with the schemes `exact`, `upto`, `batch-settlement` and `auth-capture` ([specs](https://github.com/x402-foundation/x402/tree/main/specs/schemes)). Cloudflare's proposed deferred scheme became `batch-settlement`, whose Cloudflare binding is credit-backed and authenticated by Web Bot Auth ([spec](https://github.com/x402-foundation/x402/blob/main/specs/schemes/batch-settlement/scheme_batch_settlement_cloudflare.md)), and Cloudflare's Monetization Gateway beta charges agents per request over x402 ([blog](https://blog.cloudflare.com/monetization-gateway-beta/)). TollBit, through Fastly, redirects bots to its paywall with a 302 rather than a 402 ([Fastly](https://www.fastly.com/documentation/guides/integrations/non-fastly-services/tollbit/)). Crawlee, Firecrawl, Spider and Scrapy neither recognize a 402 nor pay or sign; Firecrawl's `auto` mode would retry it through a stealth proxy. Browserbase signs requests, but only in its cloud ([signed agents](https://blog.cloudflare.com/signed-agents/)).

## Considered Options

- **Treating a 402 like a 401, 403 or 429** (ADR 0021's escalation, Firecrawl's `auto`): rejected. It changes identity to avoid a payment the site asked for, which is what the *Reddit* case rests on ([legal exposure](https://github.com/Jbmanllr/automan/issues/65)).
- **Never paying, so a 402 stays a block**: rejected. Pain #6 would get half an answer, and sites that sell access to crawlers would stay closed.
- **Cloudflare's headers only**: rejected. x402 is the open standard that Cloudflare's own newer products use.
- **Paying any domain the crawl reaches, within the currency Limit**: rejected. A broad crawl could spread its currency Limit over sites the user never chose to buy from.
- **Fetching exchange rates to compare a price with a Limit in another currency**: rejected. It would be an unconfigured call (ADR 0043) and a price feed the engine does not own (ADR 0001).
- **Cloudflare Pay Per Use** ([blog](https://blog.cloudflare.com/pay-per-use/)): out of the engine's reach. It is an account agreement in which the buyer reports usage, not a request protocol.

## Consequences

**The block cause.**
- A 402, or a redirect that the block-detection rules match as a paywall (TollBit's), is classed `payment_required`. The quoted price, when the response gives one, is recorded in Provenance.
- **Changes ADR 0021:** `payment_required` never moves a domain up a proxy tier, never retires its Session to rotate, and is never retried unless the run may pay that domain. Rules detect it, never the Decision provider.
- A page left at `payment_required` is not collected, so a run with one in scope reports Completeness `incomplete` and lists those pages.
- The Console's "Needs you" view lists the domains that asked for payment with their prices, and offers to add them to `pay_domains` for the next run.

**Site preferences.**
- **Changes ADR 0016:** RSL (Really Simple Licensing, a standard since December 2025; [rslstandard.org](https://rslstandard.org/)), declared through robots.txt, joins the Site preferences the engine always reads and records in Provenance, and is honoured under `respect_site_preferences`.

**Paying.**
- A run may pay only when all of these hold: its Session's Access mode is `identified` (ADR 0021); the domain, matched by registrable domain as in ADR 0004, is in `pay_domains` (`*` is accepted only when written explicitly); the price is at or below `max_price_per_page`, set for the run or per entry of `pay_domains`; and a currency Limit applies in some scope (ADR 0041). A run that sets paying without a currency Limit is refused at start.
- Schemes: Cloudflare's `crawler-*` headers; x402 `batch-settlement` with its Cloudflare binding; x402 `exact` and `upto` on EVM networks and Solana. x402 is built on `x402-rs` (Apache-2.0), which covers `exact`, and `alloy` for EIP-712 and EIP-3009 signing. Goodfetch contributes `upto` and `batch-settlement` to `x402-rs`.
- Each paid fetch reads the quote, reserves the worst case against the currency Limit (ADR 0041's reserved costs), journals the payment as an Effect (ADR 0010), sends it, then records what was charged. A paid fetch is never hedged or coalesced (ADR 0040) and never served from or stored in the response cache (ADR 0046).
- A payment counts only against a currency Limit in its own currency. The price table (ADR 0012) may declare fixed rates, such as USDC to USD at 1:1. The engine fetches no rates.
- **Changes ADR 0041:** a new Usage kind, `paid_fetch`, counts paid fetches and their amounts by currency. Limit rows that name it carry the must-understand flag, so an older engine refuses rather than overspends.
- Every payment goes to the Audit log and is marked on the Record's Provenance with the scheme, the amount and the payee domain.

**Exactly once.**
- For x402 payloads that can be settled only once (EIP-3009 and Permit2 nonces, a signed Solana transaction), a resumed run re-sends the journaled payload, which cannot be charged twice. If the site refuses it as already used, the payment is recorded as made and the page as not received, and a person reconciles it (ADR 0029).
- Cloudflare's headers carry no idempotency key. A payment interrupted before its response is an uncertain Effect: it is never paid again automatically, the page stays at `payment_required`, and only a person reconciles it.

**Keys.**
- **Changes ADR 0021:** `goodfetch identity create` generates the Ed25519 key of an `identified` Session, kept as a Secret reference (ADR 0015), and `goodfetch identity directory` exports the signed key directory for the user to host at `/.well-known/http-message-signatures-directory` on a domain they control, which Cloudflare requires along with registration through its Bot Submission Form ([Web Bot Auth](https://developers.cloudflare.com/bots/reference/bot-verification/web-bot-auth/)). Signing uses Cloudflare's `web-bot-auth` crate (Apache-2.0). The engine sends `Signature-Agent` in the string form Cloudflare accepts; both Web Bot Auth drafts are still individual drafts, so the conformance suite tracks the form Cloudflare verifies.
- x402 wallet keys are Secret references too, resolved only at signing.
- Registering with Cloudflare and funding an account or wallet stay with the user; the docs walk through both.
