# Extraction : analyse appariée par page (D-extraction), 9 octobre 2026

Le verdict mécanique de D-extraction reste `insufficient-n`, parce que la lane agrège le F1 par type de page : sept types, donc sept blocs. Or l'unité naturelle de l'extraction est la page. `tools/extraction-paired.ts` recalcule le F1 de chaque page à partir des prédictions enregistrées par le run retenu, avec la définition de la lane. Les extracteurs sont ensuite comparés sur la **différence** de F1 par page. Un log-ratio écarterait les pages où un extracteur obtient 0, et favoriserait celui qui échoue.

Run : `extraction-quality-test-2026-10-08T18-00-46-463Z-751a55`, split test réservé de WCXB, 511 pages. Les moyennes recalculées sont identiques à celles du run (0,884 pour rs-trafilatura), ce qui valide le recalcul. Données : [`extraction-paired-2026-10-09.json`](extraction-paired-2026-10-09.json), contre `scraper-text`, et [`extraction-paired-vs-trafilatura-2026-10-09.json`](extraction-paired-vs-trafilatura-2026-10-09.json), contre rs-trafilatura.

## Contre rs-trafilatura

| Extracteur | F1 moyen | Écart moyen (IC 95 %, bootstrap) | Hodges–Lehmann | Pages où rs-trafilatura gagne / perd / égalité | Wilcoxon |
| --- | --- | --- | --- | --- | --- |
| rs-trafilatura | 0,884 | — | — | — | — |
| Readability | 0,828 | −0,056 (−0,078 à −0,034) | −0,024 | 352 / 125 / 34 | p < 10⁻⁹ |
| dom-smoothie | 0,823 | −0,061 (−0,083 à −0,038) | −0,024 | 351 / 124 / 36 | p < 10⁻⁹ |
| scraper-text (html5ever, référence de la décision) | 0,719 | −0,165 (−0,186 à −0,144) | −0,160 | 469 / 38 / 4 | p < 10⁻⁹ |
| htmd (Markdown) | 0,523 | −0,361 (−0,383 à −0,339) | −0,364 | 482 / 23 / 6 | p < 10⁻⁹ |

## Lecture

- **rs-trafilatura est le meilleur extracteur de contenu principal, page à page, pas seulement en moyenne.** Il bat la référence html5ever sur 469 pages sur 511, et chacun des deux suivants sur environ 350 pages.
- L'écart avec Readability et dom-smoothie est petit en médiane (Hodges–Lehmann −0,024) et plus grand en moyenne (−0,06). Sur la plupart des pages, ils sont proches ; sur quelques-unes, ils échouent nettement.
- htmd convertit en Markdown sans isoler le contenu principal. C'est un format de sortie, pas un extracteur : sa place est après l'extraction.

## Ce que cela décide

La chaîne d'extraction proposée pour D-extraction :
1. rs-trafilatura (classe trafilatura) isole le contenu principal ;
2. htmd le convertit en Markdown quand le format est demandé ;
3. html5ever et scraper servent au parsing structurel (sélecteurs, champs), pas à la sélection du contenu.

Le F1 ne dépend pas de l'hôte. Le verdict du registre reste `insufficient-n`, puisque sa règle compte des blocs par type de page, mais l'analyse par page le tranche. L'adoption finale reste au porteur.

## Limites

- Un seul corpus (WCXB) ; WebMainBench reste à ajouter pour croiser.
- Word F1 sur le texte du contenu principal ; la qualité du Markdown produit (titres, listes, tableaux) n'est pas mesurée ici.
