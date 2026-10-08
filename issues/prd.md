# PRD — Automan

**Statut : proposition produit du 7 octobre 2026.** Le produit n'existe pas encore. Les comportements, APIs, interfaces et critères ci-dessous constituent le travail à réaliser et à vérifier. Aucune performance ou parité n'est annoncée comme acquise.

## Problem Statement

Les développeurs et équipes qui collectent des données web ou automatisent des applications utilisent plusieurs outils : crawler HTTP, gestion de queues, navigateur, agent, scripts de reprise, stockage et monitoring. Les contrats de sessions, retry, validation et coûts diffèrent entre ces outils. Une opération peut terminer sans exception tout en retournant des données incomplètes ou appartenant à la mauvaise identité. Un agent peut résoudre un cas inédit, mais répéter la même tâche engage encore observation et inférence ; un scraper déterministe peut devenir incorrect après une modification du site.

L'utilisateur veut un framework open source réunissant les capacités de Crawlee, Spider-rs et Stagehand, enrichi des meilleurs mécanismes des concurrents pertinents. Il doit être performant, polyvalent, fiable et agréable à intégrer, avec une console React/shadcn, une CLI, MCP et une app macOS native. Un service hébergé utilisera ce framework pour vendre infrastructure, exécution et services associés. Il n'y a pas de contrainte retenue de petite équipe ou de sortie en trois à six mois.

## Solution

Automan fournit un moteur commun de collecte et d'automatisation web. Il choisit HTTP, navigateur déterministe ou décision par modèle selon le contrat de résultat et les capacités nécessaires. La frontier, les plans, les sessions, les pools, l'admission et les budgets ont une sémantique commune locale/cloud.

Le produit offre quatre usages : scraping déterministe, opérations assistées par modèle, agent avec outils typés, et plans générés puis exécutés de façon déterministe. Les plans peuvent être réparés après dérive, avec une version candidate, des tests et une validation du résultat. Les modèles sont facultatifs et configurables ; aucun provider ni service cloud Automan n'est requis pour le chemin déterministe.

La console web est la première surface à construire. Elle sert une boucle complète création → test → run réel → données/preuves → diagnostic → reprise. SDK, CLI et MCP sont construits tôt sur les mêmes contrats ; l'app macOS native et l'infrastructure hébergée utilisent ensuite ces contrats stabilisés. La documentation complète fait partie de la livraison de chaque capacité, et reste à produire avec l'implémentation.

## User Stories

### Création et collecte

1. En tant que développeur, je veux extraire une page avec quelques paramètres afin d'obtenir rapidement un résultat structuré.
2. En tant que développeur, je veux utiliser HTTP sans navigateur et sans modèle afin de limiter le coût d'une collecte statique.
3. En tant que développeur, je veux parser CSS, XPath, JSON et JSON-LD afin d'utiliser le format adapté à ma source, selon les capacités qualifiées.
4. En tant qu'analyste, je veux récupérer contenu, tableaux, liens, métadonnées, Markdown, HTML et fichiers afin d'alimenter mes traitements.
5. En tant que développeur, je veux déclarer un schéma et normaliser dates, devises et unités afin d'obtenir des données exploitables.
6. En tant que développeur, je veux que le moyen d'accès HTTP/navigateur soit choisi et expliqué afin d'éviter les rendus inutiles sans perdre les données.
7. En tant que développeur, je veux définir seeds, découverte, labels et routage afin de parcourir un site selon sa structure.
8. En tant qu'opérateur, je veux règles de domaine/path/query, profondeur, robots, sitemaps et limites de collecte afin de contrôler le périmètre.
9. En tant que développeur, je veux déduplication et canonicalisation configurables afin de distinguer les requêtes réellement différentes.
10. En tant qu'opérateur, je veux scheduling de recrawl et détection des changements afin de maintenir des datasets à jour.
11. En tant que développeur, je veux streaming borné et extraction segmentée afin de traiter de gros volumes sans charger toutes les pages en mémoire.

### Queues, workers et pools

12. En tant qu'opérateur, je veux des queues persistantes nommées afin de reprendre une collecte après arrêt ou crash.
13. En tant qu'opérateur, je veux priorité, délai, leases et dead-letter inspectables afin de maîtriser le traitement des requêtes.
14. En tant que développeur, je veux un contrat clair de retry et d'idempotence afin de limiter pertes et doublons.
15. En tant qu'opérateur, je veux voir les producteurs et requêtes encore actives afin de savoir quand un crawl est terminé.
16. En tant qu'opérateur, je veux fixer concurrence globale, par run, worker, domaine et ressource afin de respecter capacité et quotas.
17. En tant qu'opérateur, je veux autoscaling selon CPU, mémoire, erreurs, quotas et pression des sorties afin de maximiser le débit utile.
18. En tant qu'opérateur, je veux pools de connexions HTTP, navigateurs, contextes, pages, sessions et proxies afin de réutiliser les ressources saines.
19. En tant qu'opérateur, je veux health checks, warmup, retirement, cooldown et quarantine afin d'éviter la réutilisation de ressources dégradées.
20. En tant qu'opérateur, je veux enregistrer des workers et leurs capacités afin d'exécuter une tâche sur une ressource compatible.
21. En tant qu'opérateur, je veux drainer et mettre à jour les workers afin de conserver les tâches et états d'effet correctement.
22. En tant qu'opérateur, je veux répartir les workers par région et classe de ressources afin de rapprocher l'exécution du navigateur ou du site.
23. En tant que développeur, je veux configurer timeouts, backoff, jitter et Retry-After afin de traiter les erreurs temporaires sans retries infinis.
24. En tant qu'opérateur, je veux voir la cause d'une limite de concurrence afin de distinguer manque de capacité, quota fournisseur et sink saturé.

### Navigateur et identité

25. En tant que développeur, je veux lancer ou connecter un navigateur local/distant afin de traiter les sites interactifs.
26. En tant que développeur, je veux locators, attentes et actionability afin de cliquer/remplir sur une cible identifiable et prête.
27. En tant que développeur, je veux navigation, historique, frames, shadow DOM, popups et plusieurs pages afin de couvrir les applications web complexes.
28. En tant que développeur, je veux clavier, pointeur, scroll, touch, formulaires et drag/drop afin de reproduire les interactions nécessaires.
29. En tant que développeur, je veux uploads, téléchargements, dialogs, screenshots et PDF afin d'automatiser les parcours de fichiers.
30. En tant que développeur, je veux événements, interception réseau et évaluation de code de confiance afin d'implémenter des cas avancés déclarés.
31. En tant qu'opérateur, je veux isoler identités, cookies et stockage afin de conserver le bon compte et les bonnes permissions.
32. En tant qu'utilisateur, je veux handoff d'authentification ou d'étape interactive afin de poursuivre un run qui nécessite mon intervention.
33. En tant que développeur, je veux capacités de backend déclarées et erreurs explicites afin d'éviter une dégradation silencieuse.
34. En tant que développeur, je veux configuration proxy et fingerprint cohérente afin d'utiliser les transports disponibles sans promesse de succès universel.
35. En tant qu'opérateur, je veux qu'une session ait un propriétaire identifiable afin d'éviter des commandes concurrentes incompatibles.

### IA, agents et plans

36. En tant que développeur, je veux `observe`, `act` et `extract` assistés afin de résoudre une cible ou des données sans écrire tous les sélecteurs.
37. En tant que développeur, je veux exécuter une action déjà résolue sans nouvelle inférence afin d'amortir une décision.
38. En tant qu'intégrateur agentique, je veux DOM/AX et vision avec provenance afin de fournir à mon agent une perception adaptée et bornée.
39. En tant que développeur, je veux modèles/providers interchangeables et clés personnelles afin de choisir mon coût et mon environnement.
40. En tant qu'utilisateur, je veux un agent intégré avec objectif, outils et limites afin d'exécuter une tâche non encore codifiée.
41. En tant qu'intégrateur, je veux utiliser mes propres agents via outils typés afin de conserver mon orchestration.
42. En tant que développeur, je veux enregistrer une démonstration afin de créer un plan paramétrable plutôt qu'un historique de clics figé.
43. En tant que développeur, je veux compiler une tâche réussie en plan afin de rejouer les cas récurrents sans décider chaque étape par modèle.
44. En tant que développeur, je veux versions, pré/postconditions, boucles bornées et checkpoints afin de rendre le plan inspectable et reprenable.
45. En tant qu'opérateur, je veux caches séparés de perception, plan et données afin de réutiliser les décisions sans retourner des données périmées.
46. En tant qu'opérateur, je veux limites d'actions, tokens, appels, temps et coût afin de borner une exécution agentique.

### Vérification et réparation

47. En tant qu'analyste, je veux vérifier identité, schéma, complétude et fraîcheur afin de détecter les erreurs même sans exception.
48. En tant qu'opérateur, je veux distinguer succès, données partielles, décision requise et effet inconnu afin de décider correctement des reprises.
49. En tant que développeur, je veux détecter une dérive de DOM, pagination ou mapping afin de repérer un scraper devenu incorrect.
50. En tant que développeur, je veux alternatives déterministes et réparation par modèle selon politique afin de restaurer le plan à coût borné.
51. En tant que développeur, je veux diff, tests et preuve de réparation afin d'examiner une nouvelle version concrète.
52. En tant qu'opérateur, je veux promotion automatique qualifiée ou examen manuel afin d'adapter la réparation au type d'effet.
53. En tant qu'opérateur, je veux canary, historique et rollback de version afin de détecter les régressions.
54. En tant qu'opérateur, je veux réconciliation avant répétition d'un effet incertain afin d'éviter une double action externe.
55. En tant que mainteneur, je veux conserver les validateurs requis pendant une réparation afin qu'elle ne masque pas l'erreur en abaissant le contrat.

### Résultats, surfaces et intégration

56. En tant qu'analyste, je veux datasets append/upsert, filtres, projections et exports afin de récupérer et exploiter les résultats.
57. En tant que développeur, je veux commits idempotents, outbox et provenance afin de relier résultats et tentatives après un crash.
58. En tant qu'utilisateur web, je veux créer un workflow et tester un exemple afin de comprendre son résultat avant une collecte complète.
59. En tant qu'utilisateur web, je veux chronologie, données, captures et cause d'échec afin de diagnostiquer un run dans la même interface.
60. En tant qu'opérateur web, je veux gérer queues, pools, workers, budgets et schedules afin de piloter l'exploitation.
61. En tant que développeur TypeScript/Python, je veux une SDK idiomatique locale/cloud afin d'intégrer le moteur dans mon application.
62. En tant qu'utilisateur CLI, je veux commandes composables, sortie JSON/NDJSON et codes de sortie documentés afin d'utiliser Automan dans des scripts.
63. En tant qu'intégrateur MCP, je veux transports standards, schémas et profils de permissions afin de brancher Automan à mes agents.
64. En tant qu'utilisateur macOS, je veux une app SwiftUI/AppKit native afin de gérer les workflows locaux et cloud avec les conventions système.
65. En tant qu'utilisateur macOS, je veux contrôler le travail en arrière-plan et le lifecycle du moteur afin de fermer une fenêtre sans perdre involontairement un run.
66. En tant que développeur, je veux callbacks locaux et packages distribuables avec contrats distincts afin de ne pas supposer qu'une fonction opaque se transfère vers Rust/cloud.
67. En tant qu'intégrateur, je veux API, webhooks et connecteurs de sortie afin d'insérer les automatisations dans mes systèmes.
68. En tant que mainteneur, je veux traces, mesures et fixtures reproductibles afin de vérifier les performances à qualité équivalente.

### Service et documentation

69. En tant que client cloud, je veux identités d'organisation, rôles, quotas et isolation afin d'exécuter mes workflows dans un périmètre propre.
70. En tant que client cloud, je veux flotte hébergée, schedules et capacité réservée afin de déléguer l'exploitation.
71. En tant que client cloud, je veux usage et coût par tentative/run/réparation afin de comprendre et maîtriser ma facture.
72. En tant que développeur, je veux un framework autohébergeable fonctionnel afin de ne pas dépendre du service commercial.
73. En tant que fournisseur de workflows, je veux distribuer un package versionné avec tests et permissions afin de proposer des automatisations réutilisables.
74. En tant que nouvel utilisateur, je veux quickstarts web, SDK, CLI et MCP testés afin d'obtenir un premier résultat réel.
75. En tant que développeur, je veux référence API exhaustive par SDK, CLI, MCP et protocole afin de connaître paramètres, résultats, erreurs et compatibilité.
76. En tant qu'opérateur, je veux guides queues/pools/autoscaling/reprise/stockage/observabilité afin d'exploiter Automan correctement.
77. En tant que développeur, je veux guides déterministe/agent/compilation/réparation et leurs limites afin de choisir la bonne stratégie.
78. En tant qu'utilisateur macOS ou cloud, je veux documentation de mes parcours et de leur configuration afin de comprendre installation, identité, données et lifecycle.
79. En tant que mainteneur, je veux documentation versionnée, exemples exécutés et migrations afin que la documentation corresponde au produit livré.
80. En tant qu'utilisateur d'un concurrent, je veux guides de migration et compatibilité honnêtes afin d'évaluer Automan sans supposer une API identique.

## Implementation Decisions

### Architecture et interfaces

Un moteur Rust/Tokio canonique est proposé, exposé par bibliothèque, daemon et contrat versionné. Les SDK restent des façades idiomatiques ; TypeScript/Python sont les premières langues. HTTP natif, Chromium/CDP et Playwright de compatibilité sont des backends avec capacités déclarées et suite de qualification. Un seul backend possède une session active.

La frontier et l'exécuteur de plans ont des responsabilités distinctes ; une admission commune applique ressources, quotas, budget et backpressure. Pools de workers, HTTP, navigateurs, contextes/pages, sessions, proxies et modèles ont lifecycle, santé et retrait explicites. Autoscaling local et de flotte sont séparés.

État transactionnel SQLite local et PostgreSQL cloud, artefacts en fichiers/object store, outbox pour sorties externes. At-least-once et commits idempotents dans les scopes déclarés ; aucun effet exactement une fois universel sur un site tiers. Fencing interne, session ownership et réconciliation d'effet sont des contrats différents.

La console React/shadcn avec Vite est proposée pour l'exploitation locale/cloud. Next.js reste disponible pour un besoin public/SSR justifié. L'app macOS sera SwiftUI/AppKit, contrôlant le moteur partagé. CLI, MCP et API utilisent les mêmes types et états, avec adapters de transport standards et versionnés.

### Capacités et critères d'acceptation

| ID | Capacité | Critère de livraison vérifiable |
| --- | --- | --- |
| C01 | HTTP/parsing/extraction | Jeux de données attendus obtenus sans navigateur ni modèle ; streaming et tailles bornés ; formats/capacités documentés |
| C02 | Frontier/discovery | Priorité/déduplication/scope/recrawl testés ; fin de crawl tient compte des producteurs ; queue reprise sans purge implicite |
| C03 | Durabilité/sinks | Crash aux frontières de commit sans perte de résultat reconnu ; doublons contrôlés ; outbox et export inspectables |
| C04 | Retry/error policy | Tentatives totales et budget bornés, erreurs classées, délais et effets incertains visibles |
| C05 | Pools/sessions | Ressources saines réutilisées, identité isolée, retrait/drain/quarantine testés, aucun double propriétaire actif autorisé |
| C06 | Workers/autoscale | Admission globale respectée, quotas par domaine/provider, pression du sink observable, drains et reprise testés |
| C07 | Browser | Conformité actions/attentes/frames/fichiers/réseau sur backends déclarés ; incapacité explicite avant action |
| C08 | HTTP adaptatif | Même contrat de données et découverte sur fixtures comparées ; cas vides et pagination manquante détectés |
| C09 | IA/primitives | Déterministe sans clé modèle ; appels IA typés/bornés/traçables ; candidats ambigus non choisis silencieusement |
| C10 | Agents | Outils et permissions appliqués, budgets/annulation testés, objectif vérifié distinct du message d'arrêt du modèle |
| C11 | Plans/recording | Plans paramétrés versionnés, secret refs, reprise de journal et nouvelle collecte distinguées ; replay qualifié sans modèle quand possible |
| C12 | Caches | Scopes/invalidation/freshness séparés ; changements d'identité et de données ne servent pas un résultat privé/périmé |
| C13 | Validation/repair | Erreurs silencieuses détectées, candidats testés, validateurs préservés, canary/promotion/rollback et coût visibles |
| C14 | API/SDK/CLI/MCP | Contrats équivalents, version/capabilities, streaming/cursors/erreurs ; stdio/HTTP MCP conformes à leur révision |
| C15 | Web/macOS | Parcours réels création/run/inspection/réparation/ops ; clavier et accessibility ; lifecycle local qualifié |
| C16 | Packages/connecteurs | Version immuable, entrées/sorties/permissions/tests ; callbacks locaux et code distribué clairement distingués |
| C17 | Cloud/services | Isolation multi-tenant, quotas/scheduling/usage, clés idempotentes et ledger ; limites de plan/capacités visibles |
| C18 | Documentation | Chaque capacité livrée possède guide, référence, exemple testable, erreurs, limites et migrations ; version docs = version supportée |

### Documentation complète à produire

La documentation attendue doit atteindre la couverture fonctionnelle d'une documentation Stagehand/Crawlee/Spider : introduction et quickstarts, concepts, guides thématiques complets, référence exhaustive, exemples, intégrations, déploiement et diagnostic. Elle couvre tous les pools et queues, les modes HTTP/navigateur/IA, compilation et réparation, CLI/MCP, macOS et service hébergé. Elle inclut compatibilité, versions, limites et garanties réelles.

Cette documentation **sera écrite avec le produit**. Le présent PRD définit son contenu et ses critères ; il ne transforme pas les exemples proposés en documentation d'une API déjà disponible. Une capacité n'est livrable que lorsque ses exemples pertinents fonctionnent contre la version correspondante.

### Licence et frontières

Apache-2.0 est proposé pour le framework et les SDK. Les composants empruntés conservent leur licence. Fonctions de services propriétaires, backends de licences différentes et versions incompatibles sont identifiés dans l'inventaire ; leur présence sur GitHub ne les rend pas automatiquement intégrables ou équivalents au cloud. Le framework est utile en local/autohébergement ; la monétisation porte notamment sur exploitation et services, sans verrouiller les primitives fondamentales.

## Testing Decisions

Les tests vérifient des comportements externes et des contrats : mêmes résultats attendus, états, limites, effets et reprises. Les exemples de documentation sont exécutés ; les checks structurels n'ont pas vocation à prouver la fiabilité d'un scraper.

- Fixtures HTTP/JS/API, pagination, listes virtualisées, frames/shadow DOM, authentification, uploads/downloads et formats attendus.
- Crash et partition entre attribution, action, validation, commit, acquittement et export ; ancien worker vivant, lease perdue et session quarantinée.
- Multi-worker/multi-tenant avec quotas, CPU/mémoire, pool saturé, sink lent, modèle limité et deadline globale.
- Comparaison backends à capacité/qualité équivalentes, including erreurs, complétude et découverte ; pas de comparaison HTTP contre navigateur forcé inadéquate.
- Dérives intentionnelles : sélecteur changé, mauvais élément encore valide, cible ambiguë, nombre de pages changé, champ/devise absents, identité erronée et faux message de succès.
- Réparation : candidats rejetés, tests négatifs, absence de baisse de validateur, canary et rollback ; coût de compilation/réparation inclus.
- Agents : provider simulé pour contrats et budgets, tests réels contrôlés pour efficacité ; injections dans observations traitées comme données, outil hors scope refusé.
- Compatibilité inter-SDK, CLI, API, version MCP, annulation et streams ; app macOS install/lifecycle/veille/updates ; UI clavier, mobile et états de panne.
- Benchmarks épinglés : démarrage froid/chaud, même topologie, coût et p50/p95/p99, CPU/mémoire de tout l'arbre de processus, toutes tentatives ; fixtures/validateurs et résultats défavorables publiés.

Aucun benchmark ou test moteur Automan n'a été exécuté dans ce dossier. Les objectifs chiffrés futurs seront qualifiés sur un workload défini, sans transformer une campagne finie en garantie universelle.

## Out of Scope

Pour **ce dossier**, une application de production, un moteur fonctionnel, un hébergement et la documentation utilisateur finale ne sont pas livrés ; ils constituent le développement prévu.

Pour le produit, reconstruire Chromium ou un LLM fondamental, promettre l'accès à tous les sites, l'autoréparation parfaite ou l'effet externe exactement une fois ne fait pas partie des garanties proposées. Les fonctions demandées sont conservées dans les lots ; l'ordre de livraison ne constitue pas leur suppression. La parité des backends et la disponibilité des fonctions cloud tierces sont conditionnées à leurs capacités/contrats vérifiés.

## Further Notes

Le dossier joint fournit comparaison sourcée, architecture, API proposée, design, plan de documentation et gates de développement. La matrice de capacités doit être versionnée pendant l'implémentation, avec trois états au minimum : prévu, implémenté, qualifié. La colonne « qualifié » indique version, périmètre et preuve.

Les priorités découlent des dépendances : console + API + exécution durable réelle ; frontier/pools/workers ; navigateur déterministe ; primitives IA/agents ; plans/repair ; parité des surfaces et service. CLI/MCP, documentation et instrumentation commencent dès que leurs premiers contrats sont disponibles. Aucun planning calendaire n'est fixé par cette proposition.
