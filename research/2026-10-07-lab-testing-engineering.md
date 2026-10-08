# Lab Automan — ingénierie des tests et des mesures

**Recherche du 7 octobre 2026.** Sources primaires consultées : documentation des outils, travaux des auteurs, Google Testing/SRE, ACM et NIST. Les prescriptions « maintenant » sont notre adaptation au [lab proposé](../docs/proposition-lab-automan.md), pas une norme universelle ni une preuve de performances déjà obtenue.

## 1. La bonne cible : résultat correct sous contrat explicite

Séparer trois questions : le harness mesure-t-il correctement ; chaque backend remplit-il le contrat ; avec quelles ressources et durées le remplit-il ? Un appel retourné sans erreur ne suffit pas. WebArena vérifie la correction fonctionnelle des tâches et l'état obtenu plutôt que l'identité d'une séquence d'actions. [Article et environnement des auteurs](https://webarena.dev/og/).

**Maintenant :** chaque scénario porte une entrée, une postcondition, une deadline et un oracle indépendant. Compter `verified`, `failed`, `timeout`, `unsupported`, `partial` et effets inconnus explicitement ; ne pas mélanger déclaration de l'adapter et verdict du validateur. Un scénario rapide mais incorrect reste dans les échecs et son coût dans les dépenses.

Le runner obtient la vérité depuis le serveur fixture : ensemble complet d'IDs et de valeurs, version, journal d'événements et d'effets, checksum d'un fichier. L'adapter renvoie seulement les observations qu'il a réellement obtenues. Les données attendues ne sont pas transmises avec la commande. Pour une fixture locale, cette indépendance de code n'est pas une isolation de sécurité contre un agent ayant accès à tout le filesystem : les évaluations IA futures devront aussi limiter les outils exposés.

## 2. Choisir les tests par risque, coût et fidélité

La pyramide est une heuristique ; Google propose désormais aussi SMURF : vitesse, maintenabilité, utilisation des ressources, fiabilité et fidélité. Le ratio historique 70/20/10 n'est pas une obligation applicable à tous les projets. [Google, SMURF](https://testing.googleblog.com/2024/10/smurf-beyond-test-pyramid.html), [pyramide originale](https://testing.googleblog.com/2015/04/just-say-no-to-more-end-to-end-tests.html).

| Couche | Ce que nous testons maintenant | Ce qu'elle permet de diagnostiquer |
| --- | --- | --- |
| Tests petits | Validation de protocole, IDs, statistiques, parsing, état des tentatives, erreurs d'oracle | Bug du lab sans lancer Chromium |
| Intégration locale | Serveur fixture réel + adapter ; JSONL fragmenté ; timeout et fermeture | Contrat entre composants et cycle de vie |
| Conformance navigateur | Clic/saisie/attente, DOM retardé ou remplacé, cible ambiguë, journal d'effets | Garantie réelle des primitives |
| Workflow | Petit crawl complet, formulaire et extraction rendue | Pipeline utile de bout en bout |
| Performance/stress | Charge, saturation, froid/chaud, répétitions | Coût et capacité après correction fonctionnelle |

Pas de test qui vérifie seulement qu'une chaîne constante est présente. Préférer une assertion sur une propriété observable ; vérifier les cas limites pertinents. Une couverture de lignes élevée prouve l'exécution, pas que les assertions détectent une erreur. [Stryker, explication de la mutation](https://stryker-mutator.io/blog/introduction-to-mutation-testing/).

## 3. Fixtures hermétiques et isolation

Google décrit des serveurs hermétiques avec connexions configurables, données de test et logs, permettant des tests locaux reproductibles. Il précise leurs coûts de lancement et limites de taille ; hermétique ne veut pas dire gratuit. [Hermetic Servers](https://testing.googleblog.com/2012/10/hermetic-servers.html).

**Maintenant :** serveur sur loopback et port attribué par l'OS ; corpus et graine connus ; état privé par `runId` ; base URL transmise explicitement ; profil navigateur propre au run ; queue/storage Crawlee propre à la campagne. Tests indépendants de leur ordre, sans dépendance à une page publique, un cookie personnel ou une clé IA.

Prévoir reset explicite, fermeture dans `finally`, readiness par événement/endpoint, et journal exploitable. Le reset ne doit pas effacer les résultats précédents : nouveau dossier de campagne et nouvelle identité suffisent. Le compte, les IDs, délais, mutations et graphes sont contrôlés par le corpus.

**Ensuite :** distinguer validation externe sur sites publics et campagne locale. Les services cloud nécessitent des fixtures accessibles et une région/réseau documentés. Leur variabilité fait partie d'une série séparée.

## 4. Tester le lab avec des témoins positifs et négatifs

La mutation vérifie que les tests échouent lorsqu'un défaut est introduit ; les mutants équivalents empêchent d'assimiler un score de 100 % à une garantie absolue. [Stryker, principe](https://stryker-mutator.io/docs/), [mutants équivalents](https://stryker-mutator.io/docs/mutation-testing-elements/equivalent-mutants/).

**Maintenant :** pas besoin d'ajouter un moteur de mutation à tous les adapters. Une suite de mauvais adapters/test doubles couvre les erreurs les plus dangereuses :

- Un témoin correct passe ; zéro résultat, un ID manquant, doublonné, hors domaine, une valeur périmée ou un faux message de succès échouent.
- Un adapter annonce `verified` sans effectuer le clic : le serveur n'a aucun effet, le verdict est un échec.
- JSON invalide, stdout pollué, ligne trop grande, réponse à un mauvais ID et réponse tardive n'entraînent ni faux succès ni attente illimitée.
- Crash après envoi, absence de réponse, refus de fermeture et fermeture concurrente donnent un état terminal explicite.
- `unsupported` réduit la couverture ; il n'ajoute aucune latence de zéro dans la distribution des succès.

Pour les statistics, utiliser des petits jeux numériques dont moyenne et quantiles sont calculables indépendamment ; inclure vide, un élément, valeurs égales et paramètres invalides. Pour un validateur de résultats, vérifier à la fois acceptation d'une sortie légitime et refus d'une sortie fausse.

## 5. Propriétés, modèles et transformations

fast-check génère des séquences de commandes, simplifie un contre-exemple et permet son replay par seed/path ; ses auteurs préviennent qu'un modèle copié sur l'implémentation peut reproduire le même bug. [Model-based testing](https://fast-check.dev/docs/advanced/model-based-testing/).

**Maintenant :** invariants simples avec graines sauvegardées, sans créer une seconde architecture : IDs uniques, ensemble attendu exact, compte des tentatives conservé, une seule réponse terminale par requête, reset sans fuite inter-run. Rejouer un contre-exemple enregistré devient un test de régression.

Les tests métamorphiques vérifient des relations nécessaires entre exécutions lorsque la sortie complète est difficile à connaître ; ces relations ne constituent pas une preuve de correction complète. [Travail des auteurs sur les relations métamorphiques](https://arxiv.org/html/2406.05397).

**Adaptations proposées :** changer l'ordre des liens ou ajouter des cycles ne change pas l'ensemble d'entités d'un crawl dont le contrat ignore l'ordre ; découper un message JSONL en fragments arbitraires ne change pas le message reçu ; insérer un élément décoratif sans modifier le rôle/nom accessible ne change pas la cible d'un locator sémantique. Ces relations doivent suivre le contrat : changer un label peut légitimement casser un locator exact.

**Ensuite :** modèle de leases/retries/commit pour la durabilité ; scheduler contrôlé pour races ; fuzzing ciblé du protocole. Réserver le modèle aux états effectivement implémentés.

## 6. Interaction navigateur : attendre le bon événement

Playwright documente locators, contrôles d'actionability, assertions qui attendent et isolation des tests. Il déconseille les délais fixes comme synchronisation, et signale le coût important des traces enregistrées partout. [Bonnes pratiques](https://playwright.dev/docs/best-practices), [attente temporelle](https://playwright.dev/docs/api/class-page#page-wait-for-timeout).

**Maintenant :** deadline totale, attentes par condition, event waiter armé avant l'action déclenchante, puis validation de l'effet. Les délais injectés dans la fixture sont des perturbations contrôlées ; ils ne doivent pas être copiés en sleeps dans l'adapter.

Conformance minimale : cible visible/unique, overlay temporaire, disabled devenu enabled, re-render qui invalide une référence, saisie avec événements `input`/`change`, clic et effet serveur. Une modification directe de `.value` n'est pas automatiquement équivalente à une saisie utilisateur ; le scénario dit quels événements sont nécessaires.

Séparer commande brute sur page déjà prête et action fiable dans page changeante. Si un adapter CDP ajoute des contrôles ou un resolver pour atteindre le contrat commun, mesurer aussi ces coûts. Garder diagnostics détaillés/traces dans un mode distinct des benchmarks ; captures sur échec ou replay diagnostic.

## 7. Supervision et mémoire bornées

Node documente que `kill()` envoie un signal et ne garantit pas l'arrêt de tous les descendants ; sur POSIX, un enfant `detached` devient leader d'un groupe/session. [Child process](https://nodejs.org/api/child_process.html). Les streams doivent respecter `write() === false` et attendre `drain`, ou utiliser `pipeline()` pour propager erreurs et fermeture. [Backpressure](https://nodejs.org/learn/modules/backpressuring-in-streams).

**Maintenant :** spawn direct sans shell, un processus adapter persistant, IDs de commandes, stdout réservé au protocole, stderr drainé, limites de taille/volume, startup/run/close deadlines. Fermeture idempotente ; arrêter les ressources créées par le run, puis TERM et escalade bornée si nécessaire ; surveiller les descendants navigateur et le résultat de l'arrêt. Ne pas utiliser un `pkill` global qui toucherait le navigateur de l'utilisateur.

Limiter tâches en vol, longueur de queue et buffers. Le serveur fixture et le runner ont aussi des limites. Une charge à arrivée imposée ne doit pas faire croître sans borne un tableau de promises : rejeter et compter l'admission lorsque la capacité est atteinte.

**Ensuite :** groupes/cgroups sur Linux, sous-processus de test qui ignorent TERM, navigateur interrompu au milieu d'une action, sink ralenti et grosse réponse. Les relevés RSS sont des observations avec méthode et pas une preuve de mémoire physique totale ; descendants et pages partagées comptent.

## 8. Charge ouverte, charge fermée et saturation

Le modèle fermé maintient une concurrence et ralentit ses arrivées avec le système ; le modèle ouvert impose des arrivées indépendantes des durées. k6 explique l'omission coordonnée et publie les itérations non lancées lorsque les ressources du générateur ne suffisent pas. [Modèles](https://grafana.com/docs/k6/latest/using-k6/scenarios/concepts/open-vs-closed/), [dropped iterations](https://grafana.com/docs/k6/latest/using-k6/scenarios/concepts/dropped-iterations/).

**Maintenant :** mode fermé explicite pour premiers stress, limites de concurrence et durée totale, tous les statuts comptés. Ne pas appeler ce mode une mesure de capacité à débit d'arrivée fixe.

**Dès le premier mode ouvert :** enregistrer pour chaque tâche arrivée prévue, admission, démarrage, fin, validation ; publier nombre prévu/démarré/terminé/rejeté, queue delay et retard du générateur. Conserver débit valide et échecs à côté des quantiles. Calibrer serveur/générateur séparément : leur saturation invalide une conclusion sur le crawler.

**Ensuite :** ramps, bursts, host lent, 429/Retry-After, backlog/rattrapage, fairness et soak. Trouver le débit soutenable avec qualité et backlog stable, plutôt que seulement le pic instantané. Le nombre de navigateurs reste un budget explicite, pas une boucle qui lance sans borne.

## 9. Répétitions et statistiques honnêtes

Google Benchmark distingue warmup, répétitions et interleaving aléatoire pour limiter l'effet d'un état système changeant. Kalibera/Jones analysent plusieurs niveaux de variation et recommandent de répéter au niveau qui produit l'incertitude. [Guide officiel](https://google.github.io/benchmark/user_guide.html), [réduction de variance](https://google.github.io/benchmark/reducing_variance.html), [article des auteurs](https://kar.kent.ac.uk/33611/).

**Maintenant :** horloges monotones ; froid/chaud séparés ; warmups déclarés hors résumé ; ordre randomisé par blocs avec seed ; mêmes scénarios et graines par bloc ; résultats bruts conservés. Comparer composants puis frameworks complets dans deux rails distincts. Le coût de spawn/init ne disparaît pas : publier startup et exécution.

Le bloc/processus est une unité pertinente de répétition ; 50 clics d'un même parcours ne sont pas 50 workflows indépendants. Un premier smoke prouve l'exécutabilité ; quelques répétitions donnent une exploration, pas un avantage robuste. La publication finale exigera une campagne dimensionnée selon sa variance et l'effet utile recherché.

Publier p50/p95/p99 empiriques avec `n`, population et méthode de quantile ; ne jamais moyenner les p95 de sous-groupes. Ne pas substituer durée nulle aux échecs ni leur donner arbitrairement la deadline comme latence observée. La latence des succès et le décompte de tous les états sont deux vues nécessaires. Les moyennes cachent les longues latences. [Google SRE](https://sre.google/workbook/monitoring/).

**Ensuite :** comparaison appariée par scénario/seed/bloc, intervalle d'effet calculé au niveau des blocs et courbes qualité/ressources. Figer corpus, réglage et seuil de promotion avant la campagne finale ; publier résultats défavorables et configurations optimisées séparément des défauts.

Avec zéro échec, la borne supérieure unilatérale exacte à 95 % est `1 - 0.05^(1/n)` sous hypothèse binomiale : **2,9513 %** d'échec à `n=100`, **0,09997 %** à `n=2 995`. Calcul vérifié avec Python ; il ne vaut pas pour essais corrélés d'une seule session ou pour généraliser à tous les sites. NIST décrit les intervalles binomiaux, notamment les cas à faibles effectifs/échecs. [Limites exactes](https://www.itl.nist.gov/div898/software/dataplot/refman2/auxillar/exacbici.htm).

## 10. Incidents et durabilité : vérifier les histoires

Jepsen raisonne sur les histoires d'opérations autorisées par un modèle de cohérence ; ses expériences contrôlées montrent l'intérêt de fautes ciblées et d'un validateur indépendant. [Modèles de cohérence](https://jepsen.io/consistency), [exemple de recherche de pertes d'écritures acquittées](https://jepsen.io/analyses/mongodb-3-4-0-rc3).

**Maintenant :** tester crash/hang/reprise du protocole adapter ; consigner tentative, réponse/absence, état terminal et effet serveur. Ne pas promettre durabilité d'un workflow parce qu'un worker redémarre ou qu'une queue existe.

**Ensuite, après implémentation d'une vraie persistance :** crash avant/après résultat, commit et ack, lease expirée et ancien worker vivant ; assertions sur résultats acquittés conservés, pertes, doublons, effets inconnus et fencing. Le contrat « effet unique » n'est testé que sur la fixture qui fournit l'idempotence requise ; un site tiers ne l'offre pas nécessairement.

## 11. IA : tâches, essais et oracles séparés

Anthropic distingue tâche, trial, grader, transcript et harness ; recommande environnement propre, référence qui passe et graders déterministes lorsque possible. Les ressources de l'environnement peuvent modifier les scores des agents. [Méthode d'évaluation](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents), [bruit d'infrastructure](https://www.anthropic.com/engineering/infrastructure-noise).

**Maintenant :** primitives déterministes sans provider. Le protocole prévoit coût et statut ; un faux provider ultérieur teste budgets et pannes, pas la compétence de l'agent.

**Ensuite :** données/graines/mutations finales tenues hors réglage, oracle privé inaccessible aux outils de l'agent, caches et sessions par trial ; séries discovery froide, replay, repair et réparation refusée. Compter toutes tentatives, appels/tokens, budgets et versions du modèle ; vérifier valeurs fraîches et effets réels. Une réparation ne change pas l'oracle. Jugement LLM réservé aux aspects non vérifiables par code, avec rubric et calibration indépendante.

Anthropic a documenté des agents qui identifient un benchmark puis trouvent ses réponses ; cacher simplement un fichier derrière un format ou une obfuscation ne protège pas un oracle. [Recherche sur contamination et eval awareness](https://www.anthropic.com/engineering/eval-awareness-browsecomp).

## 12. Provenance et lot initial livrable

ACM distingue artifacts disponibles, évalués et résultats validés ; un artifact utile est documenté, cohérent, complet et exécutable. Publier des scripts et données ne prouve pas à lui seul une reproduction indépendante. [Politique ACM](https://www.acm.org/publications/policies/artifact-review-and-badging-current).

**Maintenant :** manifest avec commit/diff dirty ou hash des sources, locks/checksums, runtime/OS/CPU/arch, navigateur/révision/flags, adapter/config, scenario/oracle/corpus, seed/ordre, deadlines/concurrence, cache et métriques/méthode. Dossier unique, JSONL brut, summary JSON/CSV et commande de replay. Pas de secrets, prompts privés ou cookies personnels dans le manifest. Le rapport décrit les skips/unsupported et la portée exacte des mesures.

Le plus petit lot complet est : fixture serveur ; adapters Crawlee/Spider/Stagehand et témoins ; doctor/bootstrap ; smoke ; conformance indépendante ; runner borné ; campagne exploratoire froid/chaud et concurrence ; rapport reproductible ; tests des échecs du harness. Ajouter ensuite les surfaces navigateur, load ouvert, RTT, durabilité, soak et IA par contrats successivement exécutables. Pas besoin de cluster, dashboard avancé, broker distribué ou matrice entière avant cette première preuve.
