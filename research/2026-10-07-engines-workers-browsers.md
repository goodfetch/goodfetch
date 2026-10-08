# Moteurs, workers et navigateurs : Stagehand, Crawlee et Spider-rs

Relevé du 7 octobre 2026. Le moteur du framework orchestre le travail ; le navigateur effectue le rendu et exécute le JavaScript des pages ; la bibliothèque de pilotage transmet les commandes au navigateur. « Worker » peut désigner une tâche async, un composant d'extension ou un processus distribué, selon le projet.

## Stagehand v4

Le runtime est écrit en TypeScript et distribué en JavaScript dans une **extension Chrome**, notamment son service worker. Les SDK TS/Python/Go s'exécutent dans le processus de l'application et parlent au runtime par JSON-RPC transporté sur CDP. Le service worker est un composant du navigateur, pas une queue distribuée de jobs. Stagehand v4 pilote Chromium/Chrome par CDP et ne dépend plus de Playwright. [Architecture du dépôt](https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md), [migration officielle](https://docs.stagehand.dev/v4/migrations/playwright), [placement et transport](https://www.browserbase.com/blog/stagehand-v4).

## Crawlee TypeScript

Le framework est écrit en **TypeScript, exécuté en JavaScript dans Node.js**. `AutoscaledPool` orchestre des tâches asynchrones concurrentes en fonction des ressources ; `BrowserPool` gère navigateurs et pages. Ce sont des responsabilités distinctes d'une distribution de jobs sur plusieurs machines. Les backends navigateur sont **Playwright ou Puppeteer** ; les navigateurs possibles comprennent Chromium/Chrome, Firefox et WebKit selon le backend. Les chemins HTTP n'exigent pas un navigateur et peuvent utiliser un transport natif comme Impit. [Introduction](https://crawlee.dev/js/docs/introduction), [pool async](https://crawlee.dev/js/api/core/class/AutoscaledPool), [Impit](https://crawlee.dev/js/docs/guides/impit-http-client).

## Crawlee Python

Cette implémentation possède un cœur **Python**, un `AutoscaledPool` async et un pool de navigateurs. Le backend de navigation standard est **Playwright** ; le plugin indique Chromium par défaut et permet Firefox, WebKit et Chrome selon configuration. L'intégration Stagehand est un backend distinct, avec ses propres limites de contexte et de navigateur. [Architecture](https://crawlee.dev/python/docs/guides/architecture-overview), [pool async](https://crawlee.dev/python/api/class/AutoscaledPool), [plugin Playwright et types de navigateurs](https://crawlee.dev/python/api/class/PlaywrightBrowserPlugin).

## Spider-rs — moteur, workers et navigateur

Vérification du 7 octobre 2026, sur la version publiée **2.53.9**, SHA **`e4b53c5b582357e46433fa9ba3d91f930fb67cb4`**.

| Couche | Technologie réellement utilisée |
|---|---|
| Moteur | Rust ; acquisition HTTP principalement via `reqwest`, parsing et politiques de crawl dans Spider. |
| Concurrence normale | Tâches asynchrones **Tokio**, contrôlées par sémaphores. Cela ne signifie pas un processus navigateur par worker. |
| Navigateur optionnel | **Chromey**, client CDP Rust, pilote **Chromium/Chrome** local ou un navigateur connecté via CDP. Le navigateur assure le rendu et l'exécution JavaScript. Features Cargo `chrome`/`smart` nécessaires selon le chemin choisi. |
| Distribution optionnelle | Feature `decentralized` et binaire OSS **`spider_worker`**. Serveur Rust **Warp 0.4**, transport HTTP **Hyper 1 + hyper-util**, sur Tokio ; TLS optionnel. |
| Rendu du worker distribué fourni | Code `Page::new_page_streaming()` ou `fetch_page_html_raw()` : fetch/parsing HTTP. Son manifeste n’active pas Chrome ; ce worker ne doit pas être présenté comme un worker de rendering JavaScript garanti. |

Sources figées : [dépendances et features du moteur](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml), [manifeste worker](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/Cargo.toml), [serveur et handlers HTTP](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/src/main.rs).

Les workers locaux, le worker distribué OSS et l’infrastructure privée **Spider Cloud** sont trois niveaux distincts. Ces sources ne permettent pas de décrire la flotte ou l’orchestration privée du service commercial. Aucun benchmark ni lancement local nécessaire pour cette vérification de dépendances et appels ; aucune nouvelle recherche Exa.
