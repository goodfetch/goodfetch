# Décisions du lab

Rapport généré par `pnpm lab report` depuis `decisions/` et `verdicts/`. Les chiffres viennent des verdicts calculés ; aucune valeur n'est saisie à la main.

| Décision | Statut | Verdict | Estimation option / baseline | Blocs | Hôte | Gates |
| --- | --- | --- | --- | --- | --- | --- |
| D-agent-interface | open | pas encore calculé | | | requis local-controlled | oracle-exact |
| D-ai-layer | open | pas encore calculé | | | requis local-controlled | false-success-count, oracle-exact |
| D-browser-driver | open | rust-cdp-guarded / playwright-native : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | node-cdp-guarded / playwright-native : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | actionability-conformance ok, false-success-count ok, oracle-exact échec |
|  |  | puppeteer-guarded / playwright-native : bloqué par un contrat |  | 0 | local-dev / requis local-controlled (non éligible) | actionability-conformance ok, false-success-count ok, oracle-exact échec |
| D-cloud-topology | deferred | pas encore calculé | | | requis reference | session-isolation, oracle-exact |
| D-core-language | open | rust-tokio / node : blocs insuffisants | 0.511 [0.49 ; 0.536] (95% block bootstrap) | 3 sur 5 requis | local-dev / requis local-controlled (non éligible) | oracle-exact ok, recovery-exact ok |
| D-durable-runtime | open | pas encore calculé | | | requis local-controlled | admission-no-loss, recovery-exact, oracle-exact |
| D-extraction | open | pas encore calculé | | | requis local-controlled | oracle-exact |
| D-feature-priority | open | produit : research/market/README.md | | | | |
| D-http-transport | open | pas encore calculé | | | requis local-controlled | oracle-exact, policy-compliance |
| D-identity | open | pas encore calculé | | | requis local-controlled | identity-coherence, policy-compliance, false-success-count, session-isolation |
| D-light-engine | open | pas encore calculé | | | requis local-controlled | oracle-exact, false-success-count |
| D-pricing | open | produit : research/2026-10-07-commercial-landscape.md, research/market/README.md | | | | |
| D-segment | open | produit : research/2026-10-07-direction-automan.md, research/market/README.md | | | | |
| D-sinks | deferred | pas encore calculé | | | requis local-controlled | oracle-exact, recovery-exact |
| D-storage | decided | pas encore calculé | | | requis local-controlled | oracle-exact, recovery-exact |

Un verdict ne sert une décision que si l'hôte observé atteint la classe requise et que toutes les gates de contrat passent.
