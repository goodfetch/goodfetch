# La garde Automan sur Chromium, WebKit et Firefox, 9 octobre 2026

**Objet.** Vérifier que la garde Automan protège les actions sur d'autres moteurs que Chromium, en passant par Playwright pour les moteurs sans CDP (Firefox, WebKit). C'est la condition pour qu'Automan pilote ces moteurs derrière le même contrat que son pilote CDP. Il s'agit d'un test de justesse des actions, pas d'empreinte.

**Plan d'expérience.**
- Trois matrices : actionnabilité (9 variantes), primitives (10) et navigation, frames et fichiers (7).
- Deux politiques : avec la garde et en natif, c'est-à-dire avec les seules vérifications de Playwright.
- Trois blocs.
- Moteurs épinglés par Playwright 1.63 sous `lab/.browsers` : Chromium 153, WebKit 26.6 et Firefox 155.

**Runs retenus.**

| Système | Actionnabilité | Primitives | Navigation |
| --- | --- | --- | --- |
| Conteneur Linux (arm64) | `cross-engine-actionability-2026-10-09T14-05-56-552Z-677518` | `cross-engine-primitives-2026-10-09T14-09-14-677Z-63256a` | `cross-engine-navigation-2026-10-09T14-12-41-527Z-cd6a35` |
| macOS 27.0.1 | `cross-engine-actionability-2026-10-09T13-59-29-588Z-e0a4f2` | `cross-engine-primitives-2026-10-09T14-02-07-047Z-53b744` | `cross-engine-navigation-2026-10-09T14-04-46-125Z-4d8f3c` |

## Résultats

| Moteur | Avec la garde, Linux | Avec la garde, macOS | Natif, Linux | Natif, macOS |
| --- | --- | --- | --- | --- |
| Chromium | 78/78 | 78/78 | 78/78 | 78/78 |
| WebKit | 78/78 | 78/78 | 78/78 | 77/78 |
| Firefox | 78/78 | ne démarre pas | 78/78 | ne démarre pas |

- **La garde n'a jamais laissé passer une mauvaise action**, sur les trois moteurs et les deux systèmes : 390 cellules sur 390. Elle est faite de scripts de page sur le DOM et enveloppe les gestes de Playwright sans modification.
- **En natif, Playwright laisse parfois passer un clic sur une cible encore en mouvement.**
  - Sur ces runs, c'est arrivé une fois avec WebKit sur macOS.
  - Pendant la mise au point, c'est arrivé aussi avec WebKit sous Linux et avec Chromium sous Linux.
  - Le défaut est intermittent : sa vérification de stabilité peut se faire devancer par le mouvement.
  - La garde, qui vérifie la cible au moment du geste puis intercepte l'événement, ne s'est jamais trompée sur ce cas.
- **Firefox ne démarre pas sur macOS 27.0.1.** Le build Firefox de Playwright s'arrête sur « Could not find profile folder », quel que soit l'emplacement du profil, y compris hors du bac à sable des commandes. Il fonctionne sous Linux.

## Défauts trouvés en route

- **Oracle de la molette.** La fixture exigeait 300 pixels de défilement après une molette de 600, soit l'échelle de Chromium. Firefox défile de 180 pixels par cran ; WebKit atteint 600, mais progressivement. L'oracle juge désormais le contrat lui-même : un événement `wheel` de confiance sur la zone, suivi d'un défilement réel. Les runs faits avec l'ancien oracle sont classés `superseded`.
- **Bibliothèque native dans l'image.** L'image Linux ne contient pas la bibliothèque SQLite native, que le lab charge au démarrage. Le script des campagnes la construit d'abord.

## Ce que cela change pour D-browser-driver

Le contrat navigateur peut s'appuyer sur deux backends, avec la même garde :
- **notre pilote CDP** pour Chromium ;
- **Playwright** pour Firefox et WebKit.

Pour un produit, la garde reste nécessaire même au-dessus de Playwright : ses seules vérifications ne suffisent pas toujours sur une cible en mouvement.
