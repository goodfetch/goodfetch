# Qualification des clics

`pnpm actionability 3` exécute 324 cellules : six backends, neuf variantes,
deux politiques et trois blocs randomisés. Chaque cellule démarre un adapter
neuf, utilise le même Chromium et viewport, puis conserve ses événements,
effets et verdicts. Le démarrage est séparé du temps de tâche.
`pnpm actionability 3 guarded` permet de requalifier uniquement les 162 cellules
avec la garde, tout en conservant la matrice native précédente.

Les backends sont Playwright, Puppeteer, Stagehand v4, Stagehand v4 batch,
Crawlee/Playwright et `@crawlee/stagehand`. Cette dernière extension reste sur
Stagehand v3 et son handler utilise les primitives de la page Playwright.
Aucun modèle n'est appelé ; cette campagne ne mesure pas observe/act/extract.

| Variante | Contrat observable |
| --- | --- |
| normal | Un clic trusted sur la cible, un effet exact |
| overlay | Attendre la disparition de la couverture, sans cliquer l'overlay |
| delayed | Attendre l'activation du bouton |
| hidden | Attendre que le bouton soit visible |
| moving | Attendre la fin du déplacement |
| rerender | Retrouver le bouton après remplacement, puis activation |
| never-enabled | Refuser le clic ; aucun effet |
| never-uncovered | Refuser le clic ; aucun effet, y compris sur l'overlay |
| ambiguous | Refuser un selector qui correspond à deux boutons |

La politique **native** attend seulement que le document soit initialisé,
puis appelle la primitive propre au backend. La politique **guarded** ajoute
la même garde : selector unique, visibilité, activation, rectangle stable
sur deux intervalles entre trois observations RAF espacées d'au moins 16 ms,
et hit-test au centre. Les frames à fréquence élevée ne doivent pas masquer
un déplacement qui se produit entre deux observations trop rapprochées.
La garde n'utilise pas le flag interne de disponibilité attendu par l'oracle.
Elle couvre ces fixtures, pas toute la compatibilité de Playwright.

La première matrice a exposé quatre clics prématurés du guard sur des cibles
mobiles. La première observation était prise hors RAF et le guard pouvait
accepter deux positions identiques avant le prochain déplacement. Déplacer
seulement cette observation dans RAF n'a pas suffi au test public répété.
Les deux intervalles de stabilité ci-dessus corrigent ce défaut ; les logs
red/green et la matrice de requalification gardent la preuve du changement.

L'oracle serveur vérifie la cible, le nombre d'effets, `isTrusted` et la phase
où le clic a eu lieu. Un clic prématuré ou sur l'overlay échoue, même si l'API
renvoie un succès. Pour les trois variantes négatives, l'absence d'effet ne
suffit pas : l'opération doit aussi avoir été refusée.

Le budget des guards et des primitives Playwright/Puppeteer est 800 ms.
Stagehand v4 n'expose pas le même timeout de locator dans cette version ; le
harness garde une deadline extérieure de 8 s et rapporte cette différence.
Les handlers Crawlee ont une marge d'orchestration supplémentaire pour laisser
la primitive refuser avant que le crawler ne tue la tâche. Les temps des refus
ne doivent pas être assimilés aux temps des clics réussis.

Les [garanties Playwright documentées](https://playwright.dev/docs/actionability)
fournissent la référence pour unicité, visibilité, stabilité, réception des
événements et activation. La campagne teste ces propriétés sur son corpus,
sans déduire une équivalence universelle à partir d'un nombre de cellules.

Un backend qui échoue en mode natif reste visible dans le rapport. Les temps
d'une variante ne sont interprétés que pour ses effets vérifiés, avec les
effectifs et les trois blocs. Les anciens résultats de 50 clics avaient des
conditions de disponibilité communes ; cette campagne complète leur portée.
