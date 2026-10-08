# Automan

Automan réunit collecte web, automatisation navigateur et workflows assistés par modèles. Ce vocabulaire est commun au framework, aux interfaces et au service hébergé.

## Automatisation

**Workflow** :
Définition réutilisable d'un objectif, de ses paramètres, de ses étapes et des conditions qui permettent de vérifier son résultat.
_Éviter_ : scraper pour toute automatisation, agent pour tout workflow.

**Plan** :
Représentation exécutable et versionnée d'un workflow, avec ses opérations, dépendances, conditions et exigences de capacités.
_Éviter_ : prompt, simple liste de clics.

**Run** :
Une exécution d'une version précise d'un workflow avec des paramètres et un périmètre d'identité donnés.
_Éviter_ : workflow lorsqu'on parle d'une exécution, job pour désigner indifféremment une étape ou un run.

**Tentative** :
Une exécution d'une unité de travail, initiale ou répétée après un échec ; elle reste rattachée au même run.
_Éviter_ : nouveau run pour chaque retry.

**Réparation** :
Production et qualification d'une nouvelle version d'un plan après une dérive détectée ; elle ne prouve pas à elle seule que le résultat est correct.
_Éviter_ : self-healing comme garantie universelle.

**Agent** :
Boucle de décision qui choisit des opérations à partir de l'objectif et des observations ; elle peut être fournie par Automan ou par un système externe.
_Éviter_ : agent pour une simple extraction par modèle.

## Collecte et résultats

**Frontier** :
Ensemble ordonné des requêtes de collecte à traiter, avec leur périmètre, leur priorité et leur état de prise en charge.
_Éviter_ : file de workflows, historique des URL visitées.

**Requête de collecte** :
Unité de la frontier identifiée par son URL, sa méthode, ses paramètres et son contexte de collecte.
_Éviter_ : page quand une réponse HTTP suffit.

**Dataset** :
Collection versionnée de résultats structurés, avec un contrat de schéma, de provenance et de déduplication.
_Éviter_ : cache, simple fichier de sortie.

**Artefact** :
Contenu associé à un run, tel qu'un téléchargement, une capture, une observation ou une trace, distinct des résultats structurés.
_Éviter_ : résultat vérifié pour tout artefact.

**Résultat vérifié** :
Résultat dont les conditions requises d'identité, de contenu, de complétude ou d'effet ont été satisfaites par les validateurs déclarés.
_Éviter_ : succès lorsque seul le transport ou le clic a réussi.

## Exécution

**Worker** :
Processus d'exécution qui annonce des capacités et une capacité disponible, puis prend en charge des unités de travail.
_Éviter_ : onglet, tâche asynchrone ou machine pour désigner automatiquement un worker.

**Backend** :
Implémentation d'un moyen d'accès au site : transport HTTP, navigateur CDP ou pilote Playwright, par exemple.
_Éviter_ : moteur pour chaque dépendance interchangeable.

**Session** :
Périmètre d'identité et d'état de navigation utilisé pour interagir avec un site ; il peut inclure cookies, stockage et contexte navigateur.
_Éviter_ : navigateur, connexion CDP ou compte comme synonymes.

**Pool** :
Ensemble de ressources réutilisables d'une même nature, avec admission, disponibilité, santé et retrait.
_Éviter_ : pool unique pour mélanger sessions, onglets, navigateurs et workers.

**Lease** :
Droit temporaire de prendre en charge une unité de travail ou une session, dont le détenteur et la validité sont identifiables.
_Éviter_ : verrou permanent, preuve d'exécution exactement une fois.

**Package de workflow** :
Unité distribuable qui réunit des workflows versionnés, leurs contrats d'entrée et de sortie, leurs capacités requises et leurs tests.
_Éviter_ : Actor dans le contrat public Automan, terme réservé à la comparaison avec Apify.
