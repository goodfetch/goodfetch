"""Summarize a /proc/<pid>/smaps snapshot: Rss/Anonymous by mapping, grouped."""
import re, sys
from collections import defaultdict
def parse(path):
    maps, cur = [], None
    for line in open(path):
        m = re.match(r'^([0-9a-f]+)-([0-9a-f]+) (\S+) (\S+) \S+ \S+\s*(.*)$', line)
        if m:
            cur = dict(start=int(m[1],16), end=int(m[2],16), perm=m[3], off=int(m[4],16), name=m[5].strip(), f={})
            maps.append(cur); continue
        k = re.match(r'^(\w+):\s+(\d+) kB', line)
        if k and cur is not None: cur['f'][k[1]] = int(k[2])
    return maps
def group(m, exe):
    n = m['name']
    if n.endswith(exe) or (exe and exe in n): return 'exe ' + m['perm']
    if n == '': return 'anon'
    if n.startswith('['): return n
    if '.so' in n: return 'lib ' + n.split('/')[-1].split('.so')[0]
    return 'file ' + n.split('/')[-1]
if __name__ == '__main__':
    path, exe = sys.argv[1], sys.argv[2]
    maps = parse(path)
    g = defaultdict(lambda: [0,0,0,0])
    for m in maps:
        x = g[group(m, exe)]
        x[0] += m['f'].get('Rss',0); x[1] += m['f'].get('Anonymous',0); x[2] += (m['end']-m['start'])//1024; x[3]+=1
    tot = sum(v[0] for v in g.values())
    print(f"total Rss {tot/1024:.1f} MiB")
    for k,v in sorted(g.items(), key=lambda kv:-kv[1][0]):
        if v[0] >= 64: print(f"{v[0]/1024:8.1f} MiB rss {v[1]/1024:8.1f} anon {v[2]/1024:10.1f} size  n={v[3]:4d}  {k}")
    # anon mappings by size bucket
    sizes = defaultdict(lambda:[0,0])
    for m in maps:
        if m['name']=='' :
            s=(m['end']-m['start'])//1024
            b = '<=132K' if s<=132 else ('8M+4K stack' if s in (8196,8192,2052,2048+4) else ('64M' if s==65536 else f'{s//1024}M' if s>=1024 else f'{s}K'))
            sizes[b][0]+=m['f'].get('Rss',0); sizes[b][1]+=1
    print('anon by mapping size:')
    for k,v in sorted(sizes.items(), key=lambda kv:-kv[1][0])[:15]: print(f"   {v[0]/1024:7.1f} MiB  n={v[1]:4d}  {k}")
