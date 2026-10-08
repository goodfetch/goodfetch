# Développement, qualification et critères de publication

**Proposition du 7 octobre 2026.** Les lots définissent dépendances et preuves. Ils ne limitent pas l'ambition et ne constituent pas un calendrier ni une estimation d'équipe.

## 1. Découpage par résultats vérifiables

| Lot | Résultat concret | Dépendances | Gate de livraison |
| --- | --- | --- | --- |
| A — Contrats et console | Console React/shadcn, moteur local, API, run HTTP réel, dataset et inspection | Domain model, états, schémas et journal | Création → résultat → export ; arrêt/crash/reprise sur fixture ; premiers docs exécutables |
| B — Collecte complète | Frontier durable, découverte, router, retry, outputs et recrawl | A, transaction/leases/sinks | Périmètre/déduplication/coverage corrects, crash tests sans perte de commit reconnu |
| C — Ressources et workers | Pools complets, budgets, backpressure, workers et autoscale | A/B, mesures et admission | Saturation/quotas respectés, cause visible, drainage et multi-worker qualifiés |
| D — Navigateur déterministe | Backend CDP qualifié, Playwright compatibilité, sessions et fichiers | A/C, identité/capabilities | Suite d'actionability/frames/réseau/downloads ; pas de faux succès sur locator erroné |
| E — IA et agents | observe/act/extract, modèle facultatif, vision, agent et outils externes | A/D, budgets/providers/validation | Erreurs/permissions/usage corrects avec fake provider puis efficacité sur tâches contrôlées |
| F — Plans et réparation | Recorder, compilation, caches, dérive, tests, canary et rollback | B/D/E, suites de validation | Replay frais sans inférence quand applicable ; patch borné, rejet des régressions, effet inconnu non répété |
| G — Surfaces et packages | SDK TS/Python, CLI/MCP, packages, native macOS | Contrats de A et capacités livrées | Parité des états/erreurs ; installations/lifecycle et exemples de chaque surface |
| H — Services hébergés | Flotte, organisations, schedules, rétention, usage/facturation et connecteurs | A–G nécessaires au périmètre vendu | Isolation/quotas/usage idempotents, contrats locaux/cloud cohérents, procédures et docs |

CLI/MCP, SDK, instrumentation et documentation démarrent au lot A avec leurs premières capacités. Les lots peuvent progresser en parallèle sur des contrats fixés : pools HTTP et navigateur, hosts de handlers, design macOS, par exemple. La livraison d'une capacité cloud n'attend pas une parité imaginaire de toutes les plateformes.

## 2. Premier vertical slice : web en premier, moteur réel

Construire un workflow de catalogue HTTP sur fixture, sa version et son run ; afficher chronologie, données et validation dans la console. Un run arrêté est inspectable, un crash avant/après commit se reprend, un export a un état propre. Le SDK et la CLI peuvent lancer le même workflow.

Ce lot valide architecture et parcours ; les écrans avancés peuvent attendre les capacités réelles. Il ne présente pas des pools, un agent ou une réparation simulés comme des fonctionnalités livrées. La maquette est un support de revue, pas le premier produit.

## 3. Inventaire de parité à maintenir

Pour chaque capacité des matrices de [crawl](../research/2026-10-07-crawl-capability-matrix.md) et d'[agents](../research/2026-10-07-agent-capability-matrix.md), suivre : référence/version, contrat Automan, dépendances, backend requis, état prévu/implémenté/qualifié, preuves et documentation. Les nouvelles fonctionnalités concurrentes sont examinées lorsqu'elles changent une décision, pas incorporées automatiquement.

Une fonctionnalité annoncée « complète » doit donner la liste de son périmètre : le pool inclut-il retirement/health/hooks ? Le retry borne-t-il rotations et réparations ? Le navigateur couvre-t-il frames/network/files ? La reprise couvre-t-elle pending/running et outputs, ou seulement visited ?

## 4. Tests de contrats requis

### Durabilité et effets

Crash avant/après prise de lease, réception HTTP, staging emit/discover, validation, commit SQL, acquittement, upload objet et livraison outbox. Tester deux workers, epoch périmé, partition, ancien worker vivant, deadline et redelivery. Vérifier commits uniques et données reconnues conservées ; tracer les provisoires non publiés.

Faire les mêmes interruptions sur workflow autonome sans frontier : activation d'étape, branche/DAG, timer et attente d'authentification. Vérifier la prise en charge durable d'un run même lorsqu'il ne parcourt aucune URL de crawl.

Pour une action externe : effet absent, effet confirmé, réponse perdue et effet inconnu. Aucun replay mutante automatique sans idempotence/réconciliation requise. La reprise après review/unknown terminal crée une continuation liée et revalide les checkpoints ; une attente non terminale reprend selon son contrat.

### Sessions et navigateur

Comptes distincts, logout/token expiré, rotation proxy, fermeture de page, navigation concurrente, popups, frames/OOPIF remplacés, shadow DOM, download immédiat, dialog, route interception et événements. Installer les attentes avant l'action. Vérifier ambiguïté, cible visible mais incorrecte, page fermée et référence périmée.

Qualifier chaque backend sur des versions navigateur épinglées. Ne pas annoncer Firefox/WebKit sur le backend CDP Chromium ; ne pas compter la réussite d'un backend comme qualification de tous les autres.

### Resources et autoscale

CPU saturé, mémoire Chromium élevée, sink lent, response énorme, quota par domaine/modèle, tokens réservés puis utilisés, backlog inadmissible et scale-down. Vérifier limites globales sur plusieurs workers et fairness entre tenants. Un gain de débit qui masque erreurs, backlog ou données perdues ne passe pas.

### Modèles, plans et réparation

Fake providers pour budgets, schémas, erreurs/retry et états ; providers réels contrôlés pour capacité. Observations portant instructions hostiles, données erronées et sources ambiguës. Vérifier objectifs/permissions sans copier les décisions du modèle comme oracle.

Mutations de fixtures : locator changé, même selector avec mauvais élément, langue, devise, ordre, pagination, champ absent, compte changé, faux message de réussite. Tester patch accepté/rejeté, invariants immuables, canary, coût et régression ; données fraîches après replay de plan.

### Surfaces, packaging et documentation

Contrats SDK/API/CLI/MCP équivalents et négociation de révision, stdout JSON, streams bornés, cursors expirés, cancellation, version mismatch et installation hors Cargo. Protection de l'endpoint daemon local face à page hostile/Origin/Host/CSRF et DNS rebinding. Mac : fermeture de fenêtre, veille/réveil, moteur tué, update/signature, credentials et destinations de fichiers.

Parcours web clavier/lecteur d'écran/responsive, erreurs et reconnexion ; chaque exemple documenté associé à la version est exécuté. La documentation complète a son [lot et plan de couverture](./documentation-plan.md), elle n'est pas repoussée en bloc après le développement.

## 5. Benchmarks pertinents

### Trois suites, quatre policies

1. **Collecte** : HTML statique, JSON/API, pages JavaScript, pagination, contenu volumineux et exports ; attendus exacts de données et découverte.
2. **Parcours** : export authentifié, catalogue multi-pages, formulaire/effet dans un environnement de test ; état externe utilisé comme oracle.
3. **Dérive/généralisation** : versions mutées des mêmes sites, paramètres nouveaux, comptes/langues et cas inattendus ; corpus tenu à l'écart du réglage.

Comparer déterministe, assisté, agent et compilation/replay sur des tâches où ils rendent le même service. Baselines pertinentes : HTTP/Playwright bien écrit, Crawlee/Spider/Scrapy pour collecte, Stagehand/Browser Use/Skyvern pour automation, selon leurs modes disponibles. Les adapters et versions cloud/OSS ont leurs limites déclarées.

Inclure BrowserGym/WebArena/WorkArena lorsque l'environnement et les droits de benchmark le permettent ; ces benchmarks complètent les fixtures métier et ne représentent pas toutes les tâches client. [BrowserGym](https://github.com/ServiceNow/BrowserGym), [WebArena Verified dans BrowserGym](https://github.com/ServiceNow/BrowserGym/blob/main/browsergym/webarena_verified/README.md), [WorkArena](https://github.com/ServiceNow/workarena).

### Métriques

Coût complet par résultat vérifié, taux d'erreurs silencieuses et de refus injustifiés, couverture/freshness, débit utile, latence p50/p95/p99, CPU/mémoire de tout l'arbre de processus, réseau/stockage, startup froid/chaud et interventions. Inclure toutes tentatives, tokens/providers, compilation, réparations et coûts cloud/proxy disponibles ; séparer unités mesurées et estimation monétaire.

Afficher premier run et amortissement à 1/10/100/1 000 runs, selon une cadence et dérive fixées avant mesure. Un replay ne bénéficie pas gratuitement d'une compilation exclue du tableau.

### Méthode de comparaison

Épingler versions/commits, modèles/prompts, Chrome, ressources, région, RTT, proxies, limites et date. Un worker natif dans la région navigateur n'est pas comparé à une SDK distante sans afficher cet écart. Fixer qualité et output avant chronométrage, inclure états d'échec et intervalles d'incertitude, vérifier égalité de la couverture.

Les campagnes froid/chaud, HTTP/adaptatif/browser et création/replay/repair sont distinctes. Publier fixtures, validateurs, configurations et traces expurgées, ainsi que les tâches qui cassent encore. Les défauts identifiés dans les benchmarks publics servent à éviter des comparaisons de workloads différents. [Audit existant](../research/2026-10-07-benchmark-audit-followup.md), [protocole détaillé existant](../research/2026-10-07-benchmark-and-runtime.md).

## 6. Gates de décision, sans chiffres inventés

- **Comportement** : tous les invariants obligatoires de la suite conformance passent ; chaque capacité non couverte est affichée comme non qualifiée. Une suite parfaite n'établit pas la fiabilité de tous les sites réels.
- **Backend** : remplacer/promouvoir un backend seulement à qualité comparable avec avantage utile mesuré, ou une raison de capacité/distribution clairement documentée. Les objectifs précédents de gain sont des objectifs de projet, pas des observations.
- **IA/repair** : comparaison sur corpus séparé, coûts complets et erreurs silencieuses/refus inclus ; aucune promotion par simple confiance du même modèle.
- **Publication** : contrats stables du périmètre annoncé, exemples/docs, compatibilité, issues connues, packaging et procédure de recovery vérifiés.
- **Service** : capacité/isolation/quotas et usage réellement comptabilisés ; le SLA/prix vendu découle de mesures et du périmètre supporté.

Les SLO de production — disponibilité, queue latency, capacité et recovery objectives — restent à fixer après mesure du premier système. Le dossier ne transforme pas « ultra performant » en un chiffre sans workload ni preuve.

## 7. Questions à trancher par expériences

| Expérience | Décision informée |
| --- | --- |
| CDP natif vs Playwright, mêmes actions/RTT/conditions | Backend par défaut et fonctions restant en compatibilité |
| Worker natif vs extension/helper, placement égal | Justification du batching et rôle exact de l'extension |
| Reqwest vs transports spécialisés | Profil de transport utile selon workload, coût et maintenance |
| Writer SQLite/claims PostgreSQL sous charge réelle | Taille des batches, indexes, limites et besoin éventuel de broker |
| Plan/recorder paramétré sous mutations | Périmètre compilable et efficacité de repair |
| UI/CLI/MCP sur mêmes tâches utilisateurs | Ergonomie des APIs, navigation et catalogue d'outils |
| Mac daemon/IPC, lifecycle/veille et packaging | Distribution et comportement arrière-plan |
| Cloud usage complet incluant erreurs et proxies | Unités de facturation, coût de service et offres |

L'objectif est de rendre les choix discutables par des résultats reproductibles. Aucun agent de recherche, revue documentaire ou maquette ne remplace ces expériences.
