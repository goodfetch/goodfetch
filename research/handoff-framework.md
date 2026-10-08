# Prompt de reprise — framework Automan

Tu reprends une discussion de conception et de recherche menée le 7 octobre 2026. Le nom « Automan » vient du dossier de travail ; le nom commercial du produit n’a pas été arrêté. Utilise le contexte ci-dessous pour continuer au point où nous nous sommes arrêtés.

## 1. Mon objectif et le scope actuel

Je veux développer **mon propre framework d’automatisation navigateur, similaire à Stagehand et meilleur sur des critères à définir**. Je veux posséder le produit et son architecture, tout en réutilisant des bibliothèques et frameworks existants lorsque cela a du sens.

**Je veux le cœur en Rust.** Le framework doit pouvoir être consommé par des développeurs qui écrivent leurs applications dans d’autres langages, notamment Node.js/TypeScript, Python et éventuellement Go. Un consommateur ne devrait pas avoir à installer Rust ou Cargo sur une plateforme pour laquelle nous distribuons un moteur compilé.

J’ai explicitement réduit le scope : **pour l’instant, nous concevons le framework et décidons du stack, pas toute la suite SaaS**. Le choix précis du transport, du packaging, de la bibliothèque CDP et du premier SDK reste à arrêter. Les recommandations du précédent assistant ne constituent pas des décisions déjà approuvées.

## 2. La vision produit à terme

À terme, ce moteur doit pouvoir servir une suite comprenant :

- une application web par abonnement, avec une interface engageante ;
- une application macOS native ;
- une CLI et des outils pour agents IA, notamment via MCP ;
- des fonctions de scraping, de crawling, d’automatisation navigateur et, plus tard, d’automatisation sociale ;
- un fonctionnement local et des possibilités d’exécution cloud.

Je souhaite une organisation par blocs et des « actors » réutilisables : création de scénarios, extraction de pages entières ou de données spécifiques, résultats structurés et normalisés selon le besoin, sauvegarde et export dans les formats souhaités. Un actor pourrait effectuer un crawl complet, ne récupérer que les changements ou certaines entités, avec différentes configurations de tâches.

J’ai aussi exploré les sélecteurs visuels et leur construction assistée par IA. Il s’agit d’une ambition pour le produit, pas d’une fonctionnalité déjà spécifiée pour la première version du framework.

Nous avons discuté de JavaScript dynamique, pagination, scroll et boutons « Voir plus ». Je veux comprendre comment couvrir les données disponibles par HTTP et celles nécessitant un navigateur et des interactions. Le rendu JavaScript, à lui seul, ne définit pas les actions nécessaires pour révéler toutes les données.

Au début de la discussion, j’ai également évoqué proxies, empreintes navigateur et automatisation sociale « indétectable ». Aucune garantie d’indétectabilité n’a été établie. Ces sujets ne sont pas le scope d’implémentation immédiat.

## 3. Mes références et ce que nous avons exploré

Référence principale pour le framework :

- Stagehand : https://docs.stagehand.dev/v4/first-steps/introduction
- Quickstart : https://docs.stagehand.dev/v4/first-steps/quickstart
- Code : https://github.com/browserbase/stagehand

Références de comparaison :

- Browserbase : https://www.browserbase.com/ et https://docs.browserbase.com/
- Browser Use : https://browser-use.com/ et https://github.com/browser-use/browser-use
- Crawlee : https://crawlee.dev/ et https://github.com/apify/crawlee
- Spider-rs : https://github.com/spider-rs/spider
- Chromey : https://github.com/spider-rs/chromey
- Chromiumoxide : https://github.com/mattsse/chromiumoxide
- Spider Browser : https://github.com/spider-rs/spider-browser
- Jev / TypeSafe : https://docs.typesafe.ai/

Nous avons auparavant étudié les produits autour de ces moteurs : Apify, Spider Cloud, ScrapingBee, ainsi que d’autres concurrents SaaS, dont Kadoa. J’avais demandé trois agents GPT 6.1 Sol, effort high, pour des audits Replica Plus d’Apify, Spider Cloud et ScrapingBee. Des rapports existent, avec leurs propres limites de couverture. Ils ne sont pas une implémentation du framework Automan.

Les dépôts publics d’une entreprise peuvent contenir ses SDK, bibliothèques et exemples sans contenir le code complet de sa plateforme commerciale. Distingue systématiquement la bibliothèque open source, les SDK clients et le service cloud.

## 4. Les distinctions que nous avons clarifiées

- **Chrome / Chromium** est le navigateur réel.
- **Stagehand** fournit des outils pour le piloter et exploiter son contenu. Il peut être utilisé dans un script, une application, un backend, un worker ou par un agent.
- **Browserbase** fournit notamment l’infrastructure de navigateurs cloud. Stagehand peut aussi fonctionner avec un navigateur local ou existant.
- **Browser Use** fournit notamment une boucle d’agent de navigation et ses propres outils navigateur ; il propose aussi des offres cloud. Son agent choisit les étapes à partir d’un objectif. Ses capacités ne se limitent pas à ce seul usage.
- **Stagehand v4** expose ses opérations et laisse la boucle d’agent au programme ou au framework externe. L’ancien `agent()` de Stagehand v3 a été retiré en v4.

Utiliser Stagehand depuis Browser Use est techniquement envisageable via des outils ou un adapter, mais aucune intégration officielle prête à utiliser n’a été identifiée pendant nos recherches. Le partage des pages, sessions et observations demande une conception explicite.

Nous avons également discuté de Codex et Claude Code : ils peuvent être intégrés dans des automatisations et des produits. Un framework spécialisé peut leur fournir des outils navigateur. Son intérêt commercial doit venir de sa facilité d’intégration et de résultats mesurables : fiabilité, qualité d’extraction, diagnostic, coût et exploitation. « Un agent clique sur des sites » ou « c’est écrit en Rust » ne suffisent pas à établir cette valeur.

## 5. Architecture Stagehand vérifiée pendant la discussion

Pour **v4**, les sources consultées indiquent :

- un runtime TypeScript/JavaScript dans une extension Chrome, exécutée notamment dans son service worker ;
- un pilotage direct de Chromium par CDP ; **pas de dépendance runtime Playwright ou Puppeteer** ;
- des SDK TypeScript/JavaScript, Python et Go ;
- un protocole JSON-RPC bidirectionnel transporté sur la connexion CDP ;
- un contrat partagé défini avec Zod et exporté en JSON Schema ; des modèles Python et Go générés depuis ce contrat ;
- des appels IA via Vercel AI SDK et différents providers, avec la possibilité de fournir une callback de modèle côté client ;
- trois primitives IA centrales : `act`, `observe`, `extract`, ainsi que des opérations déterministes de navigation et d’interaction.

Ces SDK ne sont donc pas trois réimplémentations indépendantes de toute la logique IA et navigateur.

Sources :

- https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md
- https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md
- https://github.com/browserbase/stagehand/blob/main/packages/extension/package.json
- https://docs.stagehand.dev/v4/migrations/v3
- https://docs.stagehand.dev/v4/migrations/playwright

Le comportement de l’API v4 n’est pas identique à celui de Playwright : surface plus petite, un contexte par navigateur et différences d’attente et de locators. Certains exemples indexés concernent encore v3 ; vérifie la version avant d’en tirer une conclusion.

## 6. Jev : notre échange récent

J’ai demandé si Stagehand intègre Jev dans sa partie IA. Nous avons trouvé une **intégration expérimentale en cours de développement** dans le dépôt Stagehand.

Au 7 octobre 2026, les PR vérifiées étaient ouvertes et non fusionnées :

- #2952 : client TypeSafe Jev et sélection des candidats — https://github.com/browserbase/stagehand/pull/2952
- #2953 : pipeline de décision pour `act` — https://github.com/browserbase/stagehand/pull/2953
- #2954 : `observe` et vérification des actions en cache — https://github.com/browserbase/stagehand/pull/2954
- #2955 : extraction par sélection puis copie des valeurs — https://github.com/browserbase/stagehand/pull/2955
- #3103 : namespace public `experimentalDecisions` en TypeScript, Python et Go — https://github.com/browserbase/stagehand/pull/3103

Jev répond à des questions typées : choix parmi des candidats, oui/non, score. Dans cette intégration, le code décrit des éléments de la page, Jev aide à sélectionner les cibles, puis le code exécute l’action ou copie les valeurs. Le LLM généraliste peut intervenir lorsque la décision ou l’extraction ne suffit pas.

La proposition #3103 sépare explicitement `stagehand.experimentalDecisions.act / observe / extract` des méthodes ordinaires, dont le pipeline LLM est conservé. Vérifie le statut des PR avant toute conclusion sur une version publiée. Les chiffres de performance décrits dans les PR sont des mesures rapportées par leurs auteurs ; nous ne les avons pas reproduits.

Pour notre moteur Rust, nous avons évoqué une interface de modèles permettant des opérations déterministes, des décisions ciblées et des appels LLM. Jev pourrait être un fournisseur optionnel. Son API HTTP est utilisable depuis Rust ; un client `jev-sdk` existe aussi : https://docs.rs/jev-sdk/latest/jev_sdk/.

## 7. Dernier échange : Stagehand, Crawlee et Spider-rs

Nous avons comparé leurs fonctions centrales :

- **Stagehand** : pilotage de navigateur, actions IA et extraction typée.
- **Crawlee** : orchestration de crawlers/scrapers, parcours des URL, files persistantes, retries par requête, concurrence ajustée aux ressources et stockage. Il propose des chemins HTTP et navigateur. Il existe une implémentation TypeScript/Node et une implémentation Python.
- **Spider-rs** : moteur de crawling Rust avec découverte des liens, concurrence, flux de pages, HTTP et fonctionnalités Chrome. Avec les fonctionnalités nécessaires, `crawl_smart()` peut combiner HTTP et rendu navigateur. Il possède aussi des fonctions d’automatisation et des intégrations IA optionnelles.
- **Spider Browser** : projet distinct exposant notamment `act`, `extract`, `observe` et un agent autour de navigateurs hébergés ; comparatif plus direct avec Stagehand que le seul crawler Spider-rs.

**Correction importante de la dernière réponse :** le précédent assistant avait regroupé trop largement « file d’URL, concurrence, retries, gestion des navigateurs et stockage » comme fonctionnalités absentes de Stagehand. J’ai contesté ce point, et il a été corrigé ainsi :

| Sujet | État précisé pour Stagehand v4 |
| --- | --- |
| File d’URL | Pas de file de crawling persistante exposée dans l’API standard, comparable à `RequestQueue` de Crawlee. |
| Concurrence | Oui : la documentation montre des extractions simultanées sur plusieurs onglets avec `Promise.all`, `asyncio.gather` et des goroutines Go. L’organisation des travaux et leurs limites reste dans l’application. |
| Retries | Certains mécanismes existent, notamment `selfHeal` pour retrouver une cible dont le sélecteur a changé et réessayer. Cela ne définit pas une politique de reprise d’un travail complet de crawling. |
| Navigateurs | Oui : lancement local ou cloud, connexion à un navigateur existant, pages et API de fermeture. Le cycle de vie doit être géré par l’appelant ; `stagehand.close()` et `browser.close()` ont des rôles distincts. Un pool automatiquement dimensionné pour une file de crawling reste une autre responsabilité. |
| Stockage | Cache géré par Browserbase pour les appels éligibles ; ce cache nécessite une session Browserbase. Les données extraites sont retournées au programme. Datasets, exports et historique des travaux relèvent de l’application. |

Sources de la correction :

- https://docs.stagehand.dev/v4/best-practices/using-multiple-tabs
- https://docs.stagehand.dev/v4/basics/act
- https://docs.stagehand.dev/v4/configuration/browser
- https://docs.stagehand.dev/v4/reference/stagehand
- https://docs.stagehand.dev/v4/best-practices/caching
- https://crawlee.dev/js/docs/introduction
- https://crawlee.dev/python/docs/introduction
- https://github.com/spider-rs/spider

L’intégration de composants doit être étudiée : une `Page` Playwright fournie par Crawlee n’est pas directement une `Page` Stagehand v4.

## 8. Proposition de stack discutée, encore à valider

Le précédent assistant a recommandé :

- cœur Rust réutilisable comme bibliothèque, accompagné d’un exécutable ;
- Tokio pour l’asynchrone ;
- Serde et un contrat partagé avec schémas pour les SDK ;
- JSON-RPC bidirectionnel, éventuellement sur stdin/stdout en local ;
- Chromium via CDP, en évaluant Chromiumoxide et Chromey ;
- Reqwest pour les appels HTTP aux providers IA ;
- tracing pour le diagnostic ;
- un SDK TypeScript en premier, puis Python et Go selon les besoins ;
- un moteur compilé par plateforme, lancé automatiquement par le SDK pendant la session ;
- un mode distant à envisager pour les environnements où l’exécution locale d’un binaire ne convient pas.

Une autre voie étudiée est l’intégration native dans le processus consommateur : `napi-rs` pour Node, PyO3/maturin pour Python, et d’autres bindings selon le langage. Ces options permettent aussi de distribuer du code Rust compilé sans imposer la toolchain Rust aux consommateurs.

Le processus séparé a été recommandé pour partager le même moteur et protocole entre SDK, CLI et future application native. Il ajoute une responsabilité de gestion du processus et de distribution. La voie native réduit cette séparation, mais demande des bindings et une maintenance propres aux environnements concernés. Le choix final n’a pas été acté.

Nous avons aussi reconnu que TypeScript serait le chemin le plus direct pour reproduire l’architecture d’extension Chrome de Stagehand. Le cœur Rust proposé implique une architecture différente ; quelques helpers JavaScript exécutés dans les pages resteraient utiles. Aucun gain de performance global n’a été démontré : navigateur, réseau, modèle et nombre d’allers-retours peuvent dominer le coût.

## 9. État concret du travail

Nous sommes en phase de recherche et de décision d’architecture. **Aucun moteur Rust ou SDK Automan n’a été implémenté ou validé dans le dossier du projet.** Aucun benchmark comparatif de notre moteur n’a été exécuté.

Dossier de travail : `/Users/jb/Dev/automan`.

Au moment de la reprise, ce dossier appartient au dépôt Git parent `/Users/jb/Dev`, sur la branche `master` ; ce n’est pas un dépôt autonome. Vérifie l’état réel avant de modifier des fichiers et préserve tout travail extérieur à Automan. Applique les instructions du projet et mes règles personnelles, notamment celles sur les suppressions de fichiers et les opérations distantes destructives.

Documents existants, à consulter selon le besoin :

- Usages, architecture v4 et retours Reddit : `/Users/jb/Dev/automan/research/stagehand-usage.md`.
- Vision initiale de la suite Rust : `/Users/jb/.codex/visualizations/2026/10/07/01a115cb-677e-7d23-a09d-5215bf8851c6/architecture-suite-rust.md`.
- Proposition d’architecture plus large : `/Users/jb/.codex/visualizations/2026/10/07/01a115cb-677e-7d23-a09d-5215bf8851c6/automan-architecture/ADR-001-shared-rust-block-runtime.md`.
- Audits des trois produits : `/Users/jb/.codex/visualizations/2026/10/07/01a115cb-677e-7d23-a09d-5215bf8851c6/replica-plus/README.md`.

Les documents de vision et l’ADR concernent un scope plus large, antérieur à ma réduction du projet. Ils servent de contexte, pas de décision finale pour le framework actuel. Les témoignages Reddit sont des déclarations d’utilisateurs ; ils ne prouvent pas la fiabilité de v4 ni une adoption mesurée.

## 10. Comment reprendre avec moi

Reprends la décision de stack et la définition du premier framework, en conservant Rust comme exigence du cœur. Explique les conséquences concrètes pour un consommateur Node, Python ou Go, ainsi que pour une future CLI et une application macOS.

Les questions encore ouvertes sont :

1. Où placer le runtime et comment le distribuer : exécutable lancé par les SDK, bindings natifs ou autre solution argumentée ?
2. Quelle base CDP Rust réutiliser et quelles fonctions navigateur devons-nous développer nous-mêmes ?
3. Quel contrat public prévoir pour sessions, pages, actions déterministes, `act`, `observe`, `extract`, schémas, événements, erreurs et annulation ?
4. Quelles responsabilités de concurrence, reprise et persistance appartiennent au framework initial, et lesquelles restent dans un futur module de crawling ?
5. Comment rendre les modèles de décision comme Jev et les LLM interchangeables, avec une possibilité d’intégration aux modèles du programme consommateur ?
6. Quels parcours et mesures permettront de définir puis vérifier « meilleur que Stagehand » ?

Présente une recommandation précise, ses alternatives et ses limites. Distingue les exigences que j’ai formulées, les fonctionnalités vérifiées chez les références et tes propositions. Utilise des exemples concrets : je me suis plusieurs fois perdu dans les différences entre navigateur, framework, agent, crawler et service cloud.

Les informations techniques évoluent vite : vérifie les sources officielles, la version et le statut des PR lorsque cela change la décision. Préfère Exa pour les recherches web et Context7 pour les documentations lorsque ces outils sont disponibles et adaptés.

Progresse avec les informations disponibles, en arrière-plan lorsque possible. Réserve les questions aux choix qui changent réellement le résultat. À ce stade, la demande de reprise concerne la conception et le choix du stack ; l’implémentation complète de la suite reste une étape ultérieure.
