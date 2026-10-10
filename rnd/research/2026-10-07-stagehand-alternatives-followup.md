# Stagehand 4.1.0 et alternatives — contrats, efficacité et fiabilité

État étudié au **7 octobre 2026**. Complément au panorama initial ; cette note ne reprend pas sa contrainte de calendrier, retirée par l’utilisateur. Automan vise une architecture ambitieuse, portable, avec un moteur canonique Rust, des SDK TypeScript/Python et une extension Go possible via le même protocole. Aucun benchmark, déploiement cloud ou workflow sur site réel n’a été exécuté pour cette note.

**Méthode :** quatre nouvelles recherches Exa, huit résultats demandés chacune, soit `sources_reviewed = 32`. Ce compteur comprend doublons et résultats écartés ; il ne mesure pas 32 sources indépendantes validées. Les documents effectivement utilisés sont cités ci-dessous. Lecture directe du tarball npm **Stagehand 4.1.0**, de ses déclarations TypeScript, du SDK et de l’extension distribués ; lectures de documentation officielle et du code public ; sept licences concurrentes ouvertes. Les conclusions sur le paquet stable sont séparées des promesses de la documentation courante et du comportement des serveurs non publics.

## 1. La bonne opposition n’est pas « Stagehand impose l’IA, Crawlee permet le code »

Stagehand possède trois surfaces différentes : un pilote Chromium déterministe, des primitives de perception/résolution assistées par modèle et des mécanismes de cache/réparation. Un appel `page.locator("#submit").click()` ne demande aucune résolution à un LLM. `observe()` demande des candidats ; `act(Action)` peut rejouer un candidat déterministement ; `act("instruction")` demande au modèle de choisir une action ; `extract()` demande une sortie structurée. Dire que Stagehand ne propose pas de sélecteurs classiques est donc faux. [API Page](https://docs.stagehand.dev/v4/reference/page), [API Locator](https://docs.stagehand.dev/v4/reference/locator), [act](https://docs.stagehand.dev/v4/basics/act), [observe](https://docs.stagehand.dev/v4/basics/observe).

Crawlee intervient principalement sur une autre dimension : file de requêtes, dispatch, retries, sessions, concurrence et choix HTTP/navigateur. Ces mécanismes ne remplacent pas à eux seuls une résolution sémantique ou une preuve de résultat métier. À travail identique, lire une API ou parser des données avec du code connu évite l’inférence ; demander à un modèle de comprendre une page peut réduire le coût de développement et de maintenance. Il faut comparer le coût complet d’une sortie correcte, pas seulement le langage du moteur ni la présence du mot IA. [Fonctionnalités Crawlee](https://crawlee.dev/js/docs/introduction).

Une intégration Crawlee–Stagehand v4 n’est pas le vieux scénario « passer la Playwright Page à Stagehand » : v4 n’accepte pas cet objet. Il faut un adaptateur de navigateur/session et une règle claire de propriété, ou laisser l’un des runtimes piloter le navigateur. La portabilité CDP ne donne pas automatiquement la compatibilité des objets, événements et contextes. [Migration Playwright](https://docs.stagehand.dev/v4/migrations/playwright).

## 2. Stagehand 4.1.0 : ce que renvoient réellement les primitives

La version stable utilisée ici est celle du [registre npm 4.1.0](https://registry.npmjs.org/@browserbasehq/stagehand/4.1.0), publiée le 9 septembre. Le [tarball exact](https://registry.npmjs.org/@browserbasehq/stagehand/-/stagehand-4.1.0.tgz) contient notamment `dist/index.d.mts`, `dist/index.mjs` et `dist/extension/service-worker.js`. Ces fichiers distribués ont été ouverts en mémoire, sans installation. Les noms de fonctions cités ci-dessous proviennent du bundle stable ; les liens GitHub servent à retrouver les modules lisibles et ne prétendent pas figer `main` à la version npm.

| Primitive stable | Entrée et objet retourné | Travail effectivement nécessaire | Ce que le succès ne démontre pas |
| --- | --- | --- | --- |
| `page.snapshot()` | `{ formattedTree, xpathMap, urlMap }` | Capture structurée sans LLM ; mapping des éléments/URL | Complétude de toutes les informations métier et état ultérieur du DOM |
| `page.locator(selector).click/fill/innerText/...` | Actions : `void` ; lectures : valeur ; `count()` : nombre | Sélecteur CSS/XPath/`text=`, résolution et opération navigateur | Justesse sémantique du sélecteur et résultat métier |
| `observe(instruction?, options?)` | `{ data: Action[], metadata }` ; `Action` contient `selector`, `description`, `method?`, `arguments?` | Snapshot puis génération structurée, ou liste servie par cache | Choix du bon candidat, exécution ou objectif atteint |
| `act(instruction: string)` | `{ data: { success, message, actionDescription, actions }, metadata }` | Attente de stabilisation DOM/réseau, snapshot, résolution modèle, exécution ; possibilité de deuxième étape | Que la bonne entité a été modifiée ou qu’un processus complet est terminé |
| `act(action: Action)` | Même `ActResult` | Rejeu direct ; pas de snapshot/inférence initiale ; réparation possible si activée | Que l’ancienne cible représente encore la même entité |
| `extract(instruction, schema)` | `{ data: valeur typée, metadata }` | Snapshot et génération structurée ; dans stable 4.1.0, deuxième appel modèle pour la complétude ; ou sortie mise en cache | Vérité, fraîcheur, exhaustivité ou provenance de chaque champ |

Les signatures et champs ont été vérifiés dans le tarball. La documentation décrit les objets [act](https://docs.stagehand.dev/v4/basics/act), [observe](https://docs.stagehand.dev/v4/basics/observe) et [extract](https://docs.stagehand.dev/v4/basics/extract). Le SDK TS applique aussi `resolvedSchema.parse(response.data)` : la conformité au schéma est un contrôle concret, mais un prix ancien correctement typé reste ancien.

**Deux nuances importantes :**

1. `act(Action)` n’utilise pas le modèle sur son chemin normal, mais **le contrôleur stable vérifie quand même qu’un modèle ou un gateway est configuré avant d’entrer dans ce chemin**. Un navigateur local sans modèle peut employer `page`/`locator`, mais ne doit pas présumer que `stagehand.act(action)` acceptera la même configuration. « Pas d’inférence » et « aucune configuration LLM requise » sont des contrats distincts. Lecture de `createStagehandController → act$2 → act` dans le tarball ; module correspondant [stagehandController.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/controllers/stagehandController.ts).
2. Dans stable 4.1.0, la fonction d’inférence `extract$1` appelle successivement `generateStructured("Extraction", ...)` et `generateStructured("Metadata", ...)`. Le second reçoit l’instruction et la sortie extraite, **pas le snapshot source**. Le service lit `completed`, le journalise, mais ne l’expose pas dans `ExtractResult` ; `cacheValue` reste la sortie même quand `completed` est faux. C’est le comportement du client/runtime public ; le serveur peut encore refuser une écriture selon ses propres règles. Cela ne permet pas d’affirmer qu’une extraction incomplète est systématiquement servie par le cloud. Modules [inférence extract](https://github.com/browserbase/stagehand/blob/main/packages/extension/inference.ts), [extractService.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/services/extractService.ts), tarball stable ci-dessus.

### Exemple exact de surface hybride, avec contrôle explicite

Exemple illustratif sur un catalogue de test : les sélecteurs et les valeurs doivent correspondre à la fixture. Il montre les retours de v4, pas un benchmark exécuté. Le modèle est configuré parce que `observe`/`extract` en ont besoin ; `locator.fill` reste déterministe.

```ts
import { localBrowser, Stagehand } from "@browserbasehq/stagehand";
import { z } from "zod/v4";

const browser = await localBrowser.launch({ headless: true });
const stagehand = await Stagehand.create({
  browser,
  model: { modelName: "openai/gpt-4.1", apiKey: process.env.OPENAI_API_KEY },
  cache: false,
  selfHeal: false,
});

try {
  const page = await browser.context.newPage("http://localhost:3000/catalogue");
  if (!(await page.waitForSelector("#search", { state: "visible" }))) {
    throw new Error("Champ de recherche absent");
  }
  await page.locator("#search").fill("SKU-42");

  const observation = await stagehand.observe(
    "Find the search submit button", { page, cache: false },
  );
  const candidates = observation.data.filter(a => a.method === "click");
  if (candidates.length !== 1) throw new Error("Cible ambiguë");

  const execution = await stagehand.act(candidates[0], { page });
  if (!execution.data.success) throw new Error(execution.data.message);
  if (!(await page.waitForSelector('[data-sku="SKU-42"]', { state: "visible" }))) {
    throw new Error("Résultat attendu absent");
  }

  const extraction = await stagehand.extract(
    "Extract SKU, name and price of the product with SKU-42",
    z.object({ sku: z.string(), name: z.string(), price: z.number() }),
    { page, cache: false, locator: page.locator('[data-sku="SKU-42"]') },
  );
  if (extraction.data.sku !== "SKU-42") throw new Error("Mauvais produit");
  console.log(extraction.data, extraction.metadata.usage);
} finally {
  await stagehand.close();
  await browser.close();
}
```

Le contrôle du SKU ne prouve pas la valeur du prix ; une comparaison déterministe avec le texte source, ou une réponse réseau autoritative, serait nécessaire pour cela. Ce point doit apparaître dans les exemples d’Automan : chaque vérification doit préciser la propriété qu’elle établit.

### Le pilote v4 n’hérite pas des garanties Playwright

La [migration officielle](https://docs.stagehand.dev/v4/migrations/playwright) dit explicitement que v4 n’a pas d’auto-waiting Playwright : un sélecteur est résolu une fois. `waitForSelector()` retourne un booléen ; une attente préalable ne remplace pas un retry par action lors d’un re-render. `goto()` attend `domcontentloaded` par défaut ; Playwright attend `load`. Pas de `getBy*`, de chaînage/filter général, de `waitForResponse/Request/Event/URL`, ni de web-first assertions. Les événements exposés par `page.on()` sont limités à `console` dans les types stables. Il existe une seule BrowserContext par navigateur. Le pilote est Chromium uniquement.

En revanche, Stagehand documente le passage d’iframes avec `>>` et le piercing CSS des Shadow DOM, y compris fermés. Ces capacités doivent être distinguées des attentes/retries : davantage de cibles accessibles ne rend pas une action temporellement fiable. Playwright documente pour `click` les contrôles de visibilité, stabilité, réception des événements et état enabled, et propose des assertions auto-réessayées. Même ces contrôles vérifient l’actionnabilité, pas un résultat métier. [Actionability Playwright](https://playwright.dev/docs/actionability), [Locators Stagehand](https://docs.stagehand.dev/v4/reference/locator).

## 3. Cache, réparation, batching : trois optimisations différentes

### Trois valeurs mises en cache

La lecture de `withCache`, `replayCachedActions`, `observe` et `extract` dans stable 4.1.0 distingue :

| Méthode | Valeur cache | Traitement d’un HIT | Risque à tester |
| --- | --- | --- | --- |
| `act(string)` | Tableau d’actions | Rejeu contre le navigateur actuel | Sélecteur toujours valide mais mauvaise entité ; effet avant une erreur tardive |
| `observe` | Tableau d’actions candidates | Retour de la liste sans nouvelle résolution LLM | Mapping devenu sémantiquement faux ou résultat de découverte dépassé |
| `extract` | **Sortie structurée** | Retour direct de la sortie sans nouvelle extraction | Résultat ancien si la clé/correspondance ne distingue pas la modification pertinente |

Le cache ne repose pas uniquement sur l’instruction : le runtime collecte les arbres AX CDP par frame et transmet URL, contenu d’arbre, instruction/options et schéma pour extract. La documentation affirme qu’un changement de contenu/structure entraîne un miss ; le mécanisme exact de canonicalisation/correspondance, les TTL et la politique de fraîcheur du service ne sont pas établis par le code public lu. On ne doit donc ni dire que chaque HIT retourne une valeur périmée, ni vendre chaque HIT comme une preuve de fraîcheur. La configuration de modèle est **délibérément exclue de la clé** : changer de modèle ne force pas une nouvelle résolution. [Cache officiel](https://docs.stagehand.dev/v4/best-practices/caching), [cacheService.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/services/cacheService.ts).

Le cache standard requiert **clé + session Browserbase** ; il n’équivaut pas à un cache OSS portable local. Les appels avec `locator` ou `ignoreLocators` le contournent ; `extract({ screenshot: true })` le contourne également, les pixels n’étant pas représentés par sa clé. Réduire le contexte peut économiser des tokens tout en empêchant les hits : mesurer cette combinaison, ne pas additionner aveuglément les gains. Les variables sont soustraites au prompt via placeholders, mais leurs valeurs figurent dans la requête cache ; la documentation conseille de désactiver le cache pour les credentials. [act](https://docs.stagehand.dev/v4/basics/act), [extract](https://docs.stagehand.dev/v4/basics/extract).

**Divergence documentaire vérifiée :** les docs courantes annoncent le cache activé par défaut. Le paquet stable 4.1.0 a `cache` optionnel dans le schéma, le SDK n’injecte pas de valeur par défaut, les métadonnées Browserbase injectent apiKey/session/region, et `buildCacheContext` utilise `initParams.cache ?? false`. `withCache` désactive alors la lookup quand l’option est absente. Cette lecture statique du chemin distribué justifie de **configurer `cache: true` explicitement et de mesurer `metadata.cache.status`** ; elle ne démontre pas le comportement d’une version cloud différente. Docs et paquet doivent être alignés avant d’utiliser le gain implicite dans un calcul financier.

### Self-heal n’est pas un oracle de la bonne cible

`takeDeterministicAction` répare après une exception de l’opération et si `selfHeal` est activé : recapture, nouveau choix, nouvel essai. Une action sans méthode reconnue retourne `success: false`. Un timeout est propagé. Le chemin de cache rejoue avec `selfHeal: false`, puis son échec déclenche le pipeline complet via le wrapper cache. Un bouton incorrect mais cliquable ne produit pas nécessairement d’exception : ce scénario ne déclenche donc pas la réparation observée. [actService.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/services/actService.ts).

Les propositions de garde sémantique Jev [PR 2954](https://github.com/browserbase/stagehand/pull/2954) traitent ce problème, mais les PR 2952–2955 et 3103 étaient ouvertes/non fusionnées lors de l’enquête initiale et `experimentalDecisions` est absent du paquet stable. Ne pas les incorporer aux garanties v4.1. Le moteur Automan doit contrôler l’identité de cible avant les effets et les postconditions après, avec abstention sur ambiguïté ; une simple confiance du modèle ne remplace pas ces deux contrats.

### Batching et agent externe

`experimentalBatch()` est présent dans stable 4.1.0. Il exécute un callback JavaScript autoportant dans le service worker, avec entrées/sorties JSON, et réduit les allers-retours SDK–runtime. Il ne supprime pas les opérations CDP locales, l’inférence demandée ni le coût navigateur. Il n’est pas une transaction : un timeout n’annule pas rétroactivement l’action déjà lancée. Pas de fermeture lexicale hôte, filesystem hôte, fermeture de navigateur ou batch imbriqué. Les contraintes de page active et de page choisie doivent être explicites dans les workflows multi-onglets. [Référence Stagehand](https://docs.stagehand.dev/v4/reference/stagehand).

Enfin, **`agent()` a été retiré en v4**. Le code mode et une boucle d’agent externe utilisent l’API ; cela ne signifie pas absence de capacité autonome dans l’écosystème Browserbase, mais le SDK stable n’embarque plus cette ancienne méthode. Réessayer aveuglément un `act()` après une erreur peut répéter un effet ; la migration conseille l’escalade au niveau de l’observation. [Migration v3](https://docs.stagehand.dev/v4/migrations/v3).

## 4. Six alternatives proches, comparées par contrat

### Percevoir, résoudre, agir, vérifier

| Approche | Capture et résolution | Exécution / objet utile | Vérification et limite |
| --- | --- | --- | --- |
| **Browser Use**, historique et beta | Historique : DOM + AX + frames, références indexées, vision facultative. Beta : agent/runtime Rust, outils scripts navigateur, wrapper Python et JSON-RPC. | Boucle multi-étape ; historique `AgentHistoryList`, actions/outils et résultat final. Des actions initiales déterministes existent. | État navigateur, budgets d’échecs et sorties structurées ; la fin de tâche de l’agent ou la validation Pydantic ne prouvent pas la vérité. Le ledger beta expose encore des correctifs de synchronisation/navigation/observabilité. |
| **Skyvern** | Page enrichie Playwright, DOM/vision ; sélecteur direct ou prompt AI ; `AILocator` Python résout paresseusement. | `act` retourne None ; click/fill renvoient notamment le sélecteur résolu ; extract renvoie les données ; agent multi-étape retourne Task/WorkflowRunResponse. | `validate(prompt)` renvoie bool via modèle. Les assertions déterministes restent possibles via Playwright. Distinguer verdict LLM et règle métier vérifiable. |
| **AgentQL** | Capture d’accessibilité/page, requête déclarative à un backend sémantique ; `standard` ou `fast`. | `queryElements` → AQLResponseProxy dont les champs pilotent des Playwright Locators ; `getByPrompt` → Locator ou null ; `queryData` → objet. | Playwright prend en charge l’exécution/actionnabilité ; la sélection sémantique est préalable. Ni le locator valide ni l’objet retourné ne prouvent l’identité métier. Le mode fast documente un arbitrage de précision. |
| **Magnitude** | Navigation/interaction guidées par vision et coordonnées ; extraction documentée sur le DOM. | `agent.act` peut accomplir une ou plusieurs actions ; `extract` retourne directement le schéma Zod ; Page/Context Playwright accessibles. | Test runner et `agent.check(description)` évalués par IA ; combiner avec assertions code quand la condition est déterministe. Un objectif high-level n’est pas comparable à un seul act Stagehand. |
| **Imprint** | Enregistrement utilisateur des requêtes et interactions, puis compilation de chaîne API + playbook DOM. | MCP typé ; réexécution réseau/browser/DOM via plusieurs backends, sans nouvelle décision LLM sur le chemin déterministe. | Verifier indépendant de compilation, suites et reçus ; maturité générale non démontrée. Les erreurs sémantiques ne sont pas réparées en changeant simplement de transport. |
| **agent-browser / Playwright CLI** | Snapshots et références ; sélecteurs classiques accessibles. Le modèle externe choisit commandes/code. agent-browser ajoute un chat AI facultatif. | Commandes courtes et sessions ; agent-browser binaire Rust/daemon ; Playwright CLI utilise Playwright et peut `run-code`. | Retour commande, exit/error et nouvelle capture ; assertions métier à écrire. Playwright possède ses contrôles actionability ; ne pas les attribuer automatiquement à tous les pilotes CLI. |

Sources des contrats : [BrowserSession](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/session.py), [DOM Browser Use](https://github.com/browser-use/browser-use/blob/main/browser_use/dom/service.py), [ledger beta](https://github.com/browser-use/browser-use/blob/main/BETA_AGENT_INTEGRATION_FEATURES.md), [SDK Skyvern](https://www.skyvern.com/docs/developers/browser-automations/actions-reference), [Page AgentQL](https://docs.agentql.com/javascript-sdk/api-references/agentql-page), [BrowserAgent Magnitude](https://docs.magnitude.run/reference/browser-agent.md), [TestCaseAgent Magnitude](https://docs.magnitude.run/reference/test-case-agent.md), [architecture Imprint](https://github.com/ashaychangwani/imprint/blob/main/docs/architecture.md), [agent-browser](https://github.com/vercel-labs/agent-browser), [Playwright CLI](https://github.com/microsoft/playwright-cli).

### Cache, rejeu et fraîcheur

| Approche | Ce qui est réutilisé | Lecture de données lors du rerun | Conséquence économique |
| --- | --- | --- | --- |
| **Browser Use Cloud V4 scripts** | Script dans workspace persistant ; chaque run redémarre un agent | Les docs demandent requête live, fetched_at, hash source, schéma et réparation du script | Réduction de redécouverte ; **tokens encore consommés** à chaque rerun. Le pattern cloud n’est pas automatiquement une fonction OSS du beta. |
| **Skyvern code caching** | Séquences exécutables, par tâche ou par bloc ; couverture progressive des branches | Script contre la page courante ; conditions/waits/code restent live | `run_with="code"` peut éviter inférence et captures de l’agent ; fallback à l’agent/régénération. L’effet attendu doit encore être vérifié. |
| **AgentQL** | Locators résolus et programme appelant | Lectures Playwright actuelles ou nouvelle queryData | Aucun moteur durable de plans compilés/fraîcheur démontré dans le contrat consulté ; backend payé à chaque nouvelle résolution sémantique. |
| **Magnitude** | Prompt caching Anthropic, mémoire et captures retenues | La boucle poursuit ses perceptions/actions | Cache de préfixe modèle ≠ replay sans LLM. README qualifie encore son cache déterministe natif « in progress ». |
| **Imprint** | workflow API + playbook DOM + backend gagnant | Nouvelle requête au site, variables/cookies/state capturés pendant le run | Peut éliminer navigateur et inférence quand fetch suffit. Les timings et slogans zéro-token du README ne sont pas des résultats reproduits. |
| **CLI** | Programme écrit, sessions/profils et artefacts | Commandes/scripts relisent le navigateur ; orchestration libre | Un script déterministe peut tourner sans modèle ; une boucle coding-agent consomme encore des tokens. Réduire l’exposition de snapshots peut économiser du contexte. |

Sources : [scripts Browser Use](https://docs.browser-use.com/cloud/agent/scripts), [cache Skyvern](https://www.skyvern.com/docs/developers/features/code-caching), [AgentQL Page](https://docs.agentql.com/javascript-sdk/api-references/agentql-page), [mémoire Magnitude](https://docs.magnitude.run/advanced/memory.md), [README Magnitude](https://github.com/magnitudedev/browser-agent), [Imprint](https://github.com/ashaychangwani/imprint), [Playwright CLI](https://github.com/microsoft/playwright-cli).

La distinction la plus importante est entre **cache de réponse**, **cache de résolution/plan**, **cache de préfixe LLM** et **cache de backend/session**. Ils n’ont ni les mêmes garanties de fraîcheur ni les mêmes économies. Automan doit afficher séparément ces catégories dans ses métriques.

### OSS, SDK et cloud : pas de parité présumée

| Projet | Licence effectivement ouverte | SDK / ouverture utiles | Limite de portabilité |
| --- | --- | --- | --- |
| **Stagehand** | [MIT](https://github.com/browserbase/stagehand/blob/main/LICENSE) | TS/Python/Go partageant contrat généré ; extension locale ou Browserbase | Cache/gateway cloud ne sont pas un serveur cache OSS inclus. Callback batch JavaScript n’est pas une fermeture native Python/Go. |
| **Browser Use** | [MIT](https://github.com/browser-use/browser-use/blob/main/LICENSE) | Agent Python historique et beta Rust explicite ; runtime terminal [MIT](https://github.com/browser-use/terminal/blob/main/LICENSE) | Cloud V4/workspaces et version Python 0.13 sont deux surfaces. Un nom identique Agent n’implique pas moteur identique. |
| **Skyvern** | [AGPL-3.0](https://github.com/Skyvern-AI/skyvern/blob/main/LICENSE) | Backend public, SDK Python/TS, self-host et cloud | Référence actuelle indique locator AI, formulaires, plusieurs opérations/iframes **Python seulement**. Anti-bot cloud distinct du code public. Ne pas recopier comme composant permissif sans décision de licence. |
| **AgentQL** | [MIT pour dépôt public](https://github.com/tinyfish-io/agentql/blob/main/LICENSE) | Exemples, clients Python/JS, Page Playwright | Dépôt public principalement exemples ; moteur de résolution service non démontré ouvert. Exécuter Chrome localement n’enlève pas la dépendance API. |
| **Magnitude** | [Apache-2.0](https://github.com/magnitudedev/browser-agent/blob/main/LICENSE) | TS, agent + runner ; Playwright/Patchright ; BYOK modèles | Pas de trois SDK équivalents établis. Accès Playwright complet disponible, contrairement à Stagehand v4. |
| **Imprint** | [MIT](https://github.com/ashaychangwani/imprint/blob/main/LICENSE) | Bun/TS, outils MCP transférables, credentials séparés | Une niche peu éprouvée ; API privée capturée et state navigateur peuvent évoluer. MCP transporte un outil, pas une sémantique commune aux pilotes. |
| **agent-browser / Playwright CLI** | [Apache-2.0 agent-browser](https://github.com/vercel-labs/agent-browser/blob/main/LICENSE), [Apache-2.0 CLI](https://github.com/microsoft/playwright-cli/blob/main/LICENSE) | CLI déterministes adaptées coding agents ; sessions ; providers via CDP | Outils de contrôle, pas seuls un runtime métier durable avec schémas, preuves, repair policy et scheduler. |

Les licences ci-dessus décrivent les fichiers/dépôts consultés, pas tous les services commerciaux d’un fournisseur. Le [README Skyvern](https://github.com/Skyvern-AI/skyvern) et sa [référence actions](https://www.skyvern.com/docs/developers/browser-automations/actions-reference) matérialisent les écarts de surface ; [Magnitude Playwright access](https://docs.magnitude.run/core-concepts/playwright.md) confirme l’accès à Page/Context.

## 5. Ce que ces comparaisons imposent à une architecture Rust ambitieuse

**Le moteur canonique Rust est cohérent pour un produit portable ; son langage n’est pas sa différenciation.** Browser Use beta et agent-browser possèdent déjà des chemins Rust. Stagehand réduit les allers-retours via une extension et le batching. Le choix du placement d’exécution, de l’IR et des contrats de vérification peut peser davantage que quelques millisecondes de sérialisation.

La frontière recommandée pour Automan est :

1. **Core Rust portable** : sessions, états/events, deadlines, cancellation, budget, concurrence, journal/replay et plans typés. SDK TS/Python/Go fins, version du protocole négociée et mêmes erreurs/results. Fonctionnement local complet, y compris le cache de plans.
2. **Adaptateurs d’exécution** : HTTP/fetch en priorité quand le contrat le permet ; Chromium CDP natif ; worker Playwright compatible pour les flows qui exigent ses locators, waits, frames, interception et assertions ; extension facultative pour le navigateur utilisateur. Capacités déclarées par backend, jamais émulation silencieuse de fonctions non supportées.
3. **Observation et résolution séparées de l’action** : DOM/AX/vision/réseau avec identité frame/target et durée de validité ; résolveur déterministe avant modèle ; plusieurs candidats, abstention et budgets de recours. Un `ResolvedTarget` porte les preuves d’identité qui justifient l’effet.
4. **Vérification explicite** : schéma, cible, postcondition, fraîcheur et provenance sont cinq contrôles distincts. `success` technique et `verified` métier ne sont pas le même champ. L’oracle doit être déterministe quand possible, et signaler son caractère probabiliste quand il dépend d’un juge LLM.
5. **Cache typé et invalidation documentée** : plans réutilisables séparés des sorties. Une extraction live relit la source ; résultat cache seulement sous une FreshnessPolicy explicite avec âge/source/version. Diff sémantique et gardes de cible avant replay ; auto-heal par segment sous un budget et une politique d’effets.
6. **Mesure comparable** : coût et durée de première compilation, replay froid/chaud, réparation, sortie correcte, faux succès et intervention humaine ; nombre de captures, RPC et inférences par primitive. Un `extract` public unique peut cacher plusieurs appels modèle.

Un sidecar Rust est une bonne frontière interlangue et d’isolation. Mais « direct CDP » doit encore démontrer actionability, coordination frame/OOPIF, invalidation de références, hit testing, Shadow DOM, navigation et courses de DOM. Une capacité d’implémentation élevée accélère la construction ; elle ne supprime pas la matrice de compatibilité et de maintenance des versions Chromium. Les travaux [agent-browser natif](https://github.com/vercel-labs/agent-browser/tree/main/cli/src/native) et [Browser Use terminal](https://github.com/browser-use/terminal) sont des points de départ à auditer, plutôt qu’une preuve automatique de parité Playwright.

**Différenciation économique plausible :** workflows fréquents dont les sorties sont fraîches, attribuables à des sources et correctement vérifiées, avec coût complet explicable et réparation bornée. La nouveauté est à établir : Skyvern, Imprint et Browser Use couvrent déjà code réutilisable et réparation ; Imprint conserve déjà des preuves de vérification. L’avantage Automan doit être démontré par les contrats, la portabilité, l’exploitation et les résultats sur un corpus tenu hors développement.

## 6. Incertitudes et expériences décisives

Cette recherche ne permet pas de classer une fiabilité universelle. Restent notamment : comportement exact du serveur cache Browserbase, maturité du beta Rust Browser Use, réparations Skyvern sur erreurs sémantiques silencieuses, généralisation Imprint hors exemples et fidélité des assertions vision Magnitude. Les exemples du rapport sont inspectés contre types/docs, pas exécutés.

Un corpus utile doit forcer : mauvaise cible conservant un XPath valide, reorder de lignes, données changeant sans changement de layout, nouvelle valeur sous URL identique, rerender entre wait et click, navigation OOPIF, Shadow DOM, expiration d’auth, timeout après effet et retry. Mesurer séparément l’échec détecté et le **faux succès**. Comparer un programme déterministe optimisé, Stagehand sans cache, avec cache explicite, observe→replay, batching, un agent multi-étape et un script compilé. Le meilleur outil change selon ce contrat ; l’objectif produit est de choisir le chemin le moins cher qui satisfait la vérification demandée.
