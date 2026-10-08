# Classement des douleurs : effectifs bruts et score pondéré

**Statut :** v1, recherche documentaire. **Date :** 2026-10-08. **Corpus :** 421 items canoniques ([corpus.jsonl](./corpus.jsonl)), codés selon le [codebook](./codebook.md).
**Ce que ce classement mesure :** la fréquence et la visibilité récente de douleurs **exprimées publiquement**. Il ne mesure ni leur prévalence chez les clients, ni l'intensité du besoin, ni la volonté de payer.

## 1. Méthode

- **n** : nombre d'items canoniques portant le code, à n'importe quelle position.
- **W** : `Σ (1 + ln(1 + engagement)) × 0,5^(âge_mois / 6)`, calculé au 2026-10-08. La demi-vie est de 6 mois. Une date `YYYY-MM` est prise au 1er du mois.
- **Sensibilité** : *W sans promo* exclut les 98 items `promo` ou `vendor_affiliated`. *W plafonné* limite l'engagement à 50, ce qui neutralise les agrégats d'avis (969, 724, 611 avis…) et les deux posts HN à plus de 350 points.
- Les 24 **codes de douleur ou de besoin** sont classés ensemble. Les 6 **codes de contexte** (cas d'usage, prix, marché) sont classés séparément (§4), parce qu'ils décrivent une demande et non une douleur.
- Les scores sont ordinaux (codebook §4). Avec un accord inter-codeurs de 83 % sur le code primaire, un écart de score de moins de 15 % environ ne permet pas de départager deux codes.

## 2. Classement des douleurs

Tri par W. Le rang selon n est donné dans la deuxième colonne pour la lecture côte à côte. Entre parenthèses, le rang dans chaque variante.

| Rang W | Rang n | Code | Libellé | n | W | n sans promo | W sans promo (rang) | W plafonné (rang) |
| ---: | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | `access-antibot-ip` | Anti-bot, captcha, fingerprint and IP blocks | 76 | 92.4 | 59 | 72.4 (1) | 88.4 (1) |
| 2 | 2 | `agent-token-latency-cost` | Token bloat and slow per-step loops | 44 | 50.0 | 28 | 21.5 (5) | 48.9 (2) |
| 3 | 3 | `integ-mcp-agent-tooling` | MCP, CLI and agent-host integration | 37 | 37.7 | 30 | 25.2 (3) | 37.3 (3) |
| 4 | 5 | `access-auth-session` | Authenticated sessions and logins | 32 | 36.9 | 20 | 20.7 (7) | 36.6 (4) |
| 5 | 7 | `cost-unpredictable-runaway` | Unpredictable or runaway spend | 28 | 36.0 | 21 | 25.9 (2) | 30.3 (5) |
| 6 | 8 | `compliance-site-owner-politeness` | Site-owner backlash and polite crawling | 25 | 29.3 | 19 | 25.2 (4) | 26.2 (7) |
| 7 | 4 | `compliance-security-privacy` | Security, privacy and regulatory requirements | 35 | 28.0 | 22 | 19.3 (8) | 26.7 (6) |
| 8 | 16 | `dx-setup-docs` | Setup, configuration and docs friction | 18 | 24.6 | 16 | 20.8 (6) | 19.8 (11) |
| 9 | 6 | `agent-deterministic-replay` | Compile/cache agent runs to deterministic code | 30 | 24.0 | 22 | 15.0 (13) | 23.5 (8) |
| 10 | 18 | `silent-false-success` | Silent empty results or false success | 17 | 23.1 | 14 | 18.5 (9) | 23.1 (9) |
| 11 | 9 | `agent-unreliable-drift` | Agent nondeterminism and task drift | 22 | 20.8 | 16 | 15.6 (11) | 20.8 (10) |
| 12 | 10 | `integ-sdk-llm-runtime` | SDK, runtime and bring-your-own LLM/browser | 21 | 19.7 | 17 | 13.9 (14) | 19.7 (12) |
| 13 | 13 | `cost-infra-proxy-spend` | Proxy/captcha/infra spend growth | 19 | 18.4 | 12 | 12.6 (18) | 17.1 (14) |
| 14 | 14 | `data-extraction-quality` | Extraction accuracy and quality measurement | 19 | 17.8 | 14 | 13.1 (15) | 17.8 (13) |
| 15 | 11 | `ops-scale-ratelimit-scheduling` | Rate limiting, scheduling and orchestration | 20 | 17.7 | 17 | 16.0 (10) | 16.7 (15) |
| 16 | 21 | `rel-hang-crash` | Hangs, loops and crashes | 14 | 16.3 | 13 | 15.0 (12) | 16.3 (16) |
| 17 | 12 | `selfhost-parity-lockin` | Self-host parity and cloud lock-in | 20 | 16.1 | 18 | 12.5 (19) | 16.1 (17) |
| 18 | 19 | `agent-human-in-loop` | Human-in-the-loop and approval gates | 16 | 15.0 | 13 | 12.8 (17) | 15.0 (18) |
| 19 | 15 | `dx-observability` | Observability, logs and replay | 18 | 14.2 | 16 | 13.1 (16) | 14.2 (19) |
| 20 | 17 | `maint-site-drift` | Target-site change and selector drift | 17 | 13.1 | 13 | 10.8 (21) | 13.1 (20) |
| 21 | 22 | `maint-upgrade-regression` | Framework upgrade regressions | 12 | 12.5 | 12 | 12.5 (20) | 12.5 (21) |
| 22 | 20 | `perf-memory-concurrency` | Memory, process and concurrency limits | 15 | 11.5 | 11 | 7.3 (23) | 11.5 (22) |
| 23 | 23 | `rel-incorrect-incomplete` | Duplicate, missing or incomplete data | 11 | 10.5 | 10 | 9.7 (22) | 9.6 (23) |
| 24 | 24 | `durability-resume` | Crash-tolerant resume and checkpointing | 5 | 6.7 | 5 | 6.7 (24) | 6.7 (24) |

**Les deux classements côte à côte (top 10) :**

| Rang | Par effectif brut (n) | Par score pondéré (W) |
| ---: | --- | --- |
| 1 | `access-antibot-ip` (76) | `access-antibot-ip` (92.4) |
| 2 | `agent-token-latency-cost` (44) | `agent-token-latency-cost` (50.0) |
| 3 | `integ-mcp-agent-tooling` (37) | `integ-mcp-agent-tooling` (37.7) |
| 4 | `compliance-security-privacy` (35) | `access-auth-session` (36.9) |
| 5 | `access-auth-session` (32) | `cost-unpredictable-runaway` (36.0) |
| 6 | `agent-deterministic-replay` (30) | `compliance-site-owner-politeness` (29.3) |
| 7 | `cost-unpredictable-runaway` (28) | `compliance-security-privacy` (28.0) |
| 8 | `compliance-site-owner-politeness` (25) | `dx-setup-docs` (24.6) |
| 9 | `agent-unreliable-drift` (22) | `agent-deterministic-replay` (24.0) |
| 10 | `integ-sdk-llm-runtime` (21) | `silent-false-success` (23.1) |

## 3. Robustesse

- **Stable dans les quatre variantes.** `access-antibot-ip` reste premier, avec un score supérieur à 1,8 fois celui du deuxième dans toutes les variantes. `integ-mcp-agent-tooling` reste troisième. `cost-unpredictable-runaway` et `compliance-site-owner-politeness` restent dans le top 7.
- **Gonflé par des sources commerciales.** `agent-token-latency-cost` passe du 2e au 5e rang sans les items promo ou affiliés : le lancement de Stagehand v4 (`hn-86`), le benchmark Browserbase (`hn-89`) et Kuri (`hn-53`) pèsent lourd. `agent-deterministic-replay` passe du 9e au 13e rang (Intuned, `demand-use-cases-26`). La douleur reste documentée par des sources indépendantes (§5), mais son poids relatif est surestimé.
- **Gonflé par les agrégats d'avis.** 68 % du poids de `dx-setup-docs` et 57 % de celui de `cost-unpredictable-runaway` viennent de pages d'avis ou de billets dont l'engagement dépasse 100, c'est-à-dire des comptes d'avis. Avec le plafond à 50, `dx-setup-docs` descend du 8e au 11e rang. `cost-unpredictable-runaway`, lui, monte au 2e rang sans promo : son signal ne dépend pas des agrégats.
- **Sous-estimés par le comptage brut.** `silent-false-success` (rang n 18, rang W 10) et `rel-hang-crash` (21 → 16) sont récents et concentrés sur 2026. Ils restent 9e et 12e sans promo.
- **Biais de date Reddit** (codebook §5). En divisant par 1,47 le poids de chaque item Reddit, on garde les mêmes dix codes en tête. Seuls les rangs 3 à 5 permutent : `cost-unpredictable-runaway` passe 3e, devant `integ-mcp-agent-tooling` et `access-auth-session`.
- **Biais de source.** 116 items GitHub viennent d'outils OSS à tracker public, comme Browser Use, Firecrawl ou Crawl4AI. Les services fermés (Bright Data, Zyte, Oxylabs) n'apparaissent qu'à travers des avis. `rel-hang-crash`, `maint-upgrade-regression` et `selfhost-parity-lockin` sont donc surreprésentés pour les produits OSS.

## 4. Codes de contexte (demande, prix, marché)

| Rang W | Rang n | Code | n | W | n sans promo | W sans promo (rang) | W plafonné (rang) |
| ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 2 | `market-adoption-stats` | 35 | 34.3 | 12 | 15.1 (5) | 31.5 (3) |
| 2 | 1 | `wtp-price-points` | 46 | 33.3 | 38 | 29.5 (1) | 33.3 (1) |
| 3 | 4 | `usecase-monitoring-leadgen` | 26 | 32.7 | 22 | 28.2 (2) | 32.7 (2) |
| 4 | 3 | `build-vs-buy-switching` | 31 | 29.1 | 19 | 22.8 (3) | 27.8 (4) |
| 5 | 5 | `usecase-qa-research-ingest` | 24 | 17.5 | 22 | 16.8 (4) | 17.5 (5) |
| 6 | 6 | `usecase-portal-workflow` | 22 | 17.4 | 14 | 10.9 (6) | 16.9 (6) |

`market-adoption-stats` est à 66 % promo ou affilié (23 sur 35) : il s'agit surtout de rapports d'éditeurs. Les cas d'usage `usecase-portal-workflow` ne résistent pas au retrait des sources promo (rang 6 sur 6) : le portail B2B est porté par des billets d'éditeurs (Anchor, TinyFish, Thinslices, Intuned) et par des offres de mission, pas par des plaintes d'utilisateurs. Voir [segments-and-needs.md](./segments-and-needs.md).

## 5. Items représentatifs par douleur principale

Pour chaque code de l'union des deux top 10, de 2 à 4 items sont présentés. Ce sont les items de plus fort poids W, en privilégiant ceux qui ne sont pas promo. `[P]` signale un item promo ou affilié.

### 5.1 `access-antibot-ip` (n 76, W 92.4)
- [`reddit-9`](https://reddit.sentinel-team.org/posts/1vuykgj/snapshots/2026-08-27T00%3A24%3A44.18942Z), 2026-08-27, r/LocalLLaMA. Selon un commentateur, ce n'est pas le mode headless qui déclenche la détection, mais les drapeaux d'automatisation et le profil vierge de Chromium lancé par Playwright. S'attacher en CDP au vrai Chrome de l'utilisateur fonctionne.
- [`reddit-27`](https://reddit.sentinel-team.org/posts/1v7m7q3/snapshots/2026-07-30T03%3A43%3A11.089892Z), 2026-07-30. Réutiliser le navigateur déjà connecté a supprimé plus de blocages que des semaines de travail sur l'empreinte.
- [`reviews-reports-11`](https://www.outflo.io/sales-tools/brightdata), 2026-09. Même chez Bright Data (G2 4,7 sur 348 avis, Capterra 4,7 sur 68), des sites cibles continuent de bloquer ou d'échouer.
- [`reviews-reports-43`](https://www.zyte.com/sowa/2026/access/barrier-stacking/), 2026-06-22 `[P]`. Le WAF seul (31 %) et WAF + JavaScript (environ 20 %) dominent ; la pile complète de barrières ne concerne que 3,3 % des sites.

### 5.2 `agent-token-latency-cost` (n 44, W 50.0)
- [`github-106`](https://github.com/microsoft/playwright-mcp/issues/889), 2025-08-14. Les tokens par tâche ont été multipliés par 6 environ entre Playwright MCP 0.0.30 et 0.0.32 (engagement 28).
- [`github-22`](https://github.com/browser-use/browser-use/issues/3768), 2025-12-15. Mêmes 20 tâches, environ deux fois plus de tokens après la mise à jour de Browser Use 0.8.1 vers 0.11.1.
- [`hn-98`](https://news.ycombinator.com/item?id=46942341), 2026-02-09. Un seul serveur Playwright MCP coûte environ 11,7 k tokens de définitions d'outils à chaque requête.
- [`hn-89`](https://news.ycombinator.com/item?id=49833761), 2026-09-24 `[P]`. Benchmark Browserbase : le même modèle obtient 74 % à 1,50 $ par tâche avec un harnais, contre 71 % à environ 10 $ avec un autre.

### 5.3 `integ-mcp-agent-tooling` (n 37, W 37.7)
- [`hn-50`](https://news.ycombinator.com/item?id=47392002), 2026-03-15 (14 réponses). Pour ce commentateur, MCP gaspille du contexte en permanence et les utilisateurs avancés pilotent Chromium via Playwright CLI. Le fil débat longuement de MCP contre CLI.
- [`hn-51`](https://news.ycombinator.com/item?id=47392518), 2026-03-15. Cas où MCP garde l'avantage : intégrations sans installation, utilisateurs non développeurs, OAuth multi-tenant, isolement des identifiants, journaux d'audit.
- [`github-24`](https://github.com/browser-use/browser-use/issues/4742), 2026-04-25. Depuis Claude Code, une capture d'écran dans un résultat d'outil corrompt la conversation, et chaque tour suivant échoue en 400.
- [`demand-use-cases-34`](https://news.ycombinator.com/item?id=46616481), 2026-01-14 `[P]` (134 points). CLI construite parce que les serveurs MCP navigateur testés ne reproduisaient pas une tâche SSO intranet.

### 5.4 `access-auth-session` (n 32, W 36.9)
- [`github-19`](https://github.com/browser-use/browser-use/issues/1520), 2025-04-29 (46). Chrome 136+ bloque le contrôle CDP sur le profil par défaut, ce qui casse le flux « mon vrai Chrome connecté ».
- [`github-43`](https://github.com/browserbase/stagehand/issues/1250), 2025-11-09. Stagehand v3 : `userDataDir` est sans effet et `storageState()` a disparu, donc impossible de persister une connexion.
- [`hn-34`](https://news.ycombinator.com/item?id=45452473), 2025-10-02. Besoin d'une connexion humaine suivie d'une passation à l'agent, avec des sessions de plusieurs mois et des alertes de reconnexion. Aucune plateforme ne le permet simplement à un coût acceptable.
- [`demand-use-cases-38`](https://github.com/microsoft/playwright-mcp/issues/922), 2025-08-20. Résolution des identifiants côté MCP pour que le LLM ne voie jamais les vrais mots de passe.

### 5.5 `cost-unpredictable-runaway` (n 28, W 36.0)
- [`reviews-reports-23`](https://www.capterra.co.nz/software/180165/zyte), 2024-11-10. En pay-as-you-go, aucun plafond de dépense sans abonnement : facture environ 40 fois supérieure à l'attendu.
- [`reddit-57`](https://reddit.sentinel-team.org/posts/1u9t2m9/snapshots/2026-06-19T20%3A07%3A29.810032Z), 2026-06-19. Un agent a relancé environ 200 fois un appel Firecrawl en timeout pendant la nuit, pour 40 $. L'utilisateur demande des budgets par tâche.
- [`reviews-reports-10`](https://thunderbit.com/blog/brightdata-review-costs-alternatives), 2026-04-20 `[P]`. Web Unlocker facture les échecs dès que des en-têtes ou cookies personnalisés sont activés.
- [`hn-76`](https://news.ycombinator.com/item?id=44834439), 2025-08-08. Objection à payer pour un résultat d'IA qui peut échouer ; l'éditeur envisage de ne pas facturer les échecs dus à la plateforme.

### 5.6 `compliance-site-owner-politeness` (n 25, W 29.3)
- [`hn-125`](https://news.ycombinator.com/item?id=49211386), 2026-08-07 (462 points). Un site de 1,5 M de pages rapporte environ 99 % de trafic robot.
- [`github-1`](https://github.com/scrapy/scrapy/issues/6597), 2025-01-01 (49). 38 000 requêtes en 17 minutes depuis une configuration Scrapy par défaut ; demande de valeurs par défaut polies.
- [`demand-use-cases-31`](https://news.ycombinator.com/item?id=45459596), 2025-10-03. Un opérateur bloque tout Browserbase après l'abus d'un seul client : les IP partagées d'un navigateur cloud sont bloquées en bloc.
- [`hn-62`](https://news.ycombinator.com/item?id=45374976), 2025-09-25 `[P]`. Une startup envisage de quitter Firecrawl pour maîtriser l'étiquette de crawl (gestion de robots.txt jugée incohérente).

### 5.7 `compliance-security-privacy` (n 35, W 28.0)
- [`github-33`](https://github.com/browser-use/browser-use/issues/4505), 2026-03-24. Le script d'installation de la CLI a tiré une version compromise de litellm (porte dérobée de vol d'identifiants).
- [`demand-use-cases-10`](https://www.upwork.com/freelance-jobs/apply/Python-Playwright-Developer-for-Secure-Browser-Automation-500-PDF-Downloads_~022104387142669699637/), 2026-09-28. Téléchargement d'environ 3 500 PDF clients : exécution locale imposée pour la confidentialité, reprise après interruption, NDA.
- [`hn-35`](https://news.ycombinator.com/item?id=45458977), 2025-10-03. Les clients refusent le stockage des identifiants ; automatiser la 2FA en annule le principe.
- [`demand-use-cases-39`](https://github.com/browser-use/browser-use/issues/4574), 2026-03-31. Supervision humaine, conservation des enregistrements et journaux d'audit pour l'EU AI Act.

### 5.8 `dx-setup-docs` (n 18, W 24.6, poids porté par les agrégats d'avis)
- [`github-13`](https://github.com/browser-use/browser-use/issues/839), 2025-02-23 (55) et [`github-14`](https://github.com/browser-use/browser-use/issues/1020), 2025-03-13 (51). Premier lancement : Chromium reste sur `about:blank` et l'agent boucle à l'étape 1.
- [`reviews-reports-3`](https://ai.g2.com/product/apify), 2026-05. Synthèse G2 (611 avis) : construire un Actor sur mesure demande un long apprentissage.
- [`hn-17`](https://news.ycombinator.com/item?id=46586819), 2026-01-12. La documentation ne dit pas à quoi Lightpanda peut servir sans risque comme moteur Playwright.

### 5.9 `agent-deterministic-replay` (n 30, W 24.0)
- [`github-39`](https://github.com/browserbase/stagehand/issues/389), 2025-01-09 (30). `extract()` devrait renvoyer son plan statique pour éviter un appel LLM par page.
- [`hn-87`](https://news.ycombinator.com/item?id=49763632), 2026-09-19. Le cache d'actions de Stagehand est lié aux sessions Browserbase et ne sert à rien avec un navigateur local. L'utilisateur le voulait en fichier versionné pour la CI.
- [`reddit-24`](https://reddit.sentinel-team.org/posts/1ve9gkj/snapshots/2026-08-07T06%3A10%3A44.151998Z), 2026-08-07. Le consensus du fil : résoudre une fois, cacher un contrat de workflow avec empreinte DOM, rejouer, et n'appeler le LLM qu'en cas de casse.
- [`demand-use-cases-19`](https://www.upwork.com/freelance-jobs/apply/Powered-Test-Agent-Playwright-LLM_~022097330742604282149/), 2026-09-08. Offre de mission : un agent QA dont les runs réussis émettent des tests Playwright déterministes pour la CI.

### 5.10 `silent-false-success` (n 17, W 23.1)
- [`reddit-1`](https://reddit.sentinel-team.org/posts/1vnydav/snapshots/2026-08-15T02%3A07%3A43.063054Z), 2026-08-15. Des clics renvoient « ok » sur des éléments qui les ignorent. Le correctif proposé est de vérifier chaque action en relisant la page.
- [`github-80`](https://github.com/unclecode/crawl4ai/issues/2116), 2026-07-31. Un blocage anti-bot revient en HTTP 500 au lieu d'un résultat typé « bloqué ».
- [`github-93`](https://github.com/apify/crawlee/issues/3629), 2026-05-06. Les handlers s'exécutent sur la page de challenge Cloudflare.
- [`hn-101`](https://news.ycombinator.com/item?id=47296933), 2026-03-08 `[P]`. Après une refonte du site, les sélecteurs CSS renvoient silencieusement des chaînes vides.

### 5.11 `agent-unreliable-drift` (n 22, W 20.8)
- [`reddit-0`](https://reddit.sentinel-team.org/posts/1vnydav/snapshots/2026-08-15T02%3A07%3A43.063054Z), 2026-08-15. 40 minutes pour deux billets Ticketmaster : boucles sur captchas et modales, panier expiré deux fois.
- [`reddit-25`](https://reddit.sentinel-team.org/posts/1uhvp85/snapshots/2026-06-29T19%3A40%3A40.485144Z), 2026-06-29. Meilleur retour sur investissement sur les outils internes fixes ; fragile sur les sites publics, peu fiable sur les tâches arbitraires.
- [`github-30`](https://github.com/browser-use/browser-use/issues/1763), 2025-05-24. L'agent perd le fil d'un objectif en plusieurs étapes et déclare la tâche terminée trop tôt.

### 5.12 `integ-sdk-llm-runtime` (n 21, W 19.7)
- [`github-31`](https://github.com/browser-use/browser-use/issues/2605), 2025-08-05 (31). Les modèles locaux (gpt-oss via Ollama) échouent sur la sortie structurée.
- [`github-48`](https://github.com/browserbase/stagehand/issues/393), 2025-01-09 (33). Long fil de demandes d'un SDK Python pour Stagehand.
- [`github-38`](https://github.com/browserbase/stagehand/issues/507), 2025-02-20 (31). Pouvoir s'attacher à une page Playwright de test existante.

## 6. Douleurs peu exprimées

`durability-resume` (n 5) et `perf-memory-concurrency` (n 15, 23e sans promo) sont en bas du classement. La reprise apparaît surtout comme une **exigence de cahier des charges** (« resumable » dans `demand-use-cases-1` et `demand-use-cases-10`) et dans une proposition Scrapy (`github-5`), rarement comme une plainte. La mémoire n'est une douleur qu'à volume élevé (`github-104`, `reddit-60`). Un commentateur HN estime que le coût du LLM écrase celui du navigateur dans les workflows d'agents (`hn-14`). Cela ne veut pas dire que ces propriétés sont inutiles. Cela veut dire qu'elles **ne se vendent pas seules** : les acheteurs les tiennent pour acquises ou ne les découvrent qu'en production.
