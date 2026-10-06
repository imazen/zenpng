#!/usr/bin/env python3
"""Join a benches/pareto.rs run into one TSV and a Pareto summary.

    scripts/pareto_report.py BENCH_LOG ZENBENCH_RESULT OUT_PREFIX

BENCH_LOG is the bench's captured stdout+stderr (the `SIZE` lines);
ZENBENCH_RESULT is the `zenbench-*.txt` file it names. Writes
OUT_PREFIX.tsv (one row per group x arm) and OUT_PREFIX.md (fits and
dominance tables).

Content class comes from the imazen-26 id (first two digits of the file
name); size class from the long edge in the name.
"""
import collections
import re
import statistics
import sys

CLASS = {
    "12": "photo", "14": "photo", "16": "photo", "20": "photo", "24": "photo",
    "30": "photo", "33": "photo", "92": "photo",
    "80": "screen", "81": "screen",
    "70": "lineart", "90": "lineart", "60": "lineart",
    "50": "document", "52": "document", "53": "document", "68": "document",
    "22": "mixed", "66": "mixed", "90": "lineart",
}


def content_class(name):
    if name.startswith("9097"):
        return "mixed"
    return CLASS.get(name[:2], "other")


def size_class(edge):
    return {64: "tiny", 256: "small", 1024: "medium"}.get(edge, "large" if edge >= 2048 else f"e{edge}")


UNIT = {"ns": 1e-6, "µs": 1e-3, "us": 1e-3, "ms": 1.0, "s": 1e3}


def load(log, result):
    sizes = {}
    for line in open(log, encoding="utf-8", errors="replace"):
        if line.startswith("SIZE\t"):
            _, g, arm, b = line.rstrip("\n").split("\t")
            sizes[(g, arm)] = int(b)
    times = {}
    pattern = re.compile(r"group=(\S+) benchmark=(\S+) .*?mean=([\d.]+)(ns|µs|us|ms|s)\b")
    for line in open(result, encoding="utf-8"):
        m = pattern.match(line)
        if m:
            g, arm, v, u = m.groups()
            times[(g, arm)] = float(v) * UNIT[u]
    return sizes, times


def fit(xs, ys):
    """Least squares y = a + b*x."""
    n = len(xs)
    if n < 2 or len(set(xs)) < 2:
        return float("nan"), float("nan")
    mx, my = sum(xs) / n, sum(ys) / n
    b = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)
    return my - b * mx, b


def main():
    log, result, out = sys.argv[1:4]
    sizes, times = load(log, result)
    rows = []
    for (g, arm), ms in sorted(times.items()):
        kind, name = g.split("/", 1)
        parts = name.split("_")
        edge = int(parts[-1])
        fmt = parts[1]
        if kind == "enc":
            b = sizes.get((g, arm))
        else:
            b = sizes.get((g, "input"))
        rows.append(dict(kind=kind, image=name, id=parts[0], fmt=fmt, edge=edge,
                         size_class=size_class(edge), content=content_class(name),
                         arm=arm, threads="mt" if arm.endswith("_mt") else "st",
                         bytes=b, ms=ms))
    # Pixels per call = throughput (px/s) x mean time.
    px = {}
    for line in open(result, encoding="utf-8"):
        g = re.match(r"group=(\S+) ", line)
        t = re.search(r"throughput=([\d.]+) (\w?)px/s", line)
        m = re.search(r"mean=([\d.]+)(ns|µs|us|ms|s)\b", line)
        if g and t and m:
            scale = {"K": 1e3, "M": 1e6, "G": 1e9}.get(t.group(2), 1)
            px[g.group(1)] = round(float(t.group(1)) * scale * float(m.group(1)) * UNIT[m.group(2)] / 1e3)
    with open(out + ".tsv", "w") as f:
        cols = ["kind", "image", "id", "fmt", "edge", "size_class", "content", "arm", "threads", "px", "bytes", "ms"]
        f.write("\t".join(cols) + "\n")
        for r in rows:
            r["px"] = px.get(f"{r['kind']}/{r['image']}", "")
            f.write("\t".join(str(r[c]) for c in cols) + "\n")

    md = []
    for kind in ("enc", "dec", "dec_idot"):
        krows = [r for r in rows if r["kind"] == kind]
        if not krows:
            continue
        md.append(f"## {kind}\n")
        # Fits per arm over all images: ms and bytes against pixels.
        md.append("| arm | n | ms = a + b·MP (a ms, b ms/MP) | bytes = a + b·MP (enc) |")
        md.append("|---|---|---|---|")
        by_arm = collections.defaultdict(list)
        for r in krows:
            if r["px"] != "":
                by_arm[r["arm"]].append(r)
        for arm, rs in sorted(by_arm.items()):
            xs = [r["px"] / 1e6 for r in rs]
            a, b = fit(xs, [r["ms"] for r in rs])
            bf = ""
            if kind == "enc":
                ba, bb = fit(xs, [r["bytes"] for r in rs])
                bf = f"{ba:.0f} + {bb:.0f}·MP"
            md.append(f"| {arm} | {len(rs)} | {a:.3f} + {b:.3f}·MP | {bf} |")
        md.append("")
        # Medians per size class and content class.
        ref = "png_high" if kind == "enc" else "png"
        for key in ("size_class", "content"):
            md.append(f"### {kind} by {key}: median time / {ref} time" + (", median size / png_high size" if kind == "enc" else ""))
            md.append("")
            groups = collections.defaultdict(lambda: collections.defaultdict(dict))
            for r in krows:
                groups[r[key]][r["image"]][r["arm"]] = r
            for cls in sorted(groups):
                imgs = groups[cls]
                arms = sorted({a for d in imgs.values() for a in d})
                stats = {}
                for arm in arms:
                    tr = [d[arm]["ms"] / d[ref]["ms"] for d in imgs.values() if arm in d and ref in d]
                    sr = [d[arm]["bytes"] / d[ref]["bytes"] for d in imgs.values() if arm in d and ref in d and d[arm]["bytes"]] if kind == "enc" else []
                    if tr:
                        stats[arm] = (statistics.median(tr), statistics.median(sr) if sr else None, len(tr))
                md.append(f"**{cls}** (n={len(imgs)})\n")
                md.append("| arm | time | size | dominated by |" if kind == "enc" else "| arm | time |")
                md.append("|---|---|---|---|" if kind == "enc" else "|---|---|")
                for arm, (t, s, n) in sorted(stats.items(), key=lambda kv: kv[1][0]):
                    if kind == "enc":
                        dom = [o for o, (t2, s2, _) in stats.items()
                               if arm.startswith("zenpng") and not o.startswith("zenpng")
                               and t2 <= t and s2 is not None and s is not None and s2 <= s and (t2 < t or s2 < s)]
                        md.append(f"| {arm} | {t:.3f} | {s:.4f} | {', '.join(dom)} |")
                    else:
                        md.append(f"| {arm} | {t:.3f} |")
                md.append("")
    with open(out + ".md", "w") as f:
        f.write("\n".join(md) + "\n")


if __name__ == "__main__":
    main()
