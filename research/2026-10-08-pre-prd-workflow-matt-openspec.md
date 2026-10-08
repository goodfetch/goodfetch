# Aller du lab au PRD v2 : comment utiliser les skills de Matt Pocock et OpenSpec

**Date :** 8 octobre 2026.

**Mise à jour du 8 octobre :** OpenSpec a été retiré du dépôt après cette étude. Seuls les skills de Matt Pocock sont utilisés. Les sections sur OpenSpec restent comme trace de la comparaison.

**Question :** Automan n'a presque pas de code produit. Il y a des recherches, deux ADR, un glossaire et un laboratoire de mesure. Que recommandent les skills de Matt Pocock et OpenSpec pour arriver à un PRD solide, puis à l'implémentation ?

**Sources :**
- Les `SKILL.md` installés dans `~/.agents/skills/`. Comparés le 8 octobre avec `mattpocock/skills@main`, ils sont identiques.
- Les pages de documentation `https://github.com/mattpocock/skills/blob/main/docs/...`.
- OpenSpec 1.14.1 : `docs/` et `docs-lab/` du dépôt `Fission-AI/OpenSpec` (tag `v1.14.1`), le schéma `spec-driven` intégré, et les skills générés dans `.claude/skills/openspec-*`.
- L'état du dépôt sur `origin/main` (`8d518cd`).

Sauf mention contraire, les citations sont des extraits courts des sources. Les passages marqués **[Interprétation]** donnent mon analyse.

---

## 1. Ce qu'il faut retenir

1. **Les deux outils font des choses différentes, au même moment.**
   - Chez Matt Pocock, le PRD s'appelle désormais **spec** : `to-prd` a été renommé `to-spec`. Ses skills vont de l'idée floue au spec, puis aux tickets.
   - OpenSpec dit explicitement ne pas être un outil de PRD : « A 40-page PRD is a different artifact with a different job » (`docs/existing-projects.md`). Une spec OpenSpec est un **contrat de comportement par changement**. Elle n'a de sens qu'une fois l'implémentation commencée.
2. **Avant le PRD, ce sont les skills de Matt qui pilotent.** OpenSpec n'apporte presque rien à ce stade, sauf `/opsx:explore`, qui fait doublon avec le grilling.
3. **Le bon point d'entrée est `/wayfinder`.** Il est fait pour un effort « too big for one session » dont « the route… [is] still unclear ». Il s'arrête quand la carte est dégagée et passe la main à `/to-spec #<map>` (`ask-matt/SKILL.md`, rubrique « On-ramps »).
4. **Vos outils existants font déjà une partie du travail de wayfinder.** Le registre `lab/decisions/D-*.json`, avec ses verdicts calculés, couvre une bonne partie de ce qu'une carte wayfinder apporte. Il ne faut pas créer un second registre : la carte doit **pointer** vers les décisions du lab, pas les recopier (§4).
5. **Trois points sont à régler avant de commencer** (§3) :
   - le PRD v1 a été produit par un skill obsolète ;
   - les tickets de phase sont marqués `ready-for-agent` alors qu'ils tiennent en une ligne ;
   - les labels `wayfinder:*` n'existent pas sur GitHub.
6. **Le chemin critique, c'est le produit, pas les benchmarks.**
   - `D-pricing` et `D-feature-priority` dépendent de `D-segment`.
   - Aucun entretien n'a eu lieu (#11).
   - Un PRD v2 sans segment choisi aurait le même défaut que le v1 : 80 user stories, sans aucune priorité.

---

## 2. Le parcours prévu par chaque outil

### Matt Pocock

Source : `ask-matt/SKILL.md`, `docs/engineering/*.md`.

```
[cartographie : /wayfinder] → /grill-with-docs → (détour /prototype) → /to-spec → /to-tickets → /implement | /implement-spec → /retro
```

| Skill | Ce qu'il fait | Ce qu'il produit | Rôle pour Automan maintenant |
|---|---|---|---|
| `wayfinder` | Carte des décisions : « producing decisions, not deliverables ». Une seule décision tranchée par session (sauf recherche). | Une issue `wayfinder:map` et des sous-issues `wayfinder:{research,prototype,grilling,task}`, avec blocages natifs GitHub | **Point d'entrée** |
| `grill-with-docs` | Interview qui pose toutes les questions en suspens à la fois. Les faits, c'est à l'agent de les trouver ; les décisions reviennent à l'utilisateur. | `GLOSSARY.md` et ADR, créés au fil de l'eau. Le reste ne vit que dans la conversation. | Trancher les tickets `grilling`, puis combler les dernières lacunes avant le spec |
| `research` | Agent en arrière-plan qui cherche un **fait externe** dans des sources primaires | Un fichier Markdown sourcé | Tickets `research` |
| `prototype` | Répond à **une** question de conception. Jetable, sans tests. | Une branche `prototype/<nom>` | Rare : votre lab va plus loin qu'un prototype (§4) |
| `to-spec` | Synthèse **sans interview** de la conversation et de la carte. Il propose d'abord les points de test (« seams »). | **Une issue GitHub** : problème, solution, user stories, décisions d'implémentation, tests, hors périmètre | **Produit le PRD v2** |
| `to-tickets` | Découpe en tranches verticales, une par fenêtre de contexte, avec blocages et sous-issues | Issues `ready-for-agent` | Après le PRD v2 |
| `domain-modeling` | Un ADR ne se justifie que si la décision est difficile à défaire, surprenante sans contexte **et** issue d'un vrai compromis | `docs/adr/NNNN-*.md`, statut `proposed`/`accepted`… | Un ADR par décision tranchée |

**Points des docs à respecter :**
- **Une seule fenêtre de contexte.** Enchaîner grill, to-spec et to-tickets sans compacter ni effacer le contexte, dans une limite d'environ 150k tokens.
- **Le spec est jetable** une fois le travail livré : « Your `GLOSSARY.md` and ADRs are the files meant to last ».
- **Erreurs connues avec wayfinder** (`docs/engineering/wayfinder.md`) :
  - L'agent écrit du code de production depuis la carte, par exemple en notant « this map carries execution » dans ses Notes.
  - Une carte de 27 tickets qui finit par se périmer. Le conseil : « scope the map to a bounded destination… Wayfinder is "prototypemaxxing", not "planmaxxing" ».
- **Bug connu de grill-with-docs.** Lancé dans un autre orchestrateur de spec, il n'écrit pas ses fichiers, sans le signaler. Il ne faut donc pas le lancer à l'intérieur de `/opsx:explore`.
- **Label `ready-for-agent` posé par `to-spec`.** Un agent autonome qui surveille ce label essaierait d'implémenter tout le spec. Il faut retirer le label après `/to-tickets`.

### OpenSpec 1.14.1

Sources : `docs/concepts.md`, `docs/workflows.md`, `docs/existing-projects.md`, `schemas/spec-driven/schema.yaml`.

- **Le modèle :**
  - `openspec/specs/` décrit le comportement actuel du système.
  - `openspec/changes/<nom>/` contient un changement proposé : `proposal.md` → `specs/` (deltas `ADDED/MODIFIED/REMOVED/RENAMED`) + `design.md` → `tasks.md`.
  - L'archivage fusionne les deltas dans `specs/`.
- **Ce qu'une spec ne contient pas :** pas de choix de bibliothèque ni de détail interne. « If the implementation can change without changing externally visible behavior, it likely does not belong in the spec ». Les choix techniques vont dans `design.md` et dans les ADR.
- **Projet sans code (greenfield) :**
  - `specs/` démarre vide et se remplit au fil des changements : « Resist the urge to back-fill everything ».
  - Les PRD et recherches existants sont de la **matière première pour explore**, pas des specs à convertir.
  - En 1.14.1, l'archivage crée bien la spec principale d'une nouvelle capacité (correction du bug #1222).
- **Avant le PRD :** `/opsx:explore` (« when you're not sure, explore before you propose »).
  - Il n'écrit rien sans votre accord.
  - `/opsx:onboard` est un tutoriel. Il cherche une petite tâche de code et n'a pas d'intérêt ici.
- **Schéma personnalisé :** on peut en forker un, par exemple `research-first` (research → proposal → tasks). Mais un artefact `prd` ou `research` n'est **jamais fusionné** dans `specs/`.
- **Stores (bêta) :** planification hors du dépôt ou multi-dépôts. Inutile pour Automan.
- **Suivi des tâches :** aucune intégration avec un outil de tickets. La convention est un changement = une branche = une PR.

**[Interprétation]** Après le PRD, OpenSpec apporte ce qui manque à Matt : une **spec de comportement durable**. Le spec de Matt est un instantané jetable, alors qu'OpenSpec tient la vérité à jour dans `openspec/specs/`. Les deux outils se complètent, à condition que chacun ait son propre rôle (§5).

---

## 3. Ce qui dans le dépôt freine le parcours

| # | Problème | Source | Correction proposée |
|---|---|---|---|
| 1 | `issues/prd.md` (PRD v1) a le format de `write-a-prd`. Ce skill vient d'un dépôt d'atelier de Matt (`ai-engineer-workshop-2026-project`), qui est **remplacé** par `to-spec`. Il écrit `issues/*.md` et interdit GitHub. | `~/.agents/.skill-lock.json` | Marquer le v1 « superseded — matière source pour le v2 ». Ne plus utiliser `write-a-prd` ni `prd-to-issues`. |
| 2 | Le PRD v1 fige Rust, React/shadcn, macOS et le périmètre « Crawlee+Spider+Stagehand » avant les décisions correspondantes. `D-core-language` est encore ouverte. | `issues/prd.md:9`, `lab/decisions/D-core-language.json` | Le v2 doit découler des verdicts, pas l'inverse. |
| 3 | Trois endroits possibles pour les specs : les GitHub Issues (`AGENTS.md`), `issues/prd.md` et `openspec/specs/`. | `AGENTS.md`, `openspec/` | Répartir explicitement (§5). |
| 4 | Les issues #6 à #9 tiennent en une ligne mais portent `ready-for-agent`. | `gh issue list` | Retirer le label : ce sont des chantiers du lab, pas des tâches entièrement spécifiées. |
| 5 | Les labels `wayfinder:map`, `:research`, `:prototype`, `:grilling` et `:task` n'existent pas. Le skill de configuration ne les crée pas. | `docs/engineering/setup-matt-pocock-skills.md` | Les créer avant la première exécution. |
| 6 | `D-storage` est « decided » sans verdict ni ADR. L'ADR-001 est « proposé » alors que `D-core-language` n'a pas de verdict valable. | `lab/decisions/D-storage.json`, `docs/rd-plan.md:76` | Aligner avant de cartographier. |
| 7 | `roadmap-validation.md` commence par « console + API », alors que `rd-plan.md` (approuvé) commence par le lab et les décisions. | `docs/roadmap-validation.md:9` | Signaler que `roadmap-validation.md` est dépassée, ou l'aligner. |
| 8 | Le guide d'entretien annoncé par `rd-plan.md:71` n'est pas dans le dépôt. | `research/market/pricing-signals.md` §5 | C'est le premier livrable produit à faire. |

---

## 4. Faire tenir ensemble wayfinder et le lab

**[Interprétation]** Votre lab ne correspond à aucun skill de Matt :
- ce n'est pas un `prototype`, qui serait jetable et sans tests ;
- construire le lab, c'est « faire », ce qui va contre la règle « Plan, don't do » de wayfinder.

La séparation la plus nette est la suivante.

| Couche | Rôle | Où ça vit |
|---|---|---|
| **Décisions** | Ce qu'il faut trancher avant le PRD v2 | Carte wayfinder : une issue `wayfinder:map` et une sous-issue par décision `D-*` ouverte |
| **Preuves** | Mesures et verdicts calculés | `lab/decisions/`, `lab/verdicts/`, `lab/reports/`. **Reste la source de vérité.** |
| **Construction du lab** | Phases P5 à P9 du harnais | Issues ordinaires (#5 à #9), traitées **hors** wayfinder, dans des sessions normales |

**La carte wayfinder :**
- **Destination, bornée :** « PRD v2 d'Automan pour le segment retenu, chaque décision bloquante tranchée par un verdict ou un ADR ».
- **Notes :** « Les campagnes du lab s'exécutent hors de la carte. Un ticket se résout quand `lab/verdicts/D-x.json` est éligible ou qu'un ADR est accepté. La carte ne porte aucune exécution. »
- **Les tickets, par type :**
  - `grilling` : `D-segment` en premier, puis `D-pricing` et `D-feature-priority`, bloquées par `D-segment`. Aussi les décisions qui se règlent en discutant : `D-ai-layer`, la portée de `D-identity`, `D-agent-interface` côté produit.
  - `task` (avec un humain) : les entretiens JTBD (#11), le test de prix, le PoC payant. Les tickets techniques dont le verdict attend une campagne du lab (`D-browser-driver`, `D-core-language`, `D-http-transport`, `D-durable-runtime`…) pointent vers leur fichier `D-*.json`, sans le recopier.
  - `research` : les faits externes qui manquent encore. Par exemple, les conditions juridiques de l'ADR-002, ou une tarification concurrente à jour.
- **Blocages :** les dépendances natives entre issues reprennent les `dependsOn` du registre `D-*`.
- **Taille :** environ 15 décisions, en dessous des 27 tickets cités comme point de rupture. Si une décision technique n'influence pas le PRD (stockage, sinks…), elle passe dans « Out of scope », pas dans un ticket.

**Résoudre un ticket :**
1. Une session tranche **une** décision : grilling, ou lecture d'un verdict.
2. `domain-modeling` écrit l'ADR si les trois conditions sont remplies (difficile à défaire, surprenant, vrai compromis). Le statut du `D-*.json` est mis à jour en même temps.
3. Un commentaire est posté sur le ticket, le ticket est fermé, puis une ligne est ajoutée dans « Decisions so far ».

---

## 5. Répartir les rôles entre Matt et OpenSpec

| Phase | Outil | Artefact qui fait foi |
|---|---|---|
| Recherche et mesure (aujourd'hui) | `wayfinder`, `grill-with-docs`, `research`, plus le lab | Carte GitHub, `lab/decisions/`, ADR, `GLOSSARY.md` |
| PRD v2 | `/to-spec #<map>` dans la **même fenêtre** qu'un dernier `grill-with-docs` | **Issue GitHub** : le PRD v2. `issues/prd.md` est archivé comme source. |
| Découpage | `/to-tickets` | Sous-issues GitHub (retirer `ready-for-agent` du PRD) |
| Implémentation, par ticket | `/opsx:propose` (ou `new`/`continue` au début), puis `/opsx:apply`, puis `/opsx:archive` après merge | `openspec/changes/` pendant le travail, `openspec/specs/` ensuite. Le ticket et la PR pointent vers le changement. |

**[Interprétation]** Pourquoi cette répartition :
- Les deux outils sont d'accord : le PRD n'est pas une spec OpenSpec.
- Le spec de Matt est jetable, alors que `openspec/specs/` dure.
- Chaque ticket devient un changement OpenSpec, ce qui donne une revue en trois temps : proposition, delta, code (`docs/team-workflow.md`).
- `/implement` de Matt et `/opsx:apply` font doublon. **Choisissez-en un.** Je recommande `/opsx:apply`, qui garde la trace dans la spec. En contrepartie, la TDD aux points de test se perd, sauf si vous ajoutez `rules.tasks` dans la config.

**Ajouts possibles à `openspec/config.yaml` plus tard** (courts, comme le recommandent les docs) :
- `rules.proposal` : citer l'issue du ticket, ainsi que la preuve du lab ou l'ADR qui justifie le changement.
- `rules.design` : citer l'identifiant de l'ADR ; un ADR `proposed` n'engage à rien.
- `rules.tasks` : chaque groupe commence par un test au point de test convenu dans le spec.

**Variante plus simple :** tout faire avec Matt (`to-tickets` → `implement`), sans OpenSpec. Vous perdez seulement la spec de comportement durable. Choisissez cette variante si vous ne voulez pas maintenir `openspec/specs/`.

---

## 6. Plan d'action

1. **Ménage, en une session :**
   - créer les labels `wayfinder:*` ;
   - retirer `ready-for-agent` des issues #6 à #9 ;
   - retirer le PRD v1 `issues/prd.md` (supprimé le 8 octobre) ;
   - noter la répartition du §5 dans `AGENTS.md` ;
   - corriger `D-storage` (verdict ou ADR).
2. **`/wayfinder`** avec la destination du §4, en une session. Le premier grilling, en largeur, doit confirmer qu'il reste bien des zones floues. Sinon, passer directement à l'étape 4.
3. **Résoudre la carte, un ticket par session**, en commençant par `D-segment`.
   - Préparer le guide d'entretien. `/to-questionnaire` peut le rédiger à partir de `research/market/`.
   - Pendant ce temps, le lab avance P6 à P9 dans des sessions séparées.
4. **Quand la carte est dégagée,** enchaîner dans une seule fenêtre de contexte : `/grill-with-docs` (dernières lacunes), puis `/to-spec #<map>` (PRD v2), puis `/to-tickets`.
5. **Implémenter** ticket par ticket avec OpenSpec (ou `/implement`), puis `/retro`.

## 7. Limites de cette analyse

- Une partie des docs OpenSpec 1.14.1 sur openspec.dev ne sont que des plans de page : `explore`, `existing codebases`, la FAQ. `https://openspec.dev/docs/existing-projects` renvoie une 404. J'ai cité à la place la version GitHub `docs/`.
- Les skills `in-progress` de Matt (`chief-of-staff`, `claude-handoff`, `loop-me`…) peuvent changer à tout moment. Ce plan ne s'appuie pas sur eux.
- Les docs de Matt conseillent de ne pas versionner les fichiers de recherche (« ADRs yes. Everything else archive or delete »). Automan a choisi de garder `research/`. Ce choix reste valable, mais ces fichiers ne sont relus automatiquement par aucun skill : il faut les donner explicitement au grilling.
