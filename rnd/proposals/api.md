# API proposée : courte à l'usage, explicite en production

**Proposition du 7 octobre 2026. Tous les exemples sont des contrats à implémenter, pas du code disponible.** Les noms finaux et les schémas devront passer une revue d'ergonomie et des tests inter-SDK. Les domaines `.example` ci-dessous sont fictifs.

## 1. Règles de conception

1. Un cas simple tient dans quelques lignes ; les paramètres avancés se regroupent dans des politiques réutilisables.
2. Déterministe sans configuration de modèle. Une instruction naturelle est un appel IA explicite, avec coût et provenance.
3. Même run, types et sémantique local/cloud. Pas d'objet interne Playwright/Chromey dans le contrat portable.
4. Référence de plan ≠ données en cache ; extraction fraîche par défaut pour un nouveau run.
5. Chaque opération renvoie son statut technique ; un run possède un résultat et un verdict de validation distincts.
6. Streaming, pagination, deadlines et annulation dès le contrat initial.
7. Les politiques coûteuses — escalation IA, réparation, proxy — sont visibles dans le plan et le journal.

## 2. Extraction ponctuelle

Une extraction par sélecteurs n'utilise pas de LLM. `access: "auto"` sélectionne HTTP ou navigateur selon capacités/historique et validation, sans introduire une boucle d'agent. Le schéma apporte des types ; les validateurs portent le contrat métier.

```ts
import { automan } from "@automan/sdk";
import { z } from "zod";

const client = await automan.connect({ target: "local" });

const result = await client.extract({
  url: "https://catalogue.example/products/p-17",
  schema: z.object({ sku: z.string(), price: z.number() }),
  selectors: {
    sku: { css: "[data-sku]", attribute: "data-sku" },
    price: { css: ".price", transform: "decimal", locale: "fr-FR" },
  },
  access: "auto",
  validate: { identity: { field: "sku", equals: "p-17" } },
});

console.log(result.data, result.validation, result.runId);
await client.close();
```

Le SDK transforme le schéma supporté vers JSON Schema et vérifie les restrictions. Une refinement JavaScript arbitraire n'est pas automatiquement portable ; elle demeure un validateur de code dans un package ou se représente par une règle standard. Python expose modèles Pydantic/JSON Schema avec la même limite contractuelle.

Une extraction assistée diffère explicitement :

```ts
const result = await client.extract({
  url: "https://catalogue.example/products/p-17",
  instruction: "Extrais le SKU et le prix unitaire TTC affichés.",
  schema: Product,
  access: "browser",
  model: "extraction-default",
  validate: ProductChecks,
});
```

`model` est un alias de provider configuré ; aucune version de modèle n'est implicitement présentée comme portable ou éternelle. Si le provider manque, l'appel IA échoue avec une erreur de configuration avant d'engager un coût. Une erreur ne déclenche pas une réparation payante par défaut dans une extraction ponctuelle.

## 3. Navigateur : déterministe et naturel dans la même session

```ts
const session = await client.browser.open({
  backend: "auto",
  identity: "supplier-readonly",
  requires: ["browser.downloads", "browser.frames"],
});

try {
  const page = await session.newPage();
  await page.goto("https://supplier.example/exports");

  // Déterministe : locator retrouvé au moment d'agir, sans modèle.
  const download = await page.expectDownload({ timeoutMs: 30_000 });
  await page.locator({ role: "button", name: "Exporter" }).click();
  const file = await download.result();
  await file.save({ destination: "artifacts", name: "export.csv" });

  // IA explicite : résolution typée, puis exécution et postcondition.
  const actions = await page.observe({
    instruction: "Trouve le bouton pour exporter le mois précédent.",
    model: "decision-default",
  });
  await page.act(actions.requireUnique(), {
    expect: { download: { mime: "text/csv" } },
  });
} finally {
  await session.close();
}
```

`observe` retourne des candidats et leurs références document/page/frame, jamais une cible silencieusement choisie parmi plusieurs. `act(Action)` exécute une action déjà résolue sans inférence. `act({ instruction, model, expect })` est un raccourci assisté distinct. Le contrat des effets dépend du workflow : cliquer sur « Valider la commande » n'hérite pas de la politique de retry d'une lecture.

`expectDownload` acquitte l'installation de l'attente avant de retourner son handle : elle précède le clic pour capturer aussi un téléchargement immédiat. Le handle porte deadline, résultat et annulation. Les attentes d'autres événements susceptibles de survenir pendant une action suivent le même contrat d'enregistrement.

Une session possède un backend propriétaire. En cas de capability manquante, choisir le backend adéquat à l'ouverture ; une session active ne passe pas arbitrairement de CDP à Playwright. Les accès directs au driver natif sont des extensions de code non portables, identifiées comme telles.

## 4. Crawl : recette portable ou handler de code

### 4.1 Recette portable

```ts
const run = await client.crawl({
  seeds: ["https://catalogue.example/"],
  workflow: "catalogue.collect@7",
  queue: { name: "catalogue-octobre", resume: true },
  scope: { sameDomain: true, maxDepth: 6, maxRequests: 100_000 },
  limits: { http: 128, browserPages: 12, perDomain: 4 },
  autoscale: true,
  retry: { maxAttempts: 4, backoff: "exponential-jitter" },
  dataset: { name: "catalogue", mode: "upsert", key: ["sku"] },
});

for await (const event of run.events()) {
  console.log(event.type, event.progress);
}
const outcome = await run.result();
```

Valeurs de concurrence **illustratives**, pas des valeurs optimales ni une promesse de capacité. Les limites par ressource sont bornées par les quotas globaux. La reprise charge la queue existante, sans purge implicite. Le dataset upsert requiert une identité stable déclarée ; un dataset append avec clé de commit est une autre politique.

### 4.2 Handler TypeScript local

```ts
const run = await client.crawl({
  seeds: ["https://catalogue.example/"],
  handler: async ({ document, request, discover, emit }) => {
    await discover({ selector: "a.product", label: "product" });
    if (request.label === "product") {
      await emit({
        key: document.attribute("[data-sku]", "data-sku"),
        value: readProduct(document),
      });
    }
  },
});
```

Ce callback demeure dans le processus Node du client attaché. Il ne se sérialise pas en JSON vers Rust et n'est pas repris au milieu de ses lignes après crash. Pour le cloud, `automan package build` crée un package versionné avec runtime/handler, dépendances, schémas, permissions et tests ; le moteur lui confie des unités de travail dans un host isolé. Les handlers doivent respecter leur contrat de retry et éviter des mutations non idempotentes cachées dans une collecte.

`emit` et `discover` acquittent le **staging**, rattaché à requête/tentative/epoch de lease. Le moteur publie données et nouvelles requêtes avec l'acquittement après réussite du handler et validation. Un handler échoué ne publie pas silencieusement ses sorties provisoires. Une politique de résultats partiels explicite peut publier un sous-ensemble validé et son état partiel ; les effets externes directs du callback restent hors de cette transaction.

## 5. Workflow versionné et réparation

```ts
const workflow = await client.workflows.create({
  name: "supplier.monthly-export",
  input: ExportInput,
  output: ExportDataset,
  identity: "supplier-readonly",
  plan: MonthlyExportPlan,
  validate: [AccountIdentity, MonthCoverage, CsvSchema],
  repair: {
    mode: "validated-auto",
    allowed: ["locator", "wait", "extraction-mapping"],
    maxCandidates: 2,
    budget: { maxModelCalls: 4 },
    tests: "supplier-export-regressions",
  },
});

const run = await workflow.run({ month: "2026-09" });
const outcome = await run.result();
```

`validated-auto` exige les tests et les validateurs requis ; sinon la réparation reste candidate `needs_review`. Les modes proposés sont `off`, `suggest`, `validated-auto`. Un workflow avec effets non idempotents ne devient pas répétable parce qu'un locator est réparé. Les valeurs limites sont illustratives ; le plafond global du run s'applique aussi à la réparation.

La compilation est un objet durable distinct :

```ts
const candidate = await client.workflows.compile({
  goal: "Télécharger l'export du mois demandé depuis le portail fournisseur.",
  parameters: ExportInput,
  identity: "supplier-readonly",
  model: "planner-default",
  validate: [AccountIdentity, MonthCoverage, CsvSchema],
});

const report = await candidate.test({ suite: "supplier-export-regressions" });
if (report.qualified) await candidate.publish();
```

Une publication fixe une nouvelle version. Le run de test affiche données/preuves/coût et refus de qualification éventuel. `qualified` signifie que les exigences de cette suite passent, pas que tous les sites futurs fonctionneront. Un recorder peut produire le même type de candidat à partir d'une démonstration, avec expurgation et paramètres revus.

## 6. Agent intégré ou externe

```ts
const run = await client.agent.run({
  goal: "Trouve et récupère les fiches des trois modèles demandés.",
  input: { models: ["A17", "B20", "C42"] },
  tools: ["browser.read", "browser.navigate", "extract", "artifacts.write"],
  domains: ["catalogue.example"],
  model: "planner-default",
  validate: RequestedModelsPresent,
  limits: { maxActions: 40, maxModelCalls: 20, timeoutMs: 180_000 },
});
```

Un agent externe appelle les outils par SDK, CLI ou MCP et peut obtenir une description compacte des capacités, puis les détails à la demande. Aucun framework d'agent n'est obligatoire. La mémoire de site/plan réutilisable est séparée de l'état conversationnel du run. Le résultat terminal est décidé par le contrat et les validateurs, pas uniquement par le texte de l'agent.

## 7. Forme commune des résultats et erreurs

```json
{
  "runId": "run_demo_42",
  "workflowVersion": "supplier.monthly-export@8",
  "status": "succeeded",
  "data": { "datasetId": "dataset_demo_9", "rowCount": 248 },
  "validation": {
    "status": "passed",
    "checks": ["account-identity", "month-coverage", "csv-schema"],
    "evidence": ["artifact_demo_12"]
  },
  "usage": { "modelCalls": 0, "browserMs": 4300, "attempts": 1 },
  "eventsCursor": "cursor_demo_27"
}
```

Données et chiffres de démonstration uniquement. Les résultats contiennent aussi périmètre d'identité, timestamps/freshness, backend/capabilities, avertissements et checks non concluants. Les secrets et observations volumineuses sont exclus de l'enveloppe par défaut.

Erreurs proposées : `CAPABILITY_UNAVAILABLE`, `TARGET_AMBIGUOUS`, `REFERENCE_STALE`, `PAGE_GONE`, `AUTH_REQUIRED`, `RATE_LIMITED`, `BUDGET_EXCEEDED`, `DATA_INCOMPLETE`, `VALIDATION_FAILED`, `LEASE_LOST`, `EFFECT_UNKNOWN`, `CLIENT_HANDLER_DISCONNECTED`. Chaque erreur indique retryabilité, effet possible, preuve, prochaine action et tentative. Une erreur inconnue d'une version nouvelle reste représentable par son code et ses détails compatibles.

`run.events({ after })` reprend par curseur. Si l'historique demandé a expiré, `EVENT_CURSOR_EXPIRED` fournit un snapshot et un nouveau point de reprise ; le client ne prétend pas avoir reçu les événements manquants. `run.cancel()` est idempotent ; son acquittement de demande ne signifie pas que toutes les opérations ont déjà cessé.

Un run `waiting` peut reprendre avec `runs.resume` après résolution de son attente. Un run terminal garde son historique : `runs.reconcile` y attache une résolution d'effet prouvée, puis `runs.fork({ fromRun, resolution })` crée un nouveau run si une continuation est nécessaire. Les checkpoints sont revalidés selon leur freshness et les effets confirmés ne sont pas répétés. Un effet inconnu sans résolution autorisée interdit le fork des étapes mutantes concernées.

## 8. CLI

```sh
automan extract 'https://catalogue.example/products/p-17' --recipe product --json
automan run supplier.monthly-export --input ./month.json --wait --json
automan crawl catalogue.collect --seeds ./urls.txt --queue catalogue --resume
automan runs inspect run_demo_42 --events --json
automan runs cancel run_demo_42
automan workflows repair supplier.monthly-export --from-run run_demo_42 --suggest
automan package build ./supplier-workflow
automan serve --local
automan mcp --stdio --profile read
```

Les chemins sont des paramètres exemples. `--json` réserve stdout aux données ; logs/diagnostics sur stderr. `--events` produit des événements NDJSON bornés et un état terminal. Codes de sortie proposés : 0 succès vérifié/commande administrative réussie, 1 échec, 2 partiel, 3 décision requise, 4 effet inconnu, 130 interruption utilisateur. Les scripts peuvent choisir comportement terminal ou détaché ; le run survit à la CLI lorsque le daemon en possède l'exécution.

Installation par binaires signés/packagés, résolution des versions compatibles, `doctor`, version/capabilities et mise à jour explicite. L'installation de navigateurs/profiles est observable ; aucun `cargo` obligatoire sur les plateformes binaires supportées.

## 9. MCP

Profils proposés : `read`, `browser`, `workflows`, `operator`. Le catalogue annoncé dépend des capacités/permissions disponibles. Les noms sont stables et les schemas d'entrée/sortie explicites.

| Outil | But et résultat |
| --- | --- |
| `automan_capabilities` | Capacités disponibles, limitations, schémas détaillés à la demande |
| `automan_extract` | Extraction ponctuelle avec schéma/politique et résultat référencé |
| `automan_browser_open` | Session/page scoped et backend qualifié |
| `automan_browser_observe` | Observation bornée avec références et génération |
| `automan_browser_act` | Action typée et postcondition ; permissions d'effet vérifiées |
| `automan_workflow_run` | Run durable d'une version ou référence explicite |
| `automan_run_inspect` | État, checks, coûts et prochains événements paginés |
| `automan_run_cancel` | Demande d'annulation idempotente |
| `automan_dataset_read` | Projection/pagination et provenance |
| `automan_repair_propose` | Candidat borné rattaché au run en échec, jamais promotion implicite |

Le mode stdio sert les agents locaux, Streamable HTTP les usages distants. L'adaptateur suit la version MCP négociée, l'autorisation et le modèle d'outils de la spécification courante ; la bidirectionnalité du RPC interne Automan ne définit pas celle de MCP. Les événements du run sont des données inspectables via outil/ressource compatible, pas des requêtes JSON-RPC arbitrairement initiées par le serveur. [MCP transports 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports), [autorisation](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

Pas de `eval`/code arbitraire dans le profil standard. Une capacité avancée explicite peut servir du code de confiance avec ses permissions propres. Les ressources/artefacts ont références, scopes et durée d'accès. Les annotations d'outil aident le client, mais ne remplacent pas l'application des permissions par le moteur.

## 10. API cloud et ergonomie avancée

Ressources REST : workflows/versions/packages, runs/steps/events, queues/requests, datasets/exports, artifacts, identities/sessions, pools/workers, schedules, repairs, usage et workspace roles. Commandes non ambiguës : start, cancel, pause admission, drain worker, propose/test/promote repair, rotate/retire session. Les opérations durables retournent identifiant et état inspectable.

Idempotency keys pour création de run et commandes répétables ; version/ETag pour changements concurrents de configuration ; cursors pour listes/events ; dates/units explicites. Les secrets sont des références. La console et macOS utilisent ces contrats au lieu d'ajouter des comportements invisibles propres à leur interface.

Les paramètres avancés de pools, hooks, plugins et schedulers sont décrits par schémas et presets importables. Une configuration faible complexité reste lisible ; l'utilisateur peut exporter la configuration effective, voir les surcharges et la cause exacte qui limite son run.
