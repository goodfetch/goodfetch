# Automan — matrice des capacités agent, plans et réparation

Recherche consolidée au **7 octobre 2026**. Ce document contribue au dossier architecture/design/PRD : il couvre le runtime navigateur, les agents, la compilation de plans, la perception, les outils et la réparation. Il ne fixe aucune limite de taille d’équipe ou de calendrier. Aucun prototype Automan ni benchmark concurrent n’a été exécuté.

**Traçabilité :** réutilisation des enquêtes locales [frameworks](./2026-10-07-browser-frameworks.md), [contrats Stagehand et alternatives](./2026-10-07-stagehand-alternatives-followup.md), [CDP et latence](./2026-10-07-stagehand-cdp-latency.md), [contrats Crawlee/Spider](./2026-10-07-crawlee-spider-contracts-followup.md). Trois nouvelles recherches Exa, huit résultats demandés chacune : `sources_reviewed = 24`, compteur de résultats demandés incluant doublons/écartés, pas de sources indépendantes validées. Lectures ciblées nouvelles : Scrapling parser/storage/licence ; Playwright MCP README/licence ; agent-browser README, MCP Rust, snapshots, cycle de vie des références et sécurité.

**Niveaux de preuve :** `P` = paquet/version publié inspecté ; `D` = documenté par le fournisseur ; `C` = code inspecté, dans cette recherche ou un rapport local cité ; `A` = proposition Automan ; `T` = comportement/performance à démontrer. `C` établit un chemin d’implémentation, pas sa fiabilité universelle. Une case « non établi » signifie que l’enquête n’a pas qualifié cette capacité ; elle ne prouve pas son absence.

Nouvelles références figées : Scrapling [`43dee004866e1c46843a9ac293cc1494aa7915d6`](https://github.com/D4Vinci/Scrapling/tree/43dee004866e1c46843a9ac293cc1494aa7915d6), agent-browser [`98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba`](https://github.com/vercel-labs/agent-browser/tree/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba), Playwright MCP [`f183dad4a52965583e3cc1d59b88cdc279e2e57d`](https://github.com/microsoft/playwright-mcp/tree/f183dad4a52965583e3cc1d59b88cdc279e2e57d). Ces SHA figent les sources consultées ; ils ne donnent pas à chaque fonctionnalité le statut de release stable.

## 1. Décision de conception

**Automan doit posséder un contrat d’exécution commun aux programmes déterministes, aux agents et aux plans compilés.** Le mode d’acquisition HTTP/CDP, le modèle et le framework choisi restent des adapters. Un navigateur, un cache, un score de similarité ou une sortie JSON valide ne doivent pas décider seuls que la tâche métier a réussi.

Les quatre modes doivent coexister et être visibles :

| Mode Automan proposé `A` | Qui décide des étapes ? | LLM pendant un run normal ? | Promesse vérifiable |
| --- | --- | --- | --- |
| Programme déterministe | Code/plan fourni par le développeur | Non | Même logique ; données actuelles lues selon la politique de fraîcheur |
| Agent interactif | Boucle externe ou intégrée, perceptions et outils | Oui, selon provider | Budget, états, tentatives et preuves inspectables ; aucune garantie universelle de succès |
| Compilation assistée | Agent explore/enregistre, puis produit un plan qualifié | Non si tous les nœuds du plan sont déterministes | Économie mesurée sur reruns ; plan versionné et résultats vérifiés |
| Plan hybride / réparation | Plan stable, escalade ciblée sur une étape | Seulement aux points déclarés | Coût d’escalade et effet de chaque reprise connus ; abstention possible |

Le replay et la réparation existent déjà chez Skyvern, Imprint et Browser Use. La matrice doit donc guider la réutilisation et les critères d’acceptation, sans revendiquer une nouveauté fictive.

## 2. Matrice — substrate et outils navigateur

| Capacité granulaire | Stagehand 4.1.0 | Playwright + CLI/MCP | agent-browser actuel | Browser Use |
| --- | --- | --- | --- | --- |
| Exécution déterministe sans inférence | `P/C` Page/Locator ; act(Action) évite l’inférence mais exige encore modèle/gateway configuré | `D/C` locators, actions, scripts ; modèle du client MCP/CLI facultatif | `D/C` commandes Rust ; chat AI facultatif | `D/C` actions/outils et scripts ; Agent historique et beta à distinguer |
| Backend Chromium | `P/C` Understudy CDP dans extension | `C` backend Chromium utilise déjà CDP ; autres backends Firefox/WebKit | `C` daemon/CDP natif Rust | `C` historique Python/CDP ; beta serveur Rust, avec helpers/worker Python |
| Snapshot pour agent | `P` arbre hybride AX/DOM, XPath et URL maps | `D` snapshots AX, profondeur/région/boxes selon outil | `D/C` AX compact, refs, removedRefs | `C` DOM + AX + frames, références, screenshot selon configuration |
| Identité de référence | `P/C` sélecteurs et IDs de frame utilisés par le runtime ; garde métier à ajouter | `D` refs de snapshot et locators ; portée dépend du document | `D/C` refs DOM survivantes conservées, remplacées invalidées, IDs non recyclés ; AX virtuelles limitées au snapshot | `C` état et indices projetés dans la boucle ; validité à qualifier selon backend |
| Frames/Shadow DOM | `D` iframe `>>`, CSS perçant shadow y compris fermé | `D/C` locators/frames ; contrat dépend du navigateur | `C` sessions OOPIF, hit-test ; `D` snapshot iframe avec limites explicites | `C` services par frame ; beta et historique pas présumés identiques |
| Auto-wait/retry d’action | `D/P` pas de contrat Playwright ; résolution ponctuelle et waits explicites | `D` actionability + assertions web-first | `C` contrôles natifs ; parité intégrale Playwright `T` | `C` watchdogs/budgets ; parité Playwright `T` |
| Réseau et scripts | `P/D` evaluate/init scripts ; pas d’interception de routes ni waits réseau équivalents | `D` interception, requêtes, console ; code serveur arbitraire via outil explicitement unsafe | `D` routes/requêtes/HAR/eval/profils MCP | `C/D` outils scripts, événements/watchdogs ; surface selon génération |
| Proximité d’exécution | `P/D` extension et experimentalBatch | `D` script/run-code à côté du pilote ; remote selon déploiement | `C` daemon résident ; batching/outils selon profil | `C` runtime beta résident et protocole SDK |
| Isolation et propriété | `P/D` une Context par browser ; créer plusieurs browsers pour profils isolés | `D` Contexts ; MCP persistent profile exclusif par instance ou isolated | `D` sessions/browser distincts ; pin-tab strict disponible pour CDP partagé | `C/D` BrowserSession, sessions beta durables ; dépend de l’identité configurée |
| MCP/CLI agent-ready | SDK TS/Python/Go `P` ; client/outillage externe à choisir | `D` CLI+skills et MCP typé ; deux usages de contexte différents | `D/C` CLI JSON et MCP stdio, profils/pagination/annotations | `D/C` terminal/harness et wrappers ; cloud SDK distinct |
| Outils fournis par la page | `P` types tools/invoke/result/cancel ; disponibilité réelle `T` | `D` collecte WebMCP et option de désactivation | `D/C` WebMCP expérimental, découverte compacte et schéma à la demande | Non qualifié dans cette enquête |

Sources : [types/bundle stable Stagehand 4.1.0](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz), [migration Playwright](https://docs.stagehand.dev/v4/migrations/playwright), [batch Stagehand](https://docs.stagehand.dev/v4/reference/stagehand), [Chromium Playwright](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/chromium/crBrowser.ts), [actionability](https://playwright.dev/docs/actionability), [Playwright CLI](https://github.com/microsoft/playwright-cli), [README MCP figé](https://github.com/microsoft/playwright-mcp/blob/f183dad4a52965583e3cc1d59b88cdc279e2e57d/README.md), [agent-browser README figé](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/README.md), [snapshot Rust figé](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/cli/src/native/snapshot.rs), [BrowserSession](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/session.py), [ledger beta](https://github.com/browser-use/browser-use/blob/main/BETA_AGENT_INTEGRATION_FEATURES.md).

**Limites de référence à conserver dans le PRD :** les [docs agent-browser figées](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/skill-data/core/references/snapshot-refs.md) indiquent un seul niveau d’iframe développé dans le snapshot et le saut de certains contenus cross-origin. Ses tests d’invalidation d’iframe n’établissent pas une couverture universelle de frames imbriquées. De même, le fait que deux frameworks utilisent CDP ne donne ni compatibilité d’objets ni mêmes attentes : `connectOverCDP` externe possède une [fidélité moindre documentée](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp) que la connexion au serveur Playwright.

## 3. Matrice — perception, planification, validation et replay

| Capacité | Stagehand | Skyvern | Magnitude | Imprint | Scrapling |
| --- | --- | --- | --- | --- | --- |
| Résolution sémantique | `P/C` observe/act via modèle sur arbre ; extract avec screenshot optionnel | `D` prompt et vision ; sélecteur + fallback AI ; AILocator Python | `D/C` vision en coordonnées, tâches de granularité variable | `C/D` interprétation lors de compilation d’enregistrement | `C` similarité structurale sans LLM |
| Agent multi-étape | `P/D` agent() retiré v4 ; boucle/code mode externe | `D/C` agents et workflows multi-étape | `D` act peut couvrir plusieurs interactions | `C` compiler agent ; runtime déterministe | Non établi ; fetch/parser/spider distincts |
| Plan/script compilé réutilisable | `P/C` actions rejouables ; pas un IR métier durable complet | `D/C` code caching par tâche/bloc/branche | `D` cache déterministe natif annoncé en cours ; pas acquis | `C/D` workflow API + playbook DOM + MCP émis | `C` propriétés d’éléments persistées, pas un plan d’effets |
| Résultats typés | `P/C` Zod/Pydantic/Go + data/metadata | `D` extract, validate bool, Task/WorkflowRunResponse | `D` Zod extract ; runner distinct | `C/D` outils MCP et validation générée | `C` Selectors/texte/JSON, contrôles métier appelant |
| Vérification indépendante de l’agent | `C` schéma ; judge completed sans snapshot source, non exposé | `D` validate LLM ; assertions code possibles via Playwright | `D` check LLM ; accès Page/Context pour code | `C/D` verifier indépendant de compilation et reçus | `C` score de ressemblance ; pas de preuve d’identité métier |
| Cache de sortie | `P/C` SDK prépare cacheValue et restitue cacheHit ; stockage/invalidation serveur opaques | Cache principalement scripts `D` | Prompt caching fournisseur `D`, pas sortie garantie | Artifacts/plan/backend ; relit la source sur chemin replay `D/C` | Enregistrement fingerprint puis relocation `C` |
| Réparation déclenchée par | `P/C` exception d’opération ; cache replay échoué → inférence | `D` cached code échoué → agent et régénération | Non qualifié comme workflow repair durable | `C/D` échecs transport escaladent le ladder ; sémantique traitée par compiler/verifier | `C` absence de résultat du sélecteur, si adaptive activé et propriétés sauvées |
| Mauvaise cible encore valide | Pas de garde sémantique acquise `P/C` | À tester avec validations du flow `T` | À tester avec assertions `T` | À tester hors suites générées `T` | Retour direct de la cible sans relocation `C` |
| Freshness métier au rerun | Politique serveur cache opaque `T` | Script live, invariants à définir `D/T` | Observations actuelles ; invariants à définir `D/T` | Requêtes actuelles ; reçus et invariants à définir `D/C/T` | Nouvelle réponse parsée ; empreinte cible ancienne, données lues actuelles `C` |

Sources : [Stagehand contrats inspectés](./2026-10-07-stagehand-alternatives-followup.md), [Skyvern actions](https://www.skyvern.com/docs/developers/browser-automations/actions-reference), [Skyvern code caching](https://www.skyvern.com/docs/developers/features/code-caching), [Magnitude BrowserAgent](https://docs.magnitude.run/reference/browser-agent.md), [Magnitude checks](https://docs.magnitude.run/reference/test-case-agent.md), [Magnitude mémoire/prompt cache](https://docs.magnitude.run/advanced/memory.md), [Imprint architecture](https://github.com/ashaychangwani/imprint/blob/main/docs/architecture.md), [verifier Imprint](https://github.com/ashaychangwani/imprint/blob/main/src/imprint/live-verifier.ts), [Scrapling adaptive docs](https://scrapling.readthedocs.io/en/latest/parsing/adaptive/), [parser figé](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/parser.py).

Deux références supplémentaires évitent de sous-estimer le marché : **Browser Use Cloud V4** conserve des scripts en workspace, relit les sources et propose réparation ; chaque rerun redémarre cependant un modèle et consomme des tokens. **AgentQL** rend des Playwright Locators après requête sémantique, ou des données après queryData ; le programme appelant garde le contrôle. [Browser Use scripts](https://docs.browser-use.com/cloud/agent/scripts), [AgentQL Page](https://docs.agentql.com/javascript-sdk/api-references/agentql-page).

**Spider agent doit aussi apparaître comme référence réelle.** Dans Spider/spider_agent 2.53.9, le loop Rust publié possède actions navigateur, outils, retours d’erreur, limites de rounds et détection de répétitions. Son success dépend principalement de plan.done du modèle. L’audit local ne démontre pas l’activation runtime des options planning_mode, confidence_strategy, html_diff_mode et self_healing : exposer une option n’établit pas son effet. [Boucle publiée](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_agent/src/automation/browser.rs), [configuration publiée](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_agent/src/automation/config.rs), [audit local](./2026-10-07-crawlee-spider-contracts-followup.md).

## 4. Scrapling : mécanisme utile, contrat plus étroit que « self-healing »

Le parser figé `C` calcule une moyenne de ressemblances : tag, texte, attributs et leurs valeurs, class/id/href/src, chemin de tags, parent et siblings. Il utilise notamment SequenceMatcher ; ce score **n’est pas une probabilité calibrée**. `relocate` parcourt tous les éléments, prend le score maximal, retourne tous les ex aequo dépassant le seuil ; le défaut est **40 %**. `xpath`/`css` retournent immédiatement les résultats d’un sélecteur qui fonctionne. L’adaptation ne commence que sur sélection vide. `auto_save` conserve le premier élément, et peut aussi remplacer l’empreinte après relocation. [Parser figé](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/parser.py).

Le stockage SQLite utilise domaine de base + identifier et `INSERT OR REPLACE`; sans URL valide, la partition peut devenir default. Il a verrou et WAL, mais cette clé n’isole pas intrinsèquement tenant, compte ou route métier. Reprendre le stockage tel quel pour une plateforme multi-tenant serait donc insuffisant. [Storage figé](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/scrapling/core/storage.py).

**Réutilisation Automan `A` :** un provider de candidats structurels économique, limité au sous-arbre pertinent, avec score brut, liste d’ex aequo et explication des caractéristiques. Ajouter invariants d’identité, marge entre premier/deuxième candidat, abstention et validation métier avant effet. Ne jamais promouvoir automatiquement la nouvelle empreinte sur la seule ressemblance. Le seuil sera calibré sur un corpus tenu hors développement `T`, pas copié comme une confiance de 40 %. Ce mécanisme peut réparer une extraction sans payer de LLM ; il ne répare pas à lui seul auth, navigation, pagination, mutation ou reprise de workflow.

## 5. Licence, dépendance cloud et choix de réutilisation

| Référence | Licence du code consulté | Frontière utile | Choix proposé `A` |
| --- | --- | --- | --- |
| Stagehand | [MIT](https://github.com/browserbase/stagehand/blob/main/LICENSE) | SDK/runtime publics ; Browserbase cache/gateway restent services | Adapter de compétences versionné ; pas dépendance cloud obligatoire du cœur |
| Browser Use / terminal | [MIT Python](https://github.com/browser-use/browser-use/blob/main/LICENSE), [MIT terminal](https://github.com/browser-use/terminal/blob/main/LICENSE) | Historique Python, beta Rust, cloud V4 différents | Étudier protocole/runtime/providers et bridge ; éviter reprise du terminal entier sans audit de couplage |
| agent-browser | [Apache-2.0](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/LICENSE) | Daemon Rust, MCP et modules natifs ; plugins hors processus | Candidat d’adapter et source de mécanismes refs/frames/pinning ; encapsuler une version |
| Playwright, CLI/MCP | [Apache-2.0 MCP](https://github.com/microsoft/playwright-mcp/blob/f183dad4a52965583e3cc1d59b88cdc279e2e57d/LICENSE), [Apache-2.0 CLI](https://github.com/microsoft/playwright-cli/blob/main/LICENSE) | Pilote/tests/outils publics ; hébergeurs distincts | Worker compatible et baseline de contrat ; CLI/MCP clients du cœur, pas deux moteurs Automan |
| Skyvern | [AGPL-3.0](https://github.com/Skyvern-AI/skyvern/blob/main/LICENSE) | Backend public ; mesures cloud anti-bot distinctes | Référence produit/bench ; réutilisation AGPL à décider explicitement |
| Magnitude | [Apache-2.0](https://github.com/magnitudedev/browser-agent/blob/main/LICENSE) | Agent/runner TS, Playwright accessible, modèle BYOK | Adapter vision/checks ; conserver nature probabiliste et coûts |
| Imprint | [MIT](https://github.com/ashaychangwani/imprint/blob/main/LICENSE) | Compiler, API/DOM ladder, verifier et outils publics | Étudier capture, paramètres de chaîne réseau et reçus ; qualifier hors exemples |
| Scrapling | [BSD-3-Clause](https://github.com/D4Vinci/Scrapling/blob/43dee004866e1c46843a9ac293cc1494aa7915d6/LICENSE) | Parser/fetchers/spider publics ; scores structurels sans modèle | Adapter Python ou provider de similarité au contrat Rust ; stockage propre |
| AgentQL | [MIT du dépôt public](https://github.com/tinyfish-io/agentql/blob/main/LICENSE) | Exemples/clients ; moteur API d’inférence non démontré ouvert | Provider optionnel ; ne pas confondre local browser et inférence locale |
| Spider agent | [MIT manifeste publié](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_agent/Cargo.toml) | Core Rust, providers et features compilées | Candidat bibliothèque ; exiger preuve de consommation de chaque option |

Ces licences sont des propriétés des sources consultées, pas des services complets. Une extraction de module doit préserver notices et obligations applicables. Les choix sont des propositions, pas un résultat d’audit complet de dépendances.

## 6. Contrats Automan à construire

Les types suivants décrivent un contrat cible `A`, pas une API publiée ni une implémentation acquise. Le schéma commun doit générer modèles SDK et contrats CLI/MCP ; les champs utiles restent idiomatiques par langage.

| Objet | Champs/garanties minimaux proposés | Pourquoi le posséder |
| --- | --- | --- |
| `CapabilitySet` | backend/version/protocol ; locators, frames, shadow, network, scripts, profiles, cancellation et restrictions | Refuser un plan incompatible avant effet ; aucune parité silencieusement simulée |
| `SessionLease` | sessionId, account/tenantRef, backend, browser/context/page/target, ownership, lease/version | Éviter les pages actives globales, collisions d’identité et fermetures par un autre client |
| `Observation` | observationId, capturedAt, URL/origin, frame/document epochs, modalités présentes, artefacts/redactions, scope | Rendre coût, provenance et validité des références explicites |
| `TargetIntent` | rôle/nom/contexte, identifiant métier, stratégie/alternatives autorisées | Garder l’intention au-delà du sélecteur |
| `ResolvedTarget` | ref backend + document/frame ; candidats/scores bruts ; source de décision ; guards et expiration | Distinguer ressemblance, identité et capacité à agir |
| `ActionRequest` | actionId, sessionLease/version, cible, arguments/secrets refs, budget, effet déclaré, préconditions | Toute mutation passe par la même politique, agent ou programme |
| `ActionResult` | exécuté/échoué/indéterminé, evidence, errorClass, tentatives, coût ; statut de l’effet distinct | Exception après clic ≠ clic non effectué |
| `VerificationResult` | règle/version, valeur observée, expected, evidence refs, verdict/unknown ; deterministic/probabilistic | Distinguer type valide, cible correcte, effet confirmé et vérité de la donnée |
| `RunResult` | verified/partial/failed/needs_review/effect_unknown ; outputs + provenance + budget/cost | Interdire success métier déduit de la seule absence d’exception ou d’un done LLM |

Un hash source prouve l’identité d’un artefact observé, pas sa vérité. Un timestamp de run ne prouve pas une requête réseau fraîche : conserver observation/request time, source et politique cache effectivement appliquée.

### Plan IR et compilation assistée

Le plan versionné doit porter inputs/output schemas, required capabilities, session/identity contract, budgets, policy, invariants et provenance de compilation. Nœuds typés : HTTP, Navigate, Observe, Resolve, Action, Extract, Assert, Wait, Branch, Loop borné, Subplan, Checkpoint, ToolCall. Une extension Code est isolée et annonce ses permissions/effets ; elle ne contourne pas le journal.

Un plan est déclaré **déterministe** uniquement si chaque chemin autorisé peut s’exécuter sans résolution modèle. Un fallback modèle explicite le rend hybride. Le compilateur ne copie pas une réponse du run d’apprentissage dans la sortie ; il produit des opérations qui relisent les sources. Les secrets deviennent des références injectées au dernier moment, pas des valeurs enregistrées dans les prompts ou le plan.

Pipeline proposé : **intent/recording → exploration budgétée → candidat IR → vérification statique → fixtures → validation live autorisée → version immutable → activation**. L’exploration conserve les alternatives rejetées et le contexte source utile ; la recette activée contient les branches explicites, les gardes et leurs limites. Les critères d’acceptation doivent être établis indépendamment de la sortie désirée par le même agent : fixture métier, réponse source, invariants ou checks développeur. Un verifier séparé qui relit les mêmes hallucinations n’est pas indépendant au sens de la vérité.

Les workflows à effets utilisent simulation/fixtures ou une autorisation couvrant leur validation live. La compilation ne crée pas par elle-même une autorisation supplémentaire. Ce principe gouverne les mutations réelles pendant apprentissage autant que pendant replay.

### Validation et réparation bornée

La réparation doit produire une **nouvelle version candidate**, avec diff, motif, source et tests ; elle ne modifie pas en place le dernier plan connu bon. Pipeline : diagnostiquer la propriété en échec → classer l’effet précédent → observer → proposer patch local → vérifier préconditions/invariants → rejouer le segment admissible → comparer sorties/effets → promouvoir ou abstention.

| Échec | Réponse admissible proposée | Réponse à exclure du contrat |
| --- | --- | --- |
| Cible disparue | Nouveau candidat structurel/AX/LLM, guards, puis action | Premier match au score maximal sans contrôle d’identité |
| Sélecteur marche, mauvaise entité | Refuser via guard métier ; réparer intention/résolution | Attendre une exception pour réparer |
| Donnée modifiée normalement | Lire la valeur actuelle et vérifier schéma/fraîcheur | Traiter chaque changement métier comme une rupture de plan |
| Extraction incomplète | Relecture/source/rendu/scoping selon diagnostics, validation | Sauver le résultat comme succès parce que JSON valide |
| Timeout après mutation possible | Vérification de l’effet puis reprise autorisée ou effect_unknown | Réexécuter aveuglément l’étape ou changer de backend pour effacer l’incertitude |
| Auth expirée | Transition explicite de session/auth, revalidation de l’identité | Rotation de compte implicite pour poursuivre |
| Backend incompatible | Autre session/backend annoncé et transfert supporté | Deux pilotes écrivant simultanément dans la même page |

Les budgets doivent couvrir toutes les couches : coût modèle, captures, transport, exécution, réparation et intervention. Une déclaration read-only d’un outil/page n’est pas une preuve suffisante d’absence d’effet. La politique de l’utilisateur peut autoriser activation automatique sur des lectures qualifiées ; les cas ambigus restent needs_review/effect_unknown. Aucune promesse exactly-once sur les effets d’un site tiers sans mécanisme d’idempotence/transaction fourni par ce site.

### Caches séparés, invalidation explicable

| Cache proposé | Valeur | Politique nécessaire |
| --- | --- | --- |
| Plan/compilation | IR et versions/tests | inputs/output schema, policy et capabilities compatibles |
| Résolution | locator/fingerprint/target intent | session/account/route/document scope, guards, abstention ; score ≠ oracle |
| Résultat | Valeur extraite | FreshnessPolicy explicite, source/version, âge, invalidation et décision HIT visible |
| HTTP/source | Réponse et validation transport | ETag/last-modified/cache rules ; live contract selon demande |
| Modèle/provider | Préfixe/tokens cache | Coûts de read/write et version modèle ; ne pas présenter comme zéro inférence |
| Session/backend | Profil, état et chemin d’accès gagnant | Tenant/account/origin, expiration, état d’auth et contraintes backend |

Le cache local de plans fait partie du produit OSS proposé. Les résultats live doivent relire une source ; servir une ancienne valeur exige une politique explicitement autorisée. Stagehand prouve pourquoi cette séparation compte : extract cache une valeur, alors que Skyvern cache surtout le code et Magnitude les préfixes de prompt.

## 7. Native CDP, extension et Playwright : adapter plutôt que conflit de propriétaires

| Backend | Meilleur usage proposé | Garantie à qualifier `T` |
| --- | --- | --- |
| HTTP Rust | APIs/HTML accessibles, parsing/extraction, crawl | Headers/auth/pagination/fraîcheur et équivalence de données ; rendu non nécessaire |
| CDP Rust natif | Moteur portable contrôlé, cloud worker proche de Chrome, scripts/actions composés | Actionability, hit-testing, OOPIF, refs, navigation, dialogs/downloads, pertes de connexion/cancel |
| Extension | Navigateur utilisateur et sessions existantes, permissions et exécution proches de la page | Compatibilité versions, permissions/contextes, cycle de service worker, connexion perdue et propriété de tab |
| Worker Playwright | Flows utilisant ses locators/attentes/réseau ; intégrations et baseline indépendante | Sérialisation, forwarding/cancellation, coûts ; capabilities selon navigateur et mode de connexion |

**Un propriétaire de session et de cycle de vie.** Le runtime accorde un lease d’exécution ; le browser worker choisi possède connexion, événements et mutation du navigateur. Un agent, SDK ou serveur MCP consomme ce contrat. Les pages d’un même compte peuvent être concurrentes si le plan l’autorise, mais les transitions dépendantes restent ordonnées. Un nouveau client ne prend pas « l’onglet actif » d’un autre.

Le [pin-tab agent-browser](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/README.md) fournit une référence concrète : fermeture de tab → tab_gone au lieu de fallback vers la voisine. Le [MCP Playwright](https://github.com/microsoft/playwright-mcp/blob/f183dad4a52965583e3cc1d59b88cdc279e2e57d/README.md) rappelle qu’un profil persistant ne peut être ouvert par plusieurs instances en parallèle. Automan doit rendre ces règles interclients natives, avec attach owned/borrowed et close qui respecte la propriété. Reprise après crash revalide URL, document, auth et checkpoints ; elle ne restaure pas des refs DOM comme si elles étaient encore vivantes.

La frontier Crawlee/Spider est séparée du moteur d’étapes. Un framework externe peut posséder sa queue ; Automan exécute alors le plan sous la requête assignée. Ne pas doubler la même requête sous deux schedulers durables. Le [bridge Crawlee publié](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json) reste Stagehand v3 : une compatibilité v4 est un travail à réaliser/qualifier, pas acquise.

## 8. Surface agent-ready : même autorité via SDK, CLI et MCP

Proposition `A` : profil core compact — session, observe, resolve, execute, extract, plan.run — puis découverte de profils compile/repair/network/debug/crawl. Les noms et schémas exacts restent à stabiliser. Pagination, artefacts hors contexte, profondeur/scope de snapshot et lecture partielle préviennent une copie permanente du DOM dans chaque réponse. Les résultats gardent IDs, état, coût, vérification et refs d’artefacts ; le texte sert de résumé, pas de seul contrat.

agent-browser a déjà ce type de profils MCP, champs typés, pagination et annotations readOnly/openWorld `D/C`. Son [implémentation figée](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/cli/src/mcp.rs) est une référence réutilisable pour l’ergonomie ; le runtime Automan doit cependant appeler son protocole canonique plutôt qu’exposer des tableaux de commandes arbitraires comme unique API. Des extraArgs de CLI ne sont pas un modèle de permissions stable.

**MCP et WebMCP sont distincts.** MCP expose nos outils au client agent. WebMCP expose potentiellement des outils déclarés par une page : découverte, catalogue borné, schéma/version/origin/frame, invocation et état/result/cancel. Ces déclarations sont des données de la page, pas des autorisations. Les annotations de sécurité/read-only ne doivent pas outrepasser policy et identité. Le [WebMCP agent-browser figé](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/README.md) décrit déjà découverte compacte et schéma à la demande ; disponibilité dépend du moteur et reste expérimentale.

Le [Playwright MCP actuel](https://playwright.dev/docs/getting-started-mcp) nomme son outil de scripts browser_run_code_unsafe et décrit son exécution dans le serveur comme équivalente à une exécution de code arbitraire. Automan doit distinguer évaluation DOM et code hôte, annoncer la capacité demandée et isoler code/plugins avec les permissions accordées. Les [contrôles agent-browser](https://github.com/vercel-labs/agent-browser/blob/98e8c79e2a1a8a8e7a88f22e4dbe89bfac232eba/docs/content/docs/security.mdx) montrent aussi des scopes limités : features opt-in, filtres de domaine incompatibles avec plusieurs modes d’attach/profile, plugins hors processus pas automatiquement sandboxés. Une liste de domaines au niveau navigateur n’est pas une frontière OS universelle.

## 9. Critères d’acceptation et lacunes à construire

Les exigences suivantes sont proposées, pas des garanties acquises :

- **CAP-01 — parité annoncée :** une même recette passe les tests de contrat sur tous les backends déclarés compatibles ; un backend incapable refuse avant effet.
- **OBS-01 — validité :** refs invalidées après remplacement de document/frame/élément, contexte et origin conservés ; captures bornées avec scope explicite.
- **ID-01 — isolation :** deux comptes/tenants ne partagent ni cookies, credentials, plan cache scoped ni mutation de tab ; attach/close respectent ownership.
- **ACT-01 — effets :** timeout avant/après effet différencié ; aucune reprise automatique non couverte ; effect_unknown survit au crash.
- **PLAN-01 — compilation :** un plan qualifié déterministe exécute toutes ses branches prévues sans LLM ; valeurs de sortie relues, jamais apprises par copie.
- **VER-01 — résultat :** faux succès, échec détecté et abstention mesurés séparément ; validators versionnés et preuves disponibles.
- **CACHE-01 — fraîcheur :** changement de données sous même URL/layout ne sert pas une ancienne valeur lorsque la politique exige live.
- **REPAIR-01 — promotion :** patch immutable, validation de cible/effet, corpus nominal + régression ; ambiguïté conduit à abstention.
- **TOOL-01 — autorité commune :** SDK/CLI/MCP suivent les mêmes budgets, annulations et policies ; page/plugin ne peut augmenter ses permissions.

Corpus indispensable `T` : cible incorrecte encore cliquable, lignes réordonnées, changement de données sans changement de layout, re-render entre wait/click, frames imbriquées/OOPIF, shadow ouvert/fermé selon backend, auth expirée, tab fermé, CDP perdu et timeout après effet. Comparer programme optimisé, agent libre, plan compilé et repair, avec première compilation et reruns froid/chaud séparés. Publier coûts complets, p50/p95, appels/tokens, intervention humaine, effets et sorties correctes. Un avantage de langage ou une option de configuration ne remplace aucun de ces résultats.

**Synthèse de réutilisation :** contrats et ergonomie Stagehand ; locators/attentes et baseline Playwright ; refs/pinning/MCP natifs agent-browser ; runtime/protocole et boucle Browser Use ; code par blocs Skyvern ; vision/checks Magnitude ; capture API/DOM et verifier Imprint ; candidats structurels Scrapling ; crawl/adaptation et agent Rust Spider. Le travail propre d’Automan est leur articulation sous un IR portable, une identité exclusive, des effets connus et une preuve de résultat qualifiée. Cette combinaison reste à implémenter et à démontrer.
