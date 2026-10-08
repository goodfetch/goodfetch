# Vérification du dossier de conception

**7 octobre 2026.** Périmètre : documents proposés et maquette ; aucun moteur Automan ou benchmark de performance exécuté.

## Documents

- Revue indépendante de la proposition, architecture, API, PRD, design, roadmap et plan de documentation : quatre corrections intégrées, aucune objection matérielle restante dans cette revue. [Rapport de revue](./2026-10-07-spec-review.md).
- Contrôle UTF-8, liens vers fichiers locaux, clôture des blocs de code, whitespace et marqueurs de conflit : 13 documents contrôlés, 65 liens locaux résolus, aucun défaut restant au contrôle.
- Exemple JSON de l'API parsé ; 80 user stories dénombrées dans le PRD.
- 194 URL externes distinctes citées dans ce sous-ensemble ; c'est un inventaire de références, pas un compte de sources indépendantes validées. Les liens sources critiques ont été consultés dans les enquêtes ; tous les liens externes n'ont pas fait l'objet d'un nouveau audit réseau global. Le lien de documentation Spider non accessible a été remplacé par sa référence docs.rs consultée.

## Maquette

HTML autonome de 61 123 octets, avec affichage explicite « Maquette — données simulées ». Aucun moteur, compte ou provider connecté. Syntaxe JavaScript contrôlée par le sous-agent qui a produit l'artefact.

Revue navigateur du parent : chronologie/preuve, onglet données, filtre de données, création d'un quatrième workflow fictif, liste des exécutions, vue réparation et qualification simulée. Changer la garde de cible invalide la qualification et désactive la promotion. Vue pools inspectée avec limites de capacité et causes supplémentaires identité/réparation. Aucune erreur ou warning console observé pendant ces parcours.

Présentation desktop inspectée visuellement ; viewport mobile émulé réellement à 390 × 844, largeur du document 390, sans overflow horizontal global. Dialog de création accessible : focus sur premier champ, Escape ferme le dialog et rend le focus au bouton « Créer un workflow ». Ces checks ne constituent pas un audit d'accessibilité complet.

La maquette simule tests, données et prix ; ses verdicts ne qualifient pas un scraper réel. Les performances, backends, durabilité, réparations, sandboxing et packaging natif devront passer les gates de [qualification](../docs/roadmap-validation.md) avec l'implémentation.
