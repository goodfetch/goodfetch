# Automan — performance, runtime et protocole de preuve

Recherche du 7 octobre 2026. Les constats ci-dessous viennent de sources primaires ; les choix et seuils proposés sont explicitement des recommandations. Aucun framework Automan n'a été implémenté et aucun benchmark comparatif n'a été exécuté pendant cette étude.

## 1. La bonne unité de performance

Un crawler HTTP, un navigateur qui exécute JavaScript, un pilote d'interactions et une boucle d'agent rendent des services différents. Le nombre de pages par seconde ne suffit pas à comparer un agent qui remplit un formulaire avec un crawler qui télécharge du HTML. Le protocole doit fixer le résultat demandé, les conditions d'exécution et le validateur avant de chronométrer.

**Recommandation : optimiser le coût et la latence d'un résultat métier vérifié**, avec une frontière de fiabilité minimale. Publier également le débit, les appels de modèle, les tokens, les octets transférés, la mémoire de tout l'arbre de processus et les interventions humaines. Un résultat absent, partiel, périmé ou attribué au mauvais compte est un échec, même si le programme termine sans exception.

La distinction entre action réussie et objectif accompli est centrale dans WebArena : ses évaluateurs vérifient les effets de la tâche, plutôt qu'une ressemblance entre suites de clics. Cela n'en fait pas un benchmark représentatif de tous les clients. [Article WebArena](https://arxiv.org/html/2307.13854v4), [dépôt canonique](https://github.com/web-arena-x/webarena).

## 2. Rust : intérêt réel, preuve nécessaire

Le temps complet comprend les attentes réseau, l'activité du navigateur, l'inférence, les allers-retours de protocole, le travail du framework et les reprises. Il faut instrumenter leurs durées et leurs chevauchements ; une simple somme de temps CPU ne décrit pas le chemin critique.

**Calcul illustratif, pas mesure :** si 5 % du temps d'un workflow vient du code hôte que Rust pourrait accélérer dix fois, le gain total vaut `1 / (0,95 + 0,05 / 10) = 1,047`, soit environ 4,7 %. Ce calcul suppose les autres coûts inchangés. Rust peut aussi réduire la mémoire, modifier la concurrence ou simplifier la distribution ; ces bénéfices doivent être mesurés séparément.

Vercel propose déjà un outil natif Rust, `agent-browser`. Le README courant indique que son daemon ne requiert ni Node ni Playwright. Le manifeste Rust utilise notamment Tokio, WebSocket, Serde et Reqwest ; `main.rs` sélectionne un mode daemon natif. L'ancien portrait « CLI Rust devant daemon Node/Playwright » ne décrit donc pas correctement l'état actuel de `main`. Ce constat porte sur les sources inspectées, et n'est pas un benchmark indépendant. [README](https://github.com/vercel-labs/agent-browser), [Cargo.toml](https://github.com/vercel-labs/agent-browser/blob/main/cli/Cargo.toml), [main.rs](https://github.com/vercel-labs/agent-browser/blob/main/cli/src/main.rs).

**Direction actuelle :** l'utilisateur a retiré la contrainte initiale de petite équipe/sortie en 3–6 mois. La synthèse recommande désormais un runtime Rust canonique pour distribution, partage entre langages et maîtrise des ressources, avec un backend Playwright de référence. Le langage reste un choix d'architecture, pas la preuve d'un gain. Le pilote natif doit qualifier les attentes d'action, les frames et les cycles de navigation.

Le seuil proposé pour une **optimisation ou substitution de backend** est un gain d'au moins 20 % sur un coût utile — coût complet, latence de bout en bout ou mémoire à débit et fiabilité constants — sur le périmètre cible. Il ne conditionne plus le choix initial du runtime Rust. Ce seuil est une règle de projet suggérée, pas une vérité du marché. Une contrainte de distribution peut aussi justifier un composant, avec sa propre comparaison de coûts de maintenance.

## 3. CDP ne remplace pas un pilote complet

Playwright vérifie notamment unicité de la cible, visibilité, stabilité, réception des événements et activation avant certains clics. Ses locators retrouvent la cible au moment de l'action. Reproduire ces garanties est un travail substantiel ; envoyer une commande CDP ne les apporte pas automatiquement. [Actionability](https://playwright.dev/docs/actionability), [Locators](https://playwright.dev/docs/locators).

Playwright précise que `connectOverCDP()` offre une fidélité inférieure à sa connexion par protocole Playwright. L'attachement CDP à Chromium reste utile, notamment pour des fournisseurs cloud, mais doit être qualifié avec les fonctions réellement utilisées. [BrowserType](https://playwright.dev/docs/api/class-browsertype).

WebDriver BiDi décrit une communication bidirectionnelle avec commandes et événements ; il constitue une piste de portabilité à suivre. Cela ne justifie pas d'implémenter plusieurs protocoles dès le premier produit. [Spécification W3C](https://www.w3.org/TR/webdriver-bidi/), [présentation Chrome](https://developer.chrome.com/blog/webdriver-bidi).

**Optimisations prioritaires proposées :** éviter l'ouverture d'un navigateur lorsque HTTP suffit ; conserver des sessions saines ; exécuter le worker près du navigateur cloud ; réduire les observations inutiles ; grouper les opérations dont les dépendances sont connues ; utiliser des attentes de conditions précises ; réduire le nombre d'appels de modèle. Regrouper aveuglément des clics dépendant d'un état dynamique ferait perdre de la fiabilité.

## 4. Navigateur léger : Lightpanda est à suivre, pas à rendre obligatoire

Lightpanda est un navigateur distinct écrit en Zig, avec JavaScript et sans moteur de rendu graphique équivalent à Chromium. Son code est sous AGPL-3.0. Le README décrit également un mode agent qui produit du PandaScript, rejouable sans modèle sur le chemin déterministe. Ce sont des capacités annoncées et documentées par le projet, non reproduites ici. [Dépôt](https://github.com/lightpanda-io/browser), [licence](https://github.com/lightpanda-io/browser/blob/main/LICENSE).

Son benchmark mérite lecture : il utilise un crawler Go/chromedp, 933 URL d'un site de démonstration Amiibo et plusieurs niveaux de parallélisme. Le document technique courant mentionne une machine AWS `m5.xlarge`, alors que le résumé du README parle de `m5.large`. Les comparaisons multi-processus Lightpanda et multi-onglets Chrome ne permettent pas de déduire un multiplicateur universel pour les workflows authentifiés, les captures visuelles ou les interactions complexes. Les chiffres publiés ne sont pas des mesures Automan. [Protocole et résultats du benchmark](https://github.com/lightpanda-io/demo/blob/main/BENCHMARKS.md#crawler-benchmark).

**Proposition :** conserver un point d'adaptation pour un navigateur léger, expérimenter plus tard sur les workloads sans besoin de fidélité visuelle, et garder Chromium comme recours qualifié. La frontière de licence et la compatibilité des APIs doivent être étudiées pour la distribution envisagée ; un processus séparé ne suffit pas, à lui seul, à conclure sur les obligations d'une intégration.

## 5. Compiler et rejouer existe déjà

La littérature et les produits convergent vers la réutilisation des décisions et du code :

| Source primaire | Ce qui est étudié ou décrit | Limite à conserver |
| --- | --- | --- |
| [Agent Workflow Memory, ICML 2025](https://proceedings.mlr.press/v267/wang25bx.html) | Induction et réutilisation de workflows pour des agents web | Réutiliser une mémoire de workflows n'est pas identique à supprimer toute inférence au runtime. |
| [WALT](https://arxiv.org/html/2510.01524v1) | Découverte de fonctionnalités de sites et transformation en outils réutilisables | Résultats sur WebArena/VisualWebArena ; pas preuve sur les portails d'un client. |
| [Agent JIT Compilation](https://arxiv.org/html/2605.21470v2) | Plans exécutables, invariants de pré/postconditions et choix selon le coût estimé | 37 tâches sur cinq applications ; certaines tâches sont sélectionnées ou construites pour comparer les stratégies de scheduling. Les gains ne se généralisent pas automatiquement. |
| [Agentic Compilation](https://arxiv.org/html/2604.09718v1) | Plan JSON déterministe produit par compilation | Prépublication ; les chiffres de fiabilité après correction humaine ne doivent pas être attribués à une exécution autonome. |
| [Growing Harness](https://arxiv.org/abs/2609.26760) | Réparations de code persistantes, localisation par traces et rejet des régressions | Prépublication récente ; formation, validation et coût hors ligne doivent être comptés. |
| [Anchor self-healing](https://docs.anchorbrowser.io/advanced/self-healing) | Fallback IA pendant le run et nouvelle version de workflow après échec | Fonction de produit commercial ; précision et coût non reproduits ici. |
| [OpenAdapt Flow](https://github.com/OpenAdaptAI/openadapt-flow) | Démonstration, compilation, replay local, vérification d'effet et arrêt explicite | Projet se déclarant bêta ; sa qualification est propre aux workflows et environnements testés. |
| [FlowReplay](https://github.com/kuilenren/flowreplay) | Format portable, locators multiples et replay | Projet alpha ; ses tableaux comparatifs sont des affirmations du mainteneur, pas une évaluation indépendante. |

**Inférence stratégique :** compiler, mettre en cache, réparer ou retourner une preuve ne constituent pas individuellement une nouveauté défendable. Une opportunité éventuelle se trouve dans la qualité d'une implémentation, sa facilité d'intégration et la maintenance d'un périmètre client précis.

## 6. Trois familles de benchmarks

### A. Extraction et crawling

Construire des fixtures servies localement : HTML statique, pagination, contenu JavaScript retardé, scroll avec liste virtualisée, API autorisée, téléchargements et session expirée. Fournir un jeu de données attendu, contrôler doublons et champs absents, et vérifier la complétude de la pagination.

Comparer Automan à un script HTTP/Playwright bien écrit, puis aux modes pertinents de Crawlee, Spider et Scrapy. Les configurations doivent atteindre la même précision et complétude. Une comparaison avec un mode navigateur forcé alors que le concurrent possède un mode HTTP adaptatif serait trompeuse.

### B. Workflows récurrents

Prendre trois workflows pilotes en lecture : télécharger un export depuis un portail authentifié, parcourir un catalogue avec pagination et stock, suivre un changement puis produire un résultat structuré. Mesurer séparément premier apprentissage et réexécutions.

Comparer les outils adéquats : Playwright manuel, Stagehand, Browser Use et Skyvern si le workflow et les interfaces disponibles permettent une comparaison équitable. Ne pas noter Crawlee sur une tâche d'agent autonome qu'il n'a pas vocation à planifier.

Tester : premier run sans cache, run chaud, paramètres nouveaux, changement de langue, changement de structure, expiration d'authentification, réponse partielle et interruption du worker. Les caches sont liés au périmètre de compte et à la version de la recette ; aucun partage implicite d'état authentifié entre clients.

### C. Généralisation et perturbations

BrowserGym et AgentLab constituent un moyen de réutiliser WebArena, WorkArena et d'autres benchmarks. AgentLab documente les problèmes de versions, d'état des sites et de dépendances entre tâches. La concurrence de runs peut corrompre leur validité si les tâches modifient le même environnement. [BrowserGym](https://github.com/ServiceNow/BrowserGym), [AgentLab](https://github.com/ServiceNow/AgentLab).

Ajouter des variantes contrôlées : bouton masqué, changement d'ordre, faux message de succès, valeur absente, iframe, shadow DOM, cookie expiré et changement de sélecteur. Mesurer aussi la fréquence d'arrêt injustifié. Une bibliothèque qui refuse tout peut afficher zéro erreur silencieuse tout en restant inutilisable.

DoomArena apporte des scénarios d'injection dans les observations et outils. Ces essais sont pertinents dès qu'une page non fiable est envoyée à un modèle : les données de la page doivent rester des données, avec capacités d'action limitées et vérification de l'effet. [Projet DoomArena](https://servicenow.github.io/DoomArena/), [article](https://arxiv.org/html/2504.14064v3).

## 7. Métriques et règles de publication

| Métrique | Définition proposée |
| --- | --- |
| Succès vérifié | L'évaluateur indépendant du plan valide l'objectif et la complétude. |
| Erreur silencieuse | Le moteur annonce un succès que le validateur réfute. |
| Arrêt injustifié | La tâche était exécutable, mais le moteur s'abstient. |
| Coût par succès vérifié | Tous les coûts des tentatives, échecs, reprises, compilation et réparation, divisés par les succès vérifiés. Séparer coût machine et travail humain. |
| Latence | p50/p95/p99 pour chaque état terminal, délai maximal et taux d'échec ; identifier clairement les quantiles portant seulement sur les réussites. |
| Débit | Succès vérifiés par minute à ressources et concurrence fixées. |
| Empreinte | Mémoire maximale et moyenne de tous les processus, CPU-seconde, trafic et stockage. |
| Réparation | Fréquence, coût, délai, taux de correction validée et régressions introduites. |
| Intervention | Nombre et minutes d'intervention humaine par 100 runs. |

Épingler versions, commits, navigateur, modèles, prompts, région, proxies, ressources, limite de concurrence et date. Publier les fixtures, recettes, validateurs et traces expurgées. Faire plusieurs répétitions, séparer les données qui servent à régler les heuristiques du jeu final, et rapporter les intervalles d'incertitude. Cent répétitions ne prouvent pas une fiabilité de 99,9 % ; elles servent surtout à détecter les défauts fréquents.

**Objectif initial suggéré :** sur au moins trois workflows pilotes, obtenir une correction comparable à un bon script manuel. Face à une baseline d'agent déclarée avant les essais, viser soit une latence médiane divisée par deux, soit un coût complet par succès réduit d'au moins 50 %. Face au meilleur chemin HTTP/navigateur déjà disponible, mesurer séparément les gains possibles de débit ou de mémoire. Ne pas promettre qu'un replay bat un script déterministe optimal. Publier les résultats défavorables et les perturbations qui cassent encore le moteur. Il s'agit d'un critère de continuation, pas d'une promesse commerciale.

**Amortissement fixé avant les essais :** présenter création, compilation, réparation et maintenance séparément, puis coût complet à 1, 10, 100 et 1 000 exécutions. Déclarer aussi cadence, durée d'observation et taux de changements du site. La seule division par les succès ne suffit pas si le nombre de répétitions est choisi après les résultats.

## 8. La fiabilité impose une sémantique de reprise

Calcul illustratif sous hypothèse d'indépendance : vingt étapes fiables à 99 % ne donnent que `0,99^20 = 81,79 %` de succès complet. Avec 99,9 % par étape, cela devient `98,02 %`. Les vrais échecs peuvent être corrélés ; les chiffres ne prédisent donc pas un taux de production.

Pour l'extraction, une reprise peut souvent être rendue idempotente par une clé et une déduplication. Pour une action externe, perdre la connexion après un clic laisse parfois son effet indéterminé. Une file persistante et des checkpoints n'offrent pas automatiquement une garantie « exactement une fois » sur un site tiers.

**Recommandation de contrat :** états `verified`, `failed`, `partial`, `needs_review` et `effect_unknown`. Après un effet incertain, lire l'état externe lorsque c'est possible avant de rejouer ; sinon suspendre selon la politique du workflow. Les premiers pilotes restent en lecture pour concentrer le travail sur complétude, extraction et reprise.

## 9. Économie illustrative de la répétition

Exemple entièrement hypothétique : 100 exécutions d'un agent à 0,50 € coûtent 50 €. Une compilation à 3 €, des runs à 0,03 € et cinq réparations à 0,50 € coûtent 8,50 €, soit 83 % de moins. Ce calcul omet volontairement des postes et ne constitue ni un prix fournisseur ni une prévision Automan. Le modèle complet doit ajouter navigateurs, proxies, erreurs, stockage, support et travail de maintenance.

Le seuil de rentabilité dépend du nombre de répétitions, du coût d'entretien et de la stabilité du site. Un workflow exécuté une seule fois peut rester mieux servi par un agent généraliste. Un workflow avec une API stable peut rester mieux servi par une intégration API directe. La bonne stratégie est de choisir le moyen suffisant le moins coûteux et de vérifier qu'il atteint l'objectif.
