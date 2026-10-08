# Stagehand, Crawlee, Spider-rs : comparaison approfondie pour Automan

Recherche du **7 octobre 2026**, prolongement du dossier de marché. L'objectif reste un framework open source ambitieux et des services autour ; l'ancienne hypothèse de petite équipe en 3–6 mois a été retirée par l'utilisateur. Ce rapport distingue capacités publiées, code inspecté, témoignages et propositions pour Automan. Aucun benchmark comparatif des moteurs n'a été exécuté.

## Conclusion qui gouverne la comparaison

**Stagehand, Crawlee et Spider-rs travaillent principalement à des couches différentes.** Stagehand résout la perception et des opérations de navigateur pour un développeur ou un agent. Crawlee organise le traitement de requêtes, les ressources, sessions, reprises et résultats. Spider constitue un moteur de collecte Rust, extensible vers Chrome et des traitements IA. Ils se chevauchent, mais les comparer comme trois agents interchangeables produit des conclusions trompeuses.

Le texte fourni décrit une tension réelle — découverte sémantique contre exécution connue — mais plusieurs oppositions sont trop absolues. Stagehand conserve des opérations déterministes et des sélecteurs ; Crawlee peut incorporer Stagehand et adapter le rendu HTTP/navigateur ; Spider possède des fonctions facultatives de navigateur et d'IA. La maintenance ne disparaît avec aucun de ces mécanismes.

**Pour Automan, la direction proposée est un runtime commun qui choisit l'exécuteur suffisant, produit des plans réutilisables et vérifie le résultat demandé.** La collecte massive et les actions applicatives doivent partager les transports et la perception, tout en gardant des règles de scheduling et de reprise distinctes. Cette proposition ne constitue pas une preuve d'avance commerciale : plusieurs concurrents possèdent déjà cache, replay, réparation et validation partielle.

## 1. Corriger précisément le texte fourni

| Affirmation fournie | Ce que les sources permettent de dire |
| --- | --- |
| « Consensus Reddit Stagehand vs Crawlee » | Les références fournies sont des pages d'accueil. Notre recherche de complément n'a pas trouvé une comparaison directe suffisamment documentée du trio. Des témoignages individuels existent, parfois contradictoires ou promotionnels. |
| « Stagehand n'utilise pas de sélecteurs classiques » | Faux comme description du framework. Ses primitives sémantiques peuvent produire des `Action`, et ses APIs page/locator permettent des opérations déterministes. Il ne faut pas confondre l'instruction fournie par le développeur et la cible finalement exécutée. |
| « Crawlee impose toujours la maintenance manuelle du DOM » | Trop large : intégration Stagehand, crawler adaptatif HTTP/Playwright et extensions possibles. Scrapling propose également une récupération structurelle sans LLM. Aucun de ces outils ne garantit une correction métier après toute modification. |
| « Crawlee gère automatiquement l'anti-bot » | Il fournit sessions, proxies et mécanismes de fingerprinting selon le backend. Ils ne garantissent pas l'accès à un site, et les intégrations Stagehand ont des limites explicites de fingerprint/isolation. |
| « Playwright dans Crawlee sera toujours plus rapide » | Un parcours connu peut éliminer des inférences, mais le résultat dépend du rendu, réseau, état de session, batching, cache et travail réellement accompli. Le mot « toujours » n'est étayé par aucun protocole commun examiné. |
| « Crawlee est gratuit et sans clés API » | Le framework peut fonctionner localement sans API de modèle ; navigateurs, machines, proxies et maintenance ont un coût. L'ajout de primitives IA ajoute les besoins du modèle ou de sa gateway. |
| « StagehandCrawler permet de combiner les deux » | Oui, avec une intégration officielle. Il faut toutefois lire les contrats TypeScript et Python séparément et vérifier les versions ; la présence de l'adapter n'établit pas la compatibilité Stagehand v4. |

Sources : [Stagehand migration depuis Playwright](https://docs.stagehand.dev/v4/migrations/playwright), [adapter TS publié](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json), [StagehandCrawler Python](https://crawlee.dev/python/docs/guides/stagehand-crawler), [crawler adaptatif publié](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts), [preuves Reddit vérifiées](./2026-10-07-reddit-evidence-followup.md).

## 2. La comparaison utile : contrat, ressources et résultat

| Axe | Stagehand | Crawlee | Spider-rs |
| --- | --- | --- | --- |
| Objet principal | Page et opérations sémantiques/déterministes | Requête et handler dans un cycle de crawl | Website, pages, liens et flux de collecte |
| Découverte de cible | Instructions, observation, locators, actions résolues | Handler choisi par le développeur ; IA via intégrations | Parsing et hooks ; navigateur/IA facultatifs |
| Rendu | Navigateur ; v4 runtime dans une extension Chrome | HTTP et plusieurs backends navigateur | HTTP natif, Chrome et smart mode selon features |
| Travail massif | Concurrence de pages/onglets ne constitue pas une frontier de crawl | Queue, déduplication, autoscaling, pools, sessions, retries, stockage | Concurrence, budgets, déduplication et diffusion des pages ; capacités additionnelles selon compilation |
| Adaptation | Résolution sémantique et mécanismes de cache/replay | Choix HTTP/Playwright fondé sur observation/comparaison des sorties | Smart mode fondé notamment sur des signaux de nécessité du rendu |
| Reprise | Fonctions locales à distinguer de la reprise d'un job complet | Reclaim/retry d'une requête et stockage configuré | Crawl et stockage à examiner selon feature ; pas assimilation à un workflow métier transactionnel |
| Vérité du résultat | Extraction structurée et jugement interne ne suffisent pas à prouver le résultat métier | Handler et callbacks de validation peuvent l'exprimer | L'application doit valider le contenu collecté et sa complétude |
| Question à benchmarker | Combien de décisions, appels et faux succès pour une tâche ? | Combien de résultats valides avec quelles ressources et reprises ? | Quel débit/empreinte pour une acquisition équivalente et quel taux d'upgrade inutile ? |

Les contrats et versions détaillés sont dans [l'étude des alternatives Stagehand](./2026-10-07-stagehand-alternatives-followup.md) et [l'inspection Crawlee/Spider](./2026-10-07-crawlee-spider-contracts-followup.md). Cette grille est une analyse, pas une matrice de performances mesurées.

## 3. Stagehand : le sujet est désormais v3 contre v4 autant qu'IA contre code

### Ce qu'il faut comparer aujourd'hui

L'étude du paquet stable **Stagehand 4.1.0** montre un runtime d'extension et un protocole partagé entre clients. La migration v4 retire l'ancienne API autonome `agent()` au profit de primitives intégrables à une orchestration externe. L'exemple `page.act(...)` d'une discussion ancienne ne décrit donc pas nécessairement la surface actuelle. [Architecture v4](https://www.browserbase.com/blog/stagehand-v4), [migration v3](https://docs.stagehand.dev/v4/migrations/v3), [protocole](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md).

Trois chemins doivent être séparés :

1. **Instruction sémantique :** demander de trouver ou faire quelque chose nécessite une interprétation et dépend du modèle, du contexte et de l'observation.
2. **Action déjà résolue :** exécuter un objet `Action` peut éviter l'inférence ; dans le paquet 4.1.0 inspecté, `act(Action)` conserve toutefois une vérification préalable de configuration modèle/gateway. « Aucune inférence pour cette branche » n'implique donc pas « toute cette API est utilisable sans config modèle ».
3. **Primitives page/locator :** voie déterministe, distincte des appels sémantiques, qui doit être qualifiée comme un driver propre.

Il ne faut surtout pas lui attribuer automatiquement les garanties de Playwright parce que certaines méthodes ont des noms proches. La [migration officielle](https://docs.stagehand.dev/v4/migrations/playwright) décrit notamment l'absence d'auto-waiting équivalent, d'assertions Playwright qui réessaient et de plusieurs attentes réseau. Le clic doit donc être étudié avec ses conditions de résolution et de disponibilité réelles.

### Extraction : conformité, complétude et vérité sont trois questions

Dans le paquet stable inspecté, le chemin d'extraction non servi par cache comporte une génération structurée puis un jugement de complétude. Ce second appel reçoit l'instruction et la sortie, pas le snapshot source. Ce jugement est utile, mais dépend du modèle ; il ne consulte pas un oracle indépendant de la tâche. Le champ interne `completed` n'est pas exposé au consommateur comme garantie de vérité métier. Le code inspecté peut renvoyer la sortie et la proposer à l'écriture cache même lorsque ce jugement est négatif ; l'acceptation par le serveur de cache non public n'a pas été vérifiée.

Un schéma qui exige un nombre et une devise ne garantit pas que le nombre est le prix courant du bon produit. Une page de résultats plausible ne garantit pas que toute la pagination a été parcourue. Cette limite concerne le contrat du résultat, au-delà d'une seule bibliothèque. Les chemins source exacts et limites de cette lecture statique sont dans [le rapport Stagehand 4.1.0](./2026-10-07-stagehand-alternatives-followup.md).

### Cache : ne pas confondre action et donnée

Les trois familles de cache examinées ne conservent pas le même objet : `observe` peut réutiliser des actions résolues, `act` rejouer des actions, et `extract` retourner directement une **valeur structurée**. Ce dernier cas impose une politique de fraîcheur si les données changent. La clé cliente transmet notamment URL, arbres d'accessibilité, instruction et schéma, mais le serveur de correspondance n'est pas public ; cette lecture ne prouve pas ses règles de TTL ou d'invalidation sémantique.

Une divergence existe entre la documentation v4 présentant le cache comme activé par défaut et les valeurs par défaut du paquet 4.1.0 inspecté. Une expérience comparative doit **configurer le cache explicitement et contrôler les métadonnées**, puis publier cache HIT/MISS et fraîcheur. Nous n'avons pas exercé le service cloud ; aucune hypothèse sur son implémentation cachée n'est convertie ici en fait.

Pour Automan, séparer cache de perception, cache de plan et cache de données devient une exigence de contrat : leurs règles d'invalidation ne peuvent pas être interchangeables.

## 4. StagehandCrawler : une composition réelle, avec deux contrats distincts

### TypeScript publié

`@crawlee/stagehand` **3.18.2** déclare `@browserbasehq/stagehand: ^3.0.0`. Cette contrainte ne couvre pas v4. Le package est séparé du bundle principal `crawlee`. [Manifest versionné](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json).

Le lancement et la connexion combinent Stagehand et Playwright sur **le même navigateur via CDP**. Ce n'est pas simplement deux APIs indépendantes déposées dans un handler : la propriété du navigateur, l'attachement des pages et les options de lancement comptent. Les wrappers `act/extract/observe` injectent la page traitée. L'interface autonome `agent` du wrapper v3 doit être étudiée séparément avant de supposer qu'elle vise toujours cette page. Le code du plugin avertit que le fingerprint est partiellement appliqué puisque Stagehand maîtrise le lancement.

Cela donne une vraie orchestration Crawlee autour de primitives Stagehand v3. Cela ne démontre ni toutes les garanties de Playwright, ni toutes les garanties de fingerprint BrowserPool, ni une migration v4 transparente.

### Python publié

Dans Crawlee Python **1.10.4**, l'extra Stagehand dépend du package Python `stagehand>=3.19.5,<4.0.0`. Ces numéros appartiennent au client Python ; ils ne sont pas interchangeables avec ceux du package npm. Le plugin utilise `AsyncStagehand`, démarre une session puis connecte Playwright à son URL CDP ; le mode local peut sélectionner Chromium fourni par Playwright. [Manifest Python](https://github.com/apify/crawlee-python/blob/v1.10.4/pyproject.toml), [guide](https://crawlee.dev/python/docs/guides/stagehand-crawler).

L'inspection confirme un contexte partagé et des limites concernant nouveaux contextes/proxies, pages incognito et fingerprint complet canvas/WebGL/screen ; les en-têtes cohérents avec un fingerprint ne constituent pas un fingerprint complet. `StagehandPage` injecte la page pour ses opérations, y compris `execute`. Le [rapport de contrats](./2026-10-07-crawlee-spider-contracts-followup.md) fournit les sources et les conséquences d'isolation détaillées.

**Conséquence pratique :** l'existence d'une intégration officielle est confirmée ; la compatibilité et l'isolation doivent être vérifiées pour le langage, la version et le backend choisis.

## 5. Crawlee : plus intéressant que « des sélecteurs avec retries »

Le concurrent architectural important est `AdaptivePlaywrightCrawler`. Dans la release TS 3.18.2, un prédicteur apprend à partir de l'hôte, du chemin et du label de requête. Des runs de détection comparent ce que le handler produit en navigateur et en HTTP ; des callbacks `resultChecker` et `resultComparator` permettent de décider si la sortie suffit. [Implémentation](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts), [prédicteur](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/utils/rendering-type-prediction.ts).

Cela est plus proche d'Automan qu'une règle « s'il y a du JavaScript, lancer Chrome ». La bonne question est : **le moyen moins coûteux produit-il le résultat demandé ?** Le comparateur par défaut examine seulement les `datasetItems`, leur nombre et leur égalité par index ; il ne définit pas à la place de l'utilisateur l'identité du produit, la période de données ou la complétude métier. Un handler qui émet uniquement des liens peut produire deux listes d'items vides jugées égales alors que les liens découverts diffèrent. Il faut donc choisir un `resultComparator` adapté au contrat de collecte. [Comparateur publié](https://github.com/apify/crawlee/blob/v3.18.2/packages/playwright-crawler/src/internals/adaptive-playwright-crawler.ts#L341).

Cette détection peut exécuter le handler plusieurs fois et gère ses écritures de résultats. Leur commit regroupe des opérations de stockage, sans constituer une transaction générale de toutes ces opérations. Elle convient à l'acquisition avec le contrat prévu ; elle ne rend pas sûre la répétition d'un achat, d'un envoi ou d'une création sur un site. De même, un retry de requête n'assure pas une exécution unique de ses effets externes. Automan doit journaliser les tentatives et vérifier l'état résultant avant de rejouer une étape à effet.

Le client [Impit](https://crawlee.dev/js/docs/guides/impit-http-client) montre aussi qu'une API TS peut déjà utiliser un transport natif Rust. Le langage du SDK ne suffit pas à déduire le coût du chemin critique. Le choix d'un runtime Rust Automan se justifie par l'unité du moteur, sa distribution et la maîtrise des ressources, puis doit être validé par mesure.

## 6. Spider-rs : référence sérieuse de moteur, pas raccourci vers un agent universel

Le dossier inspecte la crate publiée **Spider 2.53.9** et **Chromey 2.58.2**, et distingue leur source publiée des nouveautés de `main`. Spider propose HTTP, hooks de fetch, budgets, diffusion des pages, Chrome et plusieurs chemins conditionnés par features Cargo. Le code n'est pas seulement un parseur Rust. [Crate publiée](https://docs.rs/crate/spider/2.53.9/source/), [configuration de features](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml).

Son smart mode utilise des signaux de nécessité du navigateur, notamment scripts, mutations, données et marqueurs de frameworks. **Ce n'est pas le même contrat que comparer l'extraction attendue en HTTP et en navigateur.** Une page hydratée mais déjà complète côté serveur et une page apparemment complète qui omet une donnée doivent entrer dans le corpus de validation. La feature `smart` est importante : appeler la méthode sans le bon chemin compilé ne garantit pas l'upgrade Chrome. Spider possède aussi, avec `parallel_backends`, une sélection de réponses par score de qualité HTML et un `QualityValidator` personnalisable. Cette capacité ne doit pas être ignorée ; elle reste distincte d'une comparaison des sorties du handler adaptée au résultat demandé. [Détails audités](./2026-10-07-crawlee-spider-contracts-followup.md), [hook qualité publié](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14248).

La persistance et le contrôle de charge ont également des périmètres précis. Le stockage SQLite `disk` conserve visites et signatures ; ce schéma n'est pas une frontier durable pending/running/done avec acquittement. `Website::queue()` injecte des liens via un broadcast Tokio en mémoire. Une base de visites conservée ne suffit donc pas à reconstituer tous les travaux interrompus. Le sémaphore adaptatif peut être fourni explicitement ; sa présence ne garantit pas un autoscaling CPU/RAM automatique de chaque crawl. [Schéma publié](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/features/disk.rs#L96), [queue réelle](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L14692), [résolution du sémaphore](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/src/website.rs#L6956).

**Spider possède aussi un véritable agent open source local**, pas seulement un client de cloud. La crate MIT `spider_agent` **2.53.9** propose notamment `run`, `act`, `observe`, mémoire d'exécution et extraction HTML/screenshot, avec API LLM configurable et contrôle du travail en cours ; son moteur CDP repose sur Chromey selon features. L'archive publiée a été inspectée et identifie le commit `e4b53c5b582357e46433fa9ba3d91f930fb67cb4`. Ce composant mérite donc un comparateur distinct face aux primitives Stagehand et aux boucles Browser Use. [Source publiée de spider_agent](https://docs.rs/crate/spider_agent/2.53.9/source/), [contrats détaillés](./2026-10-07-crawlee-spider-contracts-followup.md).

Une réserve empêche cependant de transformer la liste de configuration en liste de garanties : dans les sources Rust publiées inspectées de `spider_agent` et `spider_agent_html`, les champs `html_diff_mode`, `planning_mode`, `self_healing` et `confidence_strategy` apparaissent dans les types/builders, sans lecture opérationnelle retrouvée. Cela ne démontre pas l'absence de ces capacités dans tout l'écosystème ou une release ultérieure ; cela signifie que **les économies et comportements associés ne sont pas établis par ces champs dans ce code publié**. Tester les traces et les différences effectives avant de reprendre les promesses de tokens.

**Le SDK cloud `spider-browser`, la crate de crawl `spider`, la crate `spider_agent` et le driver Chromey ne constituent pas un seul produit ayant toutes les mêmes capacités.** Le SDK cloud expose des primitives proches de Stagehand ; cela ne prouve pas qu'elles ont toutes le même backend et les mêmes garanties en local. [Spider](https://github.com/spider-rs/spider), [spider-browser](https://github.com/spider-rs/spider-browser), [Chromey](https://github.com/spider-rs/chromey).

Chromey est un candidat de driver CDP et possède des extensions réseau/cache/fingerprint. Une méthode `Element::click()` n'établit toutefois pas une équivalence avec toute l'actionability de Playwright : résolution au moment de l'action, visibilité, stabilité, frames, hit testing et attente armée doivent être qualifiés. Certaines commandes vendor, comme des domaines de Markdown/WebMCP, nécessitent un moteur distant compatible et ne sont pas des commandes garanties de Chrome stock.

**Recommandation :** adapter Spider pour la collecte et comparer Chromey/Chromiumoxide pour le driver ; garder Automan propriétaire de ses contrats de résultat, de plan et de reprise. Ne pas forker l'ensemble de Spider avant d'avoir identifié ce qu'un adapter étroit ne permet pas. Le protocole, le comportement et la validation doivent rester ceux d'Automan.

Les multiplicateurs de vitesse du benchmark Spider ne classent pas ce trio : Node y est le package `crawler`, pas Crawlee, et le code actuel possède des asymétries et stubs à corriger. L'[audit reproductible des sources](./2026-10-07-benchmark-audit-followup.md) épingle les commits et explique précisément les limites, sans accuser les anciens résultats d'avoir utilisé ces mêmes défauts.

## 7. Les alternatives à suivre, classées par mécanisme

### Proches de Stagehand ou complémentaires

| Projet | Pourquoi il compte | Comparaison utile pour Automan |
| --- | --- | --- |
| [Browser Use](https://github.com/browser-use/browser-use) | Boucle d'agent et outils navigateur ; évolution vers composants natifs Rust | Tâches autonomes, contexte conservé, sessions et budget de décisions ; séparer SDK, bêta native et cloud |
| [Skyvern](https://github.com/Skyvern-AI/skyvern) | Workflows, perception visuelle et génération/réutilisation de scripts | Réparation de blocs, reprise et coût amorti d'un workflow connu |
| [AgentQL](https://docs.agentql.com/) | Requêtes sémantiques et résolution d'éléments au-dessus du navigateur | Extraction/ciblage avec schéma ; distinguer SDK public et backend de service |
| [Magnitude](https://github.com/magnitudedev/browser-agent) | Automatisation d'agent fondée notamment sur la vision | Interfaces pauvres en DOM sémantique, changements visuels, coût des captures et de l'inférence |
| [Imprint](https://github.com/ashaychangwani/imprint) | Découverte et compilation de comportements réutilisables | Artefact généré, validité sur répétitions et conditions de réparation ; maturité à lire selon version |
| [agent-browser](https://github.com/vercel-labs/agent-browser) | CLI, daemon natif, snapshots et refs ; chat IA désormais facultatif | Surface d'outils et volume d'observation d'un agent externe ; le cœur déterministe reste distinct du chat |
| [Playwright MCP](https://github.com/microsoft/playwright-mcp) / [CLI](https://github.com/microsoft/playwright-cli) | Exposent un driver éprouvé à un agent | Baseline d'actionability, observation et ergonomie ; le modèle et la durabilité restent ailleurs |

Ces projets ne sont pas tous des frameworks entièrement auto-hébergeables au même périmètre. Le [rapport détaillé](./2026-10-07-stagehand-alternatives-followup.md) établit leurs mécanismes, licences observées et frontières SDK/backend. La [cartographie générale](./2026-10-07-commercial-landscape.md) couvre les services supplémentaires, dont Anchor et Airtop.

### Proches de Crawlee ou Spider

| Projet | Mécanisme à retenir | Limite de comparaison |
| --- | --- | --- |
| [Scrapy](https://docs.scrapy.org/) | Scheduler, middlewares, pipelines, throttling et jobs reprenables | Navigateur via intégration ; agent sémantique non implicite |
| [Scrapling](https://github.com/D4Vinci/Scrapling) | HTTP/navigateur, spiders et relocation structurelle de sélecteurs | Similarité structurelle ne garantit pas identité métier |
| [Crawl4AI](https://docs.crawl4ai.com/) | Rendu, Markdown, extraction structurée, sessions et dispatcher | Pipeline de corpus différent d'une boucle autonome d'actions |
| [Firecrawl](https://github.com/firecrawl/firecrawl) | API d'acquisition/extraction, formats et routage de moteurs | Déploiement OSS et cloud distincts ; cœur AGPL à examiner avant reprise de code |
| [Colly](https://github.com/gocolly/colly) | Crawl HTTP Go configurable, async, limites et storage | Pas de rendu JS intégré comparable à Chromium |
| [Ferret](https://github.com/MontFerret/ferret) | DSL et exécution de plans, v2 avec VM | v2 alpha ; distinguer inspiration d'une dépendance stable |

Leur intérêt est complémentaire : orchestration, extraction, transport et planification ne doivent pas être mis dans une seule colonne « vitesse ».

### Deux nuances concrètes souvent manquées

**Scrapling peut adapter un sélecteur sans LLM.** Dans le [parser inspecté au commit `43dee004`](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/parser.py), le fallback adaptatif de CSS/XPath intervient lorsqu'aucun élément n'est trouvé, si l'option est activée et une représentation mémorisée disponible. Il classe les ressemblances structurelles et peut renvoyer plusieurs meilleurs candidats ex aequo. Le pourcentage est un score de similarité, pas une probabilité calibrée de correction. Si un sélecteur continue à trouver un mauvais élément, ce chemin de fallback n'est pas déclenché à lui seul. Un validateur de données reste nécessaire. [Documentation](https://scrapling.readthedocs.io/en/latest/parsing/adaptive/).

**Crawl4AI propose des scripts déterministes.** Les commandes C4A telles que GO, WAIT, CLICK, TYPE et REPEAT sont un DSL, même si sa syntaxe est présentée comme proche de l'anglais. Le fait de lire une commande facilement ne la transforme pas en résolution sémantique par modèle. Cela fournit une autre référence de plan exécutable ; les stratégies d'extraction LLM restent un chemin séparé. [C4A Script](https://docs.crawl4ai.com/core/c4a-script/), [extraction LLM](https://docs.crawl4ai.com/extraction/llm-strategies/).

## 8. Reddit : des scénarios à tester, aucune preuve de consensus

Le relevé de complément a ouvert **10 fils et 6 permaliens de commentaires**. Les auteurs, dates, métriques et biais sont documentés dans [le dossier de preuves](./2026-10-07-reddit-evidence-followup.md). Aucun fil trouvé dans ce balayage ne fournit un protocole commun Stagehand/Crawlee/Spider ; cela ne prouve pas qu'aucune discussion n'existe ailleurs.

Trois observations utiles :

- Un [retour Stagehand contre Browser Use du 15 avril 2026](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/) rapporte davantage de sessions terminées avec Stagehand après des clics erronés de Browser Use. Modèles, versions et taux de succès sont absents ; l'auteur dit que cela peut ne pas se généraliser.
- Un [commentaire de juin 2026](https://www.reddit.com/r/AI_Agents/comments/1uhvp85/comment/oubl84a/) décrit au contraire trop de surveillance nécessaire avec Stagehand. Il ne fournit pas non plus de métrique appariée, mais empêche de prétendre à l'unanimité.
- Le [récit de reconstruction d'une stack](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/we_rebuilt_our_scraping_stack_4_times_in_18/) annonce 400 sites, 2 millions de pages/jour, 1,2 % d'échecs et -45 % de coût avec Browserbase + Stagehand. Aucun corpus, trace ou facture n'est donné. Hébergement, SDK et exploitation changent ensemble : ce n'est pas une mesure isolée de Stagehand.

Les témoignages orientent vers les sessions authentifiées, l'observabilité, le coût des inférences et les faux succès. Ils ne décident pas du langage du runtime ni d'un vainqueur universel. Le prétendu benchmark Rust découvert sur Reddit concerne **Crawlyx, pas Spider-rs**, et provient de son auteur.

## 9. Direction concrète pour Automan

Conserver la cible [runtime Rust autonome + protocole partagé + SDK](./2026-10-07-direction-automan.md), mais préciser ce que cette comparaison impose au produit :

1. **Task et ResultContract au centre.** Définir données attendues, identité, complétude, fraîcheur et état final observable. Le modèle propose une action ; un contrôle adapté décide si l'objectif a été atteint.
2. **Exécuteurs interchangeables.** API documentée/HTTP, parsing, navigateur déterministe et découverte sémantique. Choisir selon le résultat et l'historique, avec navigateur direct sur les routes connues comme nécessaires. Éviter un double essai HTTP puis browser à chaque requête.
3. **Plans durables et versionnés.** La première découverte produit un plan avec préconditions, cibles, attentes et postconditions. Exécuter ce plan sans nouvelle inférence lorsqu'il reste valide ; réparer l'étape diagnostiquée et enregistrer la modification.
4. **Trois caches explicites.** Perception, plan et données ont des clés, invalidations et règles de fraîcheur distinctes. Un gain de latence n'est acceptable que si la sortie reste conforme au contrat.
5. **Frontier de crawl séparée de la reprise des actions.** Queue, quotas par site, déduplication, leases et backpressure pour la collecte. Journal, idempotence quand elle existe et réconciliation de l'état après incident pour les actions. Ne pas annoncer une garantie exactly-once universelle sur des sites tiers.
6. **Session comme ressource de premier ordre.** Cookies, identité, proxy, géographie et contexte isolé doivent rester cohérents. Pooling et multiplexage ne doivent pas mélanger les comptes ou clients.
7. **Driver qualifié.** CDP natif pour la cible ; Playwright sert de référence et de backend de compatibilité. Tester attentes, frames, éléments détachés, téléchargements et reprise avant de promouvoir chaque chemin natif.
8. **Économie observable.** Compter toutes les tentatives, appels modèle, rendu, proxies et ressources ; fournir le coût par résultat réellement validé et le temps nécessaire pour diagnostiquer un incident.

La différenciation à démontrer est donc : **un même workflow tourne localement ou à distance, choisit un moyen d'accès économique, se réutilise, survit aux changements prévus et rend visibles les résultats incomplets.** Le moteur doit rester utile sans service cloud ; les services facturables peuvent porter sur navigateurs/sessions gérés, capacité, maintenance des recettes, observabilité et support. Ce positionnement est une hypothèse produit à éprouver sur des pilotes, pas une promesse de marché déjà validée.

## 10. Expériences qui doivent décider de la suite

L'[audit des benchmarks](./2026-10-07-benchmark-audit-followup.md) explique pourquoi les tableaux actuels ne suffisent pas. La prochaine comparaison doit isoler :

- HTTP avec sorties équivalentes : Spider, Crawlee HTTP, Scrapy/Colly async et Automan.
- Rendu JS : Spider Chrome, Crawlee Playwright, Crawl4AI et Automan CDP.
- Actions connues : Playwright, primitives Stagehand, CLI agent-browser et plans Automan.
- Découverte sémantique : Stagehand 4, Browser Use, Skyvern, `spider_agent` et Automan, mêmes modèles/budgets lorsque c'est possible.
- Répétitions et dérive : 1/10/100/1 000 runs, mutations de DOM contrôlées, données changées, session expirée, interruption entre action et validation.

La vitesse doit être rapportée avec couverture, taux de faux succès, mémoire totale, p50/p95, démarrage chaud/froid, coûts de toutes les tentatives et résultats de reprise. Les cas métier d'export authentifié, de catalogue paginé et de collecte différentielle proposés précédemment donnent un premier corpus ; ils ne limitent pas les APIs générales.

## Provenance et limites du dossier

La racine de ce suivi a demandé **32 résultats Exa** dans quatre recherches complémentaires, puis ouvert des sources, registres et arbres GitHub. Le suivi Reddit a demandé 55 résultats, Stagehand 32 et Crawlee/Spider 20, soit **139 résultats demandés dans ce complément**. Les compteurs sont des volumes de résultats demandés et peuvent inclure doublons ou échecs ; ils ne désignent pas autant de sources indépendantes validées.

La lecture du code publié, la vérification de contraintes de versions et l'audit statique ne prouvent pas la performance ou la réussite du cloud en conditions réelles. Les témoignages restent déclaratifs ; les scripts de benchmarks examinés n'ont pas été exécutés. Une petite expression Node et l'agrégation d'un JSON public ont vérifié respectivement une sémantique de stub et les totaux publiés. La décision Rust reste un choix de conception à qualifier, sans réduction d'ambition liée à l'équipe ou au calendrier.
