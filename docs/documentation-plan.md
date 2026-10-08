# Documentation complète à produire avec Automan

**Exigence et plan de travail — 7 octobre 2026.** Ce document décrit la future documentation du produit. Il ne constitue pas une documentation d'utilisation d'Automan, dont l'implémentation n'existe pas encore. Les exemples d'API du dossier sont des propositions.

## 1. Niveau attendu

Prévoir une documentation comparable en couverture à [Stagehand](https://docs.stagehand.dev/v4/first-steps/introduction), [Crawlee](https://crawlee.dev/js/docs/introduction) et [Spider](https://docs.rs/spider/latest/spider/), avec une distinction claire entre démarrage, concepts, guides, références et exploitation. Une collection de README et d'exemples heureux ne suffit pas à couvrir queues, workers, pools, reprise, IA et service.

Les interfaces web et macOS ont leurs guides. Les développeurs disposent d'une référence complète par SDK et d'un contrat partagé ; les intégrateurs agents disposent de CLI/MCP documentés et d'exemples d'orchestration externe. Les opérateurs disposent des limites et procédures de diagnostic/reprise.

**Règle de livraison proposée : chaque capacité implémentée et qualifiée est accompagnée de ses docs et exemples vérifiés.** Les capacités futures restent dans le plan/version de développement, sans apparaître comme disponibles dans la documentation stable.

## 2. Arborescence cible

```text
Découvrir
  Pourquoi Automan, cas d'usage, architecture, capacités et limites
  Choisir déterministe, assisté, agent ou plan compilé
  Framework open source, autohébergement et service hébergé

Démarrer
  Installer et vérifier l'environnement
  Premier workflow depuis le web
  Première extraction TypeScript / Python
  Premier crawl HTTP puis navigateur
  CLI et première intégration MCP
  Installer/utiliser l'app macOS native
  Exécuter localement, puis dans le cloud

Concepts
  Workflow / plan / version / run / tentative
  Frontier / requête / déduplication / découverte / recrawl
  Worker / processus / tâche async / navigateur / contexte / page
  Session / identité / proxy / capabilities / ownership
  Validation / complétude / fraîcheur / effet inconnu
  Lease / reprise / idempotence / journal / outbox
  Admission / concurrence / budgets / backpressure / autoscaling
  Caches perception / plan / données

Guides de collecte
  HTTP, TLS, headers, cookies, auth, redirects et téléchargements
  CSS / XPath / JSON / JSON-LD / Markdown / tableaux / nettoyage
  Seeds, routers, labels, liens, XHR/JS et sitemaps
  Scope, robots, profondeur, budgets et canonicalisation
  Pagination, infinite scroll, virtualisation et complétude
  HTTP adaptatif et qualification de l'équivalence
  Streaming, gros documents, formats et normalisation
  Datasets, append/upsert, exports, provenance et changements

Guides navigateur
  Launch/connect local et distant, backends et versions
  Locators, attentes, actionability et références invalidées
  Pages/popups, frames/OOPIF, shadow DOM et navigation
  Entrées, formulaires, upload/download, dialogs, screenshot/PDF
  Réseau, interception, événements et code de confiance
  Authentification, identités, stockage et handoff
  Sessions, proxies/fingerprints et limitations d'accès

Guides IA et agents
  Providers, modèles locaux/distant et BYOK
  Observe/act/extract et décisions typées
  DOM/AX, vision, projections et contrôle des observations
  Agent intégré et orchestration externe
  Outils et permissions, mémoire, limites et coûts
  Recorder, paramètres et compilation de plans
  Replay, validation et freshness
  Détection de dérive, repair, tests, canary et rollback

Exploiter
  Queues persistantes, priorités, leases, délais et dead-letter
  Retries, backoff, timeouts, effet inconnu et réconciliation
  Pools HTTP, navigateurs, contextes/pages, sessions, proxies, modèles
  Workers, heartbeat, capacités, régions, drainage et updates
  Concurrence, contrôleurs autoscale et causes de limitation
  Stockage, sauvegarde/restauration, rétention et migration
  Scheduling, timezone, overlap, webhooks et outbox
  Logs, traces, métriques, coût et redaction
  Déployer localement, container, autohébergement et flotte cloud

Références
  TypeScript / Python / Rust public
  CLI, options, stdout/stderr, NDJSON et codes de sortie
  MCP, transports/révisions, outils/profils/resources et auth
  API REST, événements/SSE/cursors et protocole privé lorsque public
  Schémas workflow/plan/package/configuration
  Erreurs, statuts, capabilities, limites et defaults effectifs
  Matrice backends / navigateurs / OS / versions

Interfaces et service
  Console, création, runs, données, repair et exploitation
  macOS : lifecycle, fichiers, credentials et travail en arrière-plan
  Cloud : organisations, rôles, secrets, flotte, quotas et usage
  Facturation : unités, échecs, retries/réparations et estimations
  Packages, registre, versions et provenance

Exemples et intégrations
  Catalogue, export authentifié, changements et contenu RAG
  Plans sans IA, création assistée puis replay, auto repair
  TypeScript/Python, SDK cloud et callbacks/packages
  Agents via MCP/CLI/SDK, connecteurs et sinks
  Intégrations Crawlee/Playwright et frontières de queue/session

Maintenir et contribuer
  Contribution, architecture interne et plugin/backend contract
  Fixtures, benchmarks reproductibles et méthode de qualification
  Changelog, versions, migrations, dépréciations et support
  Diagnostic, problèmes connus et limitations publiées
```

Cette navigation est une cible à affiner après les premiers tests d'usage. L'exhaustivité est suivie par la matrice de capacités, pas par le nombre de pages.

## 3. Contrat d'une page de référence

Pour chaque méthode, commande, configuration et outil : purpose, signature/schéma, paramètres requis et facultatifs, unités, defaults **effectifs**, résultat, erreurs, permissions, capabilities et versions. Inclure retry/timeout/annulation, effets possibles, ressources engagées, comportement local/cloud et cache/freshness lorsqu'ils s'appliquent.

Montrer au moins un exemple minimal et un cas d'erreur pertinent ; un exemple de configuration de pool doit préciser la ressource comptée. Indiquer ce qui dépend du backend/provider plutôt que de promettre une parité générale.

Les références générées ne remplacent pas les explications des garanties. Par exemple, un type `maxConcurrency` n'explique pas si la limite compte tâches, onglets ou requêtes HTTP ; les concepts et guides portent cette distinction.

## 4. Contrat d'un guide

Chaque guide part d'un objectif concret, précise les prérequis, choisit un workflow de référence et termine par résultat observable et diagnostic des échecs possibles. Le lecteur voit ce qu'il peut vérifier, quelles données sont persistées et quels coûts/effets peuvent être engagés.

Les guides déterministes fonctionnent sans clé de modèle. Les guides agentiques distinguent provider de test et exécution réelle, affichent budgets et provenance, et décrivent les arrêts `partial`, `needs_review` ou `effect_unknown`. Les guides de réparation montrent aussi un candidat rejeté, pas uniquement un succès.

## 5. Source, génération et versions

- Rédiger guides/concepts dans le dépôt et réviser avec les changements de comportement.
- Générer les références depuis types/schémas publics et métadonnées CLI/MCP ; vérifier les facades idiomatiques des SDK.
- Publier stable, versions supportées et développement, avec bandeau de version et matrice de compatibilité.
- Lier changelog et migration à chaque changement incompatible ; conserver les URLs documentées ou prévoir redirects.
- Documenter les valeurs defaults de la version distribuée, pas seulement celles d'une branche `main` ou d'une page marketing.
- Relier chaque capacité à ses pages, exemples et tests. Un changement de contrat vérifie ces consommateurs.

Le site de docs pourra utiliser un générateur maintenu avec recherche, navigation, blocs TypeScript/Python/Rust/CLI et export statique ; sa sélection sera faite lorsque les premiers contenus et contraintes de publication existent. Ne pas choisir une seconde plateforme d'exécution du moteur pour les besoins du site de documentation.

## 6. Vérification et critères de livraison

| Exigence | Preuve attendue |
| --- | --- |
| Couverture | Chaque capacité livrée reliée à concept/guide/référence pertinents ; aucun paramètre ou erreur public non documenté |
| Exactitude | Références générées vérifiées contre schémas et defaults distribués |
| Exemples | Quickstarts et exemples déterministes exécutés sur fixtures contrôlées en CI |
| IA | Contrats/budgets testés avec provider simulé ; exemples réels qualifiés séparément avec modèle/version/coût identifiés |
| CLI/MCP | Commandes de quickstart exécutées ; sortie JSON/erreurs/tool schemas et révision protocole vérifiés |
| Reprise | Guides crash/lease/session/effet inconnu reliés à tests de comportement, pas à slogans |
| Web/macOS | Parcours et captures mis à jour pour la version livrée ; états de panne couverts |
| Compatibilité | Backends/versions/OS supportés distingués des expérimentaux et des capacités non disponibles |
| Publication | Build du site, liens locaux/external essentiels et navigation/search contrôlés |
| Migration | Exemple avant/après et recovery des données lorsque le changement le nécessite |

La documentation ne garantit pas une vitesse ni un succès universel. Les benchmarks publiés fournissent paramètres, baselines, fixtures, qualité, coûts, résultats défavorables et limites.

## 7. Travail à intégrer aux lots

**Dès le premier lot** : introduction, installation, concepts initiaux, quickstart HTTP/web, états de run, référence API minimale et erreurs. Les pages futures sont dans le backlog, pas annoncées stables.

**À chaque lot** : guides de la capacité livrée, référence/erreurs/capabilities, exemple reproductible, migration si nécessaire et troubleshooting. Queues/pools/autoscaling et agents/repair ont leurs lots de documentation explicites, avec la même exigence que la feature.

**Avant la première publication du framework** : parcours SDK/CLI/MCP locaux, autohébergement supporté, licensing/composants, contribution, matrice de compatibilité, protocole de benchmark et liens cohérents.

**Avant le service et l'app macOS** : docs installation/lifecycle/credentials et local-cloud pour macOS ; organisations, schedules, quotas, rétention, usage/facturation et limites pour cloud. Le lancement d'une surface comprend sa documentation.
