# Matrice de qualification

Cette matrice garde distincts le code opérationnel, les fixtures non qualifiées et les expériences à ajouter. Le périmètre « tout » est un programme extensible ; aucune ligne future n'est présentée comme déjà testée.

| Famille | Opérationnel dans ce lab | Prochaine qualification / décision |
| --- | --- | --- |
| Acquisition HTTP | Pagination, exactitude des records, fraîcheur par seed, pool Crawlee, Website Spider, fetch natif ; redirect/gzip/chunked/grandes réponses contrôlés ; redirection loopback Spider refusée par sa politique SSRF | TLS/HTTP2, streaming sans matérialisation, Impit, 429/503, Retry-After, proxy |
| Rendu JS | Catalogue asynchrone ; conformité indépendante | SPA complexes, hydration, service workers, réseau bloqué, fallback adaptatif qualifié |
| Navigation | `domcontentloaded` + condition de contenu | back/forward, redirect, popup, nouvelle target, long polling, navigation abandonnée |
| Clics | 5/50 actions, effets ordonnés, overlay et postconditions ; batch Stagehand ; campagnes native/gardée exécutées sur 9 variantes dont cible bloquée/ambiguë | actionnabilité sur corpus indépendant, coordinates, stale reference entre garde et action, cancellation avec effet partiel |
| Saisie | fill, select, checkbox, événement input et effet serveur | press/type détaillé, IME, contenteditable, clavier/raccourcis, changement de validation |
| DOM | Extraction et comptage des nodes ; re-render avant interaction | Grands DOM paramétrés, mutation entre résolution et clic, snapshot DOM intégral, AX, références sémantiques |
| Frames/shadow | Shadow ouvert ; routes iframe disponibles | Adapter iframe same-origin puis OOPIF/cross-origin ; shadow fermé et slots |
| Média/fichiers | Route download et oracle de bytes disponibles | Adapter événement download, upload, screenshots (bytes + oracle visuel), PDF |
| Sessions | Page neuve, cookies/profile policies documentées | Contextes indépendants équivalents, login, cookies/storage, reuse/cold, fuite entre tenants |
| Concurrence | Closed loop, open loop, admission bornée, rejets visibles, plusieurs processus writers | Courbe capacité C1/4/16/64, queues, priorité, backpressure vers sink, fairness multi-domaines |
| Écriture JSON | JSONL buffered/sync, snapshot entier sync + rename | Encodeur seul vs sink seul, streaming sans matérialisation, gros records, gzip/zstd, fichiers shardés |
| Écriture SQLite | WAL FULL/NORMAL, DELETE FULL, transactions batchées, contention, integrity_check ; Node/Rust sync/Go sur le même artefact SQLite ; readers courts/longs et checkpoints vérifiés sur 108 cellules | indices/upsert, temp store/cache_size, gros corpus streaming, isolation du seul encodeur |
| Autres sinks | Interface worker réutilisable | Arrow/Parquet, PostgreSQL/COPY, DuckDB, objet/S3, compression ; mêmes contrats de commit et oracles |
| Incidents | Worker pendu/corrompu, descendants, faux succès ; crashes writer avant/après commit et pendant/après ACK ; nouveau coordinateur après claim/commit ; répétition de fautes avec writer lent | Crash pendant clic, kill browser, disque plein, corruption, perte réseau, fautes arbitraires répétées |
| Workflow durable | Prototypes HTTP Node et Rust/Tokio : acquisition/extraction, oracle, queue bornée, batch, commit, ack, leases, fencing et reprise idempotente ; 30 cellules natives et 42 cellules de reprise vérifiées | Heartbeat, outbox, admission continue, ordonnanceur multi-domaines, effets externes, noyau intégré Go, Chromium |
| IA | V3 bloquée au client modèle ; v4 primitives sans modèle | observe/act/extract, découverte/compile/replay/repair, même modèle et budgets, tokens/coût, cache frais/périmé |
| Topologie | SDK/worker/browser locaux | RTT injecté contrôlé, SDK distant vs co-localisé, batch distant, CPU/RAM identiques, cloud/locale séparés |
| Navigateurs/OS | Chrome for Testing local macOS | Linux dédié, Firefox/WebKit, Lightpanda/Hyperspeed, compatibilité et oracles inchangés |
| Métrologie | Bruts, manifests, SHA, répétitions, quantiles/effectifs, RSS ; sonde macOS CPU/footprint/I/O calibrée pour runtime/SQLite readers | Octets réseau, Linux/cgroups, stabilité thermique, intervalles appariés, lot holdout |

## Concurrents à ajouter

Priorité à **Chromey/chromiumoxide natif Rust et chromedp/rod Go** pour isoler le pilote ; **Crawlee AdaptivePlaywrightCrawler/Impit, Scrapy, Colly, Scrapling et Crawl4AI** pour acquisition/parsing/pools ; **agent-browser, Browser Use, Skyvern, Magnitude et AgentQL** pour perception/agents. Firecrawl, Browserbase, Browserless, Steel, Spider Cloud, Zyte et les fournisseurs de collecte appartiennent à des pistes de produit/topologie distinctes. Les versions, licences et raisons de priorité sont dans la [shortlist sourcée](../../research/2026-10-07-lab-competitor-shortlist.md).

Une librairie recensée dans la recherche n'est pas automatiquement installée ni benchmarkée. Ajouter un acteur selon une question architecturale précise, avec un adapter réel et un oracle, évite un catalogue de packages sans expériences comparables.

## Gates avant une décision d'architecture

1. Résultat exact et effets attendus sur un corpus défini ; unsupported et erreurs explicités.
2. Cycles de vie, actionnabilité et durabilité équivalents ou présentés comme des configurations différentes.
3. Courbes de charge et coût complet, pas seulement temps de commande ou moyenne globale.
4. Corpus de réglage puis holdout, perturbations et répétitions sur machine stable.
5. Prototype intégré acquisition → validation → sink → ack ; recovery vérifiée avant l'adoption du moteur durable.

Le prototype HTTP Node historique est décrit dans [workflow.md](workflow.md), et la comparaison native Node/Rust dans [runtime-workflow.md](runtime-workflow.md). Elle reprend **HTTP natif + queue bornée + sink batché à writer unique**. Rust/Tokio reste le candidat pour le noyau portable ; Node sert de référence et de backend de compatibilité. Le pilote Chromium Rust et les plans déterministes demandent encore une qualification propre. Les résultats de chaque campagne déterminent ses recommandations, sans transformer une préférence de langage en résultat mesuré.

[Qualification transversale exécutée et limites](../reports/integrated-2026-10-08.md), [audit des contrats](../reports/integrated-verification.json). Guards 162/162 sur ce corpus ; cela ne qualifie pas toutes les garanties d’actionnabilité.
