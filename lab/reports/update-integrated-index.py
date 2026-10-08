"""Add current evidence cards while preserving all previous campaigns."""
import json,re,sys,html
from pathlib import Path
root=Path(__file__).resolve().parent.parent
first=Path(sys.argv[1]);current=Path(sys.argv[2]);a=json.loads((root/'reports/integrated-analysis.json').read_text());cards=[]
for key,v in a['campaigns'].items():
 if 'directory' not in v:continue
 d=v['directory'];category='workflow' if key.startswith('runtime') else 'storage' if key.startswith('sqlite') else 'browser' if key.startswith('browser') else 'crawl'
 if 'groups' in v:
  count=sum(v['statuses'].values());ok=v['statuses'].get('verified',0);label='cellules vérifiées'
 else:
  count=v['rawCount'];ok=v['statuses'].get('verified',0);label='lignes vérifiées, toutes phases'
 titles={'runtime-native-sustained':'Runtime natif : première série','runtime-http-backends':'Workflow : backends HTTP','runtime-recovery':'Runtime : première reprise','browser-actionability':'Actionnabilité : native et guard initial','sqlite-readers':'Readers SQLite : diagnostic initial','http-transport':'Transports HTTP et redirection loopback','http-open-20':'HTTP : arrivées 20/s','http-open-100':'HTTP : arrivées 100/s','http-open-250':'HTTP : arrivées 250/s'}
 title=titles.get(key)
 if title is None:title='Runtime natif : Rust/Node corrigé' if key.startswith('runtime-native-qualified-') else 'Runtime : reprise requalifiée' if key.startswith('runtime-recovery-qualified-') else 'Actionnabilité : guard corrigé' if key.startswith('browser-actionability-') else 'SQLite : writers et readers requalifiés' if key.startswith('sqlite-readers-') else 'Spider : gzip, chunked et grandes réponses'
 states=' · '.join(f'{n} {k}' for k,n in v['statuses'].items())
 cards.append(f'<article data-category="{category}"><div class="kind">8 octobre · {category}</div><h2>{html.escape(title)}</h2><p class="metric">{ok:,} / {count:,}<span>{label}</span></p><p>{html.escape(states)}</p><p class="id">{html.escape(Path(d).name)}</p><nav aria-label="Preuves de {html.escape(title)}"><a href="../{d}/manifest.json">Manifest</a><a href="../{d}/summary.json">Résumé JSON</a><a href="../{d}/">Dossier local complet</a><a href="integrated-2026-10-08.md">Interprétation transversale</a></nav></article>')
p=root/'reports/index.html';s=p.read_text();start='<!-- integrated-evidence:start -->';end='<!-- integrated-evidence:end -->'
s=re.sub(re.escape(start)+r'.*?'+re.escape(end),'',s,flags=re.S)
s=s.replace('<div class="grid">','<div class="grid">'+start+''.join(cards)+end,1)
s=s.replace('Douze campagnes retenues, manifests et données brutes conservés.','Campagnes historiques et nouvelles qualifications, manifests et données brutes conservés.')
nav='<a href="integrated-2026-10-08.md">Qualification transversale : Rust, HTTP, navigateur et reprise</a><a href="integrated-evidence-catalog.json">SHA des nouvelles preuves</a>'
if 'Qualification transversale : Rust' not in s:s=s.replace('<nav aria-label="Documentation">','<nav aria-label="Documentation">'+nav,1)
p.write_text(s)
print(json.dumps({'addedCampaigns':len(cards),'totalCards':s.count('<article ')}))
