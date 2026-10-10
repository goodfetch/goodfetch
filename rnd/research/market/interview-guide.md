# Guide d'entretien « switch » (Jobs To Be Done)

**Statut :** v1, prêt à l'emploi. **Date :** 2026-10-08.
**Objet :** trancher `D-segment`, `D-feature-priority` et la partie qualitative de `D-pricing` à partir de décisions d'achat réelles, pas d'opinions. Le questionnaire de prix est à part : [pricing-questionnaire.md](pricing-questionnaire.md).
**Qui mène :** le porteur du projet. Je code les transcriptions avec le [codebook](codebook.md).

Un entretien « switch » reconstitue la dernière fois que la personne a **changé** d'outil (ou a décidé de construire le sien) : ce qui l'a poussée, ce qui l'a attirée, ce qui l'a retenue. Les faits passés prédisent mieux l'achat que les intentions. La règle d'or : on demande ce qui s'est passé, jamais ce que la personne ferait.

## 1. Recrutement

**Quota.** 10 à 15 entretiens, au moins 3 par segment candidat de `D-segment`, plus 2 personnes qui ont **quitté** un outil hébergé pour du maison.

| Segment (`D-segment`) | Profil à recruter | Critère d'éligibilité |
| --- | --- | --- |
| `agent-developers` | Développe un agent qui lit ou agit sur le web | A changé d'outil navigateur, de framework d'agent ou de fournisseur d'accès dans les 12 derniers mois |
| `b2b-portals` | Automatise des portails authentifiés (assurance, logistique, administration, ERP) | Au moins un portail en production ; a vécu une casse ou une expiration de session |
| `data-teams` | Équipe qui collecte des données web à volume | Budget proxy, captcha ou API d'accès existant |
| `ecommerce-pricing` | Veille de prix ou de catalogue | Suit au moins 100 produits concurrents |

**Où trouver.** Contacts directs du porteur, auteurs de fils et d'issues du corpus (`corpus.jsonl`) **contactés par les canaux publics qu'ils ont eux-mêmes ouverts**, communautés citées dans la piste marché. Ne jamais compiler de coordonnées personnelles à partir de plusieurs sources.

**Écran de sélection (5 questions, par écrit).**
1. Quel est votre rôle, et combien de personnes travaillent sur l'automatisation web chez vous ?
2. Quels outils utilisez-vous aujourd'hui pour lire ou agir sur des sites (navigateur, HTTP, API d'accès, agents) ?
3. Quand avez-vous changé d'outil pour la dernière fois, ou décidé d'en construire un ?
4. Quel est votre budget mensuel approximatif sur ce poste (infra, proxies, captchas, LLM, services) ?
5. Acceptez-vous un entretien enregistré de 45 minutes ?

Retenir en priorité les personnes qui ont un **changement daté** à raconter.

## 2. Déroulé (45 minutes)

**Consentement (2 min).** Expliquer l'objet (recherche produit, pas de vente), demander l'accord pour enregistrer, préciser que la transcription est pseudonymisée (identifiant `int-NN`) et que les noms d'entreprises clientes ne sont pas notés. Pas de démonstration du produit pendant l'entretien.

**Contexte (5 min).**
- Racontez-moi ce que vous automatisez sur le web, avec un exemple de la semaine dernière.
- Qui en dépend, et que se passe-t-il quand ça casse ?

**Chronologie du changement (20 min).** Remonter le dernier changement, étape par étape. Chercher des dates, des lieux, des personnes, des chiffres.

| Étape | Questions |
| --- | --- |
| Premier déclic | Quand avez-vous pensé pour la première fois que l'outil d'avant ne suffisait plus ? Que s'était-il passé ce jour-là ? |
| Recherche passive | Qu'avez-vous remarqué ensuite, sans chercher activement ? |
| Recherche active | Qu'avez-vous comparé ? Qu'avez-vous essayé ? Qu'est-ce qui a éliminé chaque option ? |
| Décision | Qui a décidé ? Qu'est-ce qui a fait basculer ? Qu'avez-vous dû justifier, et à qui ? |
| Premier usage | Qu'est-ce qui a déçu ou surpris pendant les premières semaines ? |
| Aujourd'hui | Que regrettez-vous de l'outil d'avant ? Qu'est-ce qui vous ferait partir à nouveau ? |

**Les quatre forces (10 min).** Relire la chronologie avec la personne pour classer ce qu'elle a dit :
- **Poussée** (ce qui rendait la situation intenable) : blocages, factures, faux succès, maintenance, sessions perdues ;
- **Attraction** (ce que la nouvelle solution promettait) ;
- **Anxiété** (ce qui faisait peur dans le changement) : verrouillage, données, conformité, coût imprévisible ;
- **Habitude** (ce qui retenait) : code existant, compétences de l'équipe, contrats.

**Relances par décision du registre (5 min, seulement si non abordé spontanément).**

| Décision | Relance factuelle |
| --- | --- |
| `D-feature-priority` | Racontez la dernière fois qu'une collecte a « réussi » avec des données fausses ou incomplètes. Comment l'avez-vous découvert ? |
| `D-feature-priority`, `D-identity` | Racontez la dernière session connectée perdue ou bloquée. Qu'avez-vous fait ensuite ? |
| `D-ai-layer` | Combien vous coûte un run d'agent aujourd'hui, et comment le savez-vous ? Avez-vous déjà figé un parcours pour ne plus appeler le modèle ? |
| `D-agent-interface` | Par où votre agent appelle-t-il ses outils : MCP, CLI, SDK, code généré ? Qu'avez-vous essayé avant ? |
| `D-identity` | Un site vous a-t-il déjà demandé de vous identifier, ou bloqué votre robot déclaré ? Qu'avez-vous fait ? |
| Conformité (`D-segment`) | Qu'a demandé votre service juridique ou sécurité avant la mise en production ? |

**Clôture (3 min).** Y a-t-il une question que j'aurais dû poser ? Peut-on vous recontacter pour un test de prix ? Enchaîner, si la personne accepte, sur le [questionnaire de prix](pricing-questionnaire.md).

## 3. Pièges à éviter

- Ne pas présenter Automan avant la fin : l'entretien porte sur leur histoire.
- Ne pas demander « utiliseriez-vous… » ou « paieriez-vous… » : demander ce qui s'est passé et combien ça a coûté.
- Relancer chaque généralité (« c'est souvent lent ») par un cas précis (« la dernière fois, c'était quand ? »).
- Noter les chiffres tels quels, avec leur unité (par mois, par page, par tâche).

## 4. Codage et restitution

- Transcription pseudonymisée sous `research/market/interviews/int-NN.md` (aucun nom de personne ni d'entreprise cliente).
- Chaque passage codé avec les codes feuilles du [codebook](codebook.md), plus quatre codes propres aux entretiens : `force-push`, `force-pull`, `force-anxiety`, `force-habit`.
- Un second codage indépendant sur un tiers des entretiens. Chaque code est noté présent ou absent par passage, ce qui permet un kappa de Cohen par famille de codes ; on vise κ ≥ 0,7, et une famille en dessous est recodée après mise au point des définitions. La piste documentaire, multi-étiquettes, n'avait mesuré qu'un accord brut ([codebook](codebook.md), §2).
- Restitution par segment : forces dominantes, déclencheurs datés, budgets réels, critères d'élimination des concurrents. Chaque constat cite ses entretiens (`int-NN`) et met à jour la ligne `evidence` des décisions concernées.

**Seuil de décision pour `D-segment`.** Un segment est retenu s'il réunit au moins trois entretiens avec un déclencheur daté lié à un mécanisme mesuré au labo (faux succès, session, coût de rejeu, politique d'accès) et un budget réel déjà dépensé sur le poste.
