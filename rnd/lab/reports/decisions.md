# Décisions du lab

Rapport généré par `pnpm lab report` depuis `decisions/` et `verdicts/`. Les chiffres viennent des verdicts calculés ; aucune valeur n'est saisie à la main.

| Décision | Statut | Verdict | Estimation option / baseline | Blocs | Hôte | Gates |
| --- | --- | --- | --- | --- | --- | --- |
| D-adaptive-routing | open | sampled-contract / browser-only [variant static] : option meilleure | 0.00423 [0.00402 ; 0.00447] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | sampled-contract / browser-only [variant js-shell] : équivalentes | 1 [0.998 ; 1.01] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | sampled-contract / browser-only [variant partial-hydration] : équivalentes | 1 [0.996 ; 1.01] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | sampled-contract / browser-only [variant stale-html] : équivalentes | 0.998 [0.993 ; 1] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | sampled-contract / browser-only [variant graphql] : équivalentes | 0.997 [0.991 ; 1] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | sampled-contract / browser-only [variant infinite-scroll] : écart réel mais sous le seuil | 0.996 [0.992 ; 0.999] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | api-capture / browser-only [variant static] : blocs insuffisants | 0.00466 [0.00422 ; 0.00514] (95% block bootstrap) | 5 sur 13 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | api-capture / browser-only [variant js-shell] : blocs insuffisants | 0.00583 [0.00527 ; 0.00659] (95% block bootstrap) | 5 sur 18 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | api-capture / browser-only [variant partial-hydration] : blocs insuffisants | 0.00616 [0.00557 ; 0.00691] (95% block bootstrap) | 5 sur 19 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | api-capture / browser-only [variant stale-html] : blocs insuffisants | 0.00611 [0.00565 ; 0.00665] (95% block bootstrap) | 5 sur 10 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | api-capture / browser-only [variant graphql] : option meilleure | 0.0083 [0.00786 ; 0.00873] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | api-capture / browser-only [variant infinite-scroll] : option meilleure | 0.00758 [0.00715 ; 0.00804] (95% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | heuristic / browser-only [variant static] : bloqué par un contrat |  | 5 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | heuristic / browser-only [variant js-shell] : bloqué par un contrat |  | 5 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | heuristic / browser-only [variant partial-hydration] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | heuristic / browser-only [variant stale-html] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | heuristic / browser-only [variant graphql] : bloqué par un contrat |  | 5 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | heuristic / browser-only [variant infinite-scroll] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | http-only / browser-only [variant static] : bloqué par un contrat |  | 5 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | http-only / browser-only [variant js-shell] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | http-only / browser-only [variant partial-hydration] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | http-only / browser-only [variant stale-html] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | http-only / browser-only [variant graphql] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | http-only / browser-only [variant infinite-scroll] : bloqué par un contrat |  | 0 | local-controlled / requis local-controlled | oracle-exact échec |
| D-agent-interface | open | cli / mcp [variant baseline] : blocs insuffisants | 1.09 [0.86 ; 1.41] (95% block bootstrap) | 24 sur 157 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | cli / mcp [variant decoy-swap] : blocs insuffisants | 1.05 [0.847 ; 1.33] (95% block bootstrap) | 24 sur 134 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | cli / mcp [variant price-change] : option moins bonne | 1.43 [1.35 ; 1.5] (95% block bootstrap) | 24 sur 9 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | sdk / mcp [variant baseline] : équivalentes | 0.954 [0.884 ; 1.02] (95% block bootstrap) | 24 sur 15 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | sdk / mcp [variant decoy-swap] : équivalentes | 1.01 [0.993 ; 1.03] (95% block bootstrap) | 24 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | sdk / mcp [variant price-change] : équivalentes | 0.983 [0.951 ; 1] (95% block bootstrap) | 24 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | code-mode / mcp [variant baseline] : bloqué par un contrat |  | 18 | local-dev / requis local-dev | oracle-exact échec |
|  |  | code-mode / mcp [variant decoy-swap] : bloqué par un contrat |  | 16 | local-dev / requis local-dev | oracle-exact échec |
|  |  | code-mode / mcp [variant price-change] : bloqué par un contrat |  | 24 | local-dev / requis local-dev | oracle-exact échec |
|  |  | code-mode-journal / mcp [variant baseline] : blocs insuffisants | 0.197 [0.159 ; 0.255] (95% block bootstrap) | 15 sur 93 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | code-mode-journal / mcp [variant decoy-swap] : blocs insuffisants | 0.195 [0.156 ; 0.251] (95% block bootstrap) | 15 sur 99 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | code-mode-journal / mcp [variant price-change] : option meilleure | 0.34 [0.333 ; 0.353] (95% block bootstrap) | 15 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
| D-ai-layer | open | compiled-replay / agent-each-run [variant baseline] : option meilleure | 0.201 [0.197 ; 0.206] (95% block bootstrap) | 12 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | compiled-replay / agent-each-run [variant relabel] : option meilleure | 0.332 [0.328 ; 0.337] (95% block bootstrap) | 12 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | compiled-replay / agent-each-run [variant decoy-swap] : option meilleure | 0.343 [0.338 ; 0.353] (95% block bootstrap) | 12 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | compiled-replay / agent-each-run [variant price-change] : option meilleure | 0.167 [0.166 ; 0.169] (95% block bootstrap) | 12 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
| D-browser-driver | open | rust-cdp-guarded / playwright-native [scenario form, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario clicks, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario js-catalog, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario form, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario clicks, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario js-catalog, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario form, rttMs 100] : bloqué par un contrat |  | 11 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario clicks, rttMs 100] : bloqué par un contrat |  | 11 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario js-catalog, rttMs 100] : bloqué par un contrat |  | 9 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario form, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario clicks, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | rust-cdp-guarded / playwright-native [scenario js-catalog, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario form, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario clicks, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario js-catalog, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario form, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario clicks, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario js-catalog, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario form, rttMs 100] : bloqué par un contrat |  | 11 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario clicks, rttMs 100] : bloqué par un contrat |  | 11 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario js-catalog, rttMs 100] : bloqué par un contrat |  | 9 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario form, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario clicks, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native [scenario js-catalog, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario form, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario clicks, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario js-catalog, rttMs 0] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario form, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario clicks, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario js-catalog, rttMs 20] : bloqué par un contrat |  | 15 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario form, rttMs 100] : bloqué par un contrat |  | 11 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario clicks, rttMs 100] : bloqué par un contrat |  | 11 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario js-catalog, rttMs 100] : bloqué par un contrat |  | 9 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario form, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario clicks, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native [scenario js-catalog, rttMs 300] : bloqué par un contrat |  | 6 | local-controlled / requis local-controlled | actionability-conformance ok, false-success-count ok, oracle-exact échec |
| D-cloud-topology | deferred | firecracker-snapshot / container-pool : bloqué par un contrat |  | 0 | local-dev / requis reference (non éligible) | session-isolation échec, oracle-exact échec |
| D-core-language | open | rust-tokio / node : option meilleure | 0.514 [0.502 ; 0.528] (95% block bootstrap) | 8 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok, recovery-exact ok |
| D-durable-runtime | open | journal-leases-epoch / journal-leases-epoch : contrat tenu |  | 5 | local-dev / requis local-controlled (non éligible) | admission-no-loss ok, recovery-exact ok, oracle-exact ok |
| D-extraction | open | htmd / html5ever-scraper [] : blocs insuffisants | 0.652 [0.595 ; 0.709] (90% block bootstrap) | 7 sur 60 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | trafilatura-class / html5ever-scraper [] : blocs insuffisants | 1.3 [1.23 ; 1.38] (90% block bootstrap) | 7 sur 28 requis | local-dev / requis local-dev | oracle-exact ok |
| D-feature-priority | open | produit : research/market/README.md | | | | |
| D-frontier | open | naive / canonical [variant crawler-trap] : bloqué par un contrat |  | 0 | local-dev / requis local-dev | oracle-exact échec |
|  |  | naive / canonical [variant large-table] : bloqué par un contrat |  | 0 | local-dev / requis local-dev | oracle-exact échec |
| D-http-transport | open | wreq / reqwest [scenario http-catalog] : non inférieure | 1.01 [0.975 ; 1.04] (90% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | impit / reqwest [scenario http-catalog] : non inférieure | 1.02 [0.989 ; 1.04] (90% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | spider-http / reqwest [scenario http-catalog] : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | oracle-exact échec |
|  |  | crawlee-http-pooled / reqwest [scenario http-catalog] : option moins bonne | 0.857 [0.834 ; 0.88] (90% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
|  |  | node-fetch / reqwest [scenario http-catalog] : non inférieure | 1 [0.985 ; 1.02] (90% block bootstrap) | 5 sur 5 requis | local-controlled / requis local-controlled | oracle-exact ok |
| D-identity | open | impit-chrome / browser-fetch [variant coherent, profile identified, purpose search] : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | identity-coherence échec, policy-compliance ok, false-success-count ok, session-isolation ok, proxy-continuity ok |
|  |  | impit-chrome / browser-fetch [variant coherent, profile standard, purpose search] : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | identity-coherence échec, policy-compliance ok, false-success-count ok, session-isolation ok, proxy-continuity ok |
|  |  | node-https / browser-fetch [variant coherent, profile identified, purpose search] : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | identity-coherence échec, policy-compliance ok, false-success-count ok, session-isolation ok, proxy-continuity ok |
|  |  | node-https / browser-fetch [variant coherent, profile standard, purpose search] : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | identity-coherence échec, policy-compliance ok, false-success-count ok, session-isolation ok, proxy-continuity ok |
| D-light-engine | open | lightpanda / chrome-headless-shell [scenario http-catalog] : bloqué par un contrat |  | 20 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | lightpanda / chrome-headless-shell [scenario js-catalog] : bloqué par un contrat |  | 20 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | lightpanda / chrome-headless-shell [scenario form] : bloqué par un contrat |  | 20 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | lightpanda / chrome-headless-shell [scenario clicks] : bloqué par un contrat |  | 20 | local-controlled / requis local-controlled | oracle-exact échec |
|  |  | lightpanda / chrome-headless-shell [scenario shadow] : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | oracle-exact échec |
| D-pricing | open | produit : research/2026-10-07-commercial-landscape.md, research/market/README.md | | | | |
| D-scheduling | open | fair-domain / fifo-global : blocs insuffisants | 2.6 [2.31 ; 2.93] (95% block bootstrap) | 5 sur 22 requis | local-controlled / requis local-controlled | admission-no-loss ok, oracle-exact ok |
| D-segment | open | produit : research/2026-10-07-direction-automan.md, research/market/README.md | | | | |
| D-session-model | open | contract / relogin-each-run [variant session-resume, tier http] : option meilleure | 0.5 [0.5 ; 0.5] (95% block bootstrap) | 5 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | contract / relogin-each-run [variant session-resume, tier browser] : option meilleure | 0.5 [0.5 ; 0.5] (95% block bootstrap) | 5 sur 5 requis | local-dev / requis local-dev | oracle-exact ok |
|  |  | status-only / relogin-each-run [variant session-resume, tier http] : bloqué par un contrat |  | 5 | local-dev / requis local-dev | oracle-exact échec |
|  |  | status-only / relogin-each-run [variant session-resume, tier browser] : bloqué par un contrat |  | 5 | local-dev / requis local-dev | oracle-exact échec |
| D-sinks | deferred | parquet-arrow / postgres-copy [batchSize 512] : option moins bonne | 0.718 [0.684 ; 0.757] (90% block bootstrap) | 6 sur 5 requis | local-dev / requis local-controlled (non éligible) | oracle-exact ok |
|  |  | parquet-arrow / postgres-copy [batchSize 32] : blocs insuffisants | 0.189 [0.179 ; 0.202] (90% block bootstrap) | 6 sur 7 requis | local-dev / requis local-controlled (non éligible) | oracle-exact ok |
|  |  | duckdb / postgres-copy [batchSize 512] : blocs insuffisants | 7.16 [6.48 ; 7.78] (90% block bootstrap) | 6 sur 16 requis | local-dev / requis local-controlled (non éligible) | oracle-exact ok |
|  |  | duckdb / postgres-copy [batchSize 32] : blocs insuffisants | 3.76 [2.99 ; 4.47] (90% block bootstrap) | 6 sur 73 requis | local-dev / requis local-controlled (non éligible) | oracle-exact ok |
| D-storage | decided | pas encore calculé | | | requis local-controlled | oracle-exact, recovery-exact |

Un verdict ne sert une décision que si l'hôte observé atteint la classe requise et que toutes les gates de contrat passent.
