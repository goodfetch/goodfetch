# Qualité d'extraction sur WCXB, 8 octobre 2026

Mesure de D-extraction et de D-light-engine sur le corpus WCXB (Murrough Foley, 2026, CC-BY-4.0, commit épinglé dans `corpora/wcxb.manifest.json`). 2 008 pages, 7 types de page. Les métriques sont celles de son `evaluate.py` : F1 au niveau des mots contre le contenu principal, part des extraits requis retrouvés, part des extraits de boilerplate présents. Les scores du lab sont identiques à ceux de `evaluate.py` à la quatrième décimale pour les cinq extracteurs.

Score de référence sur le **jeu de test réservé** (511 pages), run `extraction-quality-test-2026-10-08T18-00-46-463Z-751a55` ; le jeu de développement (1 497 pages, run `extraction-quality-dev-2026-10-08T17-58-37-038Z-ee347f`) donne le même classement.

| Extracteur | Article | Collection | Documentation | Forum | Listing | Produit | Service | F1 global | Extraits requis | Boilerplate présent | Échecs | ms par page (médiane, articles) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rs-trafilatura | 0.951 | 0.775 | 0.973 | 0.787 | 0.745 | 0.755 | 0.832 | **0.884** | 0.858 | 0.099 | 0 | 11.3 |
| dom-smoothie | 0.935 | 0.585 | 0.860 | 0.700 | 0.616 | 0.638 | 0.786 | **0.823** | 0.739 | 0.078 | 3 | 1.9 |
| readability | 0.935 | 0.590 | 0.880 | 0.715 | 0.616 | 0.637 | 0.796 | **0.828** | 0.747 | 0.087 | 0 | 9.1 |
| scraper-text | 0.805 | 0.492 | 0.781 | 0.635 | 0.612 | 0.536 | 0.666 | **0.719** | 0.762 | 0.953 | 0 | 0.8 |
| htmd | 0.631 | 0.247 | 0.600 | 0.423 | 0.373 | 0.329 | 0.441 | **0.523** | 0.763 | 0.964 | 0 | 1.1 |
- **rs-trafilatura** (portage Rust de trafilatura) domine, surtout hors des articles : collections, forums, listings et produits, là où les extracteurs de type Readability décrochent.
- **dom_smoothie** (portage Rust de Readability) égale Readability.js, avec une médiane d'environ 2 ms par page contre 9 ms pour Readability sur linkedom. Il échoue sur quelques pages, que Readability traite.
- Les deux références sans extraction (tout le texte, conversion htmd de la page entière) laissent passer presque tout le boilerplate : elles bornent le gain d'une vraie extraction de contenu principal.
- Les temps sont mesurés sur un Mac en usage (`local-dev`) et ne valent qu'à titre indicatif ; la qualité, elle, ne dépend pas de la machine.

Lightpanda (D-light-engine) n'est pas encore mesuré : il demande un moteur hors processus, à intégrer comme adapter.
