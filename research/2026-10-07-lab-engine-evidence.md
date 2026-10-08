# Laboratoire Automan — audit des trois moteurs demandés

Relevé du **7 octobre 2026**. « Crawell » est interprété comme **Crawlee** ; « Spider rs » désigne la crate Rust OSS **`spider` du dépôt spider-rs**, distincte de l'API commerciale Spider Cloud.

**Conclusion :** les trois outils doivent entrer dans le lab, avec des configurations séparées. Spider est une référence de collecte Rust HTTP/Chrome ; Crawlee une référence d'orchestration de crawl ; Stagehand une référence de pilotage et de perception de navigateur. Leurs fonctions communes permettent des expériences, leurs fonctions différentes empêchent un classement global honnête sans définir le résultat attendu.

Cette note complète les [contrats Crawlee/Spider](./2026-10-07-crawlee-spider-contracts-followup.md), l'[audit Stagehand](./2026-10-07-stagehand-alternatives-followup.md) et la [clarification CDP](./2026-10-07-stagehand-cdp-latency.md). Elle revérifie les registres et des sources officielles ; **aucune dépendance installée, aucun moteur lancé, aucun benchmark exécuté**.

## 1. Versions stables revérifiées et séparation indispensable

| Composant | Version stable constatée | Contrat utile au lab | Source officielle exacte |
|---|---|---|---|
| Crawlee TypeScript | **3.18.2** | SDK Node.js, HTTP et browser crawlers | [registre npm](https://registry.npmjs.org/crawlee/3.18.2) |
| Adapter `@crawlee/stagehand` | **3.18.2** | Peer Stagehand **`^3.0.0`**, Playwright `*` | [manifest npm](https://registry.npmjs.org/@crawlee%2Fstagehand/3.18.2) |
| Stagehand TypeScript | **4.1.0** | Pilote/runtime v4, Node **`>=22.18.0`** | [manifest npm](https://registry.npmjs.org/@browserbasehq%2Fstagehand/4.1.0) |
| Stagehand branche v3 | **3.7.3**, tag `v3-latest` | Candidat épinglé pour tester l'adapter Crawlee actuel | [registre global](https://registry.npmjs.org/@browserbasehq%2Fstagehand), [manifest 3.7.3](https://registry.npmjs.org/@browserbasehq%2Fstagehand/3.7.3) |
| Crawlee Python | **1.10.4** | Python **`>=3.10`** ; extra Stagehand **`>=3.19.5,<4.0.0`** | [PyPI exact](https://pypi.org/pypi/crawlee/1.10.4/json) |
| Spider Rust | **2.53.9** | HTTP natif, features browser/smart/disk/distribution | [crates.io exact](https://crates.io/api/v1/crates/spider/2.53.9) |
| `spider_agent` | **2.53.9** | Agent Rust facultatif, features par capacité | [crates.io exact](https://crates.io/api/v1/crates/spider_agent/2.53.9) |
| Playwright | **1.63.0** | Baseline indépendante pour le navigateur | [manifest npm](https://registry.npmjs.org/playwright/1.63.0) |
| Puppeteer | **25.12.0** | Autre baseline CDP facultative | [manifest npm](https://registry.npmjs.org/puppeteer/25.12.0) |

Les numéros npm et Python Stagehand décrivent des paquets différents : `3.7.3` npm n'est pas `3.19.5` Python. Le npm Crawlee 3.18.2 est publié le 29 septembre ; Stagehand 4.1.0 le 9 septembre ; Spider 2.53.9 le 5 septembre, selon leurs registres.

**Ne pas forcer Stagehand v4 dans `@crawlee/stagehand` 3.18.2.** Ce peer exclut v4. Créer une lane Stagehand v4 autonome et, si utile, une lane d'intégration Crawlee + Stagehand v3. Une dépendance qui s'installe avec `--force` ne prouve pas la compatibilité des objets ou du runtime. [Source versionnée de l'adapter](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json).

## 2. Stagehand : CDP, placement, garanties et coût

### Le chiffre qui motive le lab

Le billet v4 rapporte **un parcours Wikipedia de 50 actions**, toutes réalisées dans les deux variantes : batch Stagehand **14 221,7 ms**, Playwright **22 650,3 ms**, soit **1,59× le débit**, avec RTT client–browser **42,2 ms**. Les médianes annoncées sont `waitForSelector` 237,6 contre 493,2 ms, `click` 323,1 contre 628,1 ms et `goBack` 17,5 contre 139,5 ms. Les auteurs demandent explicitement de considérer cela comme **un run, pas un benchmark général**. [Billet officiel v4, 10 août 2026](https://www.browserbase.com/blog/stagehand-v4).

Le gain annoncé concerne une séquence déterministe/batch et son transport distant. Il ne démontre pas le débit d'extraction LLM, le coût total d'un agent, l'autoscaling de crawlers ou une fiabilité sur tous les sites. Le billet ne donne pas à lui seul un corpus reproductible, une distribution de runs, des p95 ni un protocole détaillé de postconditions identiques.

Playwright **utilise déjà CDP pour Chromium** : `CRBrowser` et `CRConnection` sont dans son backend. L'opposition pertinente est donc pilote, placement de l'exécution, nombre de RPC et contrôles réalisés. Les modes `connect()` Playwright et `connectOverCDP()` sont eux-mêmes distincts ; la réserve de fidélité des docs vise ce dernier mode d'attachement. [Code Chromium officiel](https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/chromium/crBrowser.ts), [référence d'attachement](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp).

### Ce qui a changé en v4

Stagehand v4 place suivi des targets/frames et dispatch CDP dans une extension Chrome, notamment son service worker. Le SDK est un client RPC ; l'extension est proche de l'état navigateur. `experimentalBatch()` exécute un callback JavaScript dans ce worker : une série évite ainsi un trajet SDK–browser par opération. Le callback doit être autonome, avec données JSON explicites ; une fermeture Node.js n'est pas transportée. Un batch n'est pas une transaction et son timeout n'annule pas un effet déjà produit. [Référence `experimentalBatch`](https://docs.stagehand.dev/v4/reference/stagehand).

V4 n'a plus de dépendance Playwright et ne reçoit plus une Playwright `Page`. La migration décrit une seule BrowserContext par browser, l'absence d'auto-waiting Playwright, de locators `getBy*`, d'assertions web-first et de route interception. `goto()` attend `domcontentloaded` par défaut ; un selector est résolu une fois. Une durée moindre peut donc acheter moins de garanties. [Migration Playwright officielle](https://docs.stagehand.dev/v4/migrations/playwright).

Un clic Playwright vérifie notamment visibilité, stabilité, réception des événements et enabled ; ses assertions réessaient. Même ces contrôles n'établissent pas que la bonne entité métier a été modifiée. Il faut deux mesures : **opération native telle que fournie**, puis **opération avec le contrat commun et l'oracle du lab**. [Actionability Playwright](https://playwright.dev/docs/actionability).

### Voies déterministes et voies IA

| Voie Stagehand v4 | Inférence sur le chemin normal | Conditions/limites à mesurer |
|---|---|---|
| `page`, `locator`, snapshot, capture | Aucune résolution LLM nécessaire | Browser/extension requis ; établir les postconditions indépendamment |
| `observe(instruction)` | Résolution modèle, hors cache admissible | Actions candidates retournées, pas objectif exécuté |
| `act(instruction)` | Interprétation puis action, hors cache | Succès technique distinct de vérité métier ; appels additionnels possibles |
| `act(Action)` | Rejeu direct sans inférence initiale | En stable 4.1.0, **configuration modèle/gateway vérifiée avant la branche directe** ; réparation peut appeler le modèle |
| `extract(instruction,schema)` | En stable 4.1.0, génération puis appel de complétude | Le deuxième appel reçoit instruction et sortie, pas le snapshot source ; conformité au schéma distincte de vérité |

Ces détails ont été revérifiés dans le **[tarball stable 4.1.0](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz)** ouvert en mémoire, sans installation : `dist/extension/service-worker.js`, fonctions `act$2`, `extract$1` et `buildCacheContext`, et types `dist/index.d.mts`. Le contrôleur lève `An LLM was not configured during Stagehand initialization` lorsque `act` n'a ni modèle ni gateway, avant le rejeu. Les primitives déterministes Page/Locator restent une voie distincte.

Un navigateur local peut utiliser une configuration modèle BYOK ou un callback `generate()`, éventuellement relié à un modèle auto-hébergé. **Le gateway Browserbase n'est pas la seule voie IA**, mais une extension locale ne signifie pas que l'inférence est locale. Publier endpoint, modèle, révision, disponibilité et latence de cette dépendance. [Configuration officielle](https://docs.stagehand.dev/v4/reference/stagehand).

`agent()` a été retiré en v4 ; le SDK expose des outils, l'orchestration revient au code ou à un agent externe. La compilation d'un script puis son exécution sans modèle n'est donc pas exclusive à Automan. [Migration v3 officielle](https://docs.stagehand.dev/v4/migrations/v3).

### Cache et données fraîches

Le cache standard est un service Browserbase : session Browserbase et clé nécessaires ; le navigateur local seul n'en bénéficie pas. Les docs courantes l'annoncent actif par défaut, mais `buildCacheContext` stable 4.1.0 contient **`defaultCaching: initParams.cache ?? false`**. Fixer explicitement `cache`, contrôler les métadonnées et séparer tests locaux et cloud. [Cache officiel](https://docs.stagehand.dev/v4/best-practices/caching), [source distribuée exacte](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz).

Dans le runtime inspecté, `act` conserve des actions, `observe` une liste de candidats, et `extract` une **sortie structurée**. Un HIT extract retourne cette valeur. La requête cache contient URL, arbres AX, instruction/options et schéma ; le modèle est omis. Cela ne prouve ni données systématiquement anciennes, ni fraîcheur universelle : matching/TTL du serveur non public restent à exercer. [Module public correspondant](https://github.com/browserbase/stagehand/blob/fdd179582b6b1f9e43c3d18ad27917cf2e66631e/packages/extension/services/cacheService.ts).

Test décisif : même URL, même layout, prix changé ; vérifier valeur retournée et HIT/MISS avec un oracle serveur. Comptabiliser toutes les captures, appels cache, inférences de réparation, tentatives échouées et tokens. « Zéro token sur HIT » n'est pas « zéro coût » : browser, réseau, validations et service subsistent.

## 3. Crawlee : baseline d'orchestration, deux runtimes

Crawlee TS est écrit en TypeScript et exécuté dans Node.js ; Python est une implémentation distincte. Tous deux proposent concurrence async, pools et crawlers HTTP/navigateur. TS apporte Playwright et Puppeteer ; Python dispose de son intégration Playwright et d'adapters propres. Ne pas extrapoler la surface ou les mesures de l'un à l'autre. [Introduction TS](https://crawlee.dev/js/docs/introduction), [architecture Python](https://crawlee.dev/python/docs/guides/architecture-overview).

`AutoscaledPool` ajuste des tâches async selon les ressources, tandis que `BrowserPool` gère browsers/pages. Un worker async de cet autoscale n'est pas par définition un processus distribué. La file `RequestQueue` déduplique par `uniqueKey`, peut ajouter/reclaim/mark handled des requêtes et se branche sur le stockage réellement choisi. [Pool officiel](https://crawlee.dev/js/api/core/class/AutoscaledPool), [queue officielle](https://crawlee.dev/js/api/core/class/RequestQueue).

**Persistance de requêtes n'est pas workflow transactionnel.** Rejouer un handler après crash peut répéter un effet externe ou une sortie déjà écrite avant acquittement. La proposition Automan doit définir ses leases, checkpoints, idempotency keys et validation d'état ; la présence de Crawlee n'établit pas une exécution exactement une fois. Cette conséquence doit être qualifiée avec un kill test et un sink contrôlé. [Cycle BasicCrawler versionné](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts).

`AdaptivePlaywrightCrawler` est une référence particulièrement utile : il compare des sorties du handler HTTP et browser, avec `resultChecker`/`resultComparator`, puis apprend un choix. Le comparateur par défaut concerne les dataset items ; sa validité dépend de ce que produit le handler. Des listes vides ne prouvent pas des liens découverts identiques. [Source adaptative 3.18.2](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts).

Le HTTP TS peut utiliser **Impit, transport natif Rust** : « SDK TypeScript » ne signifie pas « toutes les opérations réseau en JavaScript ». Ce point justifie une baseline Crawlee HTTP qualifiée, en précisant client TLS/HTTP, parser et compression. [Guide officiel Impit](https://crawlee.dev/js/docs/guides/impit-http-client).

L'adapter TS Stagehand 3.18.2 démarre Stagehand et attache Playwright via CDP. Le plugin avertit que fingerprinting reste partiel car Stagehand contrôle le lancement. Python 1.10.4 utilise `AsyncStagehand`, une session et son URL CDP ; ses limitations de contextes/pages/proxies doivent être traitées séparément. [Plugin TS figé](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-plugin.ts), [manifest Python figé](https://github.com/apify/crawlee-python/blob/v1.10.4/pyproject.toml), [guide Python](https://crawlee.dev/python/docs/guides/stagehand-crawler).

## 4. Spider-rs : ce qui existe dans la crate publiée

Source 2.53.9 liée au SHA **`e4b53c5b582357e46433fa9ba3d91f930fb67cb4`**. Le manifeste inspecté utilise Rust/Tokio, HTTP `reqwest 0.13`, et **Chromey facultatif** pour CDP. Les versions de dépendances sont des plages : un futur `Cargo.lock` doit fixer la résolution transitive. [Manifest exact](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml).

| Lane proposée | Capacités/activation | Ce qu'elle ne prouve pas seule |
|---|---|---|
| HTTP | `basic`, acquisition/parsing HTTP | Contenu exécuté par JavaScript |
| Chrome | `chrome`, Chromey, Chrome/Chromium local ou CDP | Garanties complètes des locators Playwright |
| Smart | `smart` implique `chrome` et `chrome_intercept` | Choix toujours optimal selon le résultat métier |
| Agent | `spider_agent`/`agent_chrome`, modèle configuré | Succès indépendant du modèle |
| Disk | `disk`/SQLx, ensembles SQLite | Queue pending durable et workflows transactionnels |
| Distributed | `decentralized` + `spider_worker` | Worker browser rendu ou orchestration distribuée générale |

Ne pas activer `full` ou toutes les features pour une baseline minimale. Le défaut inclut `basic`, `io_uring`, `tcp_fastopen`, `splice`, `numa`, `zero_copy` ; plusieurs optimisations sont dépendantes de plateforme. La documentation docs.rs agrège de nombreuses features : elle ne décrit pas automatiquement le binaire installé sur macOS. [Liste officielle features](https://crates.io/api/v1/crates/spider/2.53.9).

Smart emploie des signaux HTML pour décider un upgrade, et peut ouvrir Chrome paresseusement. Spider propose aussi des backends parallèles et un `QualityValidator` facultatifs : la validation ne doit pas être réduite à « présence de JS ». Mesurer taux d'upgrade, travail doublé et qualité du résultat demandé. [Chemin smart figé](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/page.rs#L8305), [hook qualité](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14248).

`setup_semaphore()` choisit un handle adaptatif fourni, un sémaphore partagé ou une limite configurée/défaut. Cela ne prouve pas que chaque exécution ajuste automatiquement CPU/RAM comme Crawlee. Le navigateur normal est réutilisé au cours du crawl : ne pas lui opposer une baseline qui lance Chromium à chaque URL. [Sémaphore](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L6956), [cycle browser](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L9180).

### Durabilité et distribution à ne pas surestimer

La base `disk` contient tables `resources` et `signatures`, sans états pending/running/done ni leases d'étapes. `persist` vaut false par défaut et le Drop efface le fichier lorsqu'il n'est pas conservé. Cette persistance sert des ensembles de visites/signatures ; un frontier après crash exige un contrat supplémentaire. [Schéma et Drop exacts](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/disk.rs).

`Website::queue()` est un canal Tokio **broadcast en mémoire** ; `persist_links()` change le statut de l'objet pour poursuivre des passes. Le flux de pages doit être consommé vers un sink et son retard observé. Ce n'est pas un journal de résultats durable. [Queue publiée](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14692).

Le worker OSS utilise Warp 0.4/Hyper 1/Tokio et ses handlers fetch/parsing HTTP ; son manifeste n'active pas Chrome. Le mot worker ne suffit donc pas pour promettre rendu distribué, leases ou restauration d'un job. L'infrastructure privée Spider Cloud est une autre surface. [Manifest worker](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/Cargo.toml), [handlers worker](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/src/main.rs).

Le benchmark public Spider oppose Spider, Colly, le paquet Node **`crawler`**, wget : **Crawlee et Stagehand n'y figurent pas**. Démarrage/concurrence/sorties ont des asymétries et les volumes corrects ne sont pas assertionnés. Ne pas utiliser ses multiplicateurs pour choisir le runtime Automan. [Protocole source figé](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/BENCHMARKS.md), [audit détaillé local](./2026-10-07-benchmark-audit-followup.md).

## 5. Installation reproductible proposée, à réaliser pendant l'implémentation

**Ne pas installer les trois dans un seul projet de dépendances.** Préparer des adaptateurs isolés sous un répertoire `lab/`, chacun exposant le contrat du harness et ses versions. Utiliser les packages publiés ; clones upstream seulement pour un profil explicitement expérimental.

1. **Crawlee TS 3.18.2 + Playwright 1.63.0** : packages/workspaces isolés, versions exactes, `pnpm-lock.yaml`, installation par `pnpm install --frozen-lockfile` pour reproduire. L'alternative npm est `package-lock.json` + `npm ci` ; choisir un gestionnaire et sa version exacte pour le lab. HTTP et browser explicitement sélectionnés ; le backend HTTP et le parser sont enregistrés.
2. **Stagehand TS 4.1.0 autonome** : lock séparé, Node exact respectant `>=22.18.0`, bundle d'extension du package. Lane sans modèle via Page/Locator, puis lane modèle/cache séparée. Fixer `cache` et `selfHeal` explicitement.
3. **Crawlee + Stagehand v3 facultatif** : lock séparé, `@crawlee/stagehand@3.18.2` et `@browserbasehq/stagehand@3.7.3`, avec peers résolus et smoke test avant qualification. Pas de mélange v3/v4.
4. **Spider `=2.53.9`** : `Cargo.lock`, toolchain Rust exacte après qualification, build `--release --locked`, features publiées. Profils HTTP, Chrome, Smart compilés/contrôlés séparément ; relever `cargo tree -e features`.
5. **Python facultatif** : environnement isolé, Crawlee `==1.10.4`, lock avec hashes et versions exactes des extras. La baseline TS reste distincte de Python.

Cargo peut unifier les features d'une même dépendance dans un build de workspace : une lane HTTP ne doit pas hériter silencieusement de Chrome/Smart parce que le harness compile tous les adapters ensemble. Utiliser projets ou invocations contrôlés et vérifier le graphe effectivement construit. [Règles Cargo officielles](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification).

### Le navigateur doit également être figé

Épingler **l'exécutable Chrome/Chromium et sa version exacte**, plateforme/architecture, checksum et flags. Fournir le même `executablePath` aux comparateurs du profil commun. Garder un second profil « navigateur fourni/recommandé par l'outil » si ce choix influence les capacités ; ses résultats ne sont pas fusionnés avec le profil commun.

Stagehand stable recherche un Chrome existant et, sur macOS, peut préférer **Canary** à Stable ; l'autodétection fausserait le contrôle du navigateur. Son manifeste d'extension annonce un minimum Chrome 116, mais le SDK utilise `Extensions.loadUnpacked` et signale une erreur si cette commande manque. Ce minimum n'est donc pas une preuve de compatibilité de toutes les builds. Exiger une préqualification extension/loadUnpacked, frames, screenshot et clic ; ensuite figer la build. [Tarball exact, SDK et manifest d'extension](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz).

Normaliser l'isolation : Stagehand v4 fournit **un contexte par browser**. Dans une expérience d'isolation de sessions/tenants, utiliser un browser par identité aussi chez les baselines, ou publier séparément le coût d'une stratégie multi-contextes disponible ailleurs. Un browser partagé multi-onglets et plusieurs browsers isolés ne rendent pas le même service.

### Données à enregistrer dans chaque résultat

Version package + lock hash + features ; runtime/compiler ; browser/extension/flags ; machine/OS ; placement client/driver/browser ; modèle/endpoint ; cache HIT/MISS ; URL corpus/seed ; concurrence et limites ; nombre de commandes/captures/inférences ; états finaux vérifiés ; tentatives et erreurs ; CPU/RSS du **groupe de processus**, bytes et coût total.

Les projets sont actuellement des candidats. Un lock est une preuve de résolution reproductible, pas une preuve de compatibilité : qualification locale indispensable, puis tests communs avant toute décision Rust/TypeScript ou CDP/Playwright.

## 6. Expériences qui départageront ces moteurs

- **Transport/browser déterministe** : Playwright, Stagehand Page/Locator, Stagehand batch, Spider/Chromey sur mêmes primitives et même oracle ; local puis RTT contrôlées, placement du driver enregistré.
- **Collecte** : Spider HTTP/Chrome/Smart et Crawlee HTTP/Playwright/adaptatif sur corpus connu, mêmes URL autorisées, contenu et ressources ; aucune IA obligatoire.
- **Perception/IA** : Stagehand observe/act/extract et `spider_agent`, même modèle/budget si possible, extraction exacte, faux succès et nombre réel d'appels.
- **Répétition et dérive** : première décision, rejeu, données fraîches, mutation DOM sémantiquement trompeuse et réparation bornée ; pas seulement taux de HIT.
- **Incident/reprise** : kill après sortie/avant acquittement, lenteur sink, perte CDP, crash browser et identité expirée ; travail perdu/doublons explicitement comptés.

**Décision proposée :** Rust/Tokio reste une hypothèse cohérente pour un runtime commun et portable. Ce lab doit déterminer quelles couches en bénéficient, quels composants réutiliser et quelles garanties posséder. Une extension placée près du browser, un worker Node co-localisé et un driver Rust distant peuvent inverser un classement fondé seulement sur le langage.
