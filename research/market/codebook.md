# Piste « demande marché » : codebook et protocole de codage

**Statut :** v1, figée pour le corpus du 8 octobre 2026 (`corpus.jsonl`, 421 items). **Date :** 2026-10-08.
**Portée :** ce document définit les codes appliqués aux items du corpus, la manière de les appliquer, les contrôles d'accord et de sources, et les règles d'exclusion. Il ne contient aucun résultat d'analyse ; les classements sont dans [pains-ranking.md](./pains-ranking.md).

## 1. Unité d'analyse

Un **item canonique** est un énoncé unique d'un acteur identifiable sur un sujet : ouverture d'issue GitHub, commentaire HN, post ou commentaire Reddit (via instantané), avis ou synthèse d'avis, rapport, billet, offre de mission. Un même fil peut produire plusieurs items s'ils portent des énoncés distincts, par exemple `reddit-0` et `reddit-1`, ou `hn-32` et `hn-33`. Les doublons d'URL et de contenu ont été fusionnés en amont par le workflow. La déduplication n'est pas rejouable à partir des seuls fichiers de ce dossier.

Champs de chaque item (`corpus.jsonl`) :

| Champ | Sens |
| --- | --- |
| `id` | Préfixe du collecteur (`github`, `hn`, `reddit`, `reviews-reports`, `demand-use-cases`) suivi d'un rang ; les trous de numérotation viennent du filtrage amont |
| `url`, `date` | Source et date de l'énoncé ; `YYYY-MM` quand seul le mois est connu. Pour Reddit, la date est celle de l'instantané (§5) |
| `source_type` | `github-issue`, `github-discussion`, `hn`, `reddit`, `review`, `report`, `blog`, `job-post`, `other` |
| `product` | Produit visé, tel que nommé par la source (`general` si aucun) |
| `kind` | `pain`, `feature-request`, `use-case`, `pricing-signal`, `statistic`, `switching`, `praise` |
| `segment` | Segment estimé : `agent-developer`, `data-team`, `b2b-portal-integration`, `ecommerce-pricing`, `seo-marketing`, `enterprise`, `research-academic`, `hobbyist`, `unknown` |
| `codes` | 1 à 3 codes feuilles du tableau §3 ; **le premier est le code primaire** |
| `engagement` | Mesure brute propre à la source (§4), non comparable d'une source à l'autre |
| `promo`, `vendor_affiliated` | Contenu promotionnel ; auteur lié à un éditeur (membre ou collaborateur du dépôt, fondateur, employé, concurrent) |
| `summary` | Paraphrase en anglais rédigée au codage ; ce n'est pas une citation |
| `quote` | Citation verbatim de moins de 15 mots, présente seulement si elle respecte cette limite |
| `source_checked`, `check_note` | Item contrôlé avec succès au §5, avec réserve éventuelle |

## 2. Protocole de codage

1. **Collecte.** Cinq collecteurs indépendants (GitHub, Reddit, HN, avis et rapports, demande et cas d'usage) ont travaillé en lecture seule, sans contourner de blocage d'accès (Reddit est passé par un site tiers d'instantanés, faute d'accès direct). Les collecteurs Reddit et HN n'ont enregistré aucun nom d'utilisateur, et le corpus n'en contient aucun. Les items impossibles à dater ont été écartés à la collecte. Les limites de couverture de chaque collecteur figurent dans le [README](./README.md#5-couverture-et-trous).
2. **Premier codeur (LLM).** Chaque item reçoit 1 à 3 codes feuilles. Le code primaire est le besoin ou la douleur principale de l'énoncé, les codes suivants décrivent des besoins secondaires explicitement présents. Les codes parents (sans `parent`) ne sont jamais attribués directement ; dans le corpus final, tous les codes utilisés sont des feuilles, et les 30 feuilles sont toutes utilisées. Moyenne : 1,79 code par item (755 attributions pour 421 items).
3. **Règles de décision.**
   - On code ce que l'énoncé **dit**, pas ce que le produit cité fait.
   - `kind = statistic` et les codes `market-adoption-stats`, `wtp-price-points`, `build-vs-buy-switching` et `usecase-*` sont des **codes de contexte** (marché, prix, cas d'usage). Ils sont classés à part des douleurs dans `pains-ranking.md`.
   - Un blocage présenté comme un crash ou un 500 relève de `silent-false-success` en plus de `access-antibot-ip`. Une boucle qui facture relève de `cost-unpredictable-runaway` en plus de `rel-hang-crash`.
   - `agent-deterministic-replay` couvre la **demande** de compiler, cacher ou rejouer. Les plaintes sur le coût d'un agent sans cette demande relèvent de `agent-token-latency-cost`.
   - Le segment est estimé à partir du contexte déclaré. Les offres de génération de leads sont rattachées à `seo-marketing`, faute de segment « ventes ».
4. **Second codeur indépendant.** Il a recodé un échantillon **stratifié de 120 items** sans reprendre les codes du premier. Résultats :
   - **Accord sur le code primaire : 83,3 %** (100/120).
   - **Jaccard moyen sur l'ensemble des codes : 0,838.**

   Ces taux sont des accords bruts, sans correction du hasard. Un kappa n'a pas été calculé : le codage est multi-étiquettes et le détail des désaccords n'a pas été transmis à cette synthèse.

   Lecture : l'accord est suffisant pour classer les familles de douleurs. Il ne permet pas de départager des codes dont les scores diffèrent de moins d'environ 15 %.
5. **Contrôle de sources** (§5). Sur 60 items tirés, on a vérifié l'URL, la date et la fidélité du résumé.

## 3. Codes

Le **n** compte les items portant le code, quelle que soit sa position, et le **n primaire** ceux où il vient en premier. Les définitions sont reproduites telles qu'elles ont servi au codage.

| Code | Libellé | Parent | Définition (texte du codage, anglais d'origine) | n | n primaire |
| --- | --- | --- | --- | ---: | ---: |
| `cost-billing` | Cost and billing | — | Parent: complaints and signals about what scraping/automation costs and how it is billed. | — | — |
| `cost-unpredictable-runaway` | Unpredictable or runaway spend | `cost-billing` | Opaque credits, credit multipliers, surcharges, bandwidth billing, expiring credits, charges on failure, and missing hard spend caps or budgets (including retry loops burning money). | 28 | 24 |
| `cost-infra-proxy-spend` | Proxy/captcha/infra spend growth | `cost-billing` | Residential proxy, captcha-solving, compute and hosting costs that grow faster than value; TCO of the access stack. | 19 | 11 |
| `access-blocking` | Access and blocking | — | Parent: getting to the content at all. | — | — |
| `access-antibot-ip` | Anti-bot, captcha, fingerprint and IP blocks | `access-blocking` | Cloudflare/WAF/captcha/fingerprint detection, stealth regressions, IP bans, proxy support, geo/locale mismatch. | 76 | 37 |
| `access-auth-session` | Authenticated sessions and logins | `access-blocking` | Login persistence, cookies/storage state, real logged-in profile reuse, 2FA/MFA, long-lived sessions, credential handling for portals. | 32 | 15 |
| `reliability-correctness` | Reliability and correctness | — | Parent: runs that hang, crash or produce wrong/incomplete results. | — | — |
| `rel-hang-crash` | Hangs, loops and crashes | `reliability-correctness` | Infinite loops, deadlocks, stuck jobs, closed targets, page/browser crashes, hosted-browser outages. | 14 | 11 |
| `rel-incorrect-incomplete` | Duplicate, missing or incomplete data | `reliability-correctness` | Duplicate requests/processing, lost items, partial crawls, nondeterministic output, content missing from rendered pages. | 11 | 9 |
| `silent-failure` | Silent failure | — | Parent: failures that are not surfaced as failures. | — | — |
| `silent-false-success` | Silent empty results or false success | `silent-failure` | Empty strings/datasets returned as success, false success toasts or 'done' claims, block reported as crash or 500, opaque errors, clicks reported ok but ignored, need for verify/assert of expected state. | 17 | 6 |
| `resume-durability` | Resume and durability | — | Parent: surviving interruption. | — | — |
| `durability-resume` | Crash-tolerant resume and checkpointing | `resume-durability` | Resumable runs after crash/interruption, checkpoint per page/step, idempotent steps so retries do not duplicate side effects, state recovery. | 5 | 3 |
| `performance-resources` | Performance and resources | — | Parent: speed and resource use of the runtime. | — | — |
| `perf-memory-concurrency` | Memory, process and concurrency limits | `performance-resources` | Memory leaks, OOM, orphaned Chrome processes, low sessions per instance, autoscaler misjudging memory, avoiding the browser (HTTP-first/JSON endpoints) for efficiency. | 15 | 10 |
| `maintenance-drift` | Maintenance and drift | — | Parent: things breaking over time. | — | — |
| `maint-site-drift` | Target-site change and selector drift | `maintenance-drift` | Redesigns breaking selectors, schema drift, constant scraper maintenance, self-healing/adaptive parsing, keeping data current. | 17 | 4 |
| `maint-upgrade-regression` | Framework upgrade regressions | `maintenance-drift` | Behaviour, performance, token or stealth regressions between tool versions; breaking API changes; dependency pin conflicts. | 12 | 3 |
| `llm-agent-cost-latency` | LLM agent cost and latency | — | Parent: cost and speed of LLM-driven browsing. | — | — |
| `agent-token-latency-cost` | Token bloat and slow per-step loops | `llm-agent-cost-latency` | Oversized snapshots/tool definitions, context exhaustion, token growth, per-step LLM latency and cost of agent runs, harness-driven cost differences. | 44 | 28 |
| `agent-deterministic-replay` | Compile/cache agent runs to deterministic code | `llm-agent-cost-latency` | Export runs as scripts, cache selectors/actions, plan once then execute, AI at dev time only, LLM called only on breakage. | 30 | 19 |
| `llm-agent-reliability` | LLM agent reliability | — | Parent: trustworthiness of agent behaviour. | — | — |
| `agent-unreliable-drift` | Agent nondeterminism and task drift | `llm-agent-reliability` | Agents drifting off-task, wrong clicks, early 'done', inaccuracy with local models, inconsistent success on real tasks. | 22 | 10 |
| `agent-human-in-loop` | Human-in-the-loop and approval gates | `llm-agent-reliability` | Pause/resume, human takeover for login/2FA/captcha, per-action or side-effect approvals (submit, pay). | 16 | 5 |
| `developer-experience` | Developer experience | — | Parent: setup, docs and debugging. | — | — |
| `dx-setup-docs` | Setup, configuration and docs friction | `developer-experience` | Install failures, first-run breakage, brittle config, unclear docs/positioning, support gaps, learning curve. | 18 | 7 |
| `dx-observability` | Observability, logs and replay | `developer-experience` | Tracing, per-step logs, token/cost logging, screenshots/recordings, audit events, health monitoring, actionable diagnostics. | 18 | 7 |
| `self-host-licensing` | Self-host and licensing | — | Parent: running it yourself. | — | — |
| `selfhost-parity-lockin` | Self-host parity and cloud lock-in | `self-host-licensing` | Self-hosted version missing features (anti-bot, live view, cache), OSS as funnel to paid cloud, deployment artifacts (Docker, Helm), licensing (AGPL). | 20 | 9 |
| `compliance-legal` | Compliance and legal | — | Parent: politeness, legal, security and privacy obligations. | — | — |
| `compliance-site-owner-politeness` | Site-owner backlash and polite crawling | `compliance-legal` | robots.txt, throttling defaults, identifiable UA, AI-crawler blocking defaults, pay-per-crawl, vendor blocklists, bot-traffic hostility. | 25 | 17 |
| `compliance-security-privacy` | Security, privacy and regulatory requirements | `compliance-legal` | Credential exposure to LLMs, supply-chain risk, PHI/HIPAA, SOC 2/GDPR/EU AI Act, ToS/account bans, unauthorised access, data provenance and retention. | 35 | 18 |
| `integration-interfaces` | Integration and interfaces | — | Parent: how the tool plugs into other systems. | — | — |
| `integ-mcp-agent-tooling` | MCP, CLI and agent-host integration | `integration-interfaces` | MCP servers, CLI vs MCP, agent skills, Claude Code/Cursor usage, stdio/context-corruption issues. | 37 | 19 |
| `integ-sdk-llm-runtime` | SDK, runtime and bring-your-own LLM/browser | `integration-interfaces` | Language SDKs, edge/serverless runtimes, attaching to existing Playwright/browser, custom or local LLM providers, job APIs. | 21 | 15 |
| `data-quality-extraction` | Data quality and extraction | — | Parent: quality of what is extracted. | — | — |
| `data-extraction-quality` | Extraction accuracy and quality measurement | `data-quality-extraction` | LLM hallucinated fields/URLs, structure fidelity, quality evals/recall, verifiable benchmarks, correct change detection, network/XHR capture. | 19 | 11 |
| `scale-ops` | Scale and operations | — | Parent: running at volume. | — | — |
| `ops-scale-ratelimit-scheduling` | Rate limiting, scheduling and orchestration | `scale-ops` | 429 backoff, distributed rate limiting, autoscaling, scheduled/recurring jobs, Kubernetes/orchestration, large-crawl operations. | 20 | 12 |
| `new-use-case` | Use cases | — | Parent: jobs customers want done. | — | — |
| `usecase-portal-workflow` | B2B portal and form workflows | `new-use-case` | Logged-in supplier/insurance/government/ERP portals, invoice and PDF retrieval, form filling, procurement. | 22 | 15 |
| `usecase-monitoring-leadgen` | Price/change monitoring and lead generation | `new-use-case` | Competitor price and listing monitoring, change alerts, Google Maps/LinkedIn lead lists, GEO/AI-visibility tracking. | 26 | 15 |
| `usecase-qa-research-ingest` | QA testing, research agents and RAG ingestion | `new-use-case` | AI-driven QA/regression tests, research agents with search+scrape, website/knowledge ingestion for RAG. | 24 | 13 |
| `pricing-wtp` | Pricing, willingness to pay and market | — | Parent: what buyers pay, how they buy, and market size. | — | — |
| `wtp-price-points` | Stated budgets and price points | `pricing-wtp` | Explicit budgets, per-unit price expectations, plan prices, willingness or refusal to pay. | 46 | 26 |
| `build-vs-buy-switching` | Build vs buy and switching | `pricing-wtp` | Moves between DIY, OSS, hosted APIs and platforms, and the reasons given. | 31 | 19 |
| `market-adoption-stats` | Market size and adoption statistics | `pricing-wtp` | Survey figures, funding, usage volumes, market forecasts, web-access barrier prevalence. | 35 | 23 |

## 4. Mesure d'engagement et pondération

`engagement` est une valeur brute propre à chaque source :

| Source | Mesure |
| --- | --- |
| Issues et discussions GitHub | Réactions + commentaires (discussions : votes + commentaires) |
| HN | Points pour une story ; nombre de réponses directes pour un commentaire, car l'API n'expose pas les points des commentaires |
| Reddit | Nombre de commentaires capturés dans l'instantané (votes non visibles) |
| Avis | Nombre d'avis de la page agrégée quand l'item résume une page (par exemple `reviews-reports-8` : 969). Une revue individuelle reçoit une petite valeur ou 0 |
| Rapports, billets, offres | 0 dans la plupart des cas |

Ces valeurs ne sont pas comparables entre sources : 969 avis Trustpilot et 969 réactions GitHub ne mesurent pas la même chose. Le score pondéré de `pains-ranking.md` reste donc **ordinal** :

`W(code) = Σ_items (1 + ln(1 + engagement)) × 0,5^(âge_en_mois / 6)`

L'âge est compté en jours depuis le 2026-10-08, divisé par 30,4375. Une date `YYYY-MM` est prise au 1er du mois, ce qui la rend légèrement plus ancienne et donc moins pondérée. Le logarithme limite l'effet des agrégats d'avis sans le supprimer. Deux variantes de sensibilité sont publiées à côté : sans les items `promo` ou `vendor_affiliated`, et avec un engagement plafonné à 50.

## 5. Contrôle des sources et règles d'exclusion

**Contrôle :** 60 items vérifiés, un par un, sur trois critères : URL accessible, date de l'énoncé, fidélité du résumé.

- 52 items ont passé le contrôle (`source_checked: true`). Huit d'entre eux portent une note `check_note`, le plus souvent une réserve mineure : généralisation d'un seul auteur (`github-94`), qualificatif non établi (`demand-use-cases-4`, `demand-use-cases-15`), cause supposée par l'auteur (`github-115`), entre autres.
- **8 échecs, soit 13,3 %, tous exclus du corpus :**

| Item exclu | Motif |
| --- | --- |
| `github-4` | Résumé infidèle : « pas de correctif » est faux. L'auteur a ensuite identifié une course avec la nouvelle file de priorité par défaut et l'a contournée |
| `reddit-5`, `reddit-10`, `reddit-23`, `reddit-69`, `reddit-71` | Date : la date de l'instantané a été prise pour celle du post, alors que le post est antérieur de 4 à environ 102 jours |
| `reviews-reports-4`, `reviews-reports-6` | Date : le mois de l'avis ne peut pas être confirmé sur la page |

**Règles d'exclusion appliquées :**

1. Un échec au contrôle de source entraîne l'exclusion, même si le contenu est fidèle.
2. Un item sans date fiable est écarté dès la collecte. Les notes des collecteurs listent les items ainsi écartés : un taux d'échec Browserbase non daté, des tarifs de prestataires sans date, etc.
3. Une citation de 15 mots ou plus est retirée, sans exclure l'item. Deux citations ont été retirées (`reddit-20`, `reddit-76`).
4. Les contenus promotionnels ou affiliés à un éditeur sont **conservés et marqués**. Le classement est publié avec et sans eux. Sur 421 items, 98 sont `promo` ou `vendor_affiliated` (75 `promo`, 95 `vendor_affiliated`).
5. Aucun nom d'utilisateur ni aucune donnée personnelle n'est conservé.

**Biais résiduel identifié :** pour Reddit, 5 des 12 items contrôlés avaient une date d'instantané postérieure à la date réelle du post. Les 68 items Reddit conservés portent la date d'instantané et peuvent donc être **plus anciens qu'indiqué**, jusqu'à environ trois mois. Leur poids de récence peut être surestimé d'un facteur allant jusqu'à environ 1,5. Test de sensibilité : en divisant par 1,47 le poids de tous les items Reddit, on retrouve les mêmes dix codes de douleur en tête ; seuls les rangs 3 à 5 permutent (voir `pains-ranking.md`, §3).

## 6. Évolution du codebook

Toute modification d'un code (ajout, fusion ou nouvelle définition) entraîne une nouvelle version : recodage complet par le premier codeur, nouvel échantillon de 120 items pour le second, nouveau contrôle de 60 sources. Les classements ne peuvent pas comparer deux versions du codebook.
