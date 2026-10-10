# Moteurs de crawl, extraction et automatisation : audit de sources

État observé le **7 octobre 2026**. Périmètre : Crawlee TS/Python, Spider/Chromey, Scrapy, Crawl4AI, Firecrawl, Scrapling, ScrapeGraphAI, Colly, Rod, chromedp, Ferret et alternatives plus spécialisées. L'objectif est un framework open source utilisable comme produit, avec une architecture ambitieuse développée par plusieurs agents.

**Conclusion pour la cible ambitieuse : un moteur canonique Rust derrière un protocole de sidecar, des SDK générés, HTTP et CDP natifs, et une orchestration de workflows distincte est une direction cohérente.** Elle ne prouve pas à elle seule de meilleures performances. Le cœur doit reprendre des briques éprouvées et définir un contrat d'actions et de résultats plus rigoureux, avec Playwright comme référence et backend de compatibilité qualifié. HTTP-first, autoscaling, pooling, récupération de sélecteurs et replay existent déjà séparément : les recombiner sans contrat de qualité mesurable ne suffit pas à établir une supériorité.

## Méthode et limites

- Recherche Exa : **80 résultats demandés** sur 8 appels, soit 10 + 10 + 15 + 10 + 5 + 10 + 10 + 10. `sources_reviewed = 80` au sens du compteur demandé, qui représente le volume de résultats de recherche, et non 80 sources intégralement auditées ou 80 sources uniques.
- Inspection primaire : fichiers README, LICENSE, manifests et chemins critiques téléchargés depuis GitHub ; métadonnées des dépôts ; versions publiées vérifiées sur npm, PyPI, crates.io et le proxy Go. Le code publié de Crawlee 3.18.2 et l'archive crate Spider 2.53.9 ont été examinés, afin de distinguer une fonctionnalité disponible d'une nouveauté sur la branche principale.
- Les dates de `pushed_at` GitHub sont des dates d'activité du dépôt, **pas nécessairement la date du dernier changement fonctionnel**. Un dépôt actif et une release publiée sont deux observations distinctes.
- Aucun benchmark local comparatif n'a été exécuté. Les chiffres des mainteneurs sont qualifiés selon leur protocole ; ils ne prouvent pas un gain sur un parcours métier, sur un site protégé, ni sur des sessions longues.
- Certaines requêtes GitHub Releases ont atteint la limite de l'API. Les registres ont permis de vérifier les principales versions ; la dernière release stable Ferret v1 et plusieurs versions de niche n'ont pas été établies ici.

## Une cartographie des couches, pas une liste de produits interchangeables

| Projet | Couche principale | HTTP | Navigateur / actions | LLM | Orchestration / fiabilité | Licence observée |
|---|---|---|---|---|---|---|
| [Crawlee TS](https://github.com/apify/crawlee) | Framework de crawl | Http/Cheerio/JSDOM/LinkeDOM, client injectable | Playwright/Puppeteer, pools ; Stagehand séparé | Intégration Stagehand | Queue, sessions, retries, autoscaling, stockage | Apache-2.0 |
| [Crawlee Python](https://github.com/apify/crawlee-python) | Framework de crawl | Parsel/BeautifulSoup, clients HTTP | Playwright, Stagehand | Intégrations facultatives | Queue et clients mémoire/fichiers/SQL/Redis, pools | Apache-2.0 |
| [Spider](https://github.com/spider-rs/spider) | Moteur Rust de collecte | reqwest, streaming, caches et hooks | Chrome via Chromey, smart mode | Features et crate agent facultatives | Concurrence, budgets, déduplication, diffusion des pages, mode distribué | MIT |
| [Chromey](https://github.com/spider-rs/chromey) | Driver CDP Rust | Primitives de transport/cache autour du navigateur | Chrome/Chromium via CDP | Pas son rôle principal | Lifecycle, events, réseau, cache, fingerprinting | MIT OR Apache-2.0 dans Cargo |
| [spider-browser](https://github.com/spider-rs/spider-browser) | SDK de navigateur distant | Via le service | API de pages cloud | act/observe/extract/agent | Retry et escalade dans l'offre cloud | MIT pour les SDK examinés |
| [Scrapy](https://github.com/scrapy/scrapy) | Framework Python de crawl | Downloader, middlewares, scheduling | Via intégrations, notamment scrapy-playwright | Extensions | Scheduler, pipelines, AutoThrottle, jobs reprenables | BSD-3-Clause |
| [Crawl4AI](https://github.com/unclecode/crawl4ai) | Extraction Web vers corpus pour IA | Stratégie HTTP et outils de collecte | Playwright, CDP distant, sessions, scripts | Extraction et stratégie sémantique facultatives | Dispatcher mémoire, rate limiting, cache | Apache annoncé + attribution ajoutée au LICENSE |
| [Firecrawl](https://github.com/firecrawl/firecrawl) | Service d'extraction/crawl avec API | Fetch et plusieurs moteurs | Service navigateur, actions selon moteur | Extract/agent et autres fonctionnalités selon offre | Queue et workers, choix de moteurs, formats | Cœur AGPL-3.0 ; SDK et certains composants MIT |
| [Scrapling](https://github.com/D4Vinci/Scrapling) | Parsing adaptatif + fetchers + spiders Python | curl_cffi et sessions | DynamicFetcher, StealthyFetcher avec Patchright | Facultatif ; parsing adaptatif sans LLM | Déduplication, checkpoint, throttling par domaine | BSD-3-Clause |
| [ScrapeGraphAI](https://github.com/ScrapeGraphAI/Scrapegraph-ai) | Graphes d'extraction assistée par modèles | Acquisition de documents | Backend navigateur / intégrations | Central dans SmartScraperGraph | Graphe de nœuds de récupération, parsing, réponse | MIT pour ce dépôt |
| [Colly](https://github.com/gocolly/colly) | Framework Go de crawl HTTP | Oui, callbacks/transport | Pas un moteur JS intégré équivalent à Playwright | Pas son rôle | Limits, cookies, cache, storage extensions | Apache-2.0 |
| [Rod](https://github.com/go-rod/rod) / [chromedp](https://github.com/chromedp/chromedp) | Drivers CDP Go | Transport navigateur | Chrome/Chromium | Pas leur rôle | Contextes, events, attentes, cancellation | MIT |
| [Ferret](https://github.com/MontFerret/ferret) | Langage/runtime déclaratif | Intégrations/capacités | Drivers/capacités | Pas obligatoire | Plans compilés et sessions dans v2 alpha | Apache-2.0 |

Cette lecture évite deux erreurs : comparer un SDK de cloud à un moteur auto-hébergeable, et comparer la vitesse d'un parseur HTML à celle d'un agent navigateur complet.

## Crawlee : le concurrent architectural à étudier en premier

### Ce qui est déjà résolu

Le code sépare le crawler, les sessions, le stockage, les transports et le pool navigateur. `AutoscaledPool` règle la concurrence selon les ressources disponibles ; `BrowserPool` gère la création et le retrait des navigateurs et expose des hooks. La queue possède une logique de déduplication, de récupération et de verrouillage. Il est plus économique de composer ces éléments que de réécrire immédiatement un scheduler universel. [AutoscaledPool](https://github.com/apify/crawlee/blob/master/packages/basic-crawler/src/internals/autoscaling/autoscaled_pool.ts), [BrowserPool](https://github.com/apify/crawlee/blob/master/packages/browser-pool/src/browser-pool.ts), [RequestQueue](https://github.com/apify/crawlee/blob/master/packages/core/src/storages/request_queue.ts).

**HTTP-first avec fallback navigateur n'est déjà plus une différenciation en soi.** `AdaptivePlaywrightCrawler` est présent dans la release publiée 3.18.2. Il emploie un handler capable de fonctionner sur un contexte HTTP limité ou un navigateur. L'accès à `page` en mode HTTP provoque une erreur qui permet le fallback. Une `resultChecker` valide la sortie ; `resultComparator` compare les sorties des deux modes. [Code publié](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts), [release 3.18.2](https://github.com/apify/crawlee/releases/tag/v3.18.2).

Le mécanisme est plus précis qu'une simple détection de balises `<script>` :

1. Le prédicteur utilise l'hôte, les segments du chemin d'URL et le label de requête. La similarité des segments sert de caractéristique à une régression logistique.
2. Une URL sans historique commence en mode navigateur et avec une probabilité de détection élevée.
3. Pour une détection, le handler navigateur est exécuté, puis le handler HTTP sur un état comparable. La différence des sorties sert à apprendre quel mode suffit.
4. Par défaut, le ratio de détection est 0,1 ; l'incertitude et le faible volume d'observations augmentent les contrôles.
5. Une prédiction « statique » permet de traiter des requêtes suivantes uniquement par HTTP, avec retour au navigateur en cas de résultat invalide ou d'erreur pertinente. [Prédicteur publié](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/rendering-type-prediction.ts).

La limite la plus pertinente pour Automan : dans la release 3.18.2, la comparaison par défaut porte sur les `datasetItems`, leur nombre et leur égalité profonde. Ce n'est pas une preuve universelle d'équivalence de deux parcours. Un handler qui ne produit aucun item peut fournir une comparaison peu informative. Un produit doit donc exprimer ses résultats attendus, plutôt que compter sur cette valeur par défaut. Le code expose les callbacks pour cela ; ce constat ne démontre pas un bug de Crawlee. [Comparateur et exécution](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts#L336).

**Autre limite majeure : le handler peut être exécuté plusieurs fois.** La documentation dans le code interdit les effets de bord autres que ceux fournis par le contexte, et capture les écritures pour ne committer que les résultats voulus. Cette stratégie convient à l'extraction ; elle ne garantit pas qu'une action externe, comme créer un enregistrement sur le site, peut être rejouée. Automan doit conserver un contrat séparé pour les étapes d'action, leurs préconditions, leurs postconditions et leur reprise. [Contrat du handler](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts#L167).

La version Python 1.10.4 possède aussi un `AdaptivePlaywrightCrawler` et un prédicteur dédié. Elle n'est donc pas un simple port limité à Playwright. [Implémentation Python](https://github.com/apify/crawlee-python/blob/master/src/crawlee/crawlers/_adaptive_playwright/_adaptive_playwright_crawler.py), [prédicteur Python](https://github.com/apify/crawlee-python/blob/master/src/crawlee/crawlers/_adaptive_playwright/_rendering_type_predictor.py).

### Attention à Stagehand et aux versions

`@crawlee/stagehand` est publié en **3.18.2**, avec une peer dependency `@browserbasehq/stagehand: ^3.0.0`. L'intégration n'est donc **pas une validation de compatibilité avec Stagehand 4**. Le package Stagehand n'apparaît pas parmi les dépendances du bundle principal `crawlee` : il se choisit séparément. Le code local de lancement utilise des options Stagehand et se branche dans l'écosystème de pool Crawlee. [Package publié/tag](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json), [manifest npm](https://registry.npmjs.org/@crawlee/stagehand/latest), [launcher](https://github.com/apify/crawlee/blob/master/packages/stagehand-crawler/src/internals/stagehand-launcher.ts).

La branche principale comprend aussi `remote-browser-pool.ts` et des abstractions de fournisseur distant. Leur présence confirme l'orientation vers des infrastructures interchangeables ; leur statut exact dans une release donnée doit être vérifié avant d'en faire une dépendance du MVP. [Source](https://github.com/apify/crawlee/blob/master/packages/browser-pool/src/remote-browser-pool.ts).

### Impit : Rust sans réécrire le produit en Rust

Apify recommande maintenant `ImpitHttpClient` pour les nouveaux projets, et indique que got-scraping n'est plus activement maintenu. Impit réalise l'impersonation HTTP et TLS, propose HTTP/3 en option, et fournit des binaires natifs Node pour Windows/macOS/Linux. La licence du dépôt et du package npm est Apache-2.0 ; npm observé : `impit` 0.14.5, `@crawlee/impit-client` 3.18.2. [Guide officiel](https://crawlee.dev/js/docs/guides/impit-http-client), [code Impit](https://github.com/apify/impit), [licence](https://github.com/apify/impit/blob/master/LICENSE.md).

Cela rend faible l'argument « le contrôleur TS serait forcément lent ». Un SDK TS peut déjà déléguer son transport à un moteur natif. La question utile est la part CPU/mémoire/latence de chaque étape, avec un vrai navigateur et un vrai réseau.

## Spider et Chromey : références de bas niveau, coût d'intégration à mesurer

Spider publié sur crates.io : **2.53.9**, 5 septembre 2026. La branche principale annonce 2.54.0. Chromey publié : **2.58.2**, 12 septembre 2026 ; la branche annonce 2.58.3. Les contenus des archives publiées ont été examinés, dont le smart mode et le contrôleur de concurrence. [Spider crate](https://crates.io/crates/spider/2.53.9), [source publiée](https://docs.rs/crate/spider/2.53.9/source/), [Chromey crate](https://crates.io/crates/chromey/2.58.2).

Le moteur ne se réduit pas à « reqwest + un parseur ». Les features incluent Chrome, interception des requêtes, caches hybrides, streaming, budgets, mode distribué, contrôles d'exécution, emprunts de page, WebDriver et plusieurs chemins de collecte. `RemoteFetcher` permet de remplacer le round-trip HTTP en conservant le scheduling ; son contrat précise que le hook s'applique au chemin HTTP et que le fournisseur reste responsable de sa cancellation. [Manifest](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml), [contrat RemoteFetcher](https://github.com/spider-rs/spider/blob/main/spider/src/fetcher.rs).

Son `smart` mode utilise notamment des signaux heuristiques : texte `noscript`, scripts qui mutent le DOM, requêtes de données, marqueurs Next/Nuxt/Gatsby et attributs de frameworks. Ces signaux alimentent un score d'upgrade vers Chrome. C'est différent du modèle Crawlee qui compare le résultat de l'extraction. Une page React rendue côté serveur peut contenir des marqueurs d'hydratation sans que le navigateur soit nécessaire pour le résultat demandé ; inversement, une extraction HTTP apparemment plausible peut manquer une donnée. Ce sont des cas à mesurer, pas des échecs établis ici. [Détection dans page.rs](https://github.com/spider-rs/spider/blob/main/spider/src/page.rs#L1876), [traitement smart](https://github.com/spider-rs/spider/blob/main/spider/src/page.rs#L8305).

La feature Cargo nécessaire est **`smart`**, qui active Chrome et l'interception. Appeler `crawl_smart()` sans cette feature suit un autre chemin compilé qui revient à `crawl()`. La README de la branche principale indique dans un passage d'activer seulement `chrome` ; la README incluse dans la crate publiée documente correctement `smart`. Pour une intégration, le manifest et le code compilé priment sur cette formulation simplifiée. [Feature flags](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml#L312), [variantes conditionnelles](https://github.com/spider-rs/spider/blob/main/spider/src/website.rs#L7625).

La concurrence adaptative n'est pas seulement un entier configurable : un contrôleur **AIMD** et un `AdaptiveSemaphore` sont présents. Le contrat expose une augmentation progressive et une diminution multiplicative, ainsi qu'un pont atomique vers le sémaphore vivant. L'application peut piloter ce mécanisme à partir des résultats observés ; la seule existence des types ne prouve pas un autoscaling optimal sur tout workload. [Source](https://github.com/spider-rs/spider/blob/main/spider/src/utils/adaptive_concurrency.rs).

Chromey est un fork de chromiumoxide. Il garde le nom de bibliothèque `chromiumoxide` tout en publiant le package `chromey`, et ajoute des composants de fingerprint, réseau, blocage, cache et contrôles du navigateur. Sa licence Cargo est `MIT OR Apache-2.0`, même si GitHub affiche Apache dans ses métadonnées. Les crates Spider fingerprint/firewall/network blocker examinées annoncent MIT. [README](https://github.com/spider-rs/chromey/blob/main/README.md), [Cargo](https://github.com/spider-rs/chromey/blob/main/Cargo.toml), [upstream](https://github.com/mattsse/chromiumoxide).

Le coût caché d'un fork profond est la matrice de compilation et la synchronisation avec CDP et Chrome. Le code principal Spider est volumineux et comporte de nombreuses branches de features : `website.rs` examiné approche 18 500 lignes. Cette taille n'établit pas une mauvaise qualité, mais reprendre tout ce système impose de suivre une surface technique plus large que le besoin d'Automan. Une équipe d'agents accélère l'implémentation ; elle ne supprime ni les courses entre events, ni la validation sur Chrome réel, ni les changements de fingerprint. Choisir un adapter étroit ou contribuer upstream préserve une frontière de responsabilité.

### Choisir une base Rust sans reprendre toute la pile Spider

**Chromiumoxide 0.9.1** est publié depuis le 25 février 2026 et porte aussi `MIT OR Apache-2.0`. **rust-headless-chrome 1.0.22**, publié le 11 juin 2026, porte MIT et constitue une autre référence, avec une ergonomie et un modèle d'exécution à comparer aux besoins Tokio. [Chromiumoxide](https://crates.io/crates/chromiumoxide/0.9.1), [rust-headless-chrome](https://crates.io/crates/headless_chrome/1.0.22).

Le choix initial peut être : **Chromiumoxide pour établir une baseline CDP stock propre ; Chromey comme candidat si ses fonctions de réseau, cache et streaming apportent un gain mesuré ; Spider comme adapter de crawl plutôt que comme fork global du runtime.** Les deux premiers descendent de la même famille et nécessitent de tester leurs versions réellement publiées. Le fork Chromey semble plus fréquemment publié à cette date ; le nombre de stars ou la fréquence des tags ne mesure pas le taux de succès des actions.

Dans les deux drivers, le chemin `Element::click()` inspecté effectue `scroll_into_view`, recherche un point cliquable, puis envoie le clic. Ce seul chemin ne constitue pas un contrat équivalent aux locators et à l'actionability de Playwright. **Le travail important pour Automan est au-dessus du driver CDP :** résolution du locator au moment de l'action, visibilité/activation/stabilité, hit testing, frames et cibles hors processus, shadow DOM, armer les attentes avant le clic, gérer les éléments détachés, puis contrôler l'état résultant. Il faut vérifier la couverture totale des drivers avant d'affirmer que tous ces mécanismes sont absents ; l'inspection établit seulement que la méthode click citée ne les garantit pas tous. [Chromey Element](https://github.com/spider-rs/chromey/blob/main/src/element.rs#L257), [Chromiumoxide Element](https://github.com/mattsse/chromiumoxide/blob/main/src/element.rs#L269).

**Autre frontière concrète : les méthodes vendor de Chromey ne sont pas des fonctions garanties de Chrome stock.** `Content.getMarkdown` et `WebMCP.listTools/callTool` appartiennent explicitement à des domaines CDP non standard exposés par certains moteurs distants. La bibliothèque documente le cas unsupported et le fallback. Le sidecar doit négocier des capacités ; ni la présence des bindings ni le nom WebMCP ne prouve un support local standard. [Content domain](https://github.com/spider-rs/chromey/blob/main/src/content_markdown.rs), [WebMCP domain](https://github.com/spider-rs/chromey/blob/main/src/webmcp.rs).

**spider-browser est autre chose** : le SDK TypeScript se connecte par défaut à `wss://browser.spider.cloud`, et le README Rust décrit explicitement un client de flotte cloud. Les méthodes de pages et d'IA, le retry et l'escalade de proxy sont exposés dans l'offre ; le dépôt SDK MIT ne constitue pas à lui seul un moteur de flotte auto-hébergeable. Le terme « Rust-powered browsers » ne signifie pas que Chromium est réimplémenté en Rust. [SDK TypeScript](https://github.com/spider-rs/spider-browser/blob/main/typescript/spider-browser.ts), [README Rust](https://github.com/spider-rs/spider-browser/blob/main/rust/README.md).

## Les autres références majeures

### Scrapy

Version PyPI observée : **2.19.0**, publiée le 10 septembre 2026. Les sources montrent le moteur, les frontières scheduler/downloader/scraper et la backpressure. Le scheduler combine file mémoire/disque et filtre de doublons ; AutoThrottle ajuste le délai à partir de la latence et de la concurrence cible. La branche récente comporte aussi des API async : la résumer uniquement par « ancien Twisted » manquerait son évolution. [Moteur](https://github.com/scrapy/scrapy/blob/master/scrapy/core/engine.py), [scheduler](https://github.com/scrapy/scrapy/blob/master/scrapy/core/scheduler.py), [throttle](https://github.com/scrapy/scrapy/blob/master/scrapy/extensions/throttle.py), [package](https://pypi.org/project/Scrapy/).

Son intérêt pour Automan est la discipline des frontières, des hooks et des files persistantes. Le rendu navigateur nécessite une intégration adaptée ; il n'est pas équivalent à une boucle d'agent ou à une chaîne d'actions compilée.

### Crawl4AI

Version publiée observée : **0.9.4**, 23 septembre 2026. Sa force est l'acquisition et la transformation du Web en contenu propre pour les modèles, avec extraction déterministe possible, sessions et CDP. Le code gère la réutilisation de connexions Playwright/CDP, les contextes et les sessions. Le dispatcher protège la mémoire avec des seuils, une hystérésis et une limite de sessions. Ces éléments sont à étudier avant de proposer soi-même « pooling » ou « memory-aware concurrency » comme nouveauté. [BrowserManager](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/browser_manager.py), [dispatcher](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/async_dispatcher.py), [stratégie](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/async_crawler_strategy.py).

Son « adaptive crawling » inclut une recherche guidée par couverture et pertinence, distincte du choix HTTP/browser de Crawlee. Le projet fournit aussi **C4A-Script**, un DSL d'interactions avec outils visuels et enregistrement : un DSL/replay n'est donc pas un espace vide. [Code adaptatif](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/adaptive_crawler.py), [documentation C4A-Script](https://github.com/unclecode/crawl4ai/blob/main/docs/md_v2/core/c4a-script.md).

**Point de licence concret :** le fichier LICENSE commence par Apache, puis ajoute une obligation d'attribution pour distributions, publications et usages publics, avec visibilité requise. Le README dit à la fois attribution recommandée et obligatoire. Éviter de traiter ce dépôt comme Apache-2.0 standard sans examiner cette addition lors d'une réutilisation. [LICENSE](https://github.com/unclecode/crawl4ai/blob/main/LICENSE#L54), [README](https://github.com/unclecode/crawl4ai/blob/main/README.md#L372).

### Firecrawl

Le dernier GitHub Release retourné par l'API était **v2.11.0**, 19 juin 2026 ; les sources de branche principale sont plus récentes. Firecrawl est une référence produit/API et extraction multi-format, mais son cœur est AGPL-3.0, tandis que les SDK et certains composants sont MIT. Les licences ne se déduisent pas du seul client npm/Python. [README/licences](https://github.com/firecrawl/firecrawl/blob/main/README.md#L886), [release](https://github.com/firecrawl/firecrawl/releases/tag/v2.11.0).

La branche examinée choisit des moteurs selon leurs capacités et plusieurs priorités : fetch, Playwright, moteurs fire-engine/CDP/TLS et parsers spécialisés. L'intégration fire-engine utilise un service distant ; son moteur n'est pas implémenté dans les fichiers wrapper examinés. Le README distingue explicitement cloud et open source. Une reproduction auto-hébergée ne doit donc pas être réputée équivalente au cloud sans preuve. [Registry de moteurs](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/scraper/scrapeURL/engines/index.ts), [wrapper fire-engine](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/scraper/scrapeURL/engines/fire-engine/index.ts).

Le SELF_HOST de branche principale annonce NuQ PostgreSQL pour la queue et scraper-microservice comme moteur navigateur par défaut, avec Redis/RabbitMQ et d'autres services du Compose. Décrire systématiquement l'architecture actuelle comme BullMQ serait périmé. Ces observations portent sur `main`, pas une validation d'installation de la release 2.11.0. [SELF_HOST](https://github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md).

### Scrapling

Version PyPI : **0.4.15**, 23 août 2026 ; dépôt BSD-3-Clause. Trois préoccupations sont séparables : parsing et récupération d'éléments, transport HTTP, transport navigateur. `StealthyFetcher` importe Patchright et ajoute une logique de challenges ; `Fetcher` utilise les sessions de transport. La réussite sur des protections changeantes ne se déduit pas de la présence de ces fonctions. [Fetchers](https://github.com/D4Vinci/Scrapling/tree/main/scrapling/fetchers), [stealth engine](https://github.com/D4Vinci/Scrapling/blob/main/scrapling/engines/_browsers/_stealth.py).

L'adaptivité ici signifie **retrouver un élément après une modification du HTML**, à partir de signatures conservées, sans nécessiter un LLM. C'est un concurrent direct de la partie « auto-healing des sélecteurs ». Il faut vérifier que l'élément retrouvé conserve le rôle et la valeur attendus ; similarité structurelle et correction métier sont deux propriétés. [Documentation adaptative](https://github.com/D4Vinci/Scrapling/blob/main/docs/parsing/adaptive.md).

Le framework spiders possède déduplication, checkpoints et reprise, quotas par domaine et AutoThrottle. Ce dernier ajuste les délais et tient compte des blocages et de Retry-After. Les benchmarks du README sont des benchmarks **de parsing / recherche d'éléments**, pas des résultats d'agents complets. [Scheduler](https://github.com/D4Vinci/Scrapling/blob/main/scrapling/spiders/scheduler.py), [engine](https://github.com/D4Vinci/Scrapling/blob/main/scrapling/spiders/engine.py), [benchmarks](https://github.com/D4Vinci/Scrapling/blob/main/benchmarks.py).

### ScrapeGraphAI

Version PyPI : **2.3.1**, 6 octobre 2026. `SmartScraperGraph` assemble FetchNode, ParseNode, GenerateAnswerNode et des branches optionnelles de raisonnement/régénération. La valeur est le passage d'une consigne à une extraction, avec modèles au centre ; ce n'est pas automatiquement le meilleur chemin pour un travail connu et répété, dont l'extraction peut être compilée. Le dépôt OSS est MIT ; les services ScrapeGraphAI associés ne se confondent pas avec l'intégralité du code de ce dépôt. [Graphe inspecté](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/main/scrapegraphai/graphs/smart_scraper_graph.py), [README](https://github.com/ScrapeGraphAI/Scrapegraph-ai), [LICENSE](https://github.com/ScrapeGraphAI/Scrapegraph-ai/blob/main/LICENSE).

## Acteurs et briques de niche qu'il serait dommage d'ignorer

| Outil | Pourquoi l'étudier | Réserve / frontière |
|---|---|---|
| [Colly](https://github.com/gocolly/colly) | Callbacks Go simples, limites par domaine, cookies, cache et transport personnalisable | HTTP-centric. Appel public à mainteneurs en mars 2025 ; activité récente ≠ absence de risque de maintenance. [Issue](https://github.com/gocolly/colly/issues/840) |
| [Rod](https://github.com/go-rod/rod) | CDP Go, pages/events/attentes, interface directe | Dernier tag retourné par proxy Go : v0.116.2, juillet 2024, malgré activité du dépôt en août 2026 |
| [chromedp](https://github.com/chromedp/chromedp) | Contextes Go et tâches CDP ; utile pour comparer simplicité, lifecycle et coût | Chrome-centric, pas une orchestration universelle. Proxy Go v0.20.1 daté du 5 octobre 2026 |
| [Geziyor](https://github.com/geziyor/geziyor) | Go HTTP + rendu JS, extraction, export et concurrence | MPL-2.0 ; dernière version proxy Go observée est une pseudo-version d'août 2024. Examiner version et fichiers modifiés avant réutilisation |
| [Ferret](https://github.com/MontFerret/ferret) | DSL, plans réutilisables, VM bytecode, exécutions isolées, capacités exposées par l'hôte | `main` = v2 alpha ; stable v1 sur branche `v1`. Une inspiration de compilation/replay, pas une dépendance v2 stable acquise |
| [Katana](https://github.com/projectdiscovery/katana) | HTTP et headless hybrid, découverte des endpoints JS/XHR, scope et remplissage de formulaires | Produit orienté pipelines de reconnaissance ; headless annoncé expérimental. MIT. [Docs](https://docs.projectdiscovery.io/opensource/katana/usage) |
| [Apache StormCrawler](https://github.com/apache/stormcrawler) | Frontier distribuée, flux continu, partitionnement par hôte, feed-back de statut, Playwright/Tika/WARC | Excellent modèle pour grande échelle, dépendance opérationnelle Storm trop lourde pour un premier moteur d'actions. Apache-2.0. [Architecture](https://github.com/apache/stormcrawler/blob/main/docs/src/main/asciidoc/architecture.adoc) |
| [Heritrix](https://github.com/internetarchive/heritrix3) | Crawl d'archivage à grande échelle, qualité d'archives | Finalité différente d'une automatisation applicative, Apache-2.0 |
| [Botasaurus](https://github.com/omkarcloud/botasaurus) / [driver](https://github.com/omkarcloud/botasaurus-driver) | Décorateurs browser/request/task, sessions, rendu et fetch depuis un navigateur, packaging d'un scraper | MIT. Les promesses d'accès à tous les sites et de réduction massive de coût sont des affirmations vendeur, pas un benchmark indépendant |
| [MechanicalSoup](https://github.com/MechanicalSoup/MechanicalSoup) | Montre qu'un parcours formulaires peut fonctionner avec Requests + parsing, sans Chrome | Ne fait pas de rendu JavaScript ; MIT. Utile comme baseline minimale |
| [DrissionPage](https://github.com/g1879/DrissionPage) | CDP et approche hybride HTTP/navigateur ; API ergonomique | **LICENSE actuel interdit l'usage commercial sans autorisation.** Source disponible ne signifie pas OSS permissif. [Licence](https://github.com/g1879/DrissionPage/blob/master/LICENSE) |
| [nodriver](https://github.com/ultrafunkamsterdam/nodriver) | Driver Python async directement en CDP, sans Selenium/Chromedriver | AGPL-3.0. Version PyPI 0.50.5 du 5 octobre 2026 ; promesses de furtivité à mesurer |
| [Zendriver](https://github.com/cdpdriver/zendriver) | Fork nodriver orienté corrections et participation communauté | AGPL-3.0, PyPI 0.17.1 du 2 octobre 2026 ; README documente la raison de fork et le besoin de traiter bugs/PR |
| [Selenium-Driverless](https://github.com/kaliiiiiiiiii/Selenium-Driverless) | CDP, interception, fetch de requêtes, API familière | **CC BY-NC-SA avec addition commerciale**, licence commerciale nécessaire pour l'objectif envisagé. [Licence](https://github.com/kaliiiiiiiiii/Selenium-Driverless/blob/master/LICENSE.md) |
| [BrowserForge](https://github.com/daijro/browserforge) | Génération cohérente headers/fingerprints, réimplémentation Python du fingerprint-suite Apify | Brique d'identité ; une génération de fingerprint seule ne garantit pas une session non détectable |
| [wreq](https://github.com/0x676e67/wreq), [curl_cffi](https://github.com/lexiforest/curl_cffi), [Impit](https://github.com/apify/impit) | Transport et impersonation sans navigateur | Comparer selon plateformes, profils et maintenance ; pas des moteurs de rendu |
| [scrapling-rs](https://github.com/chrisabruce/scrapling-rs) | Port Rust de parsing adaptatif et transport wreq, concurrent très récent | Documentation primaire découverte ; implementation, conformité aux comportements Python, maintenance et licence complète non auditées ici |
| [ScrapeForge](https://scrapeforge.io/docs/guides/anti-bot-bypass) / [StealthPlex](https://github.com/SSujitX/StealthPlex) | Escalade de moteurs et gestion de fingerprints déjà proposées par des niches | Couverture documentaire de découverte ; pas d'évaluation d'exécution ni de audit de toutes dépendances/licences |

Les claims « undetectable », « all websites » ou « Rust = fastest » sont récurrents. Les considérer comme des positionnements commerciaux à tester, jamais comme des propriétés établies du logiciel.

## Ce que les benchmarks existants permettent réellement de dire

L'exemple Impit est utile parce que le protocole est explicite : médiane de 11 passages, 2 000 requêtes séquentielles vers un serveur HTTP/2 local, réponse JSON de 1 KiB, une connexion chaude. Il compare des bibliothèques de transport, y compris Python et Node. Cette expérience peut informer une optimisation de client ; elle ne mesure ni TCP/TLS froid en Internet, ni proxies résidentiels, ni exécution JS, ni coût de modèles, ni précision d'une extraction. [Méthodologie](https://github.com/apify/impit/blob/master/README.md#L37).

Le README Scrapling compare le parseur et la localisation d'éléments sur des boucles répétées. La proximité de Parsel et Scrapling dans une tâche donnée ne prédit pas la durée d'un crawl protégé. [Benchmarks](https://github.com/D4Vinci/Scrapling/blob/main/README.md#L479).

Le langage du contrôleur peut réduire sa mémoire et sa consommation CPU. En revanche, le navigateur exécute le même moteur Chromium, les sites ont des latences variables et les appels LLM peuvent dominer le chemin critique. Il faut profiler ces parts avant de transformer une optimisation de microbenchmark en projet de réécriture.

## Direction concrète pour l'architecture cible

1. **Contrat de tâche au centre.** Une tâche reçoit un résultat attendu explicite : schéma de données, contraintes de complétude, rôle de l'élément, état observé avant/après une action. Les moteurs renvoient des faits et des artefacts, pas seulement « succès ».
2. **Deux catégories de travail.** L'acquisition/extraction autorise les essais parallèles ou les doubles exécutions comparatives. Les actions sur une application requièrent un journal et des règles de reprise adaptés à leurs effets. Garder cette distinction dans le modèle, sans multiplier les produits.
3. **Moteur canonique Rust ; SDK TS/Python à partir d'un protocole commun.** Un sidecar porte les sessions, transports, observations et actions ; les SDK n'en réimplémentent pas les invariants. Des commandes typées et versionnées exposent cancellation, deadlines, progression, résultat et artefacts. Les flux volumineux restent diffusés, et les erreurs transport/protocole/tâche sont distinctes. Le premier transport local et distant doit être le même contrat, avec l'overhead mesuré avant d'ajouter un chemin FFI différent.
4. **Adapters étroits et orchestration séparée.** HTTP natif et CDP natif constituent le chemin cible ; Playwright qualifié sert de backend de compatibilité et de référence de comportement. Une API de workflow orchestre les étapes et leur journal sans imposer une queue de crawl à tout parcours. Un adapter Spider Rust et, côté écosystème, un adapter Crawlee doivent rester facultatifs. Reprendre les idées de leurs queues/sessions/backpressure ; éviter d'embarquer deux schedulers propriétaires de la même tâche.
5. **Politique d'exécution contextualisée.** Apprendre par site, route et résultat demandé ; HTTP-only quand la qualité est validée, navigateur direct lorsque l'historique le justifie. Un fallback systématique HTTP puis browser pénalise les pages connues comme JS-only.
6. **LLM pour découvrir et réparer.** Une fois un parcours établi, exécuter les recettes déterministes avec contrôle des postconditions. Déclencher une réparation sur échec diagnostiqué ; conserver version de recette, modèle utilisé et justification de la modification. Compilation/replay existe déjà, donc la qualité vérifiée et l'expérience de maintenance doivent porter la différenciation.
7. **Pool et budgets dès le début.** Garder une identité cohérente cookies/proxy/profile dans une session, limiter concurrence et ressources par site, prévoir un délai total de tâche et un budget de retry. Pool réutilisable ne signifie pas mélanger les sessions de clients.
8. **Artefacts de diagnostic.** Événements typés, raisons de fallback, temps par étape, sorties parsées, screenshot ou DOM nécessaires au diagnostic. Masquer ce qui n'est pas nécessaire à la compréhension de l'utilisateur final.

Le modèle économique peut vendre des sessions gérées, capacité garantie, observabilité, maintien des recettes, proxies et support. Le framework gratuit doit pouvoir exécuter un workflow réel localement pour que l'offre de service ait un avantage observable, sans confondre SDK gratuit et produit open source complet.

### Ordre de réalisation et gates de validation

| Étape | Livrable | Vérification qui décide de la suite |
|---|---|---|
| Baseline et contrat | Corpus représentatif + instrumentation, baseline Playwright/Crawlee/Spider, protocole de sidecar | Qualité des résultats, latence p50/p95, temps froid/chaud, taux de succès et coût par tâche réellement réussie |
| Exécuteurs natifs | Rust HTTP/CDP, SDK générés, résultats typés, sessions et trace, premières recettes | Parité des cas critiques avec Playwright ; profil CPU/mémoire ; qualité équivalente sur les mêmes workloads |
| Fiabilité du workflow | Journal de reprise, réparation ciblée, adapter de crawl, packaging local | Reprises après crash/timeouts, absence de duplications observées sur cas représentatifs, diagnostics exploitables |
| Services gérés | Sessions à distance, hébergement, observabilité, maintenance de recettes | Coût total d'exploitation et temps de résolution comparés à l'utilisation directe des outils existants |

**Gate de promotion du chemin natif :** Rust est le choix de la cible, mais chaque remplacement de comportement Playwright doit démontrer ses effets sur un corpus à qualité égale. Garder le fallback quand un cas est incomplet. Le profiling doit distinguer contrôle, parsing, transport, moteur Chromium, réseau et LLM ; optimiser la part dominante, avec coûts de packaging et maintenance inclus. Aucun pourcentage magique ni facteur « 10× » n'est justifié avant cette expérience. Cette architecture n'implique pas de réécrire Chromium, qui reste un processus navigateur externe.

Un corpus raisonnable doit mélanger HTTP statique, HTML serveur avec hydration, JS-only, navigation avec authentification, formulaires, pagination, téléchargements, limites de débit et sessions longues. Tester une modification de DOM et un arrêt entre action et validation révèle des différences invisibles dans un simple tableau de pages par seconde. Séparer les échecs de qualité, de réseau, de protection, de driver et de reprise.

## Incertitudes restantes

- Aucune bibliothèque auditée n'a été testée ici sur le corpus Automan : classement de vitesse et fiabilité non établi.
- Les fonctionnalités cloud Spider/Firecrawl/Crawl4AI n'ont pas été exercées ; parité OSS/cloud non établie.
- La compatibilité Crawlee Stagehand 3 avec Stagehand 4 est absente de la déclaration npm examinée ; un adapter propre au produit pourrait être nécessaire.
- Le bon périmètre de copyleft dépendra de ce qui sera effectivement combiné, distribué et proposé en réseau ; ce dossier constate les licences, sans résoudre une combinaison de code qui n'existe pas encore.
- Les imports et versions non verrouillés, et notamment les forks de drivers, doivent être validés sur une matrice Chrome/OS réelle. La fréquence des mises à jour ne remplace pas les tests de compatibilité.
