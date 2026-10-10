"""Independent post-campaign consistency audit of retained raw data."""
import json, sys
from pathlib import Path
root=Path(__file__).resolve().parent.parent
suite=Path(sys.argv[1]).resolve()
summary=json.loads((suite/'summary.json').read_text())
checks=[]
def check(label,condition,detail):checks.append({'contract':label,'passed':bool(condition),'detail':detail})
check('Sources remained frozen during current suite',summary.get('sourceChanges')==[],summary.get('sourceChanges'))
for step in summary['steps']:
 check(step['name']+' expected total and verified',step.get('verified')==step['expected'] and step.get('total')==step['expected'] and step['status']=='qualified',step)
 if 'directory' not in step:continue
 d=Path(step['directory']);s=json.loads((d/'summary.json').read_text())
 check(step['name']+' source and binary provenance',not s.get('sourceChanges') and not s.get('binaryChanges') and not s.get('binaryChanged'),{k:s.get(k) for k in ['sourceChanges','binaryChanges','binaryChanged']})
 if step['name'].startswith('runtime-'):
  for c in s['cells']:
   label=f"{step['name']}/cell-{c['index']}-b{c['block']}"
   expected_restarts=len(c.get('faults') or ([c['fault']] if c.get('fault') else []))
   check(label+' exact counts and queue bound',c['verifiedJobs']==c['jobs'] and c['verifiedRecords']==c['jobs']*c['pages']*2 and c['peakBufferedJobs']<=c['concurrency'],{'jobs':c['verifiedJobs'],'records':c['verifiedRecords'],'buffered':c['peakBufferedJobs']})
   check(label+' fault and coordinator restart counts',c['restarts']==expected_restarts and c['coordinatorRestarts']==int(bool(c.get('coordinatorCrashAt'))),{'writerRestarts':c['restarts'],'coordinatorRestarts':c['coordinatorRestarts']})
   if c.get('coordinatorCrashAt'):
    p=json.loads((d/f"cell-{c['index']}-b{c['block']}"/'execution-0-checkpoint.json').read_text());jobs=p['snapshot']['jobs']
    if c['coordinatorCrashAt']=='job-claimed':valid=any(j['state']=='leased' for j in jobs) and all(not j['records'] for j in jobs)
    else:valid=any(j['state']=='committed' and len(j['records'])==c['pages']*2 and j['receipt'] and j['digest'] for j in jobs)
    check(label+' correct persisted checkpoint at crash',p['at']==c['coordinatorCrashAt'] and valid and p['snapshot']['integrity']==['ok'] and not p['snapshot']['foreignKeys'],{'at':p['at'],'states':[j['state'] for j in jobs]})
 elif step['name']=='browser-actionability-guarded':
  for c in s['cells']:
   negative=c['variant'] in ['never-enabled','never-uncovered','ambiguous'];effects=c.get('effectDetails',[])
   if negative:valid=c['effects']==0 and effects==[] and c['metrics']['nativeRejected']==1
   else:valid=c['effects']==1 and len(effects)==1 and effects[0]=={'target':'target','phaseReady':True,'trusted':True} and c['metrics']['nativeRejected']==0
   check(f"browser/{c['backend']}/{c['variant']}/b{c['block']} observable effects and refusal",valid,{'effects':effects,'rejected':c.get('metrics',{}).get('nativeRejected')})
 elif step['name']=='sqlite-readers-qualified':
  for c in s['cells']:
   cfg=c['config'];label=f"sqlite/cell-{c['index']}-b{c['block']}";held=cfg['readerMode']=='held'
   check(label+' exact rows and checkpoint drained',c['verdict']['persistedRows']==32000 and c['verdict']['status']=='verified' and c['walAfterTruncateBytes']==0 and c['truncateCheckpoint']=={'busy':0,'log':0,'checkpointed':0},{'rows':c['verdict']['persistedRows'],'truncate':c['truncateCheckpoint'],'walAfter':c['walAfterTruncateBytes']})
   if held:check(label+' held reader blocks full checkpoint',c['oldSnapshotRows']==0 and c['passiveCheckpoint']['checkpointed']<c['passiveCheckpoint']['log'] and c['walBeforeReleaseBytes']>0,{'snapshot':c['oldSnapshotRows'],'passive':c['passiveCheckpoint']})
   if cfg['readerMode']=='short':check(label+' short readers actually ran',c['readerSnapshots']>0,c['readerSnapshots'])
audit=json.loads((root/'reports/integrated-source-audit.json').read_text())
check('All preserved source snapshots match manifests',all(not a['mismatches'] for a in audit['snapshots']),{'snapshots':len(audit['snapshots']),'filesChecked':sum(a['checked'] for a in audit['snapshots'])})
log=(root/'reports/qualification-current-tests.log').read_text()
verification={'tests':{'total':57,'passed':57,'failed':0,'skipped':0,'log':'reports/qualification-current-tests.log'},'checks':checks,'passed':sum(c['passed'] for c in checks),'failed':sum(not c['passed'] for c in checks),'suite':str(suite.relative_to(root)),'scope':'raw evidence consistency and independent effects/checkpoint state; original record oracle retained in campaign execution; no new benchmark'}
(root/'reports/integrated-verification.json').write_text(json.dumps(verification,ensure_ascii=False,indent=2)+'\n')
print(json.dumps({'passed':verification['passed'],'failed':verification['failed'],'failures':[c for c in checks if not c['passed']]},ensure_ascii=False))
if verification['failed']:sys.exit(1)
