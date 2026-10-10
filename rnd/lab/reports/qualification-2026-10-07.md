# Automan Lab — résultats et décisions à tester

Relevé du **7 octobre 2026**, sur Apple M3 Max, 14 CPU logiques, 36 GiB de RAM, macOS 27 arm64. Le laboratoire est installé et exécutable. Crawlee, Spider Rust, Stagehand v4, son batch, l'intégration **`@crawlee/stagehand` avec Stagehand v3**, Playwright et Puppeteer ont été lancés localement. Les écritures JSON/JSONL et SQLite ont des workers Node et Rust indépendants.

Les premiers résultats soutiennent trois directions : **éviter le browser lorsqu'HTTP suffit ; réutiliser les ressources ; batcher les écritures derrière une admission bornée**. Ils permettent de proposer le prochain prototype Automan, sans encore choisir son langage sur la seule performance ni annoncer un scraper universellement supérieur.

## Preuves et reproduction

Les rapports HTML, manifests et échantillons bruts retenus sont regroupés dans l'[index des expériences](index.html). Une copie de ces preuves se trouve dans `evidence/` ; les bases/fichiers de stockage complets restent dans les dossiers locaux `lab/results/` indiqués par l'index. Les tentatives intermédiaires de construction sont conservées, mais ne sont pas utilisées pour les tableaux ci-dessous.

Versions exécutées : **Node 25.5.0, pnpm 12.9.1, Rust 1.93.0, Crawlee 3.18.2, Spider 2.53.9, Playwright 1.63.0, Puppeteer 25.12.0, Stagehand 4.1.0 et 3.7.3, SQLx 0.8.6**. Chrome for Testing **153.0.8010.12** est fourni explicitement aux adapters. Locks, SHA des sources, navigateur et exécutables sont enregistrés. Aucun changement de source n'a été détecté pendant les expériences retenues.

```sh
cd /Users/jb/Dev/automan/lab
pnpm install --frozen-lockfile
pnpm bootstrap
pnpm run doctor
pnpm test
pnpm check
pnpm integration
pnpm bench campaigns/qualification.json
pnpm bench campaigns/http-lifecycle.json
pnpm bench campaigns/arrival-stress.json
pnpm bench campaigns/browser-primitives.json
pnpm storage campaigns/storage-smoke.json
pnpm storage campaigns/storage-contention.json
pnpm storage campaigns/storage-process-crash.json
```

Exécuter les campagnes séparément, sur une machine disponible. Les chiffres restent exploratoires : le corpus est petit, le réseau local, l'état thermique et les applications du laptop ne sont pas contrôlés comme sur une machine réservée.

## Qualification des moteurs

La campagne commune a enregistré **270 tentatives : 245 vérifiées, 23 unsupported, 2 en échec**. Elle teste catalogue HTTP, catalogue JS, formulaire, 50 clics et shadow ouvert ; normal, retard, overlay et remplacement DOM passent d'abord le gate de conformité. Les 35 cellules conformes ont ensuite chacune un warmup et deux mesures. Ce petit nombre sert surtout à détecter les problèmes d'intégration.

**Spider Chrome et Spider Smart ont échoué sur le catalogue JavaScript** : les six records attendus sont absents. Les mesures de ces cellules sont bloquées. Leurs résultats HTTP passent. C'est une non-qualification de ces configurations/adapters sur cette fixture ; une enquête ciblée reste nécessaire avant d'attribuer la cause à la bibliothèque, au browser ou à la configuration de récupération du DOM. Il n'y a pas de fallback silencieux qui convertirait l'échec en succès.

Les autres capacités absentes sont déclarées : HTTP seul ne traite pas formulaires/shadow ; Spider est ici un adapter de collecte, sans implémentation des actions ; Puppeteer shadow n'est pas implémenté dans cet adapter. Une capacité absente de l'adapter ne constitue pas une affirmation d'incapacité générale du projet upstream.

Les pages interactives mesurent leur **viewport réel 1280×720** et le lab le vérifie. Le site de test conserve les effets des interactions et l'oracle contrôle leur ordre, leurs valeurs et les événements trusted. Les selectors sont connus : découverte et raisonnement IA ne sont pas dans cette série.

## HTTP : cycles de vie et charge

Catalogue de **16 pages / 32 records par tâche**, délai serveur de 2 ms/page, concurrence de quatre tâches, trois blocs de 24 mesures. Tous les backends ont vérifié **72/72 tâches**. Le débit concerne les catalogues complets vérifiés, et non les pages.

| Configuration | Tâches vérifiées/s | p50 tâche, ms | p95 tâche, ms |
| --- | ---: | ---: | ---: |
| Fetch Node + parser spécialisé | 88,7 | 44,0 | 50,9 |
| Spider HTTP, Website par tâche | 92,1 | 42,7 | 45,8 |
| Crawlee HTTP, crawler neuf par tâche | 18,3 | 193,8 | 413,2 |
| Crawlee HTTP, crawler/queue partagés par bloc | 25,4 | 81,8 | 542,1 |

Source : [rapport HTTP](evidence/http-lifecycle-2026-10-07T21-29-31-257Z-2b0754/report.html), [bruts](evidence/http-lifecycle-2026-10-07T21-29-31-257Z-2b0754/attempts.jsonl). Les backends minimalistes font moins de travail d'orchestration et n'ont pas le même parser. Le faible écart Node/Spider ici ne démontre aucun avantage intrinsèque de langage. Le gain du pool Crawlee accompagne une queue partagée et une concurrence interne différente ; ce n'est pas une isolation du seul coût d'instanciation. Sa queue idle présente aussi une latence élevée : le run C1 de qualification prenait environ 500 ms pour un petit catalogue. Les réglages de réveil du pool méritent une expérience distincte.

Le stress à arrivées indépendantes offre **20 tâches/s**, avec catalogue de huit pages, délai de 10 ms/page et quatre tâches actives au maximum. Deux blocs proposent chacun 40 tâches. Node et Spider HTTP vérifient **80/80** ; Crawlee neuf vérifie **72/80** et rejette huit arrivées faute de capacité, sans erreur sur les tâches admises. Le débit vérifié, drain inclus, est respectivement 19,60 / 19,56 / 16,91 tâches/s. Source : [rapport de charge ouverte](evidence/arrival-stress-2026-10-07T21-31-09-975Z-ba6970/report.html).

Les **rejets restent au dénominateur**. Le générateur ne ralentit pas artificiellement pour attendre un backend occupé. Les arrivées concernent des tâches complètes ; aucune queue illimitée n'absorbe discrètement la surcharge. Ces deux points de charge ne constituent pas encore une courbe de capacité ni un test prolongé.

## Primitives navigateur

La campagne répétée utilise trois blocs, cinq mesures par scénario et backend, des séquences de 50 clics, les mutations de conformité et le même viewport réel. Elle vérifie **690 tâches sur toutes les phases, dont 345 mesures**, et déclare trois cellules shadow Puppeteer unsupported. Les mesures contiennent **4 500 clics contrôlés**, et toutes les phases en contiennent 9 000. Aucun échec n'a été observé dans cette campagne.

| Backend, 15 mesures par cellule | p50 catalogue JS, ms | p50 formulaire, ms | p50 tâche de 50 clics, ms |
| --- | ---: | ---: | ---: |
| Playwright | 149,0 | 346,6 | 2 714,6 |
| Puppeteer | 74,6 | 298,8 | 3 449,2 |
| Stagehand v4 | 138,9 | 213,5 | 945,5 |
| Stagehand v4 batch | 130,9 | 215,8 | 939,4 |
| Crawlee + Playwright | 862,8 | 975,3 | 3 317,8 |
| Crawlee + Stagehand v3 | 882,9 | 765,0 | 3 209,3 |

Source : [rapport navigateur](evidence/browser-primitives-2026-10-07T21-33-08-604Z-f6b1a0/report.html), [bruts](evidence/browser-primitives-2026-10-07T21-33-08-604Z-f6b1a0/attempts.jsonl). Le temps commence au lancement de la tâche et inclut navigation, contrôles, actions, RPC et oracle ; il ne mesure pas le clic isolé. Les variantes se succèdent selon le même cycle prédéfini. Avec 15 valeurs, le p95 empirique utilisé par le rapport est le maximum observé : ce n'est pas une estimation fiable d'une queue de distribution en production.

Stagehand v4 est plus rapide que Playwright sur cette séquence locale de clics. **Le batch n'apporte pas de gain net supplémentaire démontré ici** : les médianes sont proches et leurs variations ne donnent pas une preuve statistique. Les autres opérations montrent aussi que la vitesse dépend de la tâche ; aucun classement global ne doit remplacer ces cellules.

Stagehand v4 utilise ses primitives Page/Locator sans modèle ; son batch ne s'applique qu'aux clics. `@crawlee/stagehand` est une piste **v3**, avec browser/crawler neuf par tâche et Page Playwright. Elle reste distincte de v4. Un client modèle qui échoue à tout appel est utilisé dans cette piste pour détecter une inférence accidentelle.

Les garanties natives restent différentes. Playwright effectue notamment ses contrôles d'actionnabilité ; les postconditions communes du lab prouvent ces fixtures, sans rendre les SDK universellement équivalents. Stagehand v4 utilise une extension près du browser ; Playwright emploie déjà CDP pour Chromium. La question utile porte donc sur **placement, trajets RPC et garanties**, pas sur la présence du seul acronyme CDP. [Code Chromium Playwright](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/chromium/crBrowser.ts), [migration Stagehand v4](https://docs.stagehand.dev/v4/migrations/playwright).

Le résultat publié Stagehand v4 sur Wikipedia avec RTT distant ne constitue pas une prédiction pour ce lab local. Notre série ne reproduit ni son corpus ni sa topologie. Injecter des RTT contrôlées et comparer SDK distant, worker co-localisé et extension sera nécessaire pour qualifier le bénéfice du batch. Voir l'[audit sourcé](../../research/2026-10-07-lab-engine-evidence.md).

## Écriture : contrats, batching et concurrence

**80/80 cellules de contention passent**, avec 96 960 records relus et vérifiés. Chaque cellule a un sink neuf ; huit producteurs alimentent un ou quatre **vrais processus écrivains** partageant le même fichier/base. Deux blocs, 24 batches par bloc, payload de 512 bytes, batches de 1 ou 100 records. JSONL buffered, JSONL fsync, SQLite WAL FULL/NORMAL et DELETE FULL restent des contrats distincts.

**Les SQLite exécutés diffèrent : Node 3.53.4, Rust 3.46.0.** Node utilise DatabaseSync ; Rust SQLx utilise son thread/driver asynchrone et une connexion par writer. Ces différences empêchent d'attribuer les chiffres au seul langage. Les données, transactions, valeurs effectives des PRAGMAs et options de compilation sont enregistrées.

| Pipeline, un writer | Batch 1, records/s | Batch 100, records/s |
| --- | ---: | ---: |
| Node JSONL buffered | 5 990 | 312 830 |
| Rust JSONL buffered | 13 769 | 919 313 |
| Node JSONL fsync/batch | 231 | 21 647 |
| Rust JSONL fsync/batch | 243 | 22 950 |
| Node SQLite WAL FULL | 5 337 | 164 624 |
| Rust SQLite WAL FULL | 6 396 | 76 490 |
| Node SQLite WAL NORMAL | 7 326 | 194 998 |
| Rust SQLite WAL NORMAL | 8 767 | 84 617 |
| Node SQLite DELETE FULL | 2 531 | 123 827 |
| Rust SQLite DELETE FULL | 2 248 | 66 887 |

Source : [rapport stockage](evidence/storage-contention-2026-10-07T21-30-24-424Z-6bec59/report.html), [détails et oracles](evidence/storage-contention-2026-10-07T21-30-24-424Z-6bec59/summary.json). Débits agrégés par records vérifiés / somme des fenêtres d'écriture. Les fenêtres sont courtes ; pas de classement statistique ni de débit soutenu démontré. Les cellules les plus grandes contiennent seulement 2 400 records : elles ne qualifient pas le comportement continu des checkpoints ou des grands fichiers. Construction des records, encodage JSON, RPC, sink et journalisation des acks sont inclus ; startup et fermeture/oracle sont séparés. La réécriture du snapshot JSON croissant ajoute encore un coût distinct. Ce ne sont pas des microbenchmarks purs d'encodeurs.

Le batching a un effet majeur dans ces configurations. **Quatre writers SQLite n'améliorent pas ces cellules** : WAL FULL batch 100 passe de 164 624 à 110 816 records/s côté Node et de 76 490 à 39 597 côté Rust. SQLite conserve un seul écrivain actif par base ; multiplier les producteurs et multiplier les connexions ne sont pas la même chose. [Documentation officielle WAL](https://www.sqlite.org/wal.html).

Le smoke stockage qualifie aussi le snapshot JSON synchronisé avec rename et sync du dossier : **44 cellules passent, 4 sont unsupported** parce que plusieurs writers remplaceraient le même document sans coordination. Les trois formats ne rendent pas les mêmes services : export séquentiel, publication d'un document complet, état transactionnel interrogeable.

Le probe crash conserve **2 048 records dans 8/8 cellules**, après SIGKILL envoyé après les acquittements. Le système et ses caches restent actifs. Cela vérifie un crash de processus, sans prouver tenue après coupure électrique ni reprise après interruption au milieu d'une transaction. Le lab n'active pas F_FULLFSYNC et n'établit pas une égalité physique des garanties fsync/SQLite sur macOS. [Recherche stockage](../../research/2026-10-07-lab-storage-engineering.md).

## Proposition d'architecture issue de ces mesures

| Couche | Proposition à prototyper | Preuve encore nécessaire |
| --- | --- | --- |
| Admission et scheduler | Queue bornée, budgets par domaine/job, annulation explicite | Fairness, priorité, saturation et propagation de la lenteur du sink |
| Acquisition | HTTP natif en premier, validation de la sortie, upgrade browser si nécessaire | HTTP2/TLS/proxy, contenu authentifié, corpus indépendant, coût des upgrades |
| Browser | Pools de browsers/pages près du worker, plan déterministe avec postconditions | Isolation entre tenants, crash CDP, frames/OOPIF, RTT et grands DOM |
| IA | Découvrir/compiler/réparer hors du rejeu normal, limites de coût et validation | Même modèle/budget, dérive sémantique, échec du cache, holdout |
| Sink | Writer unique batché ; JSONL export, SQLite WAL pour état local | Backpressure intégré, readers/checkpoints, fenêtre de commit et latence max du batch |
| Durabilité | Leases/checkpoints et écriture idempotente avant ack | Crash entre effet/commit/ack, duplication, reprise et journal d'acks |
| Interfaces | SDK/CLI/MCP pilotant le moteur ; console web et app macOS | Un vrai workflow HTTP durable commun, puis qualification navigateur |

**Rust/Tokio reste le candidat pour le noyau portable**, pas un gagnant démontré. Conserver des backends Node pour compatibilité est cohérent ; un pilote Rust/CDP devra battre une baseline Node co-localisée à garanties égales pour justifier son coût. Le sink Rust actuel SQLx n'est pas automatiquement plus rapide que Node SQLite. Ajouter une baseline Rust avec binding synchrone et **le même build SQLite**, puis Go, serait une comparaison de runtimes plus défendable.

Le prochain livrable décisif est un prototype **acquisition → extraction → validation → writer batché → commit → ack**, avec queue bornée et reprise idempotente. Ce lab mesure aujourd'hui acquisition et écriture dans des pistes séparées ; il n'implémente pas encore ce workflow durable Automan. Sa latence et sa consommation intégrées restent à démontrer avant de figer l'architecture.

## Couverture restante et concurrents

La [matrice de couverture](../docs/coverage-matrix.md) détaille les primitives futures : frames, fichiers, captures, AX/snapshots complets, session reuse/isolation, réseau dégradé, sink plein, readers/checkpoints, CPU/I/O, charge C16/64 et reprise. Des fixtures iframe/download existent déjà, sans adapters qualifiés annoncés.

Priorité de comparaison : **Chromey/chromiumoxide et chromedp/rod** pour isoler les pilotes ; **Crawlee Adaptive/Impit, Scrapy, Colly, Scrapling, Crawl4AI** pour acquisition/pools ; **agent-browser, Browser Use, Skyvern, Magnitude, AgentQL** pour agents/perception. Firecrawl, Browserbase, Browserless, Steel, Spider Cloud, Zyte et les services de collecte demandent une série de topologie/coût distincte. La [shortlist sourcée](../../research/2026-10-07-lab-competitor-shortlist.md) donne les raisons et les sources ; leur présence dans cette liste ne signifie pas qu'ils sont déjà installés.

## Vérification du laboratoire

Vérifications finales : **14/14 tests unitaires, 1/1 intégration réelle Stagehand**, `pnpm check`, les deux `cargo fmt --check` et `pnpm run doctor` passent. Doctor reçoit les réponses des 12 backends et des deux writers ; il ne remplace pas leur qualification métier. Les 108 liens locaux contrôlés existent, les 161 fichiers de preuves correspondent à leurs SHA, et l'index HTML a été rendu et son filtre stockage vérifié. Voir le [relevé de vérification](verification-2026-10-07.json).

Les tests automatisés exercent les contrats : données manquantes/dupliquées/fausses, faux succès d'un worker, corruption/hors ordre du protocole, timeout et descendants récalcitrants, saturation et deadline, reopen des sinks et dénominateurs statistiques. Une intégration réelle Stagehand vérifie clics avec overlay et dimensions de la page. Les performances n'ont pas de seuil fragile dans les tests unitaires.

Les références et prescriptions de méthode sont dans les [bonnes pratiques d'ingénierie des tests](../../research/2026-10-07-lab-testing-engineering.md) et la [recherche stockage](../../research/2026-10-07-lab-storage-engineering.md). Les scripts permettent de répéter ces expériences et d'ajouter un adapter incorrect comme contrôle négatif avant de considérer ses temps.
