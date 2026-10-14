# Tests and CI run on free tiers, and live AI calls go through the Vercel AI Gateway

Goodfetch has no revenue, so no test, CI job or build spends money until the owner says there is revenue. Every paid resource the testing ADRs proposed is declined, and each gets a free replacement or waits switched off. Live model calls go through one Vercel AI Gateway key on its free monthly credit and free-tier models. Local secrets live in a git-ignored `.env` that `scripts/env-setup.sh` fills, and CI reads the same names from GitHub Actions secrets.

The testing ADRs were written to an expert bar in which cost never counted against an option, so they proposed about $355 a month of model credits, about $7,500 a year of Chromium build runners, paid fuzzing minutes, GitHub Secret Protection, an independent assessment, the reference host's nightly use and platform accounts, each switched off until approved. On 2026-10-14 the owner declined all of them for now ("decline paid for now, use free tier for CI and builds; for AI use Vercel AI Gateway") and asked for one place to keep the tokens implementation and testing need.

Facts this rests on, checked on 2026-10-14:

- **Vercel AI Gateway** ([pricing](https://vercel.com/docs/ai-gateway/pricing), [authentication](https://vercel.com/docs/ai-gateway/authentication-and-byok)): every team gets a free tier with a monthly free credit, limited to a subset of models and rate limited per model (a `429` when exceeded). Buying credits moves the team to the paid tier; the free credit then no longer applies. Keys are read from `AI_GATEWAY_API_KEY`, never expire unless revoked, and can carry a budget. It serves OpenAI-compatible requests at `https://ai-gateway.vercel.sh/v1` and Anthropic Messages at `https://ai-gateway.vercel.sh`.
- **GitHub Actions** is free on public repositories for standard runners; a private repository on the Free plan has 2,000 included minutes a month, which ADR 0064's ceiling already budgets. A self-hosted runner costs nothing.
- **Publishing** to npm, PyPI and crates.io uses OIDC trusted publishing (ADR 0032), so no registry token exists anywhere.

## Decision

- **No paid resource until the owner declares revenue.** No job, build, account or service that bills is created or switched on. An agent that finds a paid option better states the free option it chose instead, and records the paid one as declined, never as pending.
- **CI and builds use free tiers:** GitHub-hosted standard runners within the private plan's included minutes (ADR 0064's ceiling), unlimited once the repository is public, and self-hosted runners on the owner's machine where a build needs more than a hosted runner holds. A self-hosted runner never runs a pull request from a fork.
- **Live AI calls go through the Vercel AI Gateway** with one key, `AI_GATEWAY_API_KEY`, on the free tier only:
  - the provider check, the recorder, the Evaluations and the skill Evaluations of ADR 0065 call models through the gateway's OpenAI-compatible and Anthropic Messages endpoints;
  - they use only free-tier models, and a `429` or an exhausted free credit makes the run inconclusive (ADR 0064), never a purchase;
  - no credits are bought and auto top-up stays off, so the key cannot spend money; that replaces ADR 0065's per-job paid caps;
  - Model recordings of the OpenAI-compatible and Anthropic formats are made through the gateway; whether the gateway serves Gemini's native format is unverified, and until it is, the Gemini adapter's recordings wait (its Scripted model and Fake model tests run as before);
  - local models (llama.cpp, Ollama) and the Scripted and Fake models stay the default for every test, as ADR 0065 decided.
- **Each declined resource and its replacement:**

  | Declined | Instead |
  |---|---|
  | model credits and per-provider keys (ADR 0065) | the gateway's free tier, above |
  | the Evaluation regression margin's paid runs (ADR 0065) | Evaluations sized to the free credit; the margin Δ stays proposed |
  | the Chromium build gate's hosted runners and macOS runner (ADR 0074) | a free design decided before M3: the owner's machine as a self-hosted runner, other platforms only at release, the Control build only on demand |
  | private fuzzing minutes beyond the free plan (ADR 0067) | fuzzing within the included minutes; ClusterFuzzLite and OSS-Fuzz once public, which are free |
  | the test V8 build's extra minutes (ADR 0067) | built on the owner's machine as a self-hosted runner |
  | GitHub Secret Protection while private (ADR 0067) | `gitleaks`, as ADR 0067 already does until the repository is public; GitHub's scanning is free once public |
  | an independent security assessment (ADR 0067) | the manual threat-model review; the assessment waits for revenue |
  | the reference host's nightly use (ADR 0071) | the owner's machine, which is the reference host, when it is on; no hosted timing |
  | Integration platform accounts and the Cloudflare Tunnel (ADR 0070) | free developer plans and a free Cloudflare Tunnel, used only from M4 |
  | GPU hosts, a larger cache, larger runners, hosted visual services, Antithesis | not used |

- **Local secrets live in `.env` at the repository root:** git-ignored, mode 600, written only by `scripts/env-setup.sh`, which reads the names, their purpose, the milestone that needs them and the page that creates each from the committed `.env.example`, and never prints a value. `.env.example` is the one list of every secret a contributor may need.
- **Tests never read `.env`.** Test support clears the environment (ADR 0059). Only the recipes that run Live checks load it, and CI reads the same names from GitHub Actions secrets, which fork pull requests never receive.

## Considered Options

- **Keep the paid resources switched off until approved (the testing ADRs as written):** rejected, because a list of pending purchases invites a later agent to request them and keeps paid designs (the Chromium gate's runners) as the plan of record.
- **One paid model key per provider with hard caps (ADR 0065):** rejected while there is no revenue; the gateway's free tier gives one key, no spend and the same request formats.
- **A secrets manager (1Password CLI, Doppler, SOPS):** rejected for now; a git-ignored `.env` written by a script is free, needs no account, and the names move unchanged into any manager later.
- **Making the repository public now to get unlimited free minutes:** not decided here; it is the owner's call, and this ADR works either way.

## Consequences

- **Amends ADR 0065:** live model calls go through the Vercel AI Gateway's free tier with one key instead of per-provider credentials and paid caps; the $355 monthly budget and the $150 per release candidate are withdrawn; Gemini recordings wait as above. A one-line note sits at its budget.
- **Amends ADR 0074:** the hosted build runners and the macOS runner are declined; the gate's free design is decided before M3 in [Decide the free Chromium build gate](https://github.com/Jbmanllr/automan/issues/445), which [Chromium build-and-test path](https://github.com/Jbmanllr/automan/issues/402) waits on. A one-line note sits at its cost.
- **Amends ADR 0067:** paid fuzzing minutes, GitHub Secret Protection while private and the independent assessment are declined as in the table. A one-line note sits at its owner approvals.
- **Adds to ADRs 0070 and 0071:** platform accounts and the tunnel use free plans; the reference host is the owner's machine.
- **Adds to ADR 0032:** the tree gains `scripts/` for repository helper scripts, and `.gitignore` ignores `.env` and `.env.*` except `.env.example`.
- **`AGENTS.md`** states the no-paid-resources rule; **`TESTING.md`** gains a section on free tiers and local secrets, and its live-model credential rule is edited in place.
- **Owner tickets:** [Owner purchases for security testing](https://github.com/Jbmanllr/automan/issues/441) and [Owner accounts for Integration tests](https://github.com/Jbmanllr/automan/issues/442) record the purchases as declined and the free accounts as M4 work; [Owner confirmations and decisions](https://github.com/Jbmanllr/automan/issues/392) records this decision.
