# Questionnaire de prix : Van Westendorp et Gabor-Granger

**Statut :** v1, prêt à l'emploi. **Date :** 2026-10-08.
**Objet :** mesurer la zone de prix acceptable et la probabilité d'achat pour l'unité `verified-result` de `D-pricing`, et la comparer aux deux autres options (`compute-units`, `subscription-tiers`). À poser à la fin de l'[entretien](./interview-guide.md), puis en ligne pour élargir l'échantillon.

## 1. Définir l'unité avant toute question

Une question de prix sans unité définie ne mesure rien. Lire mot pour mot l'une des deux définitions selon le segment :

- **Lecture** (`data-teams`, `ecommerce-pricing`) : « Un *résultat vérifié* est une page lue dont les données ont passé un contrôle d'exactitude. Une page bloquée, incomplète ou en échec n'est jamais facturée. Unité : 1 000 pages vérifiées. »
- **Tâche** (`agent-developers`, `b2b-portals`) : « Un *résultat vérifié* est une tâche d'agent menée à bout, dont l'effet (formulaire envoyé, données récupérées) est contrôlé. Une tâche en échec ou en faux succès n'est jamais facturée. Unité : 1 000 tâches vérifiées. »

Préciser ensuite le contexte : hébergement compris, sans engagement, plafond de dépense mensuel fixé par le client.

## 2. Van Westendorp (quatre questions, réponses libres en dollars)

Poser les questions dans cet ordre, sans proposer de valeurs :

1. **Trop cher.** À quel prix pour 1 000 résultats vérifiés jugeriez-vous l'offre si chère que vous ne l'envisageriez pas ?
2. **Cher.** À quel prix commenceriez-vous à la trouver chère, tout en l'envisageant encore ?
3. **Bon marché.** À quel prix la trouveriez-vous avantageuse ?
4. **Trop bon marché.** À quel prix serait-elle si peu chère que vous douteriez de sa qualité ?

Contrôle de cohérence : on attend trop bon marché ≤ bon marché ≤ cher ≤ trop cher. Une réponse incohérente est écartée de l'analyse et comptée à part.

**Analyse.** Tracer les distributions cumulées. On utilise les deux intersections usuelles :
- **borne basse** : trop bon marché croise cher ;
- **borne haute** : trop cher croise bon marché.

Entre les deux se trouve la zone de prix acceptable. Les courbes ne sont stables qu'à partir d'environ 30 réponses par segment. En dessous, avec les seuls 10 à 15 entretiens, on rapporte les réponses individuelles et leur médiane, sans calculer de point optimal.

## 3. Gabor-Granger (probabilité d'achat à des prix imposés)

Échelle de réponse : 1 *certainement pas*, 2 *probablement pas*, 3 *peut-être*, 4 *probablement*, 5 *certainement*. Commencer à un échelon tiré au sort, puis monter si la réponse est 4 ou 5, descendre sinon. Ne pas montrer toute l'échelle.

| Unité | Échelons (USD pour 1 000 résultats vérifiés) | Ancrage observé dans la piste marché |
| --- | --- | --- |
| 1 000 pages vérifiées | 0,50 · 1 · 2 · 4 · 8 · 15 | ~2 $/1 k pages demandés (`reddit-81`) ; 0,005 $/page affichés (`demand-use-cases-49` `[P]`) |
| 1 000 tâches vérifiées | 5 · 10 · 25 · 50 · 100 · 250 | 0,01-0,05 $/tâche en plan rejoué (`reddit-62` `[P]`) ; ~0,21 $/tâche chez Notte (`hn-75` `[P]`) |

Les ancrages servent à placer l'échelle, pas à suggérer un prix au répondant.

**Analyse.** Pour chaque échelon, compter la part de réponses 4 ou 5. Le revenu relatif vaut prix × part. On rapporte la courbe de demande et l'échelon qui maximise ce revenu relatif, par segment, avec l'effectif.

## 4. Unité préférée et plafond

- **Classement des unités.** Classez ces trois façons de payer, de la préférée à la moins aimée : au résultat vérifié, aux ressources consommées détaillées, à l'abonnement mensuel plafonné. Pourquoi la première ? (`D-pricing`)
- **Prime à la vérification.** Si un concurrent facture 1 000 pages au prix X, échecs compris, combien accepteriez-vous de payer en plus pour ne payer que les résultats vérifiés ? Réponse libre, en pourcentage.
- **Plafond.** Quel plafond mensuel fixeriez-vous dès le premier mois ?
- **Dernier incident de facturation.** Racontez la dernière facture qui vous a surpris : montant, cause, ce que vous avez fait.

## 5. Mise en ligne et suite

- Version en ligne : un formulaire anonyme, sans adresse obligatoire, avec l'écran de sélection de l'entretien et les sections 1 à 4. Objectif : 30 réponses cohérentes par segment retenu.
- Ensuite, test de prix sur une page d'atterrissage : deux variantes (résultat vérifié contre abonnement plafonné), avec pour mesure le taux d'inscription sur liste d'attente ([pricing-signals.md](./pricing-signals.md), §5).
- Seul un PoC payant constitue une preuve de paiement. Les réponses à ce questionnaire bornent les prix à tester, elles ne fixent pas le tarif.
- Les résultats vont sous `research/market/pricing-results.md` et dans `evidence` de `D-pricing`.
