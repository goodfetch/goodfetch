# Transports HTTP en v2 (D-http-transport), 9 octobre 2026

D-http-transport n'avait aucune analyse mécanique : ses runs venaient de l'ancien harnais, sans reqwest, wreq ni Impit. Deux campagnes v2 mettent les six transports candidats sur le même catalogue. La décision reçoit une analyse qui associe chaque option à son backend, avec le gate de politique d'accès lu dans la lane d'actionnabilité. Mesures en `local-dev` sur sources commitées, Mac en usage.

## Boucle fermée, catalogue à incidents : run diagnostic

`http-transport-v2-2026-10-09T15-30-27-630Z-d8bf7a` fait passer cinq variantes par le gate de conformité : page normale, redirection, gzip, chunked et grande réponse. Trois backends échouent avant toute mesure, et le moteur saute leurs blocs :

| Backend | Variante refusée | Cause |
| --- | --- | --- |
| `reqwest-http` | `http-gzip` : enregistrements absents | l'adaptateur compile reqwest sans la feature `gzip` ; le corps compressé n'est pas décodé |
| `wreq-http` | `http-redirect` : enregistrements absents | wreq 6.0.0-rc.31 a pour défaut `redirect::Policy::none()`, alors que sa documentation annonce 10 redirections ; l'adaptateur ne fixe pas de politique |
| `spider-http` | `http-redirect` : `HTTP 302` | Spider refuse une redirection vers loopback (contrôle SSRF natif) ; attendu et conservé, comme le dit la décision |

Les deux premières causes sont des défauts de configuration de l'adaptateur du lab, pas des limites des transports. Le run est classé `diagnostic` et sera refait après correction de l'adaptateur.

## Arrivées ouvertes à 250 par seconde : run retenu

`http-open-v2-2026-10-09T15-30-40-832Z-1b2a42` utilise des pages normales, 8 par tâche, et 4 tâches simultanées au plus, sur 5 blocs. Ses 20 170 tentatives sont vérifiées et aucune n'échoue. Médianes par backend :

| Backend | Tâches vérifiées/s | p50 (ms) | p95 (ms) | CPU par tâche (ms) | RSS max | Démarrage (ms) |
| --- | --- | --- | --- | --- | --- | --- |
| `reqwest-http` | 114,5 | 30,5 | 34,5 | 1,25 | 9,2 Mio | 9 |
| `wreq-http` | 115,9 | 30,5 | 33,6 | 1,31 | 7,1 Mio | 6 |
| `impit-http` | 117,5 | 30,0 | 33,6 | 2,18 | 101,8 Mio | 117 |
| `spider-http` | 122,7 | 28,5 | 32,2 | 2,86 | 31,2 Mio | 14 |
| `crawlee-http-pooled` | 76,5 | 46,3 | 58,7 | 16,23 | 526,4 Mio | 685 |
| `node-http` | 121,5 | 28,8 | 32,1 | 1,59 | 230,9 Mio | 112 |

## Lecture

- **Le débit est borné par la concurrence, pas par le transport.** Avec 4 tâches simultanées et une tâche de 8 pages, servie en 3 ms par page, le plafond tourne autour de 130 tâches/s. Cinq transports s'en approchent. Crawlee en pool reste à 76 et paie son orchestration : 16 ms de CPU par tâche et un demi-gibioctet de mémoire.
- **Ce qui sépare les transports, c'est le coût.** reqwest et wreq demandent environ 1,3 ms de CPU par tâche et moins de 10 Mio de mémoire, et démarrent en moins de 10 ms. Node consomme un peu plus de CPU mais 25 fois plus de mémoire. Impit, en émulation Chrome, double le CPU et dépasse 100 Mio.
- Le défaut reqwest, déjà confirmé, garde sa place : le transport le plus sobre, à égalité de débit. La comparaison sur incidents attend le run corrigé.

## Limites

- Hôte `local-dev` : les durées et le CPU sont indicatifs. Le verdict de non-infériorité demande `local-controlled`.
- La concurrence de 4 sature avant le transport ; une campagne à concurrence plus haute dira où chaque transport plafonne.
