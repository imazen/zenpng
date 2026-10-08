#!/usr/bin/env python3
"""Encode thread-scaling table from `pareto --group=enc` zenbench results.

Usage: thread_scaling.py <zenbench results .txt>

Expects arms zenpng_e<E>_st and zenpng_e<E>_t<N> (ZENPNG_PARETO_MT_THREADS).
Prints, per effort and input size, the median over images of speedup
(st / tN, from median times) and efficiency (speedup / N).
"""
import re, sys, statistics
from collections import defaultdict


def ms(v):
    n, u = re.match(r"([\d.]+)(ms|µs|s)", v).groups()
    return float(n) * {"ms": 1, "µs": 1e-3, "s": 1e3}[u]


t = defaultdict(dict)
for line in open(sys.argv[1]):
    m = re.match(r"group=enc/(\S+) benchmark=zenpng_e(\d+)_(st|t\d+) .*median=(\S+)", line)
    if m:
        t[(m[1], int(m[2]))][m[3]] = ms(m[4])

rows = defaultdict(lambda: defaultdict(list))
for (img, e), arms in t.items():
    if "st" not in arms:
        continue
    _, fmt, px = img.split("_")
    for a, v in arms.items():
        if a != "st":
            rows[(e, f"{fmt}_{px}")][int(a[1:])].append(arms["st"] / v)

threads = sorted({n for r in rows.values() for n in r})
print("effort\tinput\tn\t" + "\t".join(f"t{n} speedup (min-max)\teff" for n in threads))
for (e, inp), r in sorted(rows.items(), key=lambda k: (k[0][0], k[0][1])):
    cells = []
    for n in threads:
        v = r.get(n, [])
        if v:
            med = statistics.median(v)
            cells.append(f"{med:.2f} ({min(v):.2f}-{max(v):.2f})\t{med / n:.2f}")
        else:
            cells.append("-\t-")
    print(f"e{e}\t{inp}\t{len(r[threads[0]])}\t" + "\t".join(cells))
