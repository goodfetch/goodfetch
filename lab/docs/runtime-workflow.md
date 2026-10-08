# Workflow natif et qualification transversale

Le lab possède un coordinateur **Rust/Tokio natif**, avec HTTP reqwest, extraction
des fixtures, validation par un oracle HTTP indépendant, canal de résultats borné,
writer synchrone séparé, transactions SQLite et ACK persisté. Le coordinateur Node
utilise fetch dans son propre processus et un writer Node séparé. Les deux writers
chargent la même bibliothèque SQLite 3.53.4 et le même binding SQL C générique ;
la logique de jobs/leases/receipts est implémentée séparément dans Node et Rust.

Cela qualifie un prototype HTTP de bout en bout. Le parser regex est spécifique au
corpus du lab. Ce code n'est pas encore un scraper universel, un moteur Chromium
intégré, ni une preuve de reprise d'effets externes.

## Reproduire

Sélectionner Node 25.5.0, puis depuis `/Users/jb/Dev/automan/lab` :

```sh
pnpm shared-build
pnpm workflow-build
pnpm check
node --test --test-concurrency=1 tests/*.test.ts tests/integration/*.test.ts
node tools/qualification-suite.ts
```

La suite exécute les domaines **en série** : workflows Node/Rust prolongés,
backends HTTP intégrés, reprise, actionnabilité navigateur, lecteurs/checkpoints
SQLite, transports HTTP et arrivées ouvertes. Les JSON de configuration permettant
de relancer chaque campagne sont conservés dans son dossier de suite.

Les campagnes unitaires utilisent `pnpm runtime <config.json>`,
`pnpm actionability 3` et `pnpm sqlite-readers 3`. Les résultats ont des noms uniques
dans `lab/results`, avec manifests, sources figées, bruts et états relus. Les
anciennes campagnes restent disponibles ; aucune base de preuves n'est supprimée.

## Garanties vérifiées

- Les seeds créent des données fraîches dont l'oracle conserve les valeurs attendues.
- Chaque acquisition passe l'oracle avant d'entrer dans le writer.
- Au plus C jobs retiennent des résultats non acquittés ; un seul batch écrit à la fois.
- Résultats et état `committed` appartiennent à une transaction WAL FULL commune.
- L'ACK est une transaction distincte ; un receipt identique peut être rejoué.
- Une lease remplacée clôture les commits de l'ancien worker ; les données divergentes
  sont refusées lors du rejeu.
- Les crashes avant/après commit et pendant/après ACK sont injectés par SIGKILL.
- Un nouveau processus coordinateur est testé après claim ou entre commit et ACK.
- La vérification finale rouvre la base depuis le harness et contrôle chaque record,
  le nombre de jobs, l'intégrité et les clés étrangères.

L'injection d'arrêt du coordinateur comporte une pause de test de 50 ms après
l'événement choisi, afin de supprimer la course entre le signal et l'ACK. Cette
pause est retirée avant la reprise. Elle n'est pas activée dans les mesures normales.

## Métrologie et limites

Un thread d'événements coordonne chaque pipeline natif. Les transports HTTP, bindings,
encodeurs JSON et allocations restent différents : ce sont des comparaisons
d'implémentations, pas des propriétés isolées du langage. Les traces sont envoyées
au harness ; la piste Node évite une deuxième rétention des événements dans le core.
La configuration Rust est partagée par `Arc` entre producteurs ; la première
série qui copiait tout le corpus par worker et son binaire sont conservés pour
comparer l'effet de cette correction.

Startup, travail, drain et oracle sont distingués. Le débit complet avec startup et
oracle est fourni séparément. La latence par job commence à la première lease et
n'inclut pas son attente préalable dans le backlog durable. Ce backlog est fini,
soumis avant le chronométrage du travail ; l'admission continue du workflow reste
à qualifier. Les arrivées ouvertes sont actuellement testées dans la piste acquisition.

Les ressources du workflow natif couvrent le coordinateur et ses descendants ;
fixtures et oracle sont exclus. La sonde macOS utilise `proc_pid_rusage` V4 avec
conversion Mach via `mach_timebase_info`, validée contre `getrusage`. Pour un
coordinateur déjà actif, ses compteurs initiaux sont soustraits. CPU/I/O restent
des compteurs échantillonnés qui peuvent manquer la fin d'un processus. Les sommes
RSS comptent certaines pages partagées plusieurs fois ; les pics courts peuvent
être manqués. Les compteurs disque sont attribués par le kernel, pas une preuve
de persistance après coupure électrique.

Les tests d'actionnabilité séparent **primitives natives** et **primitives avec garde
commune** : unicité du selector, visibilité, activation, stabilité et hit-test. Une
action bloquée doit être refusée sans effet ; un clic réussi doit toucher la bonne
cible avec un événement trusted. Cette garde couvre les fixtures du lab, sans
prouver toute l'actionnabilité Playwright, les frames, le shadow fermé ou les races
possibles après la dernière vérification.

Les essais restent locaux sur macOS arm64 partagé. Linux réservé, réseau distant,
TLS/HTTP2, proxies, corpus indépendant de sites, agents/LLM, heartbeat, migrations,
disque plein et effets externes demandent des qualifications supplémentaires.

## Diagnostics conservés

La première suite du 8 octobre, dossier
`qualification-integrated-2026-10-07T23-16-32-138Z-d5bec9`, a été interrompue après
la découverte d'une mauvaise unité dans la première sonde CPU. Ses compteurs Mach
étaient traités comme des nanosecondes. Elle est exclue de la sélection principale ;
son `interrupted.json`, son manifest et ses sources figées expliquent l'écart.

Le harness a aussi corrigé un timer de fermeture de 400 ms qui restait référencé
après la sortie du worker. Le test de fermeture vérifie qu'aucun timer référencé
ne maintient artificiellement le superviseur en vie.
