# Marché commercial de l’automatisation web — 7 octobre 2026

> **Mise à jour du périmètre :** l'utilisateur a retiré la contrainte de petite équipe et de sortie en 3–6 mois après la première rédaction. Les passages de planning et TS-first ci-dessous portent sur cette hypothèse initiale. La cible actuelle est un moteur Rust commun aux SDK, à la CLI et au cloud ; voir [la synthèse mise à jour](./2026-10-07-direction-automan.md). La cartographie, les tarifs relevés et les réserves restent applicables.

## Conclusion pour une petite équipe, MVP en 3–6 mois

**Le créneau défendable n’est pas « un navigateur IA plus rapide ». C’est un framework open source qui transforme trois workflows B2B récurrents en automatisations portables, contrôlées et économiques, puis des services payants pour les mettre en production et les maintenir.** Le premier segment proposé est celui des développeurs, petites agences et intégrateurs SaaS qui collectent des données dans des portails fournisseurs/distributeurs authentifiés.

Cette conclusion est une **inférence stratégique**, pas une demande de marché déjà prouvée. Elle doit être testée auprès de clients. L’infrastructure navigateur, les primitives IA, le Markdown, les proxys, MCP et la réparation de sélecteurs sont déjà largement disponibles. L’avantage doit apparaître sur des tâches répétées et vérifiées, avec un coût total et un temps de maintenance inférieurs.

Le marché converge déjà vers l’IA qui construit le workflow, du code déterministe qui le rejoue, et l’IA qui intervient lors des changements. [Airtop décrit explicitement cette compilation](https://www.airtop.ai/) ; [Anchor décrit des tâches déterministes et un cache d’actions](https://anchorbrowser.io/). Ce schéma est utile, mais ne suffit donc pas à établir une nouveauté commerciale.

## Méthode et portée

- Sources commerciales officielles et dépôts des éditeurs privilégiés. Les déclarations de performances des fournisseurs sont des déclarations, pas des résultats comparatifs validés par cette recherche.
- `sources_reviewed = 155` : somme des `numResults` demandés dans **13 recherches Exa**, conformément à la convention du skill ; ce nombre n’est ni celui de sources indépendantes ni celui de pages intégralement lues.
- **93 URLs uniques demandées en lecture directe, 88 retournées et inspectées, 5 échecs de lecture.** Les recherches couvrent cinq angles initiaux, six approfondissements et deux passes de lacunes.
- **64 acteurs, produits ou familles recensés** ci-dessous ; un produit retiré est isolé comme référence historique. Apify/Crawlee et Browserbase/Stagehand sont volontairement réunis pour éviter les doubles comptes commerciaux. Robocorp/Sema4.ai et TestMu AI/LambdaTest sont également regroupés.
- Cette cartographie cherche une couverture large, du fournisseur d’infrastructure au produit vertical et aux alternatives locales. Elle ne revendique pas l’exhaustivité absolue : le marché bouge, les projets individuels et les services de niche sont nombreux.
- « Service » signifie que le produit commercial documenté est hébergé/propriétaire ; ce n’est pas une affirmation qu’aucun code client ou outil annexe n’est public. Un SDK public ne rend pas le moteur hébergé open source. Les licences non inspectées sont indiquées comme telles.
- Tarifs en USD, observés dans les sources publiques le 7 octobre 2026, sans négociation, fiscalité ou tests de facturation. Les prix ne sont pas comparables sans fixer les mêmes ressources, réseau, modèles et critères de succès.

## 1. Navigateurs distants et agents hébergés

| Acteur / source officielle | Produit et différenciation documentés | Frontière open source / service |
|---|---|---|
| [Browserbase / Stagehand](https://docs.browserbase.com/welcome/introduction) | Plateforme qui regroupe navigateurs, recherche, fetch, runtime et accès aux modèles ; Stagehand ajoute les primitives d’action, observation et extraction. | Stagehand et CLI publics ; service de flotte et plateforme commerciale séparés. |
| [Browser Use](https://browser-use.com/) | Deux produits : agent qui réalise la tâche et navigateur distant pour son propre agent. | Framework MIT ; le [README distingue agent OSS et agent cloud](https://github.com/browser-use/browser-use), sans promettre leur équivalence. |
| [Steel](https://steel.dev/) | Runtime de navigateur, sessions/profils persistants, credentials, fichiers, extensions, observation et connexions aux frameworks usuels. | [steel-browser Apache-2.0](https://github.com/steel-dev/steel-browser), auto-hébergeable ; exploitation cloud payante. |
| [Browserless](https://www.browserless.io/) | Connexions WebSocket, BrowserQL, REST et MCP ; déploiement dédié ou chez le client. | **Source disponible, licence SSPL ou commerciale**, selon le [LICENSE](https://raw.githubusercontent.com/browserless/browserless/main/LICENSE). Ne pas assimiler à un composant permissif librement réutilisable dans un SaaS propriétaire. |
| [Hyperbrowser](https://hyperbrowser.ai/docs/introduction) | Navigateurs Chrome cloud, SDK et compatibilité Puppeteer/Playwright ; [agents hébergés](https://hyperbrowser.ai/docs/agents/overview) et intégration Stagehand. | HyperAgent annoncé open source ; flotte gérée commerciale. Licence du framework non inspectée ici. |
| [Anchor](https://anchorbrowser.io/) | Identités, authentification, environnements de computer use, exécution déterministe et déploiement dans le cloud du client. | Service ; ne pas confondre SDK public et noyau d’infrastructure. |
| [Airtop](https://www.airtop.ai/) | Construit et teste des workflows, les compile en code réutilisable et les exécute à la demande ou sur calendrier ; positionnement fonctions finance, marketing et ventes. | Service et intégrations ; noyau OSS non établi dans ce corpus. |
| [KERNEL](https://www.kernel.sh/) | Sessions Chromium isolées, profils, managed auth, pools chauds, CDP/BiDi, replays et outils de processus/fichiers. | Service de calcul/navigateur ; SDK et CLI distincts du moteur hébergé. |
| [Notte](https://www.notte.cc/) | Sessions, agents et fonctions serveur, personas, coffre de secrets et replay ; transforme un workflow navigateur en API. | [Composants publics](https://github.com/nottelabs/notte), service hébergé ; frontière/licence à inspecter avant réutilisation. |
| [TinyFish](https://www.tinyfish.ai/) | Recherche, fetch, browser et agent ; authentification via coffre et profils, enregistrements, enrichissement et automation de portails. | Service ; les benchmarks ou SDK ouverts ne rendent pas toute l’offre OSS. |

## 2. Collecte, extraction, proxys et déblocage

| Acteur / source officielle | Proposition de valeur documentée | Frontière open source / service |
|---|---|---|
| [Apify / Crawlee](https://apify.com/pricing) | Exécution d’Actors, stockage, proxys et marketplace ; monétisation et distribution de connecteurs. | [Crawlee Apache-2.0](https://github.com/apify/crawlee), bibliothèque utilisable hors plateforme ; Actors et exploitation ne sont pas tous publics. |
| [Spider Cloud / spider-rs](https://spider.cloud/open-source-vs-cloud/) | Même moteur de crawl Rust entre local et cloud ; le service prend en charge flotte, réseau, anti-bot et extraction. | [Écosystème MIT](https://spider.cloud/open-source/) ; les services gérés et leur parité ne se déduisent pas de la licence. |
| [Firecrawl](https://www.firecrawl.dev/pricing) | Recherche, scrape, crawl, map, extraction et interaction, sorties prêtes pour LLM et plans de volume. | [Moteur AGPL-3.0](https://github.com/mendableai/firecrawl), service commercial ; la parité d’auto-hébergement doit être testée endpoint par endpoint. |
| [Bright Data](https://brightdata.com/products) | Proxy/réseau, Web Unlocker, Browser API, scrapers par cible, datasets et services gérés. | Offre propriétaire ; distribution de données et capacité d’accès sont les actifs majeurs. |
| [Oxylabs](https://oxylabs.io/products) | Proxys, Web Unblocker, Scraper API, navigateur, recherche et données vidéo ; outillage OxyCopilot. | Offre propriétaire ; intégration possible avec des frameworks OSS. |
| [Zyte](https://www.zyte.com/zyte-api/) | Routing automatique entre HTTP et navigateur, gestion des bannissements, sessions et extraction structurée. | Scrapy constitue un actif OSS distinct ; Zyte API et services opérés sont commerciaux. |
| [ZenRows](https://www.zenrows.com/) | Plateforme Fetch, Extract, Browser Sessions et Batch avec CLI/MCP/SDK ; anti-bot géré. | Service ; outils d’intégration distincts de la flotte. |
| [Scrapfly](https://scrapfly.io/) | API scrape, navigateur, screenshot, extraction, crawler, agent et MCP ; observabilité et moteurs réseau/navigateur opérés. | Curlium et Scrapium présentés comme propriétaires ; SDK et intégrations distincts. |
| [ScraperAPI](https://www.scraperapi.com/) | API générale et endpoints structurés Amazon, Google et Walmart ; pipeline de collecte et traitement asynchrone. | Service d’infrastructure et parsing. |
| [ScrapingBee](https://www.scrapingbee.com/llms/html-api.txt) | API URL avec JS, scénarios, screenshots, règles et extraction ; escalade automatique du coût avec plafond. | Service ; les clients publics ne constituent pas un moteur complet auto-hébergeable. |
| [ScrapingAnt](https://scrapingant.com/) | API Chrome, Markdown et extraction IA, MCP, proxys autonomes résidentiels/datacenter. | Service ; API credits et bande passante proxy distincts. |
| [Nimble](https://www.nimbleway.com/) | Recherche et collecte pour agents, plans de recherche auditables, mémoire/index et monitoring ; scénarios métier spécialisés. | Service de retrieval et données. |
| [Crawlbase](https://crawlbase.com/docs) | Crawling API, parsing par cible, rendu navigateur et MCP ; clients multilangages. | Service ; le Scraper API autonome est fermé aux nouveaux comptes depuis 2024, [la fonction est portée par Crawling API](https://crawlbase.com/docs/scraper-api). |
| [HasData](https://hasdata.com/apis/web-scraping-api) | API générale et SERP, HTML/Markdown/JSON, extraction déterministe ou IA, screenshots et options réseau. | Service ; catalogue d’endpoints spécialisés. |
| [Scrapingdog](https://www.scrapingdog.com/) | Scraping général et APIs ciblées SERP, ecommerce et plateformes sociales. | Service ; intérêt de connecteurs très ciblés. |
| [Scrape.do](https://scrape.do/) | API générale et scrapers prêts à l’emploi, anti-bot et interactions JS. | Service ; homepage indique navigateur/proxys autonomes « Coming Soon », donc ne pas compter ces deux produits comme disponibles sur cette seule preuve. |
| [Scrapeless](https://www.scrapeless.com/en) | Agent Browser, Web Unlocker, Crawl, réseau et extraction de réponses des moteurs IA. | Service ; créneau distinct d’observation de ce que répondent ChatGPT, Perplexity, etc. |
| [Scraping Fish](https://scrapingfish.com/) | Navigateurs et proxys mobiles avec un prix identique par requête réussie, sans multiplicateurs de fonctionnalités annoncés. | Service ; la simplicité budgétaire constitue son positionnement. |
| [ScrapeNinja](https://scrapeninja.net/) | Moteur HTTP avec fingerprint Chrome, moteur navigateur, retries, playgrounds et extracteurs JS. | Service ; démontre que deux chemins HTTP/navigateur sont déjà un produit courant. |

## 3. Données sémantiques, retrieval et plateformes verticales

| Acteur / source officielle | Ce que le client achète | Frontière open source / service |
|---|---|---|
| [Diffbot](https://www.diffbot.com/) | Extraction structurée, crawl, entités, knowledge graph et recherche, dont offre d’index auto-hébergeable annoncée. | Service et produit de données ; ne pas réduire à un convertisseur Markdown. |
| [Kadoa](https://www.kadoa.com/) | Pipelines pour la finance, maintenance automatisée, règles de qualité et provenance par valeur ; équipe opérationnelle lors des réparations incertaines. | Produit vertical et services ; on-prem/private cloud proposés. |
| [AgentQL](https://www.agentql.com/) | Langage de requêtes sémantiques, sélection/extraction via Playwright, debugger et API, y compris parsing PDF. | SDK/query language publics n’impliquent pas un service de résolution entièrement OSS. |
| [ScrapeGraphAI](https://github.com/ScrapeGraphAI/Scrapegraph-ai) | Pipelines d’extraction à partir de prompts et graphes ; offre cloud distincte annoncée dans le README. | Bibliothèque Python MIT ; plateforme améliorée payante distincte. |
| [DumplingAI](https://www.dumplingai.com/) | Une surface API/CLI/MCP au-dessus de plusieurs providers, routing, formats normalisés et pinning de l’endpoint fournisseur. | Agrégateur commercial ; concurrence directe pour un simple « routeur multi-vendors ». |
| [Exa](https://exa.ai/docs/reference/search-api-guide) | Recherche, index, modes de latence/profondeur et [récupération de contenu](https://exa.ai/docs/reference/contents-api-guide). | API commerciale, clients et intégrations ; couche de découverte/récupération. |
| [Linkup](https://docs.linkup.so/pages/documentation/tutorials/migrating-from-tavily) | Recherche avec réponse sourcée ou schéma JSON ; fetch de page en Markdown. | API commerciale ; alternative retrieval, pas framework d’actions. |
| [Tavily](https://docs.tavily.com/welcome) | Search, Extract, Crawl, Map et recherche ; intégration agents. | API commerciale ; [crédits par opération](https://docs.tavily.com/documentation/api-credits). |
| [Jina Reader](https://jina.ai/reader/) | Conversion de pages en contenu exploitable et tooling de retrieval. | Produit Reader et écosystème public ; licence précise/non-parité non étudiées ici. |

## 4. Agents locaux, workflows et alternatives pour utilisateurs métier

| Acteur / source officielle | Positionnement / intérêt concurrentiel | Frontière open source / service |
|---|---|---|
| [Skyvern](https://www.skyvern.com/) | Workflows visuels, auth/2FA, formulaires, collecte de documents et APIs ; conteneurs auto-hébergeables. | [AGPL-3.0](https://github.com/Skyvern-AI/skyvern) et offre commerciale. |
| [Magnitude](https://magnitude.run/) | Agent orienté vision, actions et extraction ; assertions visuelles et test runner. | [Apache-2.0](https://github.com/magnitudedev/browser-agent) ; bibliothèque plutôt qu’un clone de plateforme Apify. |
| [LaVague](https://docs.lavague.ai/en/latest/) | World Model, Action Engine qui génère du code, drivers Selenium/Playwright et outils de mesure/debug. | [Apache-2.0](https://github.com/lavague-ai/LaVague/), accompagnement entreprise annoncé. |
| [Nanobrowser](https://www.github.com/nanobrowser/nanobrowser) | Extension locale, comptes déjà connectés, BYOK, approche multi-agent. | Apache-2.0 ; alternative gratuite pour usages individuels. |
| [BrowserOS neo](https://www.browseros.com/) | Navigateur local pour agents, import des comptes connectés, compact snapshots, actions groupées et replay. | Open source annoncé ; licence de la version neo non inspectée ici. |
| [Lightpanda](https://lightpanda.io/) | Moteur headless Zig avec JavaScript, sans rendu graphique ; axe mémoire/CPU plutôt que produit métier. | Moteur public ; toute réduction de coût doit être rapprochée de sa compatibilité sur les tâches cibles. |
| [Browserflow](https://browserflow.app/) | Builder de workflows dans le navigateur ou cloud, commandes et JavaScript personnalisé. | Produit métier commercial. |
| [Axiom](https://axiom.ai/) | Extension no-code, SDK/API de steps, orchestration et exécution desktop/cloud. | Produit commercial qui couvre déjà le passage du builder à des APIs. |
| [Bardeen](https://www.bardeen.ai/) | Scraping et qualification/enrichissement de leads ; positionnement commercial vertical GTM. | Produit commercial ; éviter une concurrence frontale généraliste sur le lead generation. |
| [Browse AI](https://www.browse.ai/) | Extraction/monitoring sans code, colonnes proposées puis approuvées, exports et intégrations. | Service métier commercial. |
| [Octoparse](https://www.octoparse.com/) | Application no-code, autodétection/flowchart, logins, pagination, cloud, exports et services. | Application et cloud ; pas un framework OSS. |
| [ParseHub](https://www.parsehub.com/) | Outil de scraping visuel. | Produit commercial ; lecture textuelle pauvre, détails actuels non vérifiés dans ce corpus. |
| [ScrapeStorm](https://www.scrapestorm.com/) | Identification de données, builder visuel, exécution desktop, cloud et destinations bases/fichiers. | Produit commercial ; services enterprise proposés. |
| [Roborabbit, anciennement Browserbear](https://www.browserbear.com/) | Builder no-code, APIs REST, tâches cloud et connexions no-code. | Produit commercial ; le domaine historique affiche la marque Roborabbit. |
| [changedetection.io](https://changedetection.io/) | Monitoring de changements, browser steps, filtres, alertes et navigateur géré. | Projet auto-hébergeable et service ; licence précise non inspectée ici. |
| [Visualping](https://visualping.io/) | Monitoring visuel/texte/code, appréciation de l’importance et historique pour veille métier. | Service spécialisé. |
| [Distill](https://distill.io/) | Monitoring par sélection d’éléments, conditions, versions, pages privées/PDF et cloud. | Service et extensions ; besoin adjacent de collecte récurrente. |

## 5. QA, RPA et canaux de distribution adjacents

| Acteur / source officielle | Importance pour le projet | Frontière |
|---|---|---|
| [BrowserStack Automate](https://www.browserstack.com/automate) | Infrastructure multibrowser/devices, Playwright et outils de fiabilité/self-healing de tests. | Cloud commercial ; comparer une flotte adaptée à la QA à une flotte de collecte demande des workloads différents. |
| [Sauce Labs](https://www.saucelabs.com/) | Tests web/mobile, agents de création/diagnostic et reporting entreprise. | Service QA enterprise. |
| [TestMu AI, ex-LambdaTest](https://www.testmu.ai/) | Cloud de tests et agents QA, CLI et exécution parallèle. | La [marque actuelle se présente comme anciennement LambdaTest](https://www.testmu.ai/). |
| [Checkly](https://www.checklyhq.com/) | Monitors en code, tests Playwright, alertes et gestion de l’observabilité ; bon point de comparaison pour DX et exploitation. | Service ; tooling et SDK distincts de l’infrastructure. |
| [UiPath](https://www.uipath.com/) | Orchestration d’agents, robots, APIs et humains ; gouvernance/processus entreprise. | Plateforme commerciale ; concurrencer toute la RPA excède un MVP 3–6 mois. |
| [Robocorp / Sema4.ai](https://robocorp.com/) | Le domaine Robocorp affiche maintenant Sema4.ai, agents back-office ; les [bibliothèques navigateur Robocorp](https://robocorp.com/portal/tutorial/browser-and-web-automation) restent une référence technique. | Distinguer bibliothèques et offre entreprise courante. |
| [n8n](https://n8n.io/) | Canal d’intégration et workflows ; code, humain dans la boucle et choix local/cloud. | [Sustainable Use License et licence enterprise](https://docs.n8n.io/sustainable-use-license/) ; créer un node ou vendre du conseil est permis, proposer du n8n hébergé facturé est encadré. |
| [Dify](https://github.com/langgenius/dify) | Plateforme d’applications/workflows LLM et plugins ; destination d’un futur connecteur. | [Licence modifiée d’Apache-2.0](https://raw.githubusercontent.com/langgenius/dify/main/LICENSE) avec conditions de multitenance et frontend ; ne pas traiter comme Apache standard. |

**Référence historique, hors marché actif retenu : [Reworkd](https://reworkd.ai/) annonce la fermeture de son produit le 6 février 2025.** Ses idées d’extraction à partir de schémas restent intéressantes ; cette page n’établit pas un fournisseur aujourd’hui exploitable.

## 6. Approfondissement des six acteurs les plus structurants

### Browserbase / Stagehand

Le modèle associe un framework public à une plateforme gérée large. La distribution par le framework mène vers des services d’exécution, d’identité, de recherche et de modèle. L’architecture de ton framework doit donc rester utilisable indépendamment du fournisseur de flotte. Le rival n’est plus seulement « Chrome distant », mais la réduction du nombre d’intégrations à assembler. [Documentation officielle](https://docs.browserbase.com/welcome/introduction).

### Browser Use

Le framework public attire les développeurs ; le service vend à la fois le navigateur et l’agent qui termine une tâche. Le README explique que les chemins OSS et cloud ont des capacités différentes. Ne pas mesurer le cloud agent contre un simple framework local et attribuer toute l’écart au langage, au navigateur ou à la licence. [Frontière produit](https://github.com/browser-use/browser-use).

### Apify / Crawlee

Crawlee est la bibliothèque ; Apify fournit le runtime, la facturation, les destinations et surtout la distribution via des Actors. La marketplace peut devenir un canal initial pour tes connecteurs, sans obliger ton framework à dépendre du runtime Apify. Un avantage commercial doit couvrir la maintenance et la donnée livrée. Apify documente des pages dédiées, billing, documentation et infrastructure pour les créateurs. [Publication](https://docs.apify.com/platform/actors/publishing).

### Spider

Le produit revendique le même moteur Rust entre local et cloud, avec des bindings Python/Node. La valeur payante porte sur l’exploitation, le réseau, la qualité des sorties et l’anti-bot. C’est un bon exemple de services autour d’un noyau permissif. Mais les pages marketing ne démontrent ni la parité du navigateur cloud et de Chromium, ni la fidélité d’un workflow authentifié complexe. [Comparaison OSS/cloud](https://spider.cloud/open-source-vs-cloud/).

### ScrapingBee

Le client achète une API lisible, avec escalade HTTP → JS → premium → stealth et plafond de coût. Cette politique est déjà documentée : un routeur économique doit donc apporter une preuve supplémentaire, par exemple sur l’identité persistante, la reprise de workflows et la qualité de données récurrentes. L’auto-mode ne change pas les waits ou les scénarios du client. [Contrat HTML API](https://www.scrapingbee.com/llms/html-api.txt).

### Firecrawl

L’offre est pensée pour faire trouver, lire et structurer le web depuis une seule API. Elle constitue une concurrence forte si ton produit se réduit à Markdown/crawl/extraction pour RAG. Ses unités de facturation distinguent page, search et browser-minute. La page précise qu’un HTTP 403/404 retourné peut être facturé : l’unité technique facturée et la donnée métier utile sont des concepts différents. [Tarifs](https://www.firecrawl.dev/pricing).

## 7. Tarifs vérifiés : raisonner sur les unités, pas sur le prix d’appel

| Acteur | Unité et prix publics vérifiés | Conséquence pour ton modèle |
|---|---|---|
| [Browserbase](https://www.browserbase.com/pricing) | Developer 20 $/mois : 100 browser-hours, 25 simultanés, 1 GB proxy ; supplément 0,12 $/h et 12 $/GB. Startup 99 $ : 500 h, 100 simultanés, 5 GB ; puis 0,10 $/h et 10 $/GB. | L’heure est distincte du réseau et des modèles ; l’offre groupe plusieurs produits. |
| [Browser Use](https://browser-use.com/pricing.md) | 0,02 $/browser-hour, proxy résidentiel 5 $/GB, sortie directe/BYOP 0,20 $/GB ; minute commencée et minimum d’une minute. Agent V4 : coût modèle et frais de service 20 %, navigateur/réseau en sus. | Vendre une browser-hour seule ouvre une guerre de prix ; BYOK ne supprime pas nécessairement les frais de plateforme. |
| [Apify](https://apify.com/pricing) | Starter 19 $/mois et 19 $ d’usage inclus ; CU 0,20 $. 1 CU = 1 GB RAM pendant une heure. Scale 199 $, CU 0,16 $ ; Business 999 $, CU 0,13 $. | Le coût dépend de la RAM et du temps, avec proxys, stockage et éventuel prix d’Actor à additionner. |
| [Spider](https://spider.cloud/pricing) | 1 $/GB de contenu avant transformation + 0,001 $/minute CPU sur la page pricing ; erreurs 0 $. Pas d’abonnement ; réseau premium et extraction IA séparés. | Sortie Markdown petite ne signifie pas facture faible : mesurer les octets d’entrée et compute. Voir divergence ci-dessous. |
| [ScrapingBee plans](https://help.scrapingbee.com/en/article/available-plans-explained-kbinm/) et [contrat](https://www.scrapingbee.com/llms/html-api.txt) | Aide officielle datée du 17/10/2025 : 49,99 $/250k crédits, 99,99 $/1M, 249,99 $/3M, 599,99 $/8M. HTML : 1 crédit HTTP, 5 JS, 10 premium HTTP, 25 premium JS, 75 stealth JS. | Tarifs de plans à reconfirmer dans dashboard : page pricing dynamique peu extractible. Les mêmes 1 000 URLs peuvent consommer 1k à 75k crédits. |
| [Firecrawl](https://www.firecrawl.dev/pricing) | Hobby 19 $/mois, 5 000 crédits ; 16 $/mois si annuel. Basic scrape/crawl/map 1 crédit/page ; search 2/10 résultats ; interact 2/browser-minute ; JSON/question/highlight ajoutent 4/page sur scrape/crawl. | Comparer les fonctionnalités réellement nécessaires, le crédit n’est pas une « requête universelle ». |
| [Zyte API](https://www.zyte.com/zyte-api/) | PAYG HTTP 0,13–1,27 $/1 000 réponses ; navigateur rendu 1,01–16,08 $/1 000 selon tier du domaine. Prix réduits avec engagement. | Un coût par domaine plus cher peut être meilleur si les réponses utiles demandent beaucoup moins de maintenance. |
| [Browserless](https://www.browserless.io/pricing) | Une unit couvre jusqu’à 30 s par connexion ; reconnexion nouvelle unit. 6 units/MB résidentiel, 2 units/MB datacenter et 10/captcha réussi. | Durée et réseau convertis dans un même crédit ; comparer les plafonds et reconnexions. Prix du plan non retenu, rendu incomplet. |
| [Scraping Fish](https://scrapingfish.com/) | 0,002 $/requête réussie annoncé, options navigateur/proxy mobile regroupées. | La lisibilité de l’offre peut concurrencer une tarification sophistiquée. |
| [Apify créateurs PPE](https://docs.apify.com/actors/publishing/monetize/pay-per-event.md) | Profit du créateur = 80 % du revenu d’événements facturés − coûts plateforme ; règles spécifiques free/standby. | Marketplace = distribution et billing, pas marge brute automatique de 80 %. |

### Divergences et limites de comparaison

Spider affiche **0,0001 $/minute CPU** sur la [homepage](https://spider.cloud/) dans les résultats inspectés, mais **0,001 $** sur la [page pricing](https://spider.cloud/pricing), soit un facteur dix. La page [OSS/cloud](https://spider.cloud/open-source-vs-cloud/) emploie par ailleurs « Pay per page ». Prendre pricing comme référence de travail, mais demander une confirmation avant de budgéter une production importante ; cette enquête ne résout pas le contrat de facturation réel.

Les pourcentages de succès et multiplicateurs marketing affichés par Steel, Browser Use, Anchor, Scrapfly, Zyte ou Lightpanda ne portent pas nécessairement sur le même corpus, protocole, modèle, date ou unité. On peut apprendre de leurs méthodologies ; on ne peut pas établir un palmarès universel à partir de leurs homepages.

### Exemple calculé, pas benchmark

Hypothèse volontairement identique : **10 000 tâches, une minute navigateur chacune et 5 GB de trafic résidentiel total mesuré**. Les tâches ne nécessitent ici ni agent hébergé ni extraction payante. Cela représente 166,67 browser-hours ; choisir une minute évite ici le biais du minimum de facturation de Browser Use pour des sessions plus courtes.

- Browser Use : 166,67 × 0,02 + 5 × 5 = **28,33 $**.
- Browserbase Developer : 20 $ incluant 100 heures et 1 GB, puis 66,67 × 0,12 + 4 × 12 = **76 $**.
- Browserbase Startup : **99 $**, les heures et 5 GB étant couverts.

Le calcul ne tient pas compte des différences de disponibilité, vitesse, quotas, cycles d’abonnement, nouveaux comptes, taxes, retries ou octets réellement facturés. Il illustre un point : **le réseau peut dépasser de loin le calcul navigateur**. Réduire les octets, les reconnexions et les appels modèle peut rapporter davantage qu’optimiser le lancement du browser de quelques centaines de millisecondes. Les valeurs sont dérivées des [tarifs Browser Use](https://browser-use.com/pricing.md) et [Browserbase](https://www.browserbase.com/pricing), pas de mesures.

## 8. Où une petite équipe peut vendre des services

Les propositions suivantes sont des hypothèses à tester. « Sous-servi » signifie ici une combinaison dont l’offre développeur open source est moins évidente dans le corpus, et non l’absence de concurrents.

| Segment / besoin | Concurrence qui existe déjà | Offre initiale plausible | Preuve requise |
|---|---|---|---|
| Petites agences et SaaS B2B : collecte récurrente de catalogues, tarifs, disponibilité ou documents depuis portails authentifiés | Skyvern, TinyFish, Anchor, Airtop, Axiom et RPA couvrent des morceaux ou des offres complètes. | 3 connecteurs maintenus avec sessions persistantes, pagination, dates/versions et destinations ; framework exportable. | 3 partenaires payants, mêmes workflows exécutés plusieurs semaines, résultat réconcilié contre attente client. |
| Exécution dans le cloud du client ou en local, avec possibilité de provider géré | Steel, Browserless, Kadoa, Skyvern et plateformes RPA proposent déjà de l’auto-hébergement ou déploiement privé. | Installation/support/mises à jour, secrets et provider adapters ; simplicité de migration démontrée. | Installation reproductible, même workflow et sortie sur local + au moins un provider. |
| Maintenance mesurable, reprise après rupture et diagnostic lisible pour un développeur | Kadoa fait qualité/provenance/self-repair ; Stagehand, Airtop et Anchor optimisent l’exécution. | Contrat de workflow, tests d’acceptation, traces et ticket de réparation avec diff ; service de maintenance ciblé. | Cas de rupture contrôlés, coût de réparation et validation sans faux succès. |
| Réduction de coût à volume modéré sans réécriture | HTTP/navigateur adaptatifs chez ScrapingBee, Zyte, Spider, ScrapeNinja ; agrégation DumplingAI. | Audit d’optimisation et mode shadow qui conseille HTTP/API/replay sans changer immédiatement l’exécution. | Factures avant/après, octets, modèles, coût total et qualité maintenue. |
| Destinations et intégration dans l’outillage existant | Apify, n8n, Dify et no-code établis. | Connecteur n8n/MCP + exports S3/webhook/CSV ; déploiement et exploitation du pipeline. | Pipeline complet que le client peut reprendre ; pas simplement une démo CLI. |

### Packaging recommandé, sans SLA universel anticipé

1. **Framework OSS réellement utile seul** : création/exécution locale, workflow versionné, validation définie par le développeur, traces, reprise et interfaces de providers. Ne pas limiter artificiellement des fonctions de base nécessaires à la fiabilité.
2. **Onboarding payé par connecteur ou workflow** : accès autorisé aux portails, schéma client, auth, validation, destinations et recette. Donner un périmètre concret : sites, comptes, fréquence et limites.
3. **Maintenance récurrente** : changements de sites couverts, mises à jour, triage et réparation. Définir un délai d’intervention au périmètre pilote ; un engagement de disponibilité chiffré viendra après mesure.
4. **Cloud géré facultatif** : usage technique transparent, plafonds de dépenses, logs/replay et gestion de versions. Acheter réseau/navigateur en aval ; pas construire une flotte mondiale de proxys ou un moteur navigateur pendant le MVP.
5. **Support et déploiement privé** : installation et exploitation dans le cloud du client pour les partenaires qui le justifient. Ajouter les fonctions enterprise seulement avec demande commerciale réelle.

La tarification initiale la plus simple à tester combine **forfait d’installation + abonnement de maintenance + consommation cloud documentée**. Ne pas facturer universellement un « succès métier » : un catalogue, un PDF téléchargé et une écriture ERP exigent des définitions et des responsabilités différentes. Des crédits techniques ou des quotas de runs ne doivent pas masquer la consommation des modèles et du réseau.

Apify peut servir de canal pour vendre un connecteur tôt ; n8n/MCP peut servir d’acquisition et d’intégration. Garder les workflows exportables et le framework utilisable hors de ces plateformes. La plateforme marketplace complète, le builder métier universel et le réseau de proxys doivent rester hors MVP.

## 9. Décision produit concrète et critères d’arrêt

**Choix recommandé : TypeScript/Playwright et briques Crawlee composables, plus un format de workflow portable et un service de maintenance de connecteurs.** Acheter les navigateurs/proxys si nécessaire, et permettre l’exécution locale. Le langage Rust ou un nouveau moteur peut être étudié ensuite si des profils mesurés établissent que CPU/RAM du runtime dominent le coût client ; le catalogue commercial ne prouve pas que cette condition est remplie.

Les trois workflows pilotes doivent tester des difficultés différentes : un catalogue paginé avec tarifs personnalisés, un téléchargement de documents avec auth/MFA prise en charge explicitement, et une collecte incrémentale avec reprise après interruption. Donner à chaque workflow un oracle de résultat, une limite de coût et une procédure pour auth expirée et changement de site. Utiliser les mêmes scénarios contre les alternatives pertinentes, et mesurer cold start, p50/p95 end-to-end, exactitude/complétude, octets, appels modèle et maintenance.

Deux risques peuvent invalider le projet : les clients n’acceptent de payer que le tarif faible d’une infrastructure brute ; ou les portails exigent une intervention humaine trop fréquente pour les marges envisagées. Le MVP doit donc vendre une première intégration avant d’élargir le framework, puis montrer que la maintenance du deuxième et du troisième workflow coûte moins cher. Aucun pourcentage de fiabilité ou avantage de coût ne doit devenir une promesse commerciale avant ce test.

## Annexe — Lectures directes échouées et sources complémentaires inspectées

URLs échouées : `https://apify.com/platform`, `https://spider.cloud/docs`, `https://docs.zyte.com/zyte-api/usage/price.html`, `https://saucelabs.com/products/browser-testing`, `https://docs.browserbase.com/features/browserbase-functions`. Des pages officielles alternatives ont été utilisées ; ces échecs ne signifient pas que le produit correspondant est indisponible.

Sources complémentaires utiles : [usage et ressources d’Actors Apify](https://docs.apify.com/platform/actors/running/usage-and-resources), [Scrapfly getting started](https://scrapfly.io/docs/scrape-api/getting-started), [Browserless BaaS](https://docs.browserless.io/baas/start), [Spider écosystème](https://spider.cloud/guides/spider-oss-ecosystem/), [ScrapingBee crédit](https://help.scrapingbee.com/en/article/credit-system-explained-1h2ackp/), [Tavily Crawl](https://docs.tavily.com/documentation/api-reference/endpoint/crawl), [DataForSEO SERP](https://dataforseo.com/apis/serp-api), [SerpApi](https://serpapi.com/), [Scrapling](https://scrapling.readthedocs.io/en/latest/). Les trois derniers sont des acteurs additionnels ou projets techniques à intégrer dans le benchmark/framework corpus, hors des 64 lignes commerciales consolidées ci-dessus.
