# Automan — rapport final de R&D (pré-PRD)

**Date :** 10 octobre 2026. **Statut :** rapport post-tests, base du PRD et de la documentation définitive. **Décision finale :** porteur du projet.

Ce dossier rassemble tout ce qui a servi à **choisir** l'architecture et le stack d'Automan : les propositions écrites avant les tests, le plan de R&D, les ADR proposés, les recherches sourcées et le laboratoire qui a mesuré chaque choix. La documentation définitive du produit (`docs/`, ADR adoptés dans `docs/adr/`) reste à écrire à partir d'ici. La section 9 dit comment.

Rien de ce rapport n'est un chiffre écrit à la main : chaque résultat renvoie à un run du [ledger](lab/ledger/results-ledger.json), à un verdict calculé ([`lab/verdicts/`](lab/verdicts/)) ou à un rapport daté ([`lab/reports/`](lab/reports/)).

---

## 1. En bref

L'architecture proposée le 7 octobre tient pour l'essentiel. Les tests ne l'ont pas renversée ; ils l'ont **précisée** sur onze points, dont quatre que la proposition ne contenait pas :

1. **Un tier « API découverte »** entre HTTP et navigateur : on capture le trafic JSON pendant un échantillon navigateur, puis on rejoue l'API. Une fois la route apprise, un run est 120 à 200 fois plus rapide qu'en navigateur, sans perte d'exactitude.
2. **Un journal d'effets obligatoire pour le mode code des agents** : une correction ne rejoue jamais une action déjà reçue par le site. Sans ce journal, un modèle réel faisait 17 faux succès sur 45.
3. **Des règles de frontier apprises** : paramètres sans effet appris par signature de page, chemins piégés fermés, pagination par curseur préférée. La pagination par décalage sur une table vivante perd des lignes et se déclare complète.
4. **Un modèle de session jugé sur le contenu des pages** : un portail qui renvoie `200` avec son formulaire de login trompe tout client qui se fie au statut HTTP.

Les choix centraux sont **confirmés par mesure** :
- noyau Rust/Tokio : 0,51 fois le CPU de Node par travail vérifié ;
- SQLite WAL `synchronous=FULL` : aucune perte sur 6 coupures de courant simulées ;
- reqwest comme transport HTTP par défaut ;
- rejeu compilé pour l'IA : 0,17 à 0,34 fois le coût en modèle.

Un point **change de nature** : le pilote navigateur par défaut est un **client CDP direct avec la garde d'actionnabilité Automan**, et Playwright n'est plus la référence de comportement. La garde est exacte sur tous les moteurs. Playwright natif laisse passer des clics sur des cibles en mouvement et dépasse ses délais à distance.

Ce qui **reste ouvert** relève surtout de trois choses :
- du **matériel** : hôte Linux de référence pour publier des chiffres, et KVM pour Firecracker ;
- de la **demande** : segment, prix, priorités, sinks ;
- d'un **périmètre exclu** de cette R&D : le profil stealth.

---

## 2. Méthode et force des preuves

Chaque expérience tranche une **décision** du registre ([`lab/decisions/`](lab/decisions/)), déclarée **avant** les mesures : options, référence, métrique principale, gates de contrat, plus petit effet d'intérêt (10 %), règle statistique et classe d'hôte exigée. Le verdict est calculé mécaniquement (plan apparié par blocs, bootstrap, Wilcoxon, TOST). Il prend l'une de ces valeurs : `better`, `worse`, `equivalent`, `non-inferior`, `insufficient-n`, `blocked-by-contract` ou `contract-met`.

**Gates de contrat d'abord.** Une option qui produit un seul faux succès (un résultat déclaré complet que l'oracle indépendant réfute) est bloquée, quelle que soit sa vitesse.

**Classes d'hôte.**

| Classe | Définition | Ce qu'elle permet |
| --- | --- | --- |
| `local-dev` | Mac en usage | Contrats et comptes (logins, requêtes, tokens, lignes perdues) : ils ne dépendent pas de l'hôte |
| `local-controlled` | Mac au repos : charge par CPU au plus 0,35, CPU étranger au plus 150 %, vérifié avant le run et à chaque bloc | Décider sur des durées |
| `reference` | Linux nu dédié | Publier des chiffres. **Jamais atteint dans cette R&D** |

**Force des preuves dans ce rapport.**
- 🟢 **Fort** : contrat vérifié par oracle, ou verdict mécanique éligible.
- 🟡 **Moyen** : effet net et cohérent, mais verdict formel `insufficient-n`, ou hôte non éligible.
- 🟠 **Faible** : indicatif, une seule mesure, ou conditions trop éloignées du réel.
- ⚪ **Non testé** : recherche documentaire ou choix de conception seulement.

Toutes les durées viennent d'un Mac M3 Max (macOS 27). Aucune n'est publiable telle quelle.

---

## 3. Tableau comparatif : stack avant les tests et recommandé après

« Avant » reprend la [proposition](proposals/proposition-automan.md), l'[architecture](proposals/architecture.md), l'[ADR-001](adr/0001-runtime-language.md), l'[ADR-002](adr/0002-access-identity-stealth.md) et la [qualification du 7 octobre](lab/reports/qualification-2026-10-07.md), produits par les sessions de conception avant toute campagne de décision.

| # | Domaine | Avant les tests | Recommandé maintenant | Verdict | Force |
| --- | --- | --- | --- | --- | --- |
| 1 | Langage du noyau | Rust/Tokio « candidat, pas un gagnant démontré » ; Node et Go crédibles | **Rust/Tokio** | `better` (CPU 0,51×), éligible | 🟢 |
| 2 | Pilote navigateur | CDP natif (Chromey/Chromiumoxide à qualifier) + worker Playwright de compatibilité et de référence | **CDP direct + garde Automan** par défaut ; Playwright seulement pour Firefox et WebKit, avec la même garde | bloqué (référence Playwright en timeout) ; estimation hors contrat nette | 🟡 |
| 3 | Contrat d'actionnabilité | Playwright comme référence de comportement | **Garde Automan**, indépendante du moteur | contrat 390/390 | 🟢 |
| 4 | Transport HTTP | reqwest de base ; Impit et wreq en option | **reqwest** par défaut, wreq en variante ; options fixées par le moteur et couvertes par un gate | `non-inferior`, éligible | 🟢 |
| 5 | Identité et accès | 3 profils exclusifs par session, `identified` par défaut, catalogue `stealth` en opt-in | Inchangé ; **tier HTTP exécuté par le navigateur** dès qu'une identité doit traverser l'escalade ; politique des sites dans le moteur, pas dans le transport | contrat ; durée non éligible | 🟢 / ⚪ stealth |
| 6 | Routage HTTP ou navigateur | « HTTP adaptatif fondé sur le résultat », apprentissage par route | Inchangé dans le principe, **plus un tier API découverte** ; comparaison sur échantillon, total déclaré comme invalidation | `better`, éligible | 🟢 |
| 7 | Frontier | Canonicalisation configurable, déduplication, fin de crawl calculée | **Plus** paramètres sans effet appris, chemins piégés fermés, curseur préféré au décalage | contrat (naïf bloqué) | 🟢 |
| 8 | Sessions longues | Objet Session, pas de réutilisation aveugle des sessions expirées | **Page jugée sur le contrat de la recette ; état de session dans le journal du run** ([ADR-003](adr/0003-session-model.md)) | `better` (logins 0,5×) | 🟢 |
| 9 | Admission et ordonnancement | File bornée, budgets par domaine ; équité « à prouver » | **Files bornées par domaine, servies à tour de rôle**, sous un plafond global ; admission journalisée par commits groupés, refus `E_CAPACITY` | 2,6× pour les petits domaines, `insufficient-n` | 🟡 |
| 10 | Exécution durable | Leases, checkpoints, écriture idempotente avant acquittement | Inchangé : **journal + leases + epoch**, départ de chaque travail journalisé | `contract-met` | 🟢 |
| 11 | Stockage local | SQLite WAL, writer unique par lots ; JSONL pour l'export | **SQLite WAL `synchronous=FULL`, writer unique, lots bornés** | décidé ; coupure : 0 perte | 🟢 |
| 12 | Sinks | PostgreSQL pour le cloud ; autres sinks non examinés | PostgreSQL `COPY` pour l'état partagé ; DuckDB analytique locale (un seul écrivain) ; Parquet pour l'export par lots ; **adoption à la demande** | tous durables ; débits `insufficient-n` | 🟡 |
| 13 | Extraction | Parser DOM HTML5, lol-html en streaming, htmd, extracteur de classe trafilatura : candidats | **rs-trafilatura** pour le contenu principal, **htmd** pour la conversion Markdown, **html5ever/scraper** pour le parsing structurel | `insufficient-n` par blocs ; tranché par page | 🟢 |
| 14 | Moteur léger | Lightpanda candidat, hors processus (AGPL) | **Pas de moteur par défaut** ; tier choisi par capacités | bloqué (shadow DOM) | 🟢 |
| 15 | Couche IA | Découvrir, compiler et réparer hors du rejeu normal, budgets | Inchangé ; **plan entier vérifié par empreinte avant le premier effet**, une réparation bornée | `better` (coût 0,17-0,34×) | 🟢 |
| 16 | Interfaces agents | SDK, CLI, MCP pilotant le moteur | Un catalogue d'opérations, servi **à l'identique par MCP et par outils de fonction** ; **mode code seulement avec journal d'effets** ; CLI pour humains et scripts | MCP ≡ SDK ; mode code journalisé meilleur | 🟢 / 🟡 |
| 17 | Topologie cloud | Pool de conteneurs ou snapshot Firecracker : reporté | **Pool chaud jeté ou remis à zéro après chaque tenant** ; Firecracker à mesurer | pool : contrat ; Firecracker non testé | 🟡 / ⚪ |
| 18 | Surfaces produit | Console React/shadcn + Vite ; app macOS SwiftUI ; licence Apache-2.0 | Inchangé, **non testé** | — | ⚪ |
| 19 | Observabilité | tracing + export OpenTelemetry, conventions GenAI | Inchangé, **non testé** | — | ⚪ |
| 20 | Marché | Segments candidats issus de la recherche | Inchangé, **en attente des entretiens** | — | ⚪ |

---

## 4. Chaque décision : pourquoi changer, pourquoi rester, alternatives

### 4.1 Langage du noyau (D-core-language) 🟢

- **Avant.** Rust/Tokio proposé dans l'ADR-001, avec la réserve explicite « ne démontre pas un avantage sur le temps total ». Node et Go étaient des alternatives crédibles.
- **Mesuré.** Même workflow durable, même build SQLite, Mac calme. Rust consomme **0,51 fois le CPU de Node par travail vérifié** (8 blocs, éligible). Les écrivains synchrones sur une bibliothèque SQLite commune donnent des débits voisins entre Node, Rust et Go ([workflow, 8 octobre](lab/reports/workflow-2026-10-08.md)). La reprise est exacte sous quatre points de panne injectés ([pannes runtime](lab/reports/runtime-faults-2026-10-08.md)).
- **Pourquoi rester.** Le coût par travail compte pour un service qui facture au résultat vérifié, et Rust tient la reprise exacte. Il n'y a aucune raison de changer.
- **Alternatives plausibles.**
  - Node, si la maintenance d'un pilote natif devenait le coût dominant. Les pilotes CDP Node et Rust ont des durées identiques, donc le langage n'a aucun effet côté navigateur.
  - Go, pour la distribution ; pas mesuré sur le workflow complet.
- **Limite.** Mac, pas hôte de référence.

### 4.2 Pilote navigateur (D-browser-driver) 🟡

- **Avant.** CDP natif via Chromey ou Chromiumoxide, à qualifier, plus un worker Playwright de compatibilité et de **référence de comportement**.
- **Mesuré, sur Mac calme, avec les blocs du 8 et du 10 octobre** ([rapport](lab/reports/quiet-host-2026-10-10.md)). Temps de tâche rapporté à Playwright natif (plus bas = plus rapide) :

  | Latence pilote ↔ navigateur | Formulaire | Clics | Catalogue JS |
  | --- | --- | --- | --- |
  | 0 ms (même machine) | 1,33 | 1,21 | 0,33 |
  | 20 ms | 0,80 | 0,84 | 0,73 |
  | 100 ms | 0,70 | 0,74 | 0,79 |
  | 300 ms | 0,68 | 0,72 | 0,80 |

  Ces chiffres valent pour le CDP Rust avec garde. Le CDP Node avec garde donne les mêmes à ±0,02. Puppeteer avec garde est le plus lent.
- **Pourquoi changer.**
  - Un navigateur hébergé est toujours à distance du pilote. Dès 20 ms, le CDP direct avec garde est 16 à 32 % plus rapide.
  - Playwright natif n'est pas une référence exacte : il laisse passer un clic sur une cible encore en mouvement ([multi-moteurs](lab/reports/cross-engine-2026-10-09.md)), et il a dépassé deux fois son délai à 100 ms.
  - La garde Automan, elle, est exacte partout.
- **Pourquoi le verdict reste bloqué.** Le gate d'exactitude lit aussi les cellules de la référence, et la référence, Playwright, a échoué. Les durées ci-dessus sont une estimation hors contrat, aux intervalles très étroits (largeur < 0,1).
- **À trancher par le porteur.** Remplacer la référence de la décision par un pilote exact à toutes les latences. Je ne l'ai pas fait après coup, pour ne pas orienter le verdict.
- **Alternatives plausibles.**
  - Playwright natif sur boucle locale, s'il est 20 à 33 % plus rapide à 0 ms et que l'exactitude de la garde n'est pas exigée.
  - Le runtime en extension Chrome, à la Stagehand v4 : placement près du navigateur, non mesuré dans les campagnes finales.
- **Non tranché.** La bibliothèque CDP Rust (Chromey, Chromiumoxide ou client maison) : le prototype utilise un client WebSocket direct.

### 4.3 Contrat d'actionnabilité 🟢

- **Avant.** L'actionnabilité Playwright servait de référence.
- **Mesuré.** La garde Automan, faite de scripts de page sur le DOM (unicité, visibilité, stabilité, test de cible, interception d'événements), passe **390 cellules sur 390** sur Chromium, WebKit et Firefox, sous Linux et macOS. Elle avait déjà passé 162 cellules sur 162 sur la matrice d'actionnabilité Chromium. Deux défauts de la garde ont été trouvés et corrigés en route ([contrats pilotes](lab/reports/browser-driver-contracts-2026-10-08.md)).
- **Pourquoi changer.** Le contrat appartient à Automan et ne dépend plus du moteur ni du pilote.
- **Limite.** Le build Firefox de Playwright ne démarre pas sous macOS 27 ; il est vérifié sous Linux.

### 4.4 Transport HTTP (D-http-transport) 🟢

- **Avant.** reqwest de base ; Impit et wreq comme transports spécialisés optionnels.
- **Mesuré, sur Mac calme.**
  - wreq, Impit et Node sont non inférieurs à reqwest en débit ; Crawlee en pool est moins bon (0,86).
  - À débit égal, reqwest et wreq coûtent environ 1,5 ms de CPU par tâche, moins de 11 Mio de mémoire et une dizaine de ms de démarrage. Impit et Node coûtent 10 fois plus en CPU et en mémoire ([transports](lab/reports/http-transport-2026-10-09.md)).
  - Le gate a trouvé deux défauts de configuration d'adaptateur : reqwest compilé sans décompression gzip, et wreq sans politique de redirection (son défaut réel est `none`, contrairement à sa documentation).
  - Spider refuse les redirections vers loopback, un contrôle SSRF conservé.
- **Pourquoi rester sur reqwest.** Il est le plus sobre, au même débit, et ses options sont maîtrisées.
- **Ce qui change.** Le moteur fixe lui-même décompression, redirections et délais, et un gate de conformité les vérifie, quel que soit le transport.
- **Alternatives plausibles.**
  - wreq, au même coût, s'il faut une empreinte TLS de navigateur côté HTTP pur.
  - Impit, s'il faut une émulation Chrome et que le coût CPU est acceptable.
- **Limite.** À concurrence 4, la concurrence plafonne avant le transport : on ne sait pas où chaque transport plafonne lui-même.

### 4.5 Identité et accès (D-identity, ADR-002) 🟢 / ⚪

- **Avant.** ADR-002 : trois profils exclusifs par session (`identified` par défaut, `standard`, `stealth` en opt-in), Web Bot Auth, politique des préférences des sites, CAPTCHA par niveaux.
- **Mesuré, en contrat.**
  - Une session garde un seul profil.
  - Web Bot Auth est vérifié de façon indépendante avec la bibliothèque de Cloudflare.
  - robots.txt, Content-Signal, TDMRep et meta `noai` sont appliqués selon la finalité déclarée.
  - **Impit et le client HTTPS de Node changent l'empreinte TLS/HTTP2 au moment où la session passe au navigateur** ; seul le tier HTTP exécuté par le navigateur (`browser-fetch`) garde une identité cohérente ([identité](lab/reports/identity-2026-10-08.md), [empreinte TLS](lab/reports/tls-fingerprint-2026-10-08.md)).
  - Le pool de proxies tient le sticky, le basculement et l'épuisement typé.
- **Pourquoi rester sur l'ADR-002.** Ses principes sont confirmés.
- **Ce qui change.** Le tier HTTP d'une session dont l'identité traverse l'escalade s'exécute dans le navigateur. Les préférences des sites relèvent du moteur, pas du transport : le gate `policy-compliance` a été retiré de D-http-transport, car aucun transport ne pouvait le passer seul.
- **Alternatives plausibles.** Un transport d'impersonation (wreq, Impit) pour les sessions purement HTTP, sans escalade vers le navigateur.
- **Non testé.** Le coût en durée de la cohérence d'identité (hôte non éligible). Le **profil stealth** : navigateurs patchés, empreintes, comportement, CAPTCHA ; hors du périmètre de cette R&D.

### 4.6 Routage HTTP, navigateur ou API (D-adaptive-routing) 🟢

- **Avant.** Architecture §5.3 : routage `auto` fondé sur le résultat, appris par route ; « une taille HTML ou deux datasets vides ne suffisent pas ».
- **Mesuré.** Le même listing est servi de six façons : HTML, coquille JS, hydratation partielle, HTML périmé, GraphQL et scroll infini ([portails](lab/reports/portal-2026-10-09.md)).
  - L'heuristique « navigateur si la page HTTP est vide » déclare complets des prix périmés et des pages partielles.
  - L'échantillon comparé est exact sur 120 runs sur 120.
  - La découverte d'API, qui capture le JSON pendant l'échantillon puis infère un plan par page ou par curseur validé contre le navigateur, est exacte sur 120 runs sur 120.
  - Sur Mac calme, l'échantillon est **meilleur et éligible** (0,004 du temps navigateur sur page statique, équivalent ailleurs), et l'API découverte est meilleure sur GraphQL et le scroll infini (0,008) ([Mac calme](lab/reports/quiet-host-2026-10-10.md)).
- **Pourquoi rester.** Le principe de l'architecture est confirmé.
- **Ce qui change.** On ajoute **la découverte d'API comme tier de routage**, avec la même portée et les mêmes invalidations qu'une décision HTTP.
- **Alternatives plausibles.** Le navigateur seul : la référence exacte, mais la plus chère.
- **Limite.** Plans d'API simples seulement : pas de signature de requête ni de jeton à usage unique.

### 4.7 Frontier (D-frontier) 🟢

- **Avant.** §3.1 : canonicalisation configurable, déduplication dans un scope, fin de crawl calculée.
- **Mesuré.**
  - Avec des `sid` et des `utm` dans chaque lien, des tris permutés et un calendrier sans fin, la frontier naïve épuise son budget sans pouvoir conclure.
  - Avec des paramètres sans effet appris par signature de page, des chemins piégés fermés et le curseur préféré, la frontier canonique termine en 75 requêtes.
  - **La pagination par décalage sur une table modifiée pendant le crawl perd des lignes et se déclare complète : 15 faux succès sur 15.**
- **Ce qui change.** Les trois règles apprises sont ajoutées, et un crawl arrêté par son budget n'est jamais un résultat complet.
- **Alternatives plausibles.** Une canonicalisation déclarée seulement dans la recette : elle suffit tant qu'on connaît le site, pas pour la découverte.

### 4.8 Sessions longues (D-session-model, ADR-003) 🟢

- **Avant.** Une session a une identité, des cookies, un backend et un lease ; une session expirée n'est pas réutilisée aveuglément.
- **Mesuré.**
  - Un portail qui renvoie `200` avec son formulaire de login à l'expiration fait **20 faux succès sur 20** à un client qui se fie au statut HTTP.
  - Juger chaque page sur le contrat de la recette les supprime.
  - Reprendre l'état de session journalisé divise les logins par deux ; une session révoquée pendant l'arrêt coûte un seul login, sans perte.
- **Ce qui change.** [ADR-003](adr/0003-session-model.md) est proposé.
- **Alternatives plausibles.** Se reconnecter à chaque run : exact, mais un login de plus par reprise, ce qu'un portail avec MFA fait payer cher.

### 4.9 Admission et ordonnancement (D-scheduling) 🟡

- **Avant.** File bornée, budgets par domaine ; l'équité restait à prouver.
- **Mesuré.**
  - Avec un domaine lourd en surcharge, des files bornées par domaine servies à tour de rôle donnent **2,6 fois** plus de travail aux petits domaines qu'une file globale (IC 2,31 à 2,93, Mac calme), et 2,84 fois sur Mac chargé.
  - L'admission est journalisée par commits groupés. Le refus `E_CAPACITY` n'est jamais exécuté. Le départ de chaque travail est journalisé, ce qui préserve la politesse après un crash ([admission](lab/reports/admission-2026-10-08.md)).
- **Pourquoi `fair-domain`.** L'effet est grand et le mécanisme évident. Le verdict formel demande 22 blocs ; 5 ont été mesurés.
- **Alternatives plausibles.** File globale FIFO : plus simple, acceptable sans domaine dominant.

### 4.10 Exécution durable (D-durable-runtime) 🟢

- **Avant.** Leases, checkpoints, écriture idempotente avant acquittement.
- **Mesuré.**
  - **Contrat tenu** : admission sans perte, reprise exacte sous pannes injectées (avant ou après commit, pendant l'acquittement, verrou affamé, erreur d'écriture), oracle exact.
  - La resoumission idempotente est acquittée en doublon sans double exécution.
- **Pourquoi rester.** Le contrat tient.
- **Non éligible.** La latence d'acquittement (durée) n'a pas été mesurée sur hôte qualifié.

### 4.11 Stockage local (D-storage) 🟢

- **Avant.** SQLite WAL et writer unique par lots.
- **Mesuré.**
  - Les lots augmentent le débit dans toutes les configurations, et un écrivain fait mieux que quatre.
  - **Coupure de courant simulée** (VM Docker tuée en pleine écriture, 6 coupures) : `synchronous=FULL` ne perd aucun commit acquitté. `NORMAL` en perd (96 sur 108 803) : il résiste au crash du processus, pas à la coupure. JSONL non synchronisé perd tout. Aucune base n'est corrompue ([sinks et coupures](lab/reports/sinks-topology-2026-10-09.md)).
- **Pourquoi rester.** C'est exactement le choix décidé.
- **Ce qui change.** Tout mode « rapide » doit être affiché comme non durable face à une panne de la machine.

### 4.12 Sinks (D-sinks) 🟡

- **Mesuré.**
  - PostgreSQL `COPY` (conteneur local), DuckDB et Parquet par lot gardent chaque ligne acquittée, après un crash du writer comme après une coupure ; PostgreSQL ne perd aucun de ses 2 324 commits acquittés sur 3 coupures.
  - Débits à 512 lignes par lot : DuckDB 515 000/s, SQLite 393 000/s, JSONL 119 000/s, PostgreSQL 70 000/s, Parquet 47 000/s.
- **Pourquoi ne rien adopter maintenant.** La décision dépend de la demande des clients.
- **Ce que ces débits ne disent pas.** Les durabilités ne sont pas égales sur macOS : Node demande `F_FULLFSYNC`, SQLite et DuckDB un `fsync` simple, et PostgreSQL tourne dans une VM.
- **Vue de travail.**
  - SQLite pour l'état local ;
  - PostgreSQL `COPY` pour l'état partagé du cloud ;
  - DuckDB pour l'analytique locale, avec un seul écrivain ;
  - Parquet pour l'export par lots.

### 4.13 Extraction (D-extraction) 🟢

- **Avant.** Plusieurs candidats : parser DOM HTML5, lol-html en streaming, htmd, extracteur de classe trafilatura.
- **Mesuré.** Corpus WCXB, split test réservé, 511 pages, analyse appariée **par page** ([analyse par page](lab/reports/extraction-paired-2026-10-09.md)) :
  - rs-trafilatura atteint un F1 de 0,884 ;
  - il bat la référence html5ever de **0,165** (IC 0,144 à 0,186) sur 469 pages, et Readability ou dom-smoothie d'environ 0,06.
  - htmd convertit en Markdown sans isoler le contenu principal.
  - Le rendu par Chrome ou Lightpanda n'apporte rien sur des pages déjà rendues ([pages figées](lab/reports/frozen-pages-2026-10-08.md)).
- **Pourquoi changer.** La proposition laissait le choix ouvert. On adopte la chaîne rs-trafilatura (contenu principal), puis htmd (Markdown), avec html5ever/scraper pour les sélecteurs et les champs.
- **Alternatives plausibles.** Readability ou dom-smoothie : proches en médiane, échouent nettement sur quelques pages.
- **Non testé.** lol-html, faute d'adaptateur. La qualité du Markdown produit (titres, listes, tableaux). WebMainBench.

### 4.14 Moteur léger (D-light-engine) 🟢

- **Avant.** Lightpanda candidat, hors processus (AGPL).
- **Mesuré.** Chrome et Lightpanda appariés dans un même run :
  - **le shadow DOM échoue** sur Lightpanda ;
  - sur le reste, il consomme 1/8 à 1/20 du CPU et 1/15 à 1/65 de la mémoire de Chrome ([moteur léger apparié](lab/reports/light-engine-paired-2026-10-09.md)).
- **Ce qui change.** Il n'est pas un moteur par défaut, mais un tier choisi par les capacités déclarées, avant la création de la session.

### 4.15 Couche IA (D-ai-layer) 🟢

- **Avant.** Découvrir, compiler et réparer hors du rejeu normal, avec des budgets.
- **Mesuré, avec trois modèles réels via une gateway.**
  - Le rejeu compilé coûte **0,17 à 0,34** fois la dépense de modèle d'un agent qui replanifie à chaque run, sur 12 blocs.
  - Aucun faux succès. La vérification du **plan entier par empreinte avant le premier effet** refuse une cible remplacée par un contrôle destructif ([modèles réels](lab/reports/ai-real-models-2026-10-09.md)).
- **Pourquoi rester.** Le principe de la proposition est confirmé, et sa mécanique précisée.

### 4.16 Interfaces agents (D-agent-interface) 🟢 / 🟡

- **Mesuré.** 675 tâches, 3 modèles, 5 interfaces ([mode code journalisé](lab/reports/code-mode-journal-2026-10-09.md)).
  - MCP et outils de fonction sont **équivalents**.
  - La CLI dépend du modèle : Claude Haiku laisse 18 tâches sur 45 non terminées.
  - Le mode code brut fait 17 faux succès sur 45 avec GPT-5.6 Luna.
  - **Le mode code avec journal d'effets fait 0 faux succès**, en 3 à 5 fois moins de tours que MCP (verdict `better` sur la lecture de prix, `insufficient-n` ailleurs, IC 0,16 à 0,26).
- **Ce qui change.** On expose un seul catalogue d'opérations par MCP et par SDK, et le mode code uniquement avec le journal d'effets.

### 4.17 Topologie cloud (D-cloud-topology) 🟡 / ⚪

- **Mesuré.** Navigateurs dans Docker :
  - un pool chaud supprime environ 180 ms de démarrage par session, pour environ 39 Mio par conteneur en attente ;
  - **un pool qui rend ses conteneurs sans les remettre à zéro fait fuir le cookie d'un tenant vers le suivant, 30 fois sur 30.**
- **Ce qui change.** Un pool doit jeter ou remettre à zéro son conteneur avant chaque tenant, et ce contrat doit être un gate permanent.
- **Non testé.** Firecracker, faute de KVM.

---

## 5. Données qui ne permettent pas réellement de trancher

| Donnée | Pourquoi elle ne tranche pas | Ce que j'en ai décidé quand même | Comment la trancher |
| --- | --- | --- | --- |
| Pilote navigateur, durées | Verdict bloqué : la **référence** Playwright a dépassé ses délais, et le gate lit aussi la référence | CDP + garde par défaut, sur l'estimation hors contrat | Changer la référence pour un pilote exact (décision du porteur), puis recalculer |
| Bibliothèque CDP Rust | Non comparée : le prototype utilise un client direct | Aucune ; client direct en attendant | Qualifier Chromey et Chromiumoxide contre la garde |
| Ordonnanceur, 2,6× | `insufficient-n` (5 blocs sur 22) | `fair-domain` | 17 blocs de plus sur Mac calme |
| Mode code journalisé, tours | `insufficient-n` sur 2 variantes sur 3 (une correction présente dans certains blocs seulement) | L'offrir | Appariement par tâche plutôt que par bloc |
| API découverte, 4 rendus sur 6 | `insufficient-n` malgré un écart de deux ordres de grandeur | La retenir comme tier | Plus de blocs, ou une règle qui tienne compte de l'effet observé |
| Débits des sinks | Durabilités inégales sur macOS ; PostgreSQL dans une VM ; hôte non éligible | Aucune adoption | Durabilité égalisée (`fullfsync`), hôte Linux de référence |
| Coût de la cohérence d'identité | Durées sur hôte non éligible | Aucune ; le contrat suffit pour le choix | Rerun sur Mac calme |
| Latence d'acquittement du runtime | Hôte non éligible | Aucune ; le contrat tient | Rerun sur Mac calme |
| Transports à forte concurrence | Concurrence 4 : plafond atteint avant le transport | reqwest, sur le coût | Campagne à concurrence plus haute |
| Extraction par blocs | 7 types de page = 7 blocs | Tranché par l'analyse par page | Déjà fait |
| Toutes les durées | Mac, pas Linux nu | Décisions de conception seulement | Hôte de référence ([préparation](plan/reference-host.md)) |
| Firecracker | Pas de KVM | Pool de conteneurs | Linux avec KVM |
| Segment, prix, priorités, sinks demandés | Recherche documentaire seulement | Aucune | Entretiens ([kit](research/market/)) |
| Profil stealth | Hors périmètre | Aucune | Décision et travaux du porteur |

---

## 6. Leçons de méthode

Ce sont des défauts du laboratoire trouvés en route, tous corrigés. À garder pour la suite :

1. **Juger un bloc sur le CPU étranger, pas sur la charge moyenne.** La charge moyenne inclut le benchmark lui-même : elle a fait passer pour bruités 7 blocs propres sur 8.
2. **Une configuration par défaut n'est pas un contrat.** reqwest sans gzip et wreq sans redirections échouaient au catalogue ; seul le gate de conformité l'a vu.
3. **Un gate qui lit la référence peut bloquer une option exacte.** C'est le cas du pilote navigateur ; la référence d'une décision doit être exacte partout.
4. **La règle de taille d'échantillon ne tient pas compte de l'effet observé.** Elle demande 100 à 200 blocs pour des écarts de 100×. Il faut choisir la bonne unité d'appariement (la page, la tâche) plutôt que répéter des blocs déterministes.
5. **La durabilité dépend de l'OS.** Sur macOS, `fsync` ne vide pas le cache du disque. À égaliser avant toute comparaison de débit entre sinks.
6. **Ne jamais modifier les sources pendant une campagne.** Le moteur marque le run `invalid-provenance`, et c'est arrivé une fois.
7. **Toujours vérifier qu'un commit contient ce qu'il annonce.** Une correction refusée en cours de commande a laissé un commit au titre trompeur, et PostgreSQL non mesuré lors de la première série de coupures.

---

## 7. Ce qui reste à faire

| Travail | Pour quelle décision | Ce qu'il faut |
| --- | --- | --- |
| Référence exacte pour D-browser-driver, puis recalcul | Pilote navigateur | Décision du porteur |
| Hôte Linux nu de référence | Toutes les affirmations publiables, mémoire par cgroup | Une machine ([préparation](plan/reference-host.md)) |
| Firecracker | Topologie cloud | Linux avec KVM |
| Blocs supplémentaires sur Mac calme | Ordonnanceur, routage, mode code | Une nuit de Mac au repos |
| Entretiens | Segment, prix, priorités, sinks | Le porteur ([guide](research/market/interview-guide.md)) |
| Profil stealth | ADR-002, catalogue `stealth` | Le porteur |
| Qualité du Markdown, WebMainBench, lol-html | Extraction | Campagnes à ajouter |

---

## 8. Plan du dossier `rnd/`

| Dossier | Contenu | Statut |
| --- | --- | --- |
| [`proposals/`](proposals/) | Propositions d'avant les tests : [proposition](proposals/proposition-automan.md), [architecture](proposals/architecture.md), [API](proposals/api.md), [design produit](proposals/product-design.md), [plan de documentation](proposals/documentation-plan.md), [maquette de console](proposals/design/automan-console.html) | Historique : à relire à la lumière de ce rapport |
| [`plan/`](plan/) | [Plan de R&D](plan/rd-plan.md), [hôte de référence](plan/reference-host.md) | Phases P0 à P9 |
| [`adr/`](adr/) | ADR **proposés** : [0001 langage](adr/0001-runtime-language.md), [0002 accès et identité](adr/0002-access-identity-stealth.md), [0003 sessions](adr/0003-session-model.md) | À adopter dans `docs/adr/` |
| [`research/`](research/) | Recherches sourcées ([index](research/README.md)) et [piste marché](research/market/) | Sources datées |
| [`lab/`](lab/) | Harnais de test : moteur, lanes, fixtures et oracles, [décisions](lab/decisions/), [ledger](lab/ledger/), [verdicts](lab/verdicts/), [rapports](lab/reports/) ; mode d'emploi dans [`lab/README.md`](lab/README.md) | Exécutable depuis `rnd/lab` |

---

## 9. Pour Wayfinder : passer de la R&D à la documentation définitive

Ordre recommandé :

1. **Adopter les décisions.** Pour chaque ligne 🟢 du tableau de la section 3, écrire l'ADR définitif dans `docs/adr/`, au format Matt Pocock (configuration dans `docs/agents/domain.md`). Il cite son verdict (`rnd/lab/verdicts/D-*.json`) et son rapport. Les ADR 0001 à 0003 de `rnd/adr/` sont des brouillons à reprendre. Les candidats évidents :
   - noyau Rust/Tokio ;
   - CDP + garde Automan ;
   - reqwest et le gate de transport ;
   - routage par échantillon et tier API découverte ;
   - frontier apprise et curseur ;
   - modèle de session ;
   - files équitables par domaine ;
   - journal + leases + epoch ;
   - SQLite WAL FULL ;
   - chaîne d'extraction ;
   - rejeu compilé ;
   - catalogue d'opérations agents avec mode code journalisé ;
   - pool de conteneurs remis à zéro.
2. **Mettre à jour le vocabulaire** dans `GLOSSARY.md` : route apprise, tier API découverte, contrat de page de session, journal d'effets, garde d'actionnabilité, faux succès, classe d'hôte.
3. **Réécrire l'architecture** dans `docs/` à partir de [`proposals/architecture.md`](proposals/architecture.md), en y intégrant les onze précisions de la section 1. Retirer Playwright du rôle de référence de comportement.
4. **Garder ouverts** les points 🟡 et ⚪ comme questions explicites du PRD, avec ce qui les tranchera (section 5).
5. **Écrire le PRD** quand les entretiens auront donné segment, prix et priorités.

Le laboratoire reste l'outil de qualification : toute nouvelle option se déclare dans `lab/decisions/` avant d'être mesurée.
