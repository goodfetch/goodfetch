"""Render the report from audited campaign aggregates; no benchmark execution."""
import json, sys
from pathlib import Path
root=Path(__file__).resolve().parent.parent
analysis=json.loads((root/'reports/integrated-analysis.json').read_text())
current=Path(sys.argv[1]).resolve()
cs=json.loads((current/'summary.json').read_text())
campaigns=analysis['campaigns']
def by(prefix):return next(v for k,v in campaigns.items() if k.startswith(prefix))
def fmt(x,n=2):return '—' if x is None else f'{x:,.{n}f}'.replace(',', ' ').replace('.', ',')
def link(v):return f"[résumé](../{v['directory']}/summary.json), [manifest](../{v['directory']}/manifest.json)"
def table(headers,rows):return '\n'.join(['| '+' | '.join(headers)+' |','| '+' | '.join(['---']*len(headers))+' |',*['| '+' | '.join(map(str,r))+' |' for r in rows]])
native=by('runtime-native-qualified-');recovery=by('runtime-recovery-qualified-');guard=by('browser-actionability-');readers=by('sqlite-readers-');
# The first SQLite series is diagnostic; select the current step explicitly.
reader_dir=next(s['directory'] for s in cs['steps'] if s['name']=='sqlite-readers-qualified')
readers=campaigns[Path(reader_dir).name]
spider=by('spider-http-transport-supported-')
sections=[]
def add(s):sections.append(s.strip())
add('''# Qualification transversale Automan — 8 octobre 2026

**Rust/Tokio reste le candidat pour le noyau portable.** Node/TypeScript est le témoin expérimental et la voie de compatibilité. Le workflow natif Rust atteint un débit proche de Node avec moins de CPU et de footprint mesurés ; aucune de ces campagnes ne qualifie encore un pilote Chromium/CDP Rust.

Le prototype exécute **acquisition → extraction → validation → queue bornée → writer batché → commit → ACK**, séparément dans Node et Rust. Le writer est un processus dédié. Les deux ledgers chargent la même bibliothèque SQLite 3.53.4 et le même shim SQL C générique, en WAL FULL ; leur logique de jobs est indépendante. L'extraction est spécialisée pour le corpus du lab.

La première suite couvre les cinq domaines demandés, y compris les transports et la saturation. Une seconde suite requalifie les corrections de configuration Rust, stabilité des clics et initialisation SQLite. Les campagnes sont séquentielles ; les données et diagnostics initiaux restent conservés.''')
add(f"[Première suite](../{analysis['suite']}/summary.json), [requalification](../{current.relative_to(root)}/summary.json). Sources/binaires inchangés pendant les campagnes retenues. [Analyse reproductible](integrated-analysis.json), [catalogue SHA-256](integrated-evidence-catalog.json), [audit des sources](integrated-source-audit.json), [audit des contrats](integrated-verification.json).")
add('## Runtime complet : Rust conserve son intérêt')
add(f"**{native['statuses'].get('verified',0)}/30 cellules vérifiées**, trois blocs randomisés, **{native['jobs']:,} jobs / {native['records']:,} records**. C × 192 jobs par cellule, 16 pages × 2 records/job, délai serveur 4 ms/page, C1/4/16, batch 1/4 avec B ≤ C. Fenêtres de travail d'environ quinze secondes. Débit pondéré = jobs exacts / temps travail + finalisation. Le débit complet ajoute startup et oracle externe.")
nrows=[]
for c,b in [(1,1),(4,1),(4,4),(16,1),(16,4)]:
 n=next(g for g in native['groups'] if g['config']['runtime']=='node' and g['config']['concurrency']==c and g['config']['batchJobs']==b)
 r=next(g for g in native['groups'] if g['config']['runtime']=='rust' and g['config']['concurrency']==c and g['config']['batchJobs']==b)
 nrows.append([c,b,fmt(n['weightedJobsPerSecond']),fmt(r['weightedJobsPerSecond']),fmt(n['completeJobsPerSecond']),fmt(r['completeJobsPerSecond']),fmt(n['cpuMsPerJob']),fmt(r['cpuMsPerJob'])])
add(table(['C','Jobs/batch','Node jobs/s','Rust jobs/s','Node complet/s','Rust complet/s','Node CPU ms/job','Rust CPU ms/job'],nrows))
add('''À C16/B1 : environ 197 jobs/s pour les deux implémentations ; Rust utilise 1,42 ms CPU/job contre 2,78 pour Node, soit environ 49 % de CPU mesuré en moins. Footprint maximal échantillonné par cellule : Rust 136–178 MiB, Node 314–325 MiB. Cela soutient Rust pour l'efficacité à garanties testées comparables, sans prouver un scraper universellement plus rapide.

La copie de tout le corpus de configuration par producteur Rust a été remplacée par un partage immuable `Arc`. La première série C16/B1 donnait 205,55 Rust / 205,30 Node jobs/s et 204–248 / 306–327 MiB de footprint. Les deux séries ont été réalisées à des moments différents sur un Mac partagé : le changement est réel dans le code, mais cette différence de mémoire et de débit n'est pas un essai causal isolé du seul `Arc`. La première série et son binaire sont préservés. Le batching B4 du workflow complet ne gagne pas systématiquement sur B1 : acquisitions, taille réelle des batches et drain comptent aussi.''')
add(link(native)+'\n\n[Première série native](../results/runtime-native-sustained-2026-10-07T23-21-59-298Z-bf6ecb/summary.json), [binaire Rust précédent](evidence-integrated/artifacts/automan-workflow-rust-before-arc).')
add('## Acquisition HTTP : comparer les cycles de vie et la saturation')
back=campaigns['runtime-http-backends']
add(f"**{back['statuses'].get('verified',0)}/24 cellules**, {back['jobs']:,} jobs / {back['records']:,} records dans le workflow durable. Le runtime est Node lorsqu'un backend externe est utilisé ; les ressources comprennent alors ses descendants. Rust natif est une autre implémentation complète. Un pool partagé et un crawler neuf par tâche ont des coûts différents.")
add(table(['Runtime/backend','C','Jobs/s pondéré','Min–max par bloc','CPU ms/job'],[[g['config']['runtime']+'/'+g['config'].get('backend','native'),g['config']['concurrency'],fmt(g['weightedJobsPerSecond']),fmt(g['blockJobsPerSecond']['min'])+'–'+fmt(g['blockJobsPerSecond']['max']),fmt(g['cpuMsPerJob'])] for g in sorted(back['groups'],key=lambda g:g['key'])]))
add('''Des écarts importants entre blocs apparaissent, particulièrement pour Crawlee pooled et certaines cellules C16. Le diagnostic du host indique d'autres activités lourdes ; il documente un facteur de confusion possible et ne prouve pas la cause de chaque ralentissement. Aucune pénalité générale de runtime ne peut être déduite de ces seules variations.''')
add(link(back)+'\n\n[Diagnostic de charge](host-load-2026-10-08.json).')
add('''Les campagnes open-loop offrent 20/100/250 tâches/s, concurrence 4, trois fenêtres de 12 s, huit pages avec 3 ms de délai. Elles concernent l'acquisition seule, sans ledger durable. Une tâche refusée compte dans le dénominateur ; les tâches admises peuvent finir après la fenêtre. `queueMs` mesure le retard du générateur, sans queue métier cachée.''')
openrows=[]
for rate in (20,100,250):
 v=campaigns[f'http-open-{rate}']
 for s in v['summary']['summaries']:
  m=s['measurement'];openrows.append([rate,s['backend'],m['verified'],m['attempts'],m['rejected'],fmt(s['throughput']['verifiedTasksPerSecond']),fmt(m['successLatencyMs']['p95'])])
add(table(['Offert/s','Backend','Exactes','Offertes','Rejetées','Exactes/s','p95 succès ms'],openrows))
add('''Node fetch et Spider HTTP constituent de bons chemins simples sur ce corpus. Le pool évite certains coûts d'un crawler neuf par tâche, sans effacer ses garanties et frais d'orchestration. Les rejets à 250/s exposent une capacité bornée ; une faible latence sur les seuls succès ne signifie pas que toute la charge a été servie.''')
add('\n\n'.join(link(campaigns[f'http-open-{r}']) for r in (20,100,250)))
add(f"Transport : Node/Crawlee neuf/Crawlee pooled vérifient chacun 15 conformités et 45 mesures dans la première série. Spider refuse les trois redirections loopback : sa politique SSRF native bloque les sauts vers une adresse privée, même si le départ local est intentionnel. Ce n'est pas une preuve qu'il ne sait pas suivre les redirections publiques. La politique n'a pas été contournée ; le [source Spider 2.53.9 conservé](evidence-integrated/artifacts/spider-2.53.9-website.rs) contient `is_ssrf_redirect` et `ssrf_limited_policy`. La série séparée normal/gzip/chunked/grande réponse donne **{spider['statuses'].get('verified',0)}/60 lignes vérifiées** ({spider['measureCount']} mesures normales ; les variantes sont des contrôles de conformité).")
add(link(campaigns['http-transport'])+'\n\n'+link(spider))
add('## Navigateur : le gain brut de Stagehand exige des garanties explicites')
browser=campaigns['browser-actionability']
add('''Six backends × neuf variantes × trois blocs. La politique native n'ajoute que l'attente d'initialisation du document. Le guard commun impose unicité, visibilité, activation, stabilité sur deux intervalles entre trois observations RAF espacées d'au moins 16 ms, puis hit-test. L'oracle vérifie cible, ordre, phase et événement trusted. Pour les trois variantes négatives, il exige refus + zéro effet ; une cellule vérifiée n'implique donc pas toujours un clic.

Le guard initial avait quatre faux succès sur cible mobile. Un test public répété les a reproduits ; la correction a passé seize interactions puis la nouvelle matrice. La première tentative limitée à déplacer l'observation dans RAF a encore échoué : son log diagnostique reste conservé.''')
add(table(['Backend','Natif : conformes/27','Guard corrigé : conformes/27','Variantes natives non conformes'],[[g['key'].split('/')[0],g['statuses'].get('verified',0),next(x['statuses'].get('verified',0) for x in guard['groups'] if x['key'].split('/')[0]==g['key'].split('/')[0]),', '.join(v['variant'] for v in g['variants'] if v['statuses'].get('failed',0)) or '—'] for g in browser['groups'] if g['key'].endswith('/native')]))
add(table(['Backend gardé','p50 tâche normale ms','p50 cible mobile ms','p50 startup normal ms'],[[g['key'].split('/')[0],fmt(next(v for v in g['variants'] if v['variant']=='normal')['taskMs']['p50']),fmt(next(v for v in g['variants'] if v['variant']=='moving')['taskMs']['p50']),fmt(next(v for v in g['variants'] if v['variant']=='normal')['startupMs']['p50'])] for g in guard['groups']]))
add('''Le résultat historique de 50 clics (Stagehand 0,95 s / Playwright 2,71 s) utilisait des conditions communes de disponibilité. La nouvelle matrice montre pourquoi il ne faut pas le transformer en remplacement immédiat de Playwright : les primitives natives Stagehand testées échouent souvent sur overlay, activation, mutation et ambiguïté. Playwright, Crawlee/Playwright et l'extension Crawlee/Stagehand v3 passent cette matrice native ; le handler de l'extension utilise une page Playwright attachée par CDP.

Le guard corrigé fournit une autre configuration explicitement testée ; il ne prouve pas toutes les garanties de Playwright ni la suppression de la course entre garde et clic. Les refus et les clics réussis ont des latences distinctes. Stagehand v4 n'expose pas le même timeout de locator : budget des guards/Playwright/Puppeteer 800 ms, deadline extérieure Stagehand 8 s. Les startups sont séparés. Aucun appel LLM n'est effectué ; observe/act/extract et self-heal restent hors mesure.''')
add(link(browser)+'\n\n'+link(guard)+'\n\n[Contrat navigateur](../docs/browser-actionability.md), [régression rouge](qualification-guard-red.log), [tentative RAF encore en échec](qualification-guard-green.log), [régression corrigée](qualification-guard-calibrated-green.log).')
add('## SQLite : batches, writers, readers et checkpoints')
add(f"**{readers['statuses'].get('verified',0)}/108 cellules vérifiées**, **{readers['rows']:,} records**. Node N-API, Rust FFI synchrone et Go/cgo chargent le même artefact C : W1/4, B100/1000, lecteurs absents/courts/longs, trois blocs. Chaque cellule contient 32 000 lignes de 512 octets, WAL FULL, huit producteurs. Les fenêtres d'écriture sont courtes ; l'oracle relit chaque record et l'intégrité après fermeture.")
add(table(['Binding','Writers','Batch lignes','Lignes/s écriture','Lignes/s cycle complet','p95 RPC ACK ms'],[[*g['key'].split('/')[:2],g['key'].split('/')[3],fmt(g['weightedRowsPerSecond'],0),fmt(g['completeRowsPerSecond'],0),fmt(g['ackLatencyMs']['p95'])] for g in sorted(readers['groups'],key=lambda g:g['key']) if '/none/' in g['key']]))
add('''B1000 augmente le débit W1 dans les trois bindings. W4 n’est pas systématiquement plus lent : Node/B100 gagne dans cette série courte et Rust/B1000 est proche de W1. Les gros batches W4 augmentent toutefois nettement la latence RPC p95, sans gain de débit cohérent ; les résultats historiques et cette série motivent le writer unique comme base, pas une règle universelle.

Les 31 échecs de démarrage de la première matrice Rust/Go W4 surviennent avant insertion. Le schéma et le mode WAL étaient créés simultanément. L'initialisation commune est maintenant effectuée une seule fois avant d'ouvrir les vrais writers concurrents ; le test W4 avec reader long est passé de rouge à vert. Les writers restent indépendants pendant la fenêtre mesurée : la correction ne sérialise pas artificiellement leurs écritures.

Un reader long doit continuer à voir zéro ligne pendant les inserts. PASSIVE laisse alors des frames non checkpointées ; après libération, TRUNCATE réussit avec WAL à zéro. Les lecteurs courts effectuent des transactions de 2 ms toutes les 10 ms, avec compte croissant borné. Tous ces invariants sont contrôlés dans l'audit. Un writer dédié et des transactions batchées restent le point de départ soutenu par ces expériences ; plusieurs writers ne suppriment pas le verrou d'écriture unique de SQLite. [SQLite WAL](https://www.sqlite.org/wal.html).

Les p95 RPC viennent des ACKs bruts. Les mesures CPU/footprint de cette piste incluent le coordinateur déjà vivant, dont les compteurs initiaux sont soustraits ; ses allocations antérieures empêchent de classer la mémoire intrinsèque des langages. Les résultats JSON/JSONL, snapshot durable, NORMAL/FULL et SQLx historiques restent dans les rapports antérieurs, avec leurs contrats et builds séparés.''')
add(link(readers)+'\n\n[Contrat readers/checkpoints](../docs/sqlite-readers.md), [diagnostic des 31 échecs](sqlite-initialization-diagnostics.json), [régression rouge](qualification-writers-red.log), [six cas corrigés](qualification-writers-green.log), [écritures historiques](workflow-2026-10-08.md).')
add('## Durabilité : deux runtimes, mêmes invariants observables')
add(f"**{recovery['statuses'].get('verified',0)}/42 cellules vérifiées**, {recovery['jobs']:,} jobs / {recovery['records']:,} records, 42 redémarrages de writer et 12 redémarrages de coordinateur. SIGKILL avant commit, après commit, pendant ACK, après ACK ; nouveau coordinateur après claim ou commit ; trois fautes répétées avec writer ralenti. La série initiale avait également 42/42 cellules vérifiées.")
add(table(['Runtime/cas','Cellules','Redémarrages writer','Redémarrages coord.','Jobs/s cycle complet','p50 reprise coord. ms'],[[g['key'],g['statuses'].get('verified',0),g['restarts'],g['coordinatorRestarts'],fmt(g['completeJobsPerSecond']),fmt(g['coordinatorRecoveryMs']['p50'])] for g in sorted(recovery['groups'],key=lambda g:g['key'])]))
add('''Les résultats et le receipt sont commités avant ACK, puis ACK est enregistré dans une transaction distincte. Le rejeu d'un job déjà commité réutilise le receipt ; les anciens tokens de lease sont rejetés. L'oracle externe rouvre la base après arrêt et compare chaque record. Les checkpoints au moment du crash attestent un claim sans résultat ou un commit avec résultats avant la reprise.

Pour la reprise de coordinateur, les débits utilisent le temps complet extérieur : le temps de travail du processus repris seul omettrait une partie de l'incident. La queue de résultats reste bornée par C même avec writer lent. Ces essais prouvent les invariants du prototype sur crashes de processus ; ils ne qualifient pas coupure électrique, effet externe exactly-once, heartbeat ou cluster cloud.''')
add(link(recovery)+'\n\n[Workflow et reproduction](../docs/runtime-workflow.md).')
add('## Vérification, reproductibilité et choix encore ouverts')
add('''L’audit post-campagne passe **510 contrôles**, dont effets trusted/refus, bornes, redémarrages, checkpoints et sources figées. Il complète les oracles de records exécutés dans les campagnes.

Les **57 tests de contrat passent**, zéro échec/skip, dont faux succès, protocole/timeout, reprise des deux runtimes, fencing, calibration CPU macOS, clic mobile et startup W4. TypeScript et format Rust passent ; le binaire Rust actuel a été reconstruit avant requalification. [Log des tests](qualification-current-tests.log), [build](qualification-current-build.log).

Les manifests fixent versions, seeds, environnement, SHA des sources et binaires. Les sources sont copiées avant les essais et auditées à nouveau après. Le catalogue référence les preuves originales sans exporter les inputs privés de l'oracle ; les bases et exports complets restent dans les dossiers de résultats. L'analyse agrège les événements bruts pour les quantiles, sans moyenner des p95 de blocs. Les erreurs des comparateurs et les erreurs initiales du lab restent distinguées.

La sonde macOS est calibrée contre getrusage et convertit les compteurs Mach avec `mach_timebase_info`. Elle mesure le coordinateur natif et descendants, exclut fixture/oracle dans la piste native, échantillonne à 100 ms et peut manquer des pics ou fins de processus. Le RSS peut compter des pages partagées plusieurs fois ; footprint et octets I/O sont des indicateurs distincts. La première série aux unités CPU erronées a été interrompue et exclue. Un timer de fermeture de 400 ms a été corrigé avant les campagnes retenues.

Le lab soutient aujourd'hui : poursuivre Rust pour le noyau HTTP durable ; conserver Node pour référence et compatibilité ; préférer un chemin HTTP simple lorsque le rendu n'est pas nécessaire ; garder Playwright comme référence d'interaction robuste et évaluer Stagehand avec ses garanties explicites ; commencer SQLite avec un writer et des transactions batchées. Ce sont des directions de prototype fondées sur ce corpus, pas une architecture finale figée.

Il reste à qualifier un pilote Rust/CDP co-localisé face à Node, Linux dédié, TLS/HTTP2/proxies/429 et retry, parsing général/streaming, corpus holdout, sessions multi-tenant, admission continue/heartbeat, sinks supplémentaires et perception/LLM avec budgets identiques. Les prochaines expériences doivent lever ces incertitudes précises avant toute promesse de performance universelle.''')
add('[Matrice de couverture](../docs/coverage-matrix.md), [gates et preuves](../docs/qualification-next.md), [index de toutes les expériences](index.html).')
(root/'reports/integrated-2026-10-08.md').write_text('\n\n'.join(sections)+'\n')
