# Index des recherches concurrentielles et benchmarks

**Recherches du 7 octobre 2026.** Cet index réunit les notes de marché, analyses techniques, audits de benchmarks publiés et conclusions utilisés pour concevoir Automan. Les rapports contiennent leurs sources, versions/commits inspectés et limites.

**Statut des benchmarks :** les notes initiales analysent les protocoles et résultats publics. La phase suivante a construit le [lab local](../lab/README.md) et exécuté des campagnes contrôlées de crawl, primitives navigateur et stockage. Ces mesures exploratoires qualifient des configurations sur des fixtures ; elles ne constituent pas un classement de tous les acteurs recensés ni une preuve de supériorité universelle.

## 1. Parcours de lecture recommandé

1. [Cartographie commerciale](./2026-10-07-commercial-landscape.md) pour le marché et les modèles de service.
2. [Comparaison Stagehand / Crawlee / Spider](./2026-10-07-stagehand-crawlee-spider-comparison.md) pour les différences de contrats et les chevauchements.
3. [Matrice crawl, pools et workers](./2026-10-07-crawl-capability-matrix.md) et [matrice agents, plans et réparation](./2026-10-07-agent-capability-matrix.md) pour les capacités consolidées.
4. [Audit des benchmarks publics](./2026-10-07-benchmark-audit-followup.md) pour interpréter les chiffres existants.
5. [Performance et protocole de preuve](./2026-10-07-benchmark-and-runtime.md) pour les comparaisons à exécuter ensuite.

Les propositions produit actuelles se trouvent dans le [rapport consolidé](../docs/proposition-automan.md) et le [PRD](../issues/prd.md). Elles s'appuient sur ces recherches, sans transformer leurs conclusions en performances déjà démontrées.

## 2. Marché et familles de concurrents

| Rapport | Contenu |
| --- | --- |
| [Paysage commercial](./2026-10-07-commercial-landscape.md) | 64 acteurs, produits ou familles : navigateurs hébergés, agents, collecte/extraction, proxies, plateformes de données et RPA ; Browserbase, Apify, Spider, Browser Use, ScrapingBee, Firecrawl et acteurs de niche ; offres, unités de prix observées et frontières OSS/cloud |
| [Frameworks navigateur et agents](./2026-10-07-browser-frameworks.md) | Stagehand, Browser Use, Skyvern, Magnitude, AgentQL, agent-browser, recorder/replay et alternatives ; versions, architectures, licences et limites |
| [Moteurs de crawl](./2026-10-07-crawl-engines.md) | Crawlee, Spider, Scrapy, Colly, Crawl4AI, Firecrawl, Scrapling et autres moteurs ; HTTP/browser, scheduling, workers, persistance et extraction |
| [Plateformes et services](./2026-10-07-product-platform-options.md) | Console React/shadcn, Vite/Next, macOS native, daemon/SDK/MCP, local/cloud et exploitation/monétisation ; références Apify, Browserbase, Spider et Firecrawl |
| [Recherche et direction initialement consolidées](./2026-10-07-direction-automan.md) | Synthèse des investigations de marché et mécanismes retenus ; prolongée par le dossier de conception actuel |

**Périmètre :** couverture large, sans prétention d'exhaustivité mondiale ni audit de chaque option de chaque dépôt. Tous les acteurs recensés n'ont pas été approfondis au même niveau. Les fonctions d'un service commercial ne sont pas présumées présentes dans son dépôt public.

## 3. Analyses techniques détaillées

| Rapport | Questions traitées |
| --- | --- |
| [Stagehand / Crawlee / Spider : comparaison approfondie](./2026-10-07-stagehand-crawlee-spider-comparison.md) | Déterminisme/IA, HTTP adaptatif, stockage, sessions, concurrence, intégration StagehandCrawler, limites des comparaisons et recommandations |
| [Matrice crawl, ressources et workers](./2026-10-07-crawl-capability-matrix.md) | Frontier, découverte, recrawl, queues, leases, retries, pools, autoscaling, workers, sorties et licences ; approfondissements Katana et StormCrawler |
| [Matrice agents, plans et réparation](./2026-10-07-agent-capability-matrix.md) | Primitives navigateur, perception/vision, références, agents, compilation, cache, validation, réparation, CLI/MCP et frontières de licence/cloud |
| [Contrats Stagehand et alternatives](./2026-10-07-stagehand-alternatives-followup.md) | Stagehand publié 4.1.0 : appels, cache, self-heal et différences Playwright ; alternatives, replay et qualité des résultats |
| [Contrats Crawlee et Spider](./2026-10-07-crawlee-spider-contracts-followup.md) | Queues/retries/stockage TS et Python, intégration Stagehand, modes Spider, visited versus frontier, workers et agent Rust publié |
| [Moteurs, workers et navigateurs](./2026-10-07-engines-workers-browsers.md) | Tableau des langages, runtimes, processus, pilotes et navigateurs réellement utilisés |
| [Langage et architecture : niveau de consensus](./2026-10-07-language-architecture-evidence.md) | Convergences observées, absence de consensus universel, langage SDK versus moteur et topologie |
| [ADR du runtime](../docs/adr/0001-runtime-language.md) | Proposition Rust/Tokio avec SDK et CDP ; alternatives, conséquences et qualification requise |
| [ADR-002 accès, identité et stealth](../docs/adr/0002-access-identity-stealth.md) | Profils `identified` (défaut), `standard` et `stealth` complet en opt-in ; répartition open source/cloud, niveaux CAPTCHA, garde-fous et qualification |
| [Témoignages Reddit audités](./2026-10-07-reddit-evidence-followup.md) | Threads/commentaires identifiés, sessions/coûts/fiabilité ; témoignages distingués des preuves comparatives |

## 3 bis. Demande marché

| Document | Contenu |
| --- | --- |
| [Synthèse demande marché](./market/README.md) | 421 signaux publics codés (GitHub, Reddit, HN, avis, rapports 2025–2026), classement des douleurs, besoins par segment, signaux de prix, forces et ratés des concurrents ; rattachés aux décisions D-segment, D-pricing et D-feature-priority. Volonté de payer non mesurée : entretiens à mener |

## 4. Benchmarks : documents et niveau de preuve

| Document | Benchmarks ou mesures examinés | Statut dans cette recherche |
| --- | --- | --- |
| [Audit des benchmarks publics](./2026-10-07-benchmark-audit-followup.md) | Spider versus Colly, package Node `crawler` et wget ; benchmark d'agents Stagehand 3.2.1 / Browser Use / Sentinel ; benchmark de crawlers Markdown/RAG ; évaluations Stagehand | Inspection des méthodes, code et résultats publics, commits épinglés ; expression Node vérifiée et totaux JSON recalculés ; tâches de benchmark non exécutées |
| [Stagehand, CDP et latence](./2026-10-07-stagehand-cdp-latency.md) | Parcours publié de 50 actions, batch Stagehand v4 versus Playwright, RTT et placement | Lecture du résultat des mainteneurs ; un run publié, non reproduit ; aucune généralisation à toutes les tâches |
| [Performance, runtime et protocole de preuve](./2026-10-07-benchmark-and-runtime.md) | Benchmark Lightpanda ; publications sur mémoire/compilation/réparation ; BrowserGym/WebArena/WorkArena ; coût, fiabilité et métriques | Analyse de sources et proposition de protocole ; chiffres illustratifs explicitement identifiés ; aucun benchmark Automan exécuté |
| [Moteurs de crawl](./2026-10-07-crawl-engines.md) | Arguments et microbenchmarks de transports/moteurs, notamment Impit et Spider | Chiffres et configurations des sources analysés ; aucune preuve d'un gain global Automan |
| [Roadmap de qualification](../docs/roadmap-validation.md) | Suites HTTP, browser, agents, replay/dérive et incidents ; comparateurs et gates | Travail futur : définir et exécuter des essais équivalents, avec qualité, coûts complets et versions fixées |

**Interprétation centrale :** les expériences ne rendent pas toutes le même service. Un crawl HTTP, une conversion Markdown et un agent multi-étape ne se classent pas à partir d'un seul tableau de pages par seconde. Le dossier cherche à identifier les asymétries et les preuves manquantes, plutôt qu'à déclarer un vainqueur universel.

## 5. Validation et notes de contexte

| Document | Rôle |
| --- | --- |
| [Revue indépendante des spécifications](./2026-10-07-spec-review.md) | Cohérence des contrats proposés, quatre corrections intégrées ; aucune qualification du moteur |
| [Vérification du dossier](./2026-10-07-dossier-validation.md) | Liens, exemples et maquette simulée contrôlés ; périmètre et limites des checks |
| [Contexte de reprise](./handoff-framework.md) | Historique et questions de l'exploration |
| [Usages de Stagehand](./stagehand-usage.md) | Note de contexte complémentaire sur les modes d'utilisation |

## 6. Actualité des conclusions

Les premières notes de marché et de frameworks conservaient une hypothèse de petite équipe/sortie en 3–6 mois. Leur notice d'ouverture précise que cette contrainte a été retirée. Pour les choix Automan, privilégier le [dossier consolidé actuel](../docs/proposition-automan.md), l'[architecture](../docs/architecture.md) et le [PRD](../issues/prd.md). Les documents initiaux restent utiles pour les sources et le raisonnement historique.

Les données de prix, versions et offres sont datées du relevé ; une décision ultérieure doit revalider les éléments susceptibles d'avoir changé. Les compteurs `sources_reviewed` désignent des résultats de recherche demandés, avec doublons/écartés, et ne sont pas un nombre de sources indépendantes expérimentalement validées.

## 7. Proposition de laboratoire comparatif

La demande suivante privilégie un **lab de mesure avant la console** : installation isolée des concurrents, primitives, stress tests et choix du moteur/driver/placement fondés sur des preuves. Le rapport initial a été suivi d'une [implémentation exécutable](../lab/README.md). Les notes de versions/concurrents ci-dessous décrivent leur phase de recherche préalable ; les mesures et limites actuelles sont distinguées dans le lab.

| Document | Rôle |
| --- | --- |
| [Proposition du lab Automan](../docs/proposition-lab-automan.md) | Synthèse du dossier, périmètre d'installation, harness, oracles, primitives, stress, RTT, IA, métriques et expériences pour décider de l'architecture |
| [Versions et preuves du trio](./2026-10-07-lab-engine-evidence.md) | Crawlee, Spider-rs et Stagehand revérifiés ; compatibilité v3/v4, features, placement et limites des chiffres publiés |
| [Shortlist des concurrents](./2026-10-07-lab-competitor-shortlist.md) | Témoins Playwright/Puppeteer/CDP Rust/Go, crawlers, agents, navigateurs alternatifs et services ; local/cloud et licences distingués |
| [Ingénierie des tests du lab](./2026-10-07-lab-testing-engineering.md) | 26 sources primaires : oracles, conformance, isolation, charge ouverte/fermée, statistiques, fautes, reproductibilité et CI |
| [Ingénierie du stockage](./2026-10-07-lab-storage-engineering.md) | 16 sources primaires : JSON/JSONL, SQLite WAL FULL/NORMAL, batching, producteurs versus writers, sync, integrity et crashes |
| [Lab opérationnel](../lab/README.md) | Installation, backends, commandes, protocole, campagnes, outputs et limites de preuve |
| [Résultats locaux et architecture à prototyper](../lab/reports/qualification-2026-10-07.md) | Campagnes exécutées, débits vérifiés, batching/contention, échecs explicites et limites des conclusions de langage |
| [Couverture expérimentale](../lab/docs/coverage-matrix.md) | Code opérationnel versus backlog ; gates avant adoption d'une architecture |

## 8. Qualification intégrée — 8 octobre 2026

- [Contrat et méthode : workflow et SQLite commun](2026-10-08-workflow-and-shared-sqlite.md).
- [Résultats exécutés : 153 cellules qualifiées](../lab/reports/workflow-2026-10-08.md), avec Node/Rust sync/Go, reprise idempotente et comparaison du réveil du pool Crawlee.
