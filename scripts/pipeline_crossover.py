#!/usr/bin/env python3
"""Decode-pipeline crossover table from `pareto --group=pdec` zenbench results.

Usage: pipeline_crossover.py <inputs dir> <label>=<zenbench results .txt> ...

Run the bench with ZENPNG_PIPELINE_MIN_BYTES=0 (every image pipelines in
decode_mt) and ZENPNG_PARETO_PDEC_DECODE_ONLY=1. Prints one row per image
(median decode_mt / decode_st per machine) and a per-format, per-size median
summary. Filtered bytes come from each PNG's IHDR: height * (row bytes + 1).
"""
import re, struct, sys, statistics
from collections import defaultdict
from pathlib import Path

BPP = {"gray8": 1, "rgb8": 3, "rgba8": 4, "rgb16": 6, "gray16": 2, "rgba16": 8}


def ms(v):
    n, u = re.match(r"([\d.]+)(ms|µs|s)", v).groups()
    return float(n) * {"ms": 1, "µs": 1e-3, "s": 1e3}[u]


def load(path):
    t = defaultdict(dict)
    for line in open(path):
        m = re.match(r"group=pdec/(\S+) benchmark=(decode_st|decode_mt) .*median=(\S+)", line)
        if m:
            t[m[1]][m[2]] = ms(m[3])
    return {k: v["decode_mt"] / v["decode_st"] for k, v in t.items() if len(v) == 2}


def filtered(png):
    w, h = struct.unpack(">II", png.read_bytes()[16:24])
    fmt = png.stem.split("_")[1]
    return h * (w * BPP[fmt] + 1)


def main():
    inputs = Path(sys.argv[1])
    runs = [(a.split("=", 1)[0], load(a.split("=", 1)[1])) for a in sys.argv[2:]]
    names = sorted(set.intersection(*(set(r) for _, r in runs)))
    print("image\tfiltered_bytes\t" + "\t".join(f"{l}_mt/st" for l, _ in runs))
    groups = defaultdict(list)
    for n in names:
        fb = filtered(inputs / f"{n}.png")
        print(f"{n}\t{fb}\t" + "\t".join(f"{r[n]:.3f}" for _, r in runs))
        _, fmt, px = n.split("_")
        groups[(fmt, int(px))].append((fb, [r[n] for _, r in runs]))
    print()
    print("format\tpx\tn\tmedian_filtered_MiB\t" + "\t".join(f"{l}_median" for l, _ in runs))
    for (fmt, px), v in sorted(groups.items()):
        med = [statistics.median(x[1][i] for x in v) for i in range(len(runs))]
        fb = statistics.median(x[0] for x in v) / 2**20
        print(f"{fmt}\t{px}\t{len(v)}\t{fb:.2f}\t" + "\t".join(f"{m:.3f}" for m in med))


main()
