# Transports HTTP en v2 (D-http-transport), 9 octobre 2026

D-http-transport n'avait aucune analyse mécanique : ses runs venaient de l'ancien harnais, sans reqwest, wreq ni Impit. Deux campagnes v2 mettent les six transports candidats sur le même catalogue. La décision reçoit une analyse qui associe chaque option à son backend. Le gate `policy-compliance` est retiré de cette décision : robots.txt et les autres préférences des sites relèvent de la politique de site du moteur (D-identity), et aucun transport ne les appliquait seul, la référence comprise. Mesures en `local-dev` sur sources commitées, Mac en usage.

## Boucle fermée, catalogue à incidents

Le premier run, `http-transport-v2-2026-10-09T15-30-27-630Z-d8bf7a`, est `diagnostic`. Il fait passer cinq variantes par le gate de conformité : page normale, redirection, gzip, chunked et grande réponse. Trois backends y échouent avant toute mesure :

| Backend | Variante refusée | Cause |
| --- | --- | --- |
| `reqwest-http` | `http-gzip` : enregistrements absents | l'adaptateur compilait reqwest sans la feature `gzip` ; le corps compressé n'était pas décodé |
| `wreq-http` | `http-redirect` : enregistrements absents | wreq 6.0.0-rc.31 a pour défaut `redirect::Policy::none()`, alors que sa documentation annonce 10 redirections ; l'adaptateur ne fixait pas de politique |
| `spider-http` | `http-redirect` : `HTTP 302` | Spider refuse une redirection vers loopback (contrôle SSRF natif) ; attendu et conservé, comme le dit la décision |

Les deux premiers étaient des défauts de configuration de l'adaptateur du lab. Ils sont corrigés : les deux piles décodent gzip, brotli, deflate et zstd, et wreq suit 10 redirections.

Run retenu après correction : `http-transport-v2-2026-10-09T16-13-29-199Z-130b39`, 5 blocs. Cinq transports passent le catalogue entier ; Spider reste bloqué par son refus SSRF. Médianes par backend :

| Backend | Tâches vérifiées/s | p50 (ms) | p95 (ms) | CPU par tâche (ms) | RSS max | Démarrage (ms) |
| --- | --- | --- | --- | --- | --- | --- |
| `reqwest-http` | 127,3 | 13,7 | 60,2 | 1,62 | 10,2 Mio | 5 |
| `wreq-http` | 128,4 | 12,9 | 61,5 | 1,43 | 8,0 Mio | 4 |
| `impit-http` | 123,7 | 14,1 | 61,7 | 16,69 | 95,4 Mio | 175 |
| `node-http` | 119,2 | 14,9 | 63,3 | 18,70 | 115,8 Mio | 154 |
| `crawlee-http-pooled` | 82,0 | 35,1 | 79,8 | 70,22 | 277,0 Mio | 696 |
| `spider-http` | bloqué (redirection loopback refusée) | | | | | |

Les trois campagnes de contrat qui utilisent ces adaptateurs ont été refaites avec les binaires corrigés :
- `http-incidents-2026-10-09T16-17-14-005Z-41e172` ;
- `access-policy-2026-10-09T16-19-19-822Z-f128cf` ;
- `tls-fingerprint-2026-10-09T16-20-40-847Z-1d6b5d`.

Les statuts par backend et par variante sont les mêmes que les 8 octobre. La seule exception, l'échec intermittent de Crawlee HTTP sur l'empreinte TLS, était déjà connue.

## Arrivées ouvertes à 250 par seconde

Le premier run, `http-open-v2-2026-10-09T15-30-40-832Z-1b2a42`, tournait avec les anciens binaires. Il est remplacé par `http-open-v2-2026-10-09T16-13-49-759Z-f9b6c9`, avec des pages normales, 8 par tâche, et 4 tâches simultanées au plus, sur 5 blocs. Ses 17 422 tentatives sont vérifiées et aucune n'échoue :

| Backend | Tâches vérifiées/s | p50 (ms) | p95 (ms) | CPU par tâche (ms) | RSS max | Démarrage (ms) |
| --- | --- | --- | --- | --- | --- | --- |
| `reqwest-http` | 116,0 | 30,6 | 33,0 | 1,51 | 9,4 Mio | 11 |
| `wreq-http` | 116,2 | 30,9 | 32,9 | 1,74 | 7,3 Mio | 10 |
| `impit-http` | 117,1 | 30,5 | 33,2 | 2,45 | 100,0 Mio | 152 |
| `spider-http` | 113,4 | 29,7 | 38,7 | 4,24 | 30,6 Mio | 16 |
| `crawlee-http-pooled` | 46,9 | 72,6 | 119,5 | 30,32 | 512,3 Mio | 1019 |
| `node-http` | 119,0 | 29,6 | 32,5 | 2,48 | 228,3 Mio | 169 |

## Lecture

- **Le débit est borné par la concurrence, pas par le transport.** Avec 4 tâches simultanées et une tâche de 8 pages, servie en 3 ms par page, le plafond tourne autour de 130 tâches/s. Cinq transports s'en approchent. Crawlee en pool reste loin derrière : son orchestration coûte 30 à 70 ms de CPU par tâche et plusieurs centaines de mégaoctets.
- **Ce qui sépare les transports, c'est le coût.** reqwest et wreq demandent environ 1,5 ms de CPU par tâche, moins de 11 Mio de mémoire, et démarrent en une dizaine de millisecondes. Sur le catalogue à incidents, Impit et Node consomment 10 fois plus de CPU et 10 fois plus de mémoire.
- **Une configuration par défaut n'est pas un contrat.** Deux transports sur six échouaient au catalogue à cause de leur configuration, pas de leur pile : la décompression manquait, et wreq ne suivait aucune redirection par défaut, contrairement à sa documentation. Le moteur doit fixer ces options lui-même et les couvrir par le gate de conformité, quel que soit le transport choisi.

## Verdict mécanique

`pnpm lab verdict D-http-transport` est calculé sur le seul run en boucle fermée. En arrivées ouvertes, tout transport qui suit atteint le débit offert : cette métrique n'y départage rien. La règle est la non-infériorité à 10 % contre reqwest :

| Option | Résultat | Rapport de débit (IC) |
| --- | --- | --- |
| wreq | non inférieur | 1,005 (0,983 à 1,028) |
| Impit | non inférieur | 0,958 (0,927 à 0,987) |
| Node | `insufficient-n` (5/14 blocs) | 0,901 (0,808 à 0,976) |
| Crawlee en pool | `insufficient-n` (5/14 blocs), nettement en dessous | 0,623 (0,567 à 0,683) |
| Spider | bloqué par le contrat | — |

L'hôte `local-dev` ne rend pas ce verdict éligible : il faut `local-controlled`. Il confirme pourtant la lecture : à débit égal, le choix se fait sur le coût CPU et mémoire, et reqwest reste le défaut, avec wreq comme variante au même coût.

## Limites

- Hôte `local-dev` : les durées et le CPU sont indicatifs. Le verdict de non-infériorité demande `local-controlled`.
- La concurrence de 4 sature avant le transport ; une campagne à concurrence plus haute dira où chaque transport plafonne.
