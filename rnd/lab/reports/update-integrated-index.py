"""Add current evidence cards while preserving all previous campaigns."""
import json,re,sys,html
from pathlib import Path
root=Path(__file__).resolve().parent.parent
first=Path(sys.argv[1]);current=Path(sys.argv[2]);a=json.loads((root/'reports/integrated-analysis.json').read_text());cards=[]
for key,v in a['campaigns'].items():
 if 'directory' not in v:continue
 d=v['directory'];category='workflow' if key.startswith('runtime') else 'storage' if key.startswith('sqlite') else 'browser' if key.startswith('browser') else 'crawl'
 if 'groups' in v:
  count=sum(v['statuses'].values());ok=v['statuses'].get('verified',0);label='verified cells'
 else:
  count=v['rawCount'];ok=v['statuses'].get('verified',0);label='verified rows, all phases'
 titles={'runtime-native-sustained':'Native runtime: first series','runtime-http-backends':'Workflow: HTTP backends','runtime-recovery':'Runtime: first recovery','browser-actionability':'Actionability: native and initial guard','sqlite-readers':'SQLite readers: initial diagnostic','http-transport':'HTTP transports and loopback redirect','http-open-20':'HTTP: 20/s arrivals','http-open-100':'HTTP: 100/s arrivals','http-open-250':'HTTP: 250/s arrivals'}
 title=titles.get(key)
 if title is None:title='Native runtime: corrected Rust/Node' if key.startswith('runtime-native-qualified-') else 'Runtime: requalified recovery' if key.startswith('runtime-recovery-qualified-') else 'Actionability: corrected guard' if key.startswith('browser-actionability-') else 'SQLite: requalified writers and readers' if key.startswith('sqlite-readers-') else 'Spider: gzip, chunked and large responses'
 states=' · '.join(f'{n} {k}' for k,n in v['statuses'].items())
 cards.append(f'<article data-category="{category}"><div class="kind">October 8 · {category}</div><h2>{html.escape(title)}</h2><p class="metric">{ok:,} / {count:,}<span>{label}</span></p><p>{html.escape(states)}</p><p class="id">{html.escape(Path(d).name)}</p><nav aria-label="Evidence for {html.escape(title)}"><a href="../{d}/manifest.json">Manifest</a><a href="../{d}/summary.json">JSON summary</a><a href="../{d}/">Complete local folder</a><a href="integrated-2026-10-08.md">Cross-cutting interpretation</a></nav></article>')
p=root/'reports/index.html';s=p.read_text();start='<!-- integrated-evidence:start -->';end='<!-- integrated-evidence:end -->'
s=re.sub(re.escape(start)+r'.*?'+re.escape(end),'',s,flags=re.S)
s=s.replace('<div class="grid">','<div class="grid">'+start+''.join(cards)+end,1)
s=s.replace('Douze campagnes retenues, manifests et données brutes conservés.','Historical campaigns and new qualifications, manifests and raw data preserved.')
nav='<a href="integrated-2026-10-08.md">Cross-cutting qualification: Rust, HTTP, browser and recovery</a><a href="integrated-evidence-catalog.json">SHAs of the new evidence</a>'
if 'Cross-cutting qualification: Rust' not in s:s=s.replace('<nav aria-label="Documentation">','<nav aria-label="Documentation">'+nav,1)
p.write_text(s)
print(json.dumps({'addedCampaigns':len(cards),'totalCards':s.count('<article ')}))
