# Mesures de durée sur Mac calme (local-controlled), 10 octobre 2026

La session « Implement Spec - 1 » (projet timeframe), dont les compilations Swift chargeaient le Mac, s'est mise en pause à la demande du porteur. Chaque campagne a attendu que l'hôte se qualifie en `local-controlled` avant de démarrer. Un bloc est exclu du verdict si le CPU étranger dépasse 150 % à sa fin, ce CPU excluant les processus du run lui-même (correction du 10 octobre).

Runs retenus :
- `browser-driver-rtt-0-2026-10-09T23-21-22-913Z-2bbed1` : 8 blocs propres sur 8 ;
- `browser-driver-rtt-20-2026-10-09T23-53-33-623Z-3dd31a` : 7 sur 8 ;
- `browser-driver-rtt-100-2026-10-10T00-17-31-377Z-ae1e25` : 5 sur 6 ;
- `browser-driver-rtt-300-2026-10-10T03-02-31-508Z-5366fd` : 4 sur 6 ;
- `http-transport-v2-2026-10-10T02-45-30-587Z-b23a72` ;
- `durable-scheduler-2026-10-10T02-45-45-825Z-edc50c` ;
- `durable-admission-2026-10-10T02-46-32-139Z-ef1b25` ;
- `portal-routing-2026-10-10T02-47-13-937Z-4adc3c` ;
- `light-engine-paired-2026-10-10T02-55-16-672Z-17bf22`.

Ils remplacent comme preuves les runs `local-dev` des mêmes campagnes. Les runs du pilote navigateur du 8 octobre, déjà en `local-controlled`, restent cités et s'ajoutent aux nouveaux.

## Verdicts

| Décision | Résultat sur Mac calme | Avant (Mac chargé) |
| --- | --- | --- |
| **D-adaptive-routing** | **meilleur, éligible** : `sampled-contract` sur page statique à 0,004 fois le temps du navigateur, équivalent ailleurs (il garde le navigateur). `api-capture` **meilleur** sur GraphQL et le scroll infini (0,008) ; `insufficient-n` sur les autres rendus, à 0,005-0,006 | `insufficient-n`, non éligible |
| **D-http-transport** | **non inférieurs, éligible** : wreq 1,01, Impit 1,02, Node 1,00 contre reqwest ; Crawlee en pool **moins bon** (0,86) ; Spider bloqué par le contrat | non inférieurs, non éligible |
| **D-scheduling** | `insufficient-n` (5 blocs sur 22 demandés) : la file équitable donne 2,6 fois plus de travail aux petits domaines (IC 2,31 à 2,93) | meilleur (2,84), non éligible |
| **D-light-engine** | bloqué par le contrat (shadow DOM), désormais en `local-controlled` | bloqué |
| **D-browser-driver** | **bloqué par le contrat** : Playwright natif, la référence, dépasse deux fois son délai sur le catalogue JS à 100 ms | `worse` à 0 ms |

### Pilote navigateur : estimation hors contrat

Le gate d'exactitude lit les cellules de l'option et de sa référence. Les deux timeouts de Playwright natif à 100 ms bloquent donc toutes les comparaisons, alors que les pilotes avec garde-fou sont exacts partout. Les durées, recalculées sans ce gate, avec les blocs du 8 et du 10 octobre (rapport de temps de tâche contre Playwright natif, plus bas = plus rapide) :

| Pilote | Latence | Formulaire | Clics | Catalogue JS |
| --- | --- | --- | --- | --- |
| CDP Rust + garde | 0 ms | 1,33 | 1,21 | 0,33 |
| CDP Rust + garde | 20 ms | 0,80 | 0,84 | 0,73 |
| CDP Rust + garde | 100 ms | 0,70 | 0,74 | 0,79 |
| CDP Rust + garde | 300 ms | 0,68 | 0,72 | 0,80 |
| CDP Node + garde | toutes | identique au Rust à ±0,02 près | | |
| Puppeteer + garde | 0 à 300 ms | 1,62 à 2,32 | 1,62 à 1,84 | 0,41 à 0,93 |

Tous ces écarts sont nets : les intervalles de confiance font moins de 0,1 de large.

## Lecture

- **Le point de bascule du pilote navigateur se situe entre 0 et 20 ms.** Sur une boucle locale, le garde-fou coûte 20 à 33 % sur les formulaires et les clics : il vérifie l'élément avant d'agir. Dès 20 ms de latence entre pilote et navigateur, le CDP direct avec garde-fou devient 16 à 32 % plus rapide que Playwright, et l'écart grandit avec la latence. Un navigateur hébergé est toujours à distance : **CDP avec garde-fou est le bon défaut**. Le langage du pilote ne change rien : Rust et Node sont identiques. Puppeteer avec garde-fou reste le plus lent.
- **Playwright natif n'est pas une référence exacte à distance** : deux timeouts sur le catalogue JS à 100 ms. D-browser-driver choisit déjà le garde-fou ; la référence de la décision mériterait d'être un pilote exact sur toutes les latences.
- **Le routage adaptatif est éligible** : l'échantillon comparé ne coûte rien de plus que le navigateur là où il le garde, et sur une page statique il retombe à 0,4 % de son temps. La découverte d'API divise le temps par 120 à 200 une fois la route apprise.
- **Le transport ne départage rien en débit**, à concurrence 4. Le choix de reqwest repose sur son coût CPU et mémoire, mesuré le 9 octobre.
- **La file équitable par domaine reste nettement meilleure** (2,6 fois), mais plus dispersée que le 8 octobre. Il faudrait 22 blocs pour la marge de 10 %. L'effet n'est pas en doute ; le verdict formel attend plus de blocs.

## Limites

- Mac calme, mais pas un hôte de référence : rien n'est publiable avant l'hôte Linux nu.
- Seuls les blocs dont le CPU étranger restait sous 150 % comptent ; à 300 ms, 4 blocs propres sur 6, complétés par ceux du 8 octobre.
