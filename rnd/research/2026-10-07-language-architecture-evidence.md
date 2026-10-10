# Langage et architecture : consensus ou convergences observées ?

Relevé du **7 octobre 2026**. Cette note réutilise les inspections du dossier et vérifie directement quatre familles de sources primaires : Stagehand, Crawlee/Impit, Spider et agent-browser. Aucun nouveau benchmark, sondage communautaire ou balayage Reddit n’a été réalisé. Les lectures de `main` décrivent les sources courantes, pas nécessairement une release distribuée. L’hypothèse de petite équipe et de calendrier limité a été retirée.

## Ce que l’on peut affirmer

**Les sources examinées n’établissent aucun consensus sur un meilleur langage.** Elles montrent plusieurs architectures viables et des décisions distinctes selon la couche du produit. La présence de Rust dans plusieurs projets établit son usage, pas une préférence majoritaire de la communauté, une supériorité générale ou une obligation de migration. Nous n’avons ni enquête représentative ni comparaison couvrant les mêmes contrats, ressources et résultats.

**Stagehand v4** déplace la gestion des cibles, frames et commandes CDP dans une extension proche de la page. Les SDK TypeScript, Python et Go communiquent avec ce runtime partagé. Le protocole bidirectionnel repose sur JSON-RPC ; ses schémas Zod produisent JSON Schema et des modèles clients. La décision documentée porte sur le placement de l’état et l’unité des contrats, sans imposer Rust. Le chiffre de batching du blog est explicitement un seul parcours, pas un classement général. [Architecture v4](https://www.browserbase.com/blog/stagehand-v4), [protocole courant](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md).

**Crawlee TypeScript** sépare l’acquisition HTTP/navigateur des mécanismes de queue, stockage, sessions, hooks et régulation de concurrence. Son adapter Impit permet un transport natif : Impit est une bibliothèque Rust avec interfaces Node et Python. C’est un contre-exemple concret à l’opposition « SDK TypeScript donc tout le chemin coûteux est JavaScript ». La documentation 3.18 annonce Impit comme futur défaut de la prochaine version majeure ; cela ne signifie pas qu’il est déjà le défaut partout. [Introduction Crawlee](https://crawlee.dev/js/docs/introduction), [adapter Impit](https://crawlee.dev/js/docs/guides/impit-http-client), [bibliothèque Impit](https://github.com/apify/impit).

**Spider** emploie Rust, Tokio et des transports HTTP ; son manifeste courant relie le chemin Chrome à Chromey. Les features conditionnent les capacités disponibles. Ce choix fournit une référence native sérieuse, mais le manifeste ne prouve ni actionability équivalente à Playwright ni résultat métier correct. Les constats détaillés sur la release 2.53.9 restent séparés des changements de `main`. [Manifeste courant](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml), [inspection de la release](2026-10-07-crawlee-spider-contracts-followup.md).

**agent-browser** documente actuellement une CLI et un daemon Rust utilisant CDP directement, sans Node ou Playwright nécessaires au daemon. Le processus persiste entre commandes. Cela confirme la faisabilité d’un exécuteur natif avec sessions ; cela ne démontre pas que reproduire son driver serait préférable à réutiliser Playwright pour chaque besoin. [Architecture et exigences courantes](https://github.com/vercel-labs/agent-browser#architecture).

## Convergences plus solides que le classement des langages

**Inférence à partir de ces architectures :** distinguer interface développeur, exécution et transport paraît plus utile que choisir un langage unique pour toutes les couches. Un SDK peut rester idiomatique tandis que le moteur, le navigateur et le transport vivent ailleurs. Proximité du navigateur, sessions réutilisées, commandes groupées et HTTP lorsque suffisant peuvent réduire le travail et les échanges indépendamment du langage.

Ces convergences restent partielles. L’extension Stagehand et le daemon natif n’ont pas le même cycle de vie ; le crawler Crawlee et un agent ne possèdent pas les mêmes règles de reprise. Aucun protocole partagé ne rend leurs objets `Page` interchangeables. Les schémas garantissent une forme de message ; la parité fonctionnelle exige aussi des tests de comportement.

## Avantages et coûts pour Automan

Le tableau suivant est une **analyse de conception**, pas un verdict communautaire.

| Choix | Avantage recherché | Coût à accepter |
| --- | --- | --- |
| TypeScript | Intégration directe à l’écosystème Playwright/Crawlee, recettes et outils développeurs ; transport natif possible sans réécrire tout le moteur. | Déploiement Node et dépendances ; plusieurs clients exigent un protocole ou des bindings ; les opérations CPU et allocations doivent être profilées. |
| Rust | Bibliothèque et exécutable communs, contrôle des ressources et partage du comportement entre SDK ; candidat cohérent pour HTTP, streaming et workers. | Packaging par plateforme, complexité async, IPC et qualification du driver. Les garanties mémoire ne prouvent ni fraîcheur des données ni réussite d’un workflow. |
| Python | SDK, extensions d’extraction et intégration aux applications/modèles ; possibilité de déléguer les opérations au moteur natif. | Environnement et distribution à gérer ; duplication du runtime à éviter. Les attentes réseau et navigateur interdisent de déduire la latence du seul langage. |
| Go | Alternative plausible pour un service concurrent et un exécutable autonome, ou client du protocole partagé. | Évaluer les bibliothèques navigateur et transports selon les capacités requises ; un second moteur Go créerait une deuxième implémentation à maintenir. |

## Recommandation contextualisée

Pour l’ambition annoncée, **Rust canonique avec SDK TypeScript/Python** constitue une recommandation cohérente ; Go peut devenir un client supplémentaire. Ce choix vise distribution, unité du comportement et maîtrise des ressources. Il ne résulte pas d’un consensus démontré.

Le moteur devrait séparer frontier de crawl, plans durables, sessions, acquisition HTTP/navigateur, résolution sémantique et validation. Un backend CDP natif reste candidat à qualifier ; Playwright fournit une référence et un backend de compatibilité. Le protocole versionné doit expliciter capacités, erreurs, annulation, budgets et ownership. Le routing choisit le chemin adapté au résultat attendu, sans imposer un essai HTTP avant chaque navigateur.

La décision devient vérifiable avec un corpus commun : sortie fraîche et complète, mauvaise cible encore cliquable, perte CDP après effet, expiration d’auth et interruption du sink. Publier coût complet, débit valide, mémoire totale, faux succès et interventions. **Rust peut améliorer un composant ; seul le benchmark du système prouvera l’avantage d’Automan.**
