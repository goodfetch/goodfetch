"""mem-research: a sampled run's RSS and anonymous memory over time, ~14 rows."""
import sys

for f in sys.argv[1:]:
    rows = [tuple(int(x) for x in l.split()) for l in open(f) if l.strip()]
    step = max(1, len(rows) // 14)
    print(f"== {f} ({len(rows)} samples)")
    for i, (t, rss, anon) in enumerate(rows):
        if i % step == 0 or i == len(rows) - 1:
            print(f"  {t:6d} ms  rss {rss/1024:6.1f}  anon {anon/1024:6.1f}")
