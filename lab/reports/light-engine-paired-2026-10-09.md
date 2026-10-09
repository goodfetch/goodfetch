# Moteur léger contre Chrome, apparié dans un même run (D-light-engine), 9 octobre 2026

Jusqu'ici, Lightpanda et Chrome tournaient dans des runs séparés : le verdict, qui apparie les blocs à l'intérieur d'un run, n'avait rien à comparer. Une campagne bench peut désormais lister plusieurs navigateurs harnais (`browserHosts`). Chaque pilote tourne alors une fois par navigateur et par bloc, dans un ordre tiré au sort, et le navigateur devient un facteur de la cellule.

Run : `light-engine-paired-2026-10-09T16-29-54-931Z-c62471`, `local-dev`. Il couvre 4 pilotes, 5 scénarios, 5 blocs et 5 itérations, avec la garde Automan sur les pilotes CDP et Puppeteer, et Playwright natif.

## Résultats

CPU et mémoire du pilote et du navigateur ensemble, par pilote, en médiane sur les blocs :

| Pilote | CPU par tâche, Chrome (ms) | CPU par tâche, Lightpanda (ms) | Rapport | RSS max, Chrome | RSS max, Lightpanda |
| --- | --- | --- | --- | --- | --- |
| `rust-cdp` | 1243 | 58 | 0,05 | 2860 Mio | 43 Mio |
| `node-cdp` | 1289 | 97 | 0,07 | 2830 Mio | 93 Mio |
| `puppeteer` | 1209 | 145 | 0,12 | 2911 Mio | 116 Mio |
| `playwright` | 3296 | 190 | 0,06 | 1788 Mio | 203 Mio |

Statuts par scénario :

| Scénario | Chrome | Lightpanda |
| --- | --- | --- |
| `http-catalog`, `js-catalog`, `form`, `clicks` | vérifiés, tous pilotes | vérifiés, tous pilotes |
| `shadow` | vérifié ; Puppeteer `unsupported` | **échec** (timeout) pour `rust-cdp`, `node-cdp` et Playwright ; Puppeteer `unsupported` |

Sur les catalogues, une tâche dure 3 à 10 ms avec Lightpanda contre 60 à 170 ms avec Chrome. Sur les formulaires et les clics, la durée est dominée par les attentes de la garde, et l'écart se réduit (170 à 240 ms contre 260 à 415 ms).

## Verdict mécanique

D-light-engine est **bloqué par le contrat** (`oracle-exact`) : le shadow DOM échoue sur Lightpanda. Le gate porte sur toute la matrice, si bien qu'un scénario incompatible bloque l'option, quels que soient ses chiffres sur le reste.

## Ce que cela dit pour l'architecture

- Lightpanda ne peut pas être le moteur par défaut : il ne couvre pas tout le contrat d'actionnabilité.
- Sur un corpus compatible, il coûte 8 à 20 fois moins de CPU et 15 à 65 fois moins de mémoire. C'est l'ordre de grandeur qui justifie un **tier choisi par capacités**. Comme le prévoit l'architecture (§5.2), `capabilities()` et le manifeste du workflow déclarent ce qu'une tâche exige, et le moteur choisit le backend avant de créer la session, jamais en cours de route.
- La route apprise du routage adaptatif peut porter ce choix au même titre que HTTP, navigateur ou API : une route compatible validée sur échantillon passe au moteur léger.

## Limites

- Hôte `local-dev` : CPU et durées indicatifs ; le verdict demande `local-controlled`.
- La matrice d'actionnabilité sur Lightpanda (`light-engine-actionability-…1d5ca3`) ne porte pas l'hôte comme facteur. Elle reste lue dans son propre rapport, et le gate `false-success-count` est retiré de la décision.
