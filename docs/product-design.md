# Design produit et interfaces

**Proposition du 7 octobre 2026.** La console React/shadcn, l'app macOS et les parcours décrits restent à implémenter. La [maquette HTML](../design/automan-console.html) est un artefact de design autonome avec données simulées ; elle n'est pas une app React ni une intégration du moteur.

## 1. Objet central : un résultat et son histoire

L'interface doit répondre à trois questions : qu'est-ce que je veux obtenir, est-ce que le run l'a obtenu, et que faire si le résultat manque ou est incertain ? Le run assemble chronologie, données et preuves. Les métriques d'infrastructure expliquent le résultat ; elles ne remplacent pas son verdict.

La création montre URL/objectif, entrée/sortie et critères de vérification. Les paramètres avancés — pools, proxy, budgets, hooks et backends — apparaissent quand l'utilisateur les demande. Le parcours ordinaire ne requiert pas de comprendre CDP, Tokio ou leases.

### Rôles servis

| Utilisateur | Parcours principal | Contrôle avancé |
| --- | --- | --- |
| Développeur | Créer, tester, exporter plan/SDK, inspecter un échec | Code, schemas, capabilities, erreurs et traces |
| Analyste | Collecter, vérifier, filtrer, exporter et suivre les changements | Couverture, freshness et provenance |
| Intégrateur agentique | Trouver les outils, permissions et schémas ; brancher CLI/MCP | Budgets, refs/observations et résultats paginés |
| Opérateur | Identifier blocage, capacité, santé et reprise | Queues/pools/workers, limites, drains et costs |
| Utilisateur macOS | Exécuter localement, gérer identités/fichiers et travail en arrière-plan | Même moteur, mêmes plans, contrôle du daemon |

## 2. Navigation et structure de la console

Navigation principale : **Workflows, Exécutions, Données, Réparations, Infrastructure**. Identités, intégrations, schedules et préférences de workspace sont accessibles depuis leur contexte et la configuration. Un sélecteur visible choisit local/cloud et workspace ; le changement n'importe pas une session authentifiée implicitement.

La maquette compacte regroupe données dans le détail du run et représente quatre entrées principales ; le produit complet ajoute la bibliothèque de datasets et les pages de configuration nécessaires.

```text
Sélecteur environnement/workspace          Recherche / commande / utilisateur
────────────────────────────────────────────────────────────────────────────
Navigation    Liste filtrable        Détail de l'objet sélectionné
Workflows     Nom / version          Objectif, entrées, contrat de sortie
Exécutions    État / date            Chronologie | Données | Preuves | Usage
Données       Dataset / export      Projection, coverage, freshness, export
Réparations   Cause / candidate      Diff, tests, résultat, promotion/rollback
Infrastructure                     Queues | Workers | Pools | Limites
```

À l'ouverture, sélectionner un workflow ou un run récent utile ; une installation vide propose « Créer un workflow » et un exemple déterministe. Ne pas inventer un tableau de bord rempli de statistiques lorsque rien n'a encore été exécuté.

## 3. Parcours de création

### 3.1 Collecte déterministe

1. Choisir « Collecter des données », saisir URL/seeds et définir les champs.
2. Choisir règles ou assistance IA pour construire la recette ; afficher provider/budget si assistance.
3. Prévisualiser un résultat sur un échantillon borné, puis définir identité et complétude utiles.
4. Enregistrer une version et démarrer un run ; proposer SDK/CLI pour la même configuration.
5. Montrer données, couverture et statut ; exporter ou définir une fréquence.

L'assistance à la création peut produire un plan rejouable sans modèle. Le test affiche les appels réellement nécessaires ; l'utilisateur ne suppose pas qu'un schéma JSON prouve les valeurs.

### 3.2 Automatisation d'un parcours

Choisir « Automatiser un parcours », sélectionner identité, entrée paramétrée et résultat attendu. L'utilisateur peut écrire des étapes, enregistrer une démonstration ou demander un plan à l'IA. Les étapes et conditions sont visibles avant qualification/publication. Une saisie de secret crée une référence d'identité, sans l'insérer dans le plan exporté.

Une étape nécessitant authentification propose handoff vers la session correspondante. L'interface montre compte/domaine visés et les données nécessaires ; l'expiration d'authentification ne déclenche pas une réparation de locator par défaut.

### 3.3 Agent ponctuel

Saisir objectif, paramètres, domaines/outils permis et budget. Une progression expose décisions, outils et coûts ; les captures sont disponibles au détail. Le message de l'agent reste distinct du verdict final. L'utilisateur peut créer un candidat de plan réutilisable après une exécution qualifiée.

## 4. Détail d'un run

### Chronologie

Chaque étape montre opération compréhensible, durée, résultat, tentatives et preuve. Les interruptions sont placées à l'endroit concerné : timeout, quota modèle, absence de données, mauvaise identité, locator ambigu. Un filtre permet les détails techniques ; la vue principale explique la conséquence.

L'état s'affiche en texte, pas seulement en couleur. `succeeded` est rendu « Résultat vérifié » si tous les checks requis passent ; un clic technique réussi n'utilise pas ce libellé. `partial` précise la couverture manquante. `effect_unknown` affiche l'action possiblement effectuée et la résolution attendue.

### Données et preuves

Table virtualisée/paginée, schéma, filtres, projection, provenance par ligne/champ lorsque disponible, pagination/coverage et freshness. Les grands artefacts sont chargés à la demande. Export démarre un objet inspectable avec progression et refus éventuel.

Une preuve indique sa nature : état externe, checksum, identité, test de structure ou jugement par modèle. Les informations sensibles sont expurgées selon droits ; « capture manquante » est expliqué, pas remplacé par un placeholder de succès.

### Usage

Afficher consommé/réservé/plafond, appels et tokens, navigateur, trafic/proxy et stockage. Coût estimé et prix appliqué sont distingués ; local sans grille connue montre les unités. Les retries et réparations restent dans le coût du run et dans leur ventilation.

### Commandes

Run actif : arrêter, suspendre admission selon contrat, fournir l'authentification/décision requise. Run terminal : inspecter, proposer réparation, réconcilier effet, puis lancer une continuation liée. Une continuation crée un nouveau run, avec checkpoints revalidés et historique conservé.

## 5. Réparations

Une réparation réunit échec d'origine, version de base, diagnostic, patch candidate, tests, canary, coût et décision. La comparaison avant/après montre locator/mapping/attente modifié et le résultat de validation ; elle ne montre pas seulement une instruction IA réécrite.

États : diagnostic, candidate, tests en cours, rejected, qualified, promoted, rolled back. Les critères métier requis restent visibles et ne peuvent être abaissés par un patch automatique. « Promouvoir » n'est disponible que si la politique et les tests l'autorisent. « Rejeter » conserve le motif et l'historique.

Une réparation de lecture peut être automatique selon politique ; le détail indique pourquoi. Une action d'effet inconnu demande réconciliation avant répétition. Le rollback indique sa portée : version de plan, pas effacement des effets externes.

## 6. Infrastructure et paramètres avancés

**Queues** : ready/running/delayed/dead-letter, producteurs, âge, priorité, scope, retries et raisons de retard. Inspecter/requeue une requête selon droits, exporter un checkpoint, voir la condition de fin de crawl.

**Workers** : version/capabilities/région, capacité disponible, CPU/mémoire de leur groupe de processus, activité et heartbeat. Enregistrer, drainer, mettre à jour, arrêter selon l'état réel. Le choix d'arrêt montre les runs qui peuvent être repris et les effets à réconcilier.

**Pools** : chaque ressource a sa vue et ses unités. Processus Chromium ≠ contextes ≠ pages ≠ sessions. Voir état, warmup, santé, retiré/quarantined, plafond et raison d'indisponibilité. Hooks et overrides sont visibles dans la configuration effective.

**Concurrence/autoscale** : désirée, admise, active et limitée ; domaine, provider, mémoire, capacité ou sink responsable. Le mode fixe permet de reproduire un benchmark. Les paramètres avancés sont regroupés et exportables ; ils ne polluent pas le premier formulaire.

## 7. Direction visuelle

### Tokens proposés

| Token | Valeur | Usage |
| --- | --- | --- |
| Surface | `#F7F9FC` | Fond de travail |
| Panneau | `#FFFFFF` | Détail et données |
| Texte | `#172B4D` | Titres/contenu principal |
| Action | `#2453C6` | Sélection, focus et action principale |
| Vérifié | `#087F72` | Verdict validé accompagné de texte |
| Attention | `#9A5B00` | Décision requise/partiel, avec contraste vérifié |

Typographie système sans-serif, hiérarchie par taille/poids et alignement gauche ; monospace pour code seulement. Espacements 4/8/12/16/24/32, bordures pour séparer objets/chronologie, courbes modérées selon composant. Palette et contraste réels à contrôler dans l'implémentation, pas à déduire du choix de noms de couleurs.

Le caractère du produit vient de la **chronologie reliée aux preuves**, pas d'une grille de cartes décoratives. Table/liste et inspecteur partagent l'espace ; les confirmations expliquent le changement. Une animation accompagne une action ou progression, respecte reduced motion et ne détourne pas l'attention.

### shadcn et comportement

Composants candidats : navigation/sidebar, resizable panels, data table, tabs, dialog/sheet, command palette, forms, select, progress, tooltip et notifications. Ils sont composés autour des contrats produit ; un composant de base ne prouve pas l'accessibilité de l'application finale. [shadcn/ui](https://ui.shadcn.com/docs), [React](https://react.dev/learn), [options de console](../research/2026-10-07-product-platform-options.md).

Revue du parti pris : conserver les états et actions comparables entre listes, chronologie et réparation ; retirer statistiques et accents qui n'aident pas à décider. Les tokens ne remplacent pas la vérification de la maquette à différentes tailles.

## 8. App macOS réellement native

SwiftUI pour fenêtre, sidebar, listes, inspector et commandes ; AppKit lorsque nécessaire pour fichiers, raccourcis, intégration de fenêtres et contenus avancés. Aucun écran web React encapsulé ne constitue seul le résultat attendu.

App et daemon partagent plans/états via contrat local ; la fermeture de fenêtre n'est pas équivalente à la fin du moteur. L'utilisateur choisit si des runs continuent, voit l'activité en arrière-plan et peut arrêter/drainer. Veille/réveil déclenchent health/reconnect et réconciliation si nécessaire ; la socket CDP n'est pas supposée avoir survécu.

Fonctions propres au Mac : Keychain, file picker et permissions de destinations, notifications pour run/décision, raccourcis, menu de commandes, gestion des identités locales et moteur signé/notarisé. Un agent de lancement permanent s'active selon choix explicite et mécanisme système. Le cloud utilise les mêmes vues métier, avec son contexte visible. [Sources et contraintes Apple](../research/2026-10-07-product-platform-options.md).

## 9. États UI et qualification

Chaque écran couvre vide, chargement, succès, partiel, panne réseau, absence de droit, quota et données obsolètes. La perte SSE reconnecte avec curseur ou annonce un gap d'historique ; elle ne transforme pas un run serveur en échec local. Les actions utilisent identifiants idempotents et des états d'attente lisibles.

Tests : navigation clavier complète, focus visible et restitution du focus après dialog, lecteur d'écran sur statuts/tables, labels/champs/erreurs, responsive et contraste ; longs noms/URL/valeurs, gros datasets et faible bande passante. Les captures du design valident la présentation ; elles ne prouvent pas le comportement du moteur.

La maquette sert à examiner la navigation et les décisions produit. Les interactions simulées seront remplacées par les contrats réels du premier vertical slice ; aucune fonctionnalité n'est qualifiée sur la seule base de son apparence.
