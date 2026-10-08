# Automan Lab

Laboratoire local opérationnel : moteurs de crawl, primitives navigateur et stockage, avec données contrôlées et validation indépendante. Construction des 7–8 octobre 2026. Les premiers résultats qualifient des configurations sur ces fixtures ; ils ne démontrent pas une supériorité universelle ni l'architecture finale d'Automan.

Les [nouveaux résultats du workflow durable et de SQLite commun](reports/workflow-2026-10-08.md) prolongent le lab avec Node/Rust sync/Go, reprise et réveil du pool Crawlee.

La [qualification transversale exécutée](reports/integrated-2026-10-08.md) ajoute le
[workflow natif Node et Rust](docs/runtime-workflow.md), les crashes pendant/après
ACK, l'actionnabilité native ou gardée, les lecteurs/checkpoints SQLite et les
transports HTTP. Ces campagnes ne sont pas encore un choix final de noyau.

Les [résultats exécutés et décisions à tester](reports/qualification-2026-10-07.md) incluent HTTP, primitives navigateur et écritures concurrentes. L'[index HTML](reports/index.html) donne accès aux manifests, données brutes et rapports retenus.

## Décisions, claims et ledger

Le lab est piloté par le [plan de R&D](../docs/rd-plan.md). Chaque campagne sert une décision de `decisions/` : options, baseline, métrique principale du [catalogue](metrics/catalog.json), gates de contrat, effet minimal Δ, règle et classe d'hôte. Les affirmations publiques candidates sont dans `claims/`, déclarées avant mesure. Les notes éditoriales ne portent aucun chiffre mesuré : les chiffres viennent des verdicts.

```sh
pnpm lab validate   # schémas et références du registre, des claims et du ledger
pnpm lab import     # réexprime results/ en run.v2 sous ledger/imported et régénère le ledger
pnpm lab ledger     # rôle, statut, rerun et classes de validité de chaque run
```

### Moteur v2 et classes d'hôte

```sh
pnpm lab preflight local-controlled          # vérifie l'hôte : charge, mémoire, thermique, secteur, processus étrangers, sources commitées
pnpm lab run campaigns/v2/bench/http-smoke.json
```

`pnpm lab run` exécute une campagne v2 (`schemas/campaign-bench.v2.schema.json`) ; un ancien `campaigns/*.json` est converti et plafonné à `local-dev`. Le moteur enchaîne provenance (SHA git, état dirty, empreinte des sources), contrôle de l'hôte, gate de conformité, warmup, blocs mesurés et finalisation dans `results/<id>/run.v2.json` et `attempts.v2.jsonl`. Seuils des classes `local-controlled` et `reference` : `hosts/*.json`. Un hôte qui ne passe pas les gates est refusé, ou dégradé explicitement si la campagne l'autorise ; des sources non commitées plafonnent à `local-dev`.

Toutes les campagnes sont des données versionnées sous `campaigns/v2/` : `bench/` pour le moteur d'attempts, et `storage/`, `workflow/`, `runtime/`, `actionability/`, `sqlite-readers/` pour les lanes (`schemas/campaign-lane.v2.schema.json`). Chaque campagne nomme ses décisions et demande une classe d'hôte. Une lane garde son exécuteur de cellule et son oracle propres ; le moteur générique (`src/core/lanes.ts`) ajoute provenance, contrôle de l'hôte, blocs randomisés, statuts typés, `run.v2.json` et `cells.raw.jsonl` avec le détail complet de chaque cellule. Les anciens CLI (`pnpm storage`, `pnpm workflow`, `pnpm runtime`, `pnpm actionability`, `pnpm sqlite-readers`) appellent les mêmes exécuteurs et restent disponibles pour comparaison.

```sh
pnpm lab validate                                          # registre + toutes les campagnes v2
pnpm lab suite campaigns/v2/suites/qualification-current.json   # enchaîne les campagnes et compare aux comptes attendus
```

Une suite (`schemas/suite.v2.schema.json`) déclare pour chaque étape les comptes attendus ; elle échoue si un compte diffère ou si la provenance a changé pendant le run. Un run v2 neuf entre dans le ledger comme `unreviewed` jusqu'à sa classification.

### Verdicts et rapport

```sh
pnpm lab verdict            # verdicts/<décision>.json pour chaque décision dotée d'un bloc `analysis`
pnpm lab report             # reports/decisions.md, généré depuis decisions/ et verdicts/ uniquement
```

Une décision déclare dans `analysis` la lane, les facteurs qui désignent chaque option, les facteurs d'appariement et la strate principale. Pour chaque bloc apparié, le verdict calcule ln(option / baseline) de la métrique principale. Il produit ensuite un intervalle par bootstrap de blocs seedé : (1 − α) en supériorité, (1 − 2α) en équivalence et en non-infériorité. Hodges–Lehmann et Wilcoxon signé exact sont donnés en contrôle de robustesse.

L'issue est mécanique, comparée à ±ln(1 + Δ) : `better`, `worse`, `equivalent`, `trivial`, `non-inferior` ou `inconclusive`. Elle vaut `blocked-by-contract` si une gate échoue ou n'est pas encore évaluable, et `insufficient-n` si les blocs sont moins nombreux que ceux exigés par la dispersion observée (minimum cinq). Un verdict n'est éligible pour la décision que si l'hôte observé atteint la classe requise.

Méthode : le statut d'une tentative vient d'un code d'erreur typé (`src/core/errors.ts`), jamais du texte d'un message. Un timeout ne touche que la tentative bloquée : ses voisines en vol sont `aborted` (`E_SIBLING_ABORT`), hors taux d'échec, et le worker redémarre dans la limite de `maxRestarts`. Fixtures et oracle tournent dans un processus séparé (`fixtures/worker.ts`) ; `taskMs` ne couvre que l'appel `run`, et `oracleMs` est mesuré à part. Une métrique sans sonde sur l'hôte vaut `null`, avec sa raison dans `metricAvailability`, jamais 0. Les tentatives bloquées par un gate restent visibles en `skipped-gate`.

### Réseau, Linux et CI locale

```sh
pnpm lab ci     # typecheck, tests unitaires et de contrat, validation du registre et des campagnes
```

Une campagne bench peut déclarer `network: { link: "browser-origin", rttMs }`. Le moteur calibre d'abord le profil : des ping-pong d'un octet, directs et à travers le proxy, sont alternés, et le run consigne la médiane appariée du délai ajouté. Hors tolérance (le plus grand de 2 ms et 10 %), les métriques de temps et de ressources deviennent indisponibles ; les métriques de contrat restent valides. Les fixtures sont ensuite servies à travers un proxy TCP en espace utilisateur (`src/net/latency-proxy.ts`), sans privilège, sur macOS comme sur Linux. Il ajoute la moitié du RTT dans chaque sens, préserve l'ordre des octets et ne libère jamais un paquet avant son échéance. Le lien `driver-browser` arrive avec l'adapter CDP de P6 ; perte et gigue relèvent de `tc netem` sur l'hôte de référence.

Sur Linux, le CPU de l'arbre de processus vient de `/proc/<pid>/stat` et la mémoire proportionnelle (PSS anonyme + swap) de `smaps_rollup`. Les compteurs cgroup v2 demandent un scope systemd par run et restent déclarés indisponibles d'ici là. Préparation de l'hôte : [`docs/reference-host.md`](../docs/reference-host.md).

### Pilotes navigateur (D-browser-driver)

```sh
pnpm lab run campaigns/v2/bench/browser-driver-rtt-0.json          # puis -20, -100, -300 : lien driver-browser
pnpm lab run campaigns/v2/actionability/browser-actionability-extended.json
```

Deux pilotes CDP directs envoient la même séquence de commandes pour chaque opération : `node-cdp` (`adapters/cdp-node`) et `rust-cdp` (`adapters/cdp-rust`, compilé par `pnpm bootstrap`). Ils injectent les mêmes scripts de page, lus depuis `adapters/browser-scripts/` : attentes, extraction, et la garde Automan (unicité, visibilité, activation, stabilité, hit-test, shadow DOM compris). Une comparaison entre les deux isole donc le langage du protocole. Chaque tâche rapporte son nombre de commandes CDP, et un test d'intégration exige qu'il soit identique entre les deux langages.

La matrice `browser-driver-primitives` (lane `actionability`, `config.scenario: "primitives"`) vérifie dix gestes avec un adapter neuf par cellule : survol, double clic, clic droit, glisser-déposer par événements pointeur, molette, touche Entrée, `contenteditable`, saisie Unicode, cible déplacée sous le pointeur (un leurre prend sa place) et cible remplacée par un contrôle destructeur (`retarget`, qui doit être refusé sans effet). L'oracle exige un seul effet du bon type, issu d'un événement `isTrusted`, et aucun effet sur un leurre. HTML5 drag-and-drop natif et composition IME sont hors de cette matrice.

Avec `browserHost: "harness"`, le harnais lance un Chromium par pilote et par bloc, avec les mêmes flags (`src/core/browser-host.ts`). Il échantillonne ce navigateur avec l'adapter, donc le CPU et la mémoire du navigateur comptent pour tous les pilotes. Playwright et Puppeteer s'y connectent par CDP. Un profil `network: { link: "driver-browser" }` place le proxy de latence entre le pilote et le navigateur. `actionPolicies` fixe la politique de chaque pilote : `guarded` applique la garde avant chaque clic, saisie et sélection, `native` s'en remet aux garanties du pilote.

Le bench produit une cellule par pilote, scénario et bloc, ce qui permet d'apparier les blocs. Les facteurs `policy`, `rttMs` et `browserHost` y figurent. Dans une décision, `analysis.gateLanes` indique la lane où chaque gate lit ses cellules : la conformité d'actionnabilité et l'absence de faux succès viennent de la matrice, pas du bench. Une gate ne passe que sur des cellules de l'option elle-même. Chaque option analysée est comparée à la baseline, avec ses propres gates.

`ledger/classification.json` porte le jugement humain sur chaque run (rôle, raison, validité A contrat / B relatif même hôte / C absolu, décisions, rerun). L'import y ajoute les faits mécaniques : comptes, provenance, intégrité recalculée contre les catalogues SHA de `reports/`. Il est idempotent et ne modifie jamais `results/`. Les runs importés restent plafonnés à la classe d'hôte `local-dev`.

## Démarrer

Prérequis : Node >=24, pnpm **12.9.1**, Rust/Cargo et outils de compilation C/C++. La version Node exécutée **25.5.0** est indiquée dans `.node-version` ; d'autres versions demandent une requalification, notamment pour SQLite. L'environnement exécuté est consigné dans chaque manifest. Le lab utilise le stripping TypeScript natif de Node, sans serveur de développement.

```sh
cd /Users/jb/Dev/automan/lab
pnpm install --frozen-lockfile
pnpm bootstrap
pnpm check
pnpm test
pnpm integration
pnpm run doctor
pnpm bench campaigns/smoke.json
```

`bootstrap` installe Chrome for Testing dans `.browsers/`, puis compile les trois variantes Spider et le writer Rust en release, avec leurs lockfiles. Les téléchargements/builds initiaux peuvent prendre plusieurs minutes. Les profils et les outputs du lab sont séparés du Chrome personnel. Aucun compte cloud ou modèle n'est nécessaire pour ces campagnes.

`bootstrap` construit aussi les bindings SQLite communs sur macOS arm64 : Node N-API, Rust FFI synchrone et Go/cgo. Go 1.27.1 est installé localement sous `.toolchains/`, sans modifier le Go global. Les archives sont épinglées par SHA-256 ; le source SQLite 3.53.4 est aussi vérifié contre le SHA3-256 publié. Ce build natif est actuellement qualifié uniquement sur macOS arm64.

Le `smoke` complet peut sortir avec le code 1 lorsqu'un concurrent ne respecte pas le contrat d'une fixture : le rapport est quand même produit. `unsupported` représente une capacité absente ; un résultat incomplet reste `failed`. `doctor` contrôle installation et protocole, pas la justesse métier.

## Installations et configurations

| Backend | Version | Travail et cycle de vie |
| --- | --- | --- |
| `node-http` | Node du manifest | Fetch natif + parser spécialisé pour les fixtures ; témoin minimal, pas un scraper général |
| `crawlee-http` | Crawlee 3.18.2 | CheerioCrawler neuf par tâche, queue en mémoire, concurrence interne 1 |
| `crawlee-http-pooled` | Crawlee 3.18.2 | Un crawler `keepAlive` et une queue par bloc ; pool partagé entre les tâches |
| `spider-http` | Spider 2.53.9 | `basic,sync`, sans features par défaut ; Website par tâche, concurrence interne 1 |
| `spider-chrome` | Spider 2.53.9 | Binaire séparé avec `chrome` ; browser géré par Spider |
| `spider-smart` | Spider 2.53.9 | Binaire séparé avec `smart`, qui active Chrome côté dépendance ; décisions de rendu natives |
| `playwright` | 1.63.0 | Browser par bloc, page/context neuf par tâche |
| `puppeteer` | puppeteer-core 25.12.0 | Browser par bloc, page par tâche ; shadow non qualifié dans cet adapter |
| `stagehand` | 4.1.0 | Browser local + extension, primitives Page/Locator ; cache et self-heal désactivés |
| `stagehand-batch` | 4.1.0 | `experimentalBatch` pour la séquence de clics seulement ; autres scénarios identiques à `stagehand` ; concurrence 1 |
| `crawlee-playwright` | Crawlee 3.18.2 / PW 1.63.0 | Crawler et browser par tâche ; garanties natives de Playwright |
| `crawlee-stagehand` | @crawlee/stagehand 3.18.2 / Stagehand 3.7.3 | Intégration demandée, lancée localement ; Page Playwright attachée par CDP ; frontière modèle interdite |
| stockage Node | `node:sqlite` du Node exécuté | Une connexion synchrone par processus écrivain |
| stockage Rust | SQLx 0.8.6 | Tokio + pool limité à une connexion par processus écrivain |
| `node-shared` | Node 25.5.0 / SQLite 3.53.4 commun | Binding N-API synchrone, même bibliothèque C partagée |
| `rust-sync` | Rust / SQLite 3.53.4 commun | FFI synchrone, même bibliothèque C partagée |
| `go-sync` | Go 1.27.1 / SQLite 3.53.4 commun | cgo synchrone, même bibliothèque C partagée |

**`@crawlee/stagehand` n'accepte actuellement que Stagehand v3.** Le workspace `adapters/stagehand-v3` évite de lui injecter v4. Les dépendances sont fixées dans pnpm/Cargo.lock. Un warning de peer Zod subsiste pour le fournisseur Ollama transitif de v3 ; ce fournisseur est hors qualification. L'adapter v3 remplace explicitement le client modèle par un client qui échoue à tout appel. V4 utilise les primitives sans instruction IA ni configuration modèle. Les variantes IA, cache cloud et réparation ne sont pas mesurées ici.

Les scripts d'installation de `esbuild` et `better-sqlite3` sont autorisés explicitement. Ceux de `@google/genai`, `bufferutil` et `protobufjs` sont désactivés ; leurs fonctions optionnelles ne sont pas qualifiées. Aucune installation avec `--force`.

Le build macOS 27/Xcode 27 a nécessité `release.build-override.strip=false, debug=true` pour rendre les dylibs de compilation chargeables. L'exécutable reste compilé en release. La feature Spider `basic` entraîne aussi des dépendances SQLite via sa feature TLS ; elle n'implique pas une persistance du crawl dans ces essais.

## Campagnes de crawl et navigateur

```sh
pnpm bench campaigns/http-stress.json
pnpm bench campaigns/http-lifecycle.json
pnpm bench campaigns/arrival-stress.json
pnpm bench campaigns/browser-primitives.json
pnpm bench campaigns/qualification.json
pnpm bench campaigns/smoke.json --backends playwright,stagehand,crawlee-stagehand
```

Chaque appel produit `results/<nom>-<date>-<id>/` : `manifest.json`, `attempts.jsonl`, `summary.json`, `summary.csv`, `report.html`. Les dossiers sont uniques, les anciennes expériences sont conservées. Pour régénérer un rapport :

```sh
pnpm report results/<dossier>
```

Le catalogue HTTP est paginé. Le catalogue JavaScript charge ses données par fetch. L'oracle vérifie tous les IDs, prix, doublons et omissions. Les formulaires vérifient nom, select, checkbox, événement de saisie et effet serveur. Les clics vérifient la séquence et les événements déclarés trusted. Les mutations actuelles sont overlay temporaire, bouton retardé et remplacement DOM. Shadow DOM ouvert est testé ; iframe/download ont des fixtures, mais leurs adapters ne sont pas qualifiés et ne sont pas annoncés comme supportés.

Une condition explicite détermine la disponibilité de la page et la postcondition. Les wrappers utilisent `domcontentloaded`, puis cette condition. Le polling de condition à 8 ms n'est pas une attente arbitraire de succès. Les contrôles supplémentaires natifs de Playwright restent actifs ; Stagehand ne reçoit pas silencieusement toutes ses garanties. Le test prouve le résultat de ces fixtures, pas une équivalence universelle d'actionnabilité.

Pour chaque backend et bloc : démarrage séparé, conformité pour chaque mutation, warmup exclu des statistiques, puis mesures. Toute non-conformité coupe les mesures de la cellule. Les blocs randomisent l'ordre des backends par seed. Les variantes n'ont pas toutes un ordre randomisé : ce point doit être renforcé avant une publication statistique.

`closed` maintient un nombre borné de tâches actives. `open` produit des arrivées indépendantes et rejette quand la capacité est occupée. Il n'y a pas de queue FIFO cachée. `queueMs` désigne le retard d'admission/générateur, pas le temps dans une queue métier. La deadline borne l'admission ; les tâches déjà actives restent bornées par leur timeout. Un timeout met fin au worker et à son groupe de processus, donc peut faire échouer les autres tâches de ce worker.

Le débit est celui des **tâches complètes vérifiées**, pas celui des pages ni celui des RPC. Le rapport distingue quantiles des succès, quantiles de toutes les tentatives démarrées, erreurs, rejets, unsupported, et effectifs. Il agrège les échantillons bruts, jamais les p95 des blocs. Les détails par bloc et les fenêtres de mesure restent disponibles.

## Campagnes d'écriture

```sh
pnpm storage campaigns/storage-smoke.json
pnpm storage campaigns/storage-contention.json
pnpm storage campaigns/storage-process-crash.json
pnpm shared-build
pnpm storage campaigns/storage-shared-contention.json
pnpm storage campaigns/storage-shared-checkpoint.json
```

| Contrat | Acquittement |
| --- | --- |
| `jsonl-buffered` | Append terminé côté OS ; aucune synchronisation disque demandée |
| `jsonl-durable` | Append puis synchronisation du fichier pour chaque batch |
| `json-snapshot-durable` | Document JSON entier croissant, fichier temporaire synchronisé, rename, synchronisation du directory ; un seul écrivain |
| `sqlite-wal-full` | Une transaction par batch, WAL, synchronous FULL |
| `sqlite-wal-normal` | Une transaction par batch, WAL, synchronous NORMAL ; série distincte de FULL |
| `sqlite-delete-full` | Une transaction par batch, journal DELETE, synchronous FULL |

`producers` règle les tâches actives du générateur. `writers` crée de vrais processus indépendants, chacun avec une connexion, sur **le même sink de la cellule**. Plusieurs writers SQLite restent soumis à un seul writer actif dans SQLite. `busy_timeout` et `wal_autocheckpoint=1000` sont explicites ; les PRAGMAs effectifs, `sqlite_version` et options de compilation sont consignés. Les trois nouveaux bindings chargent **un seul fichier SQLite commun**, dont le chemin réellement chargé, SHA-256, source ID, flags/options et PRAGMAs sont contrôlés. Les sources de compilation sont liées au manifest et un binaire périmé est refusé. Leurs bindings, allocations et encodages restent différents : cette comparaison est plus contrôlée, sans isoler le seul langage. Les pistes historiques Node natif / Rust SQLx gardent leurs builds séparés.

Des versions SQLite différentes interdisent d'attribuer un écart au seul langage.

Le workload construit les mêmes records et encode du JSON même pour la piste relationnelle. `serializationMs` mesure construction + encodage par batch ; `writerAckMs` inclut aussi le sink ; le temps principal inclut les échanges worker. Ce n'est pas un microbenchmark pur de JSON.stringify/serde_json. Le snapshot réécrit tout le document, tandis que JSONL append et SQLite insert : comparer ces usages distincts selon le besoin, sans classement commun aveugle.

Après drain et fermeture, l'oracle rouvre les fichiers, compare tous les records attendus aux batches acquittés, cherche pertes/doublons/valeurs incorrectes et lance `PRAGMA integrity_check`. Seules les lignes relues correctement comptent dans le débit utile. La fermeture/checkpoint et l'oracle ont un temps distinct `finalizationMs`. Les bytes incluent les sidecars SQLite présents au moment du contrôle.

La campagne crash envoie SIGKILL **après les acquittements** et rouvre le sink. Elle vérifie la conservation après crash de processus, pas l'arrêt au milieu d'une transaction, la reprise idempotente ni une coupure électrique. Les limites de macOS/fsync/F_FULLFSYNC sont détaillées dans la recherche stockage.

## Workflow intégré durable

```sh
pnpm workflow campaigns/workflow-throughput.json
pnpm workflow campaigns/workflow-recovery.json
pnpm workflow campaigns/workflow-pool-wakeup.json
```

Le [contrat du prototype historique](docs/workflow.md) documente acquisition/extraction → validation → queue bornée → writer batché → commit → ack, leases et receipts idempotents, avec son sink Node SQLite incorporé. Le [nouveau workflow natif](docs/runtime-workflow.md) implémente ce cycle séparément dans Node et Rust/Tokio, avec le même artefact SQLite chargé par les deux writers. Go reste qualifié dans la piste d'écriture.

```sh
pnpm workflow-build
node tools/qualification-suite.ts
node tools/qualification-current.ts
```

## Reproductibilité et limites

- Les fixtures et l'oracle privé vivent dans le coordinateur ; le worker reçoit seulement l'URL publique de sa tâche. Cela évite les faux succès accidentels ; ce n'est pas une isolation contre un adapter hostile capable de falsifier des événements ou d'inspecter le host.
- Les manifests récents fixent configs, seeds, runtime réel, OS, CPU, mémoire, browser et SHA-256, locks, sources et exécutables Rust. Une modification de source pendant une campagne récente entraîne un code de sortie 1 et un avertissement. Les premières campagnes de construction précèdent ce garde-fou.
- Les pistes historiques échantillonnent le RSS à 250 ms. Les nouvelles pistes runtime et lecteurs SQLite utilisent aussi une sonde macOS CPU/RSS/footprint/I/O à 100 ms, calibrée contre getrusage ; elles peuvent manquer des pics et des fins de processus. Les pages partagées peuvent être additionnées. Le workflow historique inclut le coordinateur/oracle ; le workflow natif mesure son coordinateur et ses descendants, avec fixture et oracle hors du périmètre. Les limites matérielles CPU/RAM ne sont pas imposées.
- Le lab borne concurrence, appels, sortie JSONL, délai et corpus : au plus un million de records de fixture retenus par le coordinateur, et 64 MiB de corpus estimé par cellule de stockage. Les plus grands tests nécessitent un oracle streaming. Il n'est pas un sandbox réseau et ne fournit pas une isolation matérielle. Les tests actuels ciblent loopback ; aucune campagne sur des sites publics n'est nécessaire.
- Profiles, cache OS et connexions réutilisées diffèrent selon backend. Les cycles de vie sont des lignes séparées. `crawlee-http-pooled` sert précisément à éviter de conclure à partir du seul coût d'un crawler neuf par tâche.
- p95/p99 avec quelques dizaines d'échantillons restent exploratoires. La borne zéro échec suppose des essais Bernoulli indépendants/représentatifs ; ce corpus ne les établit pas. Aucun SLO de production n'est annoncé.
- Ne pas lancer plusieurs campagnes ou compilations en parallèle pour interpréter des performances. Les essais de construction ont parfois coexisté avec une compilation ; ils servent d'abord à la qualification et doivent être répétés sur un host stable.

Voir la [matrice de couverture](docs/coverage-matrix.md), le [protocole adapter](docs/protocol.md), les [bonnes pratiques générales](../research/2026-10-07-lab-testing-engineering.md) et celles du [stockage](../research/2026-10-07-lab-storage-engineering.md). La [shortlist sourcée](../research/2026-10-07-lab-competitor-shortlist.md) distingue les concurrents à ajouter des backends installés.

## Ajouter une expérience

Définir d'abord le résultat observable et l'oracle ; ajouter une fixture indépendante du backend, puis une capacité adapter. Faire refuser au lab un adapter incorrect avant de collecter ses temps. Donner à chaque configuration un nom explicite, un cycle de vie, des budgets, des versions et un contrat de durabilité. Ajouter un JSON de campagne et exécuter `pnpm check`, `pnpm test`, puis une qualification réelle ciblée.

Les tests automatisés couvrent oracle, persistance, faux succès, protocole corrompu/hors ordre, timeout et descendants, surcharge/admission, métriques et validation des configs. Les campagnes réelles couvrent les bibliothèques installées. Les performances ne doivent pas devenir des assertions instables sur un laptop ; la CI portable valide les contrats, et les comparaisons de performance utilisent une machine réservée.

Le runner TypeScript est un outil de mesure qui pilote plusieurs processus/langages. Son choix ne fixe pas le langage du futur moteur Automan. Rust/Tokio reste le candidat pour le noyau portable ; Node est la référence de mesure et une voie de compatibilité. La comparaison du workflow HTTP natif Rust/Node est vérifiée sur 30 cellules ; le pilote Chromium natif Rust et un noyau intégré Go restent à qualifier.

[Commandes de reproduction et régénération des preuves](reports/qualification-reproduce.md). Les guards corrigés passent 162/162 cellules et les readers/checkpoints SQLite 108/108 ; les échecs des configurations natives restent visibles dans le rapport.
