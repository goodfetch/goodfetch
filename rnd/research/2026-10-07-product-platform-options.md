# Automan : plateformes, surfaces et frontière produit

**État étudié : 7 octobre 2026. Statut : propositions de conception à intégrer au PRD et aux ADR.** Aucune application, exécution cloud, mesure de performance ou qualification de sécurité n’a été réalisée pour cette note. La contrainte de petite équipe et de calendrier 3–6 mois a été retirée ; elle ne gouverne aucun choix ci-dessous.

Cette note complète l’[ADR du runtime](../adr/0001-runtime-language.md), les [contrats Crawlee/Spider](2026-10-07-crawlee-spider-contracts-followup.md) et l’[audit Stagehand](2026-10-07-stagehand-alternatives-followup.md). Elle vérifie les surfaces de plateforme auprès des sources officielles et distingue leurs faits de nos recommandations. Les documents courants sont consultés à cette date ; leurs liens ne constituent pas une version figée à installer. Les versions, SDK MCP et capacités du navigateur devront être épinglés lors de l’implémentation.

## 1. Décisions proposées

Automan devrait exposer **un runtime canonique Rust**, des SDK TypeScript/Python, une CLI, un adapter MCP, une console web et une app macOS native. Une exécution locale doit être utile sans abonnement, modèle obligatoire ou connexion au cloud Automan. Le futur SaaS apporte exécution hébergée, flotte, partage, gouvernance et exploitation ; il utilise les mêmes contrats et workers sans prétendre que toute sa topologie existe sur un ordinateur local.

**Choix web explicite : React + shadcn/ui + Vite pour la console opérationnelle**, produite comme bundle statique. Le daemon peut la servir en local ; le SaaS sert le même produit derrière son API et son identité. Next.js est un choix pertinent pour le site public et les pages nécessitant rendu serveur, et devient une alternative à la console si un besoin concret de SSR ou de BFF justifie ce serveur. Ce choix SPA est contextualisé, pas une recommandation générale contre les frameworks React.

**Choix macOS explicite : SwiftUI avec AppKit pour les composants spécialisés**, reliée au daemon commun. Une app essentiellement constituée d’une WebView React ne satisfait pas cette décision. Réutiliser schémas, données et tokens de design ; implémenter réellement navigation, fenêtres, commandes, listes, raccourcis et intégrations système avec les APIs natives.

**Choix de distribution : binaire/daemon plutôt que FFI comme interface principale des SDK et de l’app.** La bibliothèque Rust reste disponible pour l’embarqué ; un binding FFI exige un bénéfice mesuré ou une contrainte de déploiement précise. Le protocole privé Automan et le protocole standard MCP sont deux contrats distincts.

## 2. Ce que le choix de runtime change réellement

Les références ne démontrent aucun consensus universel sur Rust. Stagehand v4 place son runtime dans une extension Chrome ; Crawlee existe en TS et Python ; Spider est Rust. Impit montre aussi qu’un moteur exposé en TS/Python peut déléguer le transport à du natif. Le langage client, la localisation de l’état et le coût total d’une exécution sont des décisions différentes. [Stagehand](https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md), [Crawlee Python](https://crawlee.dev/python/docs/guides/architecture-overview), [Impit](https://github.com/apify/impit), [Spider](https://docs.rs/crate/spider/2.53.9/source/).

| Architecture | Avantage de contrat et de topologie | Coût ou limite à qualifier | Avis Automan |
| --- | --- | --- | --- |
| Rust canonique + clients minces | Une implémentation des sessions, budgets, crawl et reprise ; binaire commun local/cloud ; contrôle des ressources du cœur | Packaging par OS/architecture, IPC, async interprocessus et maintenance des adapters navigateur | Cible proposée, sans promesse de gain de latence liée au seul langage |
| TS-first + API/clients | Intégration directe Playwright/Crawlee et code de recettes web ; même écosystème pour console et outils | Node et dépendances dans le worker ; clients non TS passent également par un contrat ; profilage mémoire/CPU nécessaire | Alternative crédible, et backend Playwright de compatibilité dans la cible Rust |
| Python-first + API/clients | Intégration aux pipelines de données, agents et bibliothèques de modèles | Distribution de l’environnement ; extensions natives possibles ; mêmes enjeux d’ownership et durabilité | Alternative crédible ; SDK Python proposé sans deuxième moteur autonome |
| Runtime en extension Chrome | État et opérations près de la page ; clients indépendants ; accès à certaines intégrations navigateur | Cycle de vie de l’extension et du navigateur, permissions, distribution ; acquisition HTTP et journal durable restent à organiser ailleurs | Adapter ou recorder possible, pas unique autorité de toute la plateforme |

Une extension a donc une place utile même avec Rust : perception proche du DOM, capture du recorder, liaison à un navigateur utilisateur. Son stockage ne doit pas devenir la seule source du journal de reprise. Une extension ou un worker TS qui possède la session doit être le seul contrôleur de celle-ci pendant son usage ; pas deux boucles CDP concurrentes. Le driver Rust, le worker Playwright et l’extension annoncent leurs capacités réelles derrière le contrat public. Le choix Chromey/Chromiumoxide reste une qualification séparée.

Le gain de placement, de batching ou d’évitement d’un rendu se mesure à qualité équivalente, avec Chromium, IPC, modèle et toutes les tentatives dans le coût. Une CLI Rust associée à un navigateur et à un worker Node n’est pas un processus unique sans GC.

## 3. Frontend : SPA retenue, Next.js possible sous conditions

React recommande de commencer les nouvelles apps avec un framework ; sa documentation autorise aussi une construction avec Vite lorsque les contraintes justifient le choix, en rappelant les responsabilités de routage et de données qui en découlent. Notre contrainte spécifique est une console connectée à un exécuteur local ou distant, sans besoin de SEO pour les runs authentifiés. [Créer une app React](https://react.dev/learn/creating-a-react-app), [construction depuis zéro](https://react.dev/learn/build-a-react-app-from-scratch).

Vite produit un bundle déployable sur un hébergement statique. Le moteur d’exécution peut donc servir ce bundle sans ajouter Node en production locale. Il faut choisir explicitement le routeur, le client de requêtes, la gestion des abonnements, des erreurs et de l’identité ; Vite ne fournit pas ces contrats. [Build Vite](https://vite.dev/guide/build).

Next.js peut également faire un export statique. Dans ce mode, les fonctionnalités qui nécessitent un serveur à la requête ne sont pas disponibles ; du code serveur exécuté au build ne devient pas une API locale dynamique. Les cookies, Server Actions et flux d’authentification nécessitant son serveur ne doivent pas être utilisés comme si le même bundle exporté les possédait. Un déploiement Next avec serveur peut fournir SSR/BFF, avec un processus et une frontière d’authentification supplémentaires à opérer. [Export statique Next.js](https://nextjs.org/docs/app/guides/static-exports), [déploiement](https://nextjs.org/docs/app/getting-started/deploying).

shadcn/ui distribue du code de composants que l’application possède et adapte ; il propose des installations Vite et Next. Ce choix aide la cohérence visuelle sans prendre en charge à lui seul les écrans, l’accessibilité de leur composition ou les performances des grandes tables. [Présentation](https://ui.shadcn.com/docs), [Vite](https://ui.shadcn.com/docs/installation/vite), [Next](https://ui.shadcn.com/docs/installation/next).

La console doit présenter les opérations produit : projets, recettes, frontier de crawl, runs, données, sessions, preuves, coûts et connexions. Elle distingue clairement résultat vérifié, partiel, demande d’intervention et effet incertain. « Annulation demandée » reste visible jusqu’à l’acquittement du runtime ; une déconnexion d’interface ne devient pas automatiquement un run annulé. La donnée possède sa fraîcheur et sa provenance propres, même quand le plan est issu du cache. Le détail technique des drivers appartient aux diagnostics, pas au parcours nominal.

**Gate du choix SPA :** réaliser les parcours sur daemon local et API cloud avec le même build, routage profond, connexion/reconnexion et pagination de données volumineuses. Réévaluer Next si SSR, intégration organisationnelle ou BFF indispensable change ce résultat. Mesurer une navigation et une liste réelle ; ni React ni Next ne prouve la fluidité de la console.

## 4. macOS : vraie app native et runtime partagé

SwiftUI décrit des vues, contrôles et scènes ; Apple prévoit l’intégration d’AppKit. `NSViewRepresentable` permet d’intégrer une vue AppKit avec son lifecycle et sa coordination dans SwiftUI. Notre app peut employer SwiftUI pour la structure et AppKit pour les listes complexes, l’édition, les commandes ou les comportements de fenêtre qui le nécessitent. [SwiftUI](https://developer.apple.com/documentation/swiftui), [NSViewRepresentable](https://developer.apple.com/documentation/swiftui/nsviewrepresentable).

Le daemon gère journal, navigateur et runs. L’app est un client disposant de sélection de fichiers, menus, raccourcis, fenêtres et accès aux secrets via une intégration native. Keychain est l’API système de stockage chiffré de petits secrets ; il faut qualifier les règles d’accès entre app, helper et CLI au lieu de supposer qu’une signature leur donne automatiquement un accès partagé. [Keychain Services](https://developer.apple.com/documentation/security/keychain-services).

Deux lifecycles sont proposés : lancer le runtime à la demande, ou activer un service de fond par utilisateur pour les runs et horaires locaux. `SMAppService`, disponible à partir de macOS 13, gère des helpers inclus dans le bundle ; leur enregistrement et leur lancement restent soumis à l’approbation système applicable. Un LaunchAgent utilisateur est la cible normale ; aucune nécessité de privilèges root n’est établie. Définir le macOS minimal après inventaire des APIs, avec macOS 13 comme candidat cohérent pour cette intégration. [SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice).

Le canal principal proposé est IPC local par utilisateur, avec permissions du socket et vérification des pairs, plus un adapter HTTP loopback pour la console. La CLI et les SDK peuvent lancer un sous-processus en stdio ou se connecter au daemon selon une option explicite. Un seul daemon écrit le journal d’un workspace ; démarrage concurrent et découverte de version doivent être résolus sans course. Quitter l’UI, mettre la machine en veille et supprimer la connexion client ont des effets documentés séparément.

FFI évite la sérialisation mais transfère au binding l’ABI, la propriété mémoire, les appels async, l’annulation, les callbacks et le partage des erreurs. Un crash du code embarqué peut affecter son hôte. Le processus séparé apporte une frontière de cycle de vie et de diagnostic ; il ne constitue pas à lui seul un sandbox de sécurité. Apple précise qu’un enfant `Process` hérite du sandbox de l’app sandboxée et recommande XPC lorsqu’un helper nécessite des entitlements différents. Un broker XPC peut donc être utile pour une distribution sandboxée, sans réécrire le moteur Rust. [Process](https://developer.apple.com/documentation/foundation/process).

Distribution directe proposée : bundle signé Developer ID, Hardened Runtime, notarisation et ticket attaché aux livrables concernés. La notarisation Apple est un contrôle automatisé du code et de la signature, pas une validation du produit comparable à App Review. Qualifier helper Rust, worker Node éventuel, browser géré et mise à jour signée ensemble. La distribution Mac App Store est une décision distincte, nécessitant une qualification réelle des sandbox/entitlements et du lancement des navigateurs. [Notariser un logiciel macOS](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

## 5. Contrats SDK, CLI, API et MCP

Les SDK sont minces sur l’exécution mais idiomatiques : types, builders, helpers async, streaming et erreurs propres à chaque langue. Ils ne recréent pas l’algorithme de routing, les retries ou la sémantique de succès. Une callback utilisateur reste du code exécuté par son hôte ; elle ne migre pas automatiquement vers le cloud. Un provider de modèles passe par une référence de provider côté worker ou un adapter explicitement disponible côté client, avec timeouts et limites.

| Surface | Contrat principal | Propriété qu’elle ne doit pas s’attribuer |
| --- | --- | --- |
| Bibliothèque Rust | API interne publique de modules, qualification des versions | Isolation de code arbitraire par le seul langage |
| SDK TS/Python, CLI | Protocole privé Automan versionné ; stdio ou connexion au daemon/service | Possession d’un second moteur ou reprise autonome d’une session |
| Console/app macOS | Commandes, snapshots et événements autorisés | Source d’autorité des états terminaux et coûts |
| API cloud | Jobs, sessions, recettes, datasets, événements et artefacts | Authentification par un `tenantId` fourni librement par le client |
| Adapter MCP | Outils et ressources, selon une version MCP qualifiée | Durabilité, identité métier ou protocole privé Automan implicites |

Le protocole privé peut reprendre la proposition JSON-RPC bidirectionnelle sur stdio/WebSocket sécurisé de l’ADR, avec types générés, erreurs structurées, budgets, limites de messages et capacités explicites. Les artefacts volumineux sont transférés par références et flux bornés. L’API publique peut offrir commandes HTTP et événements SSE avec curseur Automan ; aucun besoin établi n’impose de remplacer toutes ces surfaces par WebSocket.

**Attention à la version MCP :** `latest` pointe actuellement vers la révision **2026-07-28**, dont les transports standards sont stdio et Streamable HTTP. Elle porte version/capacités par requête. Il ne faut pas attribuer au MCP actuel une initialisation/session de protocole ou des requêtes JSON-RPC indépendantes envoyées par le serveur comme dans les révisions antérieures. Le contrat privé bidirectionnel Automan reste possible. [Transports MCP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports).

Streamable HTTP 2026 a supprimé le GET de stream et les sessions de protocole ; les interactions sampling/elicitation passent par des demandes intégrées aux résultats. La spec impose validation d’Origin et recommande bind localhost/auth en local. Ses SSE ne supportent pas `Last-Event-ID` ; notre cursor de journal n’est donc pas une garantie offerte par son transport. Qualifier les versions effectivement prises en charge par les clients ciblés et leur SDK ; un adapter de compatibilité peut être nécessaire. [Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http).

En cloud, l’auth Automan est obligatoire dans la proposition même si l’authorization est optionnelle dans le standard MCP. Pour un endpoint MCP HTTP protégé, appliquer les exigences MCP/OAuth : découverte des métadonnées, tokens destinés à cette ressource et validation des audiences ; ne pas recycler un bearer d’un autre provider. En stdio, la spec propose de récupérer les credentials depuis l’environnement plutôt que d’appliquer ce flux HTTP. [Authorization MCP](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

Un appel long crée un `RunId` persistant et retourne un reçu après persistance de son acceptation. Observation, reprise et annulation se font sur ce run. Annuler une requête MCP ou fermer un stream annule le travail de cette requête selon la version de protocole ; la correspondance avec l’annulation d’un job détaché doit être explicitement définie par l’outil. Une perte de réponse à `run.create` se résout avec une clé d’idempotence et une lecture du reçu, sans créer silencieusement un deuxième workflow.

## 6. Local/cloud : parité des résultats, différences d’exploitation

La parité vise une recette versionnée, ses exigences, ses résultats, erreurs, vérifications et politiques. Elle ne signifie pas copie de cookies, accès à des fichiers locaux, IP/proxy identiques, capacité illimitée ou portabilité de fonctions arbitraires. Chaque environnement annonce ses capacités. L’export d’un projet est inspectable, avec recette, schémas et références de secrets ; les valeurs secrètes ne voyagent pas implicitement.

Le local conserve journal, checkpoints et datasets dans des adapters embarqués, SQLite étant la proposition pour le journal. Le cloud a un control plane pour identité, ordonnanceur, admission, quotas, secrets et facturation ; des workers exécutent les mêmes plans près des navigateurs. PostgreSQL et stockage d’objets sont des adapters proposés. Il faut un schéma logique commun et des tests des transitions, pas une abstraction qui prétend que SQLite et un système distribué ont les mêmes transactions.

Le cloud autorise une commande par principal, organisation et workspace dérivés d’une identité validée. Les capacités distinguent lecture de données, exécution, publication de recettes, accès aux connexions et actions externes. Les secrets sont des références résolues juste à temps avec portée et audit. Le contenu d’une page, les fichiers téléchargés et les réponses de modèle restent des données non fiables ; ils ne peuvent étendre les autorisations du run.

L’isolation doit couvrir comptes navigateur, cookies, caches de données, clés, proxy, stockage temporaire et artefacts. Un contexte navigateur est une unité d’état ; ce n’est pas une isolation complète de code client non fiable. Les recettes de confiance peuvent partager un worker selon politique. Pour code/plugins tiers arbitraires, choisir container ou VM/sandbox avec restrictions de ressources, réseau et filesystem ; qualifier les garanties réellement obtenues. L’accès au réseau interne et aux fichiers locaux doit être une capacité explicite pour les déploiements qui en ont besoin.

Une cible utile est trois modes compatibles : ordinateur local, service autohébergé, SaaS Automan. Un adapter de navigateur distant, par exemple Browserbase, est un choix d’infrastructure ; il ne force pas à envoyer les données ou modèles à son gateway. Les modes BYOK, provider hébergé et BYOC se décrivent séparément, avec emplacement des données, responsabilité d’exploitation et coût.

## 7. Durabilité : définir les garanties avant les écrans

Séparer la frontier de crawl du journal des étapes de workflow. Une page collectée peut être replanifiée selon la politique de fraîcheur ; une action « envoyer une commande » ne doit pas être rejouée sur le même principe. Le propriétaire de la queue attribue un lease/epoch ; un ancien worker ne peut pas acquitter une nouvelle tentative. Le journal attribue un identifiant à chaque tentative et effet, et conserve acceptation, début, observations, résultat et validation.

Une écriture locale de checkpoint doit avoir une frontière transactionnelle définie et testée. Le cloud doit définir les écritures atomiques en base, l’outbox d’événements, la publication d’artefacts et leur réconciliation. Une opération externe ne devient pas exactement-once grâce au journal. Après crash entre effet externe et acquittement, le run est `effect_unknown` jusqu’à réconciliation ou intervention ; idempotence du provider ou clé métier peuvent permettre un retry sûr quand leur contrat le garantit.

Résultat final, preuve et facturation se relient à la même tentative. Un échec d’export n’annule pas fictivement un effet déjà vérifié ; un échec de validation n’efface pas les coûts engagés. Les caches séparent perception, plan et donnée. Un plan peut être réutilisable tandis que son résultat est périmé. Les caches tenant/session ne se partagent qu’à portée explicitement autorisée.

L’UI affiche le snapshot autoritaire et rattrape les événements avec curseur, déduplication et pagination. Un client lent ne bloque pas le worker et n’accumule pas une mémoire non bornée. Au redémarrage du daemon, au réveil du Mac ou au reconnect cloud, les interfaces relisent d’abord l’état durable. Définir les règles d’horaires locaux manqués durant la veille plutôt que promettre une disponibilité permanente de l’ordinateur.

## 8. Ce que montrent réellement les services étudiés

**Apify** distingue l’Actor, programme cloud avec entrées structurées, configuration Docker et schémas, de l’exploitation du service et de sa distribution dans le Store. Cette unité déployable inspire un package de recette versionné ; un adapter Actor pourrait aussi distribuer un worker Automan. La consommation inclut compute, stockage, transferts et proxies. Un prix de run affiché selon le plan actuel n’est pas un ledger historique suffisant pour notre facturation. [Actors](https://docs.apify.com/actors), [usage et ressources](https://docs.apify.com/actors/running/usage-and-resources).

**Browserbase** fournit navigateurs, APIs et services managés. Functions héberge du code TypeScript près du navigateur, crée une session par invocation et conserve résultats/logs ; cela confirme l’intérêt du placement, sans prouver que le runtime Rust Automan est accepté tel quel dans ce service. Le chiffre fournisseur « moins de 5 ms » n’est pas une mesure générale de nos workflows. Stagehand OSS et flotte managée sont des périmètres différents. [Présentation](https://docs.browserbase.com/introduction), [Functions](https://docs.browserbase.com/platform/functions/overview), [licence Stagehand](https://github.com/browserbase/stagehand/blob/main/LICENSE).

**Spider** présente son crawler OSS comme un cœur réutilisable, avec un service qui ajoute exploitation de flotte et capacités d’infrastructure. Sa page marketing classe aussi certaines fonctions IA côté cloud, alors que notre lecture actuelle trouve bien `spider_agent` OSS : il faut préciser module/version, et ne pas généraliser cette frontière marketing en « aucun agent local ». La parité du navigateur personnalisé cloud n’est pas démontrée par la présence de Chromey OSS. [OSS vs cloud](https://spider.cloud/open-source-vs-cloud/), [audit du code publié](2026-10-07-crawlee-spider-contracts-followup.md).

**Firecrawl** documente les APIs de collecte et processing dans le stack autohébergé ; les formats LLM nécessitent un provider configuré. Agent/Browser, dashboard et contrôles d’entreprise appartiennent à ses offres cloud selon disponibilité. L’autohébergement transfère auth, TLS, persistance, upgrades et recovery à l’opérateur ; son quickstart n’établit pas une production sécurisée et durable. Le dépôt principal est AGPL-3.0, contrairement aux références permissives : ne pas transformer une inspiration fonctionnelle en reprise de code sans inventaire de composants/licences. [OSS ou cloud](https://docs.firecrawl.dev/contributing/open-source-or-cloud), [self-host](https://docs.firecrawl.dev/contributing/self-host), [licence](https://github.com/firecrawl/firecrawl/blob/main/LICENSE).

Ces exemples montrent des frontières possibles entre code accessible et service opéré ; ils ne prouvent ni une demande solvable pour Automan, ni une marge, ni une équivalence fonctionnelle acquise. Les limites publiques peuvent changer ; contrat et capacité testée priment sur un nom de produit.

## 9. Monétisation proposée et contrat de coût

Le cœur OSS peut rester complet pour exécuter, collecter, valider, reprendre et exporter localement. Proposer une licence permissive explicite pour le code propriétaire du projet, sous réserve de l’inventaire de ses dépendances. Les SDK, protocoles et formats restent documentés. La différence commerciale porte sur les ressources et les services rendus : hébergement, flotte isolée, schedulers disponibles, proxies intégrés, stockage géré, collaboration, SSO, audit, support et développement/maintenance de recettes.

Les intégrations sur mesure et leur maintenance sont une offre de service distincte du SaaS. Le Store de recettes est une option de distribution à valider, pas une hypothèse de revenus du PRD. Les fonctionnalités qui garantissent la correction d’un run local ne devraient pas dépendre d’un abonnement pour se comporter correctement.

Définir avant le pricing une comptabilité d’usage append-only : run/tentative, poste consommé, unité, quantité, provider, mode BYOK/managé, heure, région et version du tarif. Séparer estimation, consommation observée, coût d’infrastructure et montant facturable. Une réparation, une page échouée et des appels de modèle annulés peuvent consommer des ressources ; leur facturabilité doit être transparente dans le contrat commercial. Les crédits éventuels ont une conversion versionnée, pas une unité technique magique.

Le budget est une politique d’admission et d’exécution : plafond, reservation, dépenses engagées, seuil d’arrêt et réconciliation des données provider retardées. Aucun plafond exact ne doit être promis si un appel déjà engagé peut le dépasser. Tester facturation dupliquée, événements perdus, provider en retard, changement de tarif et remboursement ; le ledger métier ne se réduit pas au graphique des métriques. Le prix par résultat vérifié reste une métrique produit avec couverture/échecs visibles, pas une excuse pour omettre les coûts des tentatives.

## 10. Décisions ouvertes et gates vérifiables

| Décision ou risque | Preuve requise avant d’en faire une promesse |
| --- | --- |
| Driver natif principal et capacités | Même corpus CDP/Playwright : attentes, navigation, OOPIF/frames, téléchargements, auth, perte de connexion et cancellation ; qualité équivalente avant gain de vitesse |
| Parité Rust/SDK/local/cloud | Tests de contrat d’états, erreurs, budgets, ordre/idempotence et compatibilité de version ; aucune divergence cachée par des types générés |
| MCP courant et clients existants | Versions de protocole/SDK épinglées ; clients réels visés ; transports, auth, input-required, notifications et annulation testés par version |
| Processus local et secrets | Deux clients concurrents ; découverte/upgrade ; permissions IPC ; page web hostile face à l’endpoint local ; Keychain/helper/CLI et absence de secrets dans traces |
| App macOS et packaging | Mac propre, signature/notarisation, helper/browser, absence de blocage UI, permission de fond, arrêt/redémarrage/veille, update et recovery ; architectures cibles explicites |
| Durabilité et effets externes | Injection de crash avant/après effet, validation, commit et acquittement ; leases expirés ; preuve d’absence de replay aveugle et visibilité d’`effect_unknown` |
| Isolation SaaS | Tests croisés tenant/session/cache/artefact/secrets ; code tiers et réseau hostile ; limites mémoire/CPU/disque ; modèle de sandbox publié selon l’offre |
| Console et app sur grands volumes | Pagination, export, backpressure, consommation bornée, reconnexion avec curseur ; clavier/lecteur d’écran et tâches produit réelles |
| Modèle économique | Usage/coût réconcilié et facture explicable ; tolérance aux duplications ; entretiens/offres payées validant la valeur sans extrapoler les benchmarks concurrents |
| Frontière OSS/cloud et licence | Matrice publique fonctionnalités/responsabilités ; installation autohébergée reproductible ; composants et licences épinglés ; contrat d’export des projets |

Les gates organisent une ambition commune, sans calendrier artificiel. Le PRD devrait fixer les parcours et les garanties visibles ; les ADR devraient fixer ownership, persistence, isolation, protocole et packaging. Tout gain commercial ou technique non validé demeure une hypothèse identifiée.
