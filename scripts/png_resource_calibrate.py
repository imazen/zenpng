#!/usr/bin/env python3
"""Calibrate PNG encode/decode resource use (peak mem + wall + CPU time).

Drives `examples/png_probe` (one process per op → clean per-op VmHWM peak)
across compression-level (effort) x resolution x alpha x depth x content.
ENCODES at each effort (encoder marginal working set + wall + user/sys),
then DECODES the produced PNG. Real content downscaled (Lanczos,
downscale-only) to a pixel ladder so per-pixel slope separates from fixed
overhead. Threads pinned to 1 in the probe (wall ~= user single-thread).
"""
import argparse, subprocess, datetime, socket
from pathlib import Path
from PIL import Image
Image.MAX_IMAGE_PIXELS = None


def gen_variant(src, n, outdir):
    im = Image.open(src)
    if max(im.size) < n:
        return None
    im = im.convert("RGB").resize((n, n), Image.LANCZOS)
    p = outdir / f"{Path(src).stem}_{n}.png"
    im.save(p)
    return p


def run(b, png, mode, effort, depth, alpha, outp):
    out = subprocess.run([b, str(png), mode, str(effort), str(depth), alpha, str(outp)],
                         capture_output=True, text=True).stdout
    return {k: v for k, v in (t.split("=", 1) for t in out.split() if "=" in t)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/release/examples/png_probe")
    ap.add_argument("--sizes", default="256,512,1024")
    ap.add_argument("--efforts", default="1,6,13,19,24")
    ap.add_argument("--alphas", default="rgb")
    ap.add_argument("--depths", default="8")
    ap.add_argument("--content-file", default=None)
    ap.add_argument("--content", action="append", default=[])
    ap.add_argument("--out", default=None)
    a = ap.parse_args()

    sizes = [int(x) for x in a.sizes.split(",")]
    efforts = [int(x) for x in a.efforts.split(",")]
    alphas = a.alphas.split(",")
    depths = [int(x) for x in a.depths.split(",")]
    content = [c.split(":") for c in a.content]
    if a.content_file:
        for line in Path(a.content_file).read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                content.append(line.split(":"))

    date = datetime.date.today().isoformat()
    out = Path(a.out or f"benchmarks/png_resource_{date}.tsv")
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    tmp = Path("/tmp/_pngcal"); tmp.mkdir(exist_ok=True)

    cells = []
    for src, cls in content:
        for n in sizes:
            v = gen_variant(src, n, tmp)
            if v:
                cells.append((v, n, cls))

    rows = []
    total = len(cells) * len(alphas) * len(depths) * len(efforts)
    i = 0
    for (png, n, cls) in cells:
        for al in alphas:
            for d in depths:
                for e in efforts:
                    i += 1
                    op = tmp / f"{png.stem}_{al}_{d}_{e}.png"
                    enc = run(a.bin, png, "encode", e, d, al, op)
                    dec = run(a.bin, png, "decode", e, d, al, op)
                    px = n * n
                    if not enc.get("bytes"):
                        print(f"[{i}/{total}] {cls} {n} {al} d{d} e{e} ENCODE FAILED", flush=True)
                        continue
                    rows.append((cls, d, al, n, px, e, "encode", int(enc["peak_kb"]), int(enc["delta_kb"]),
                                 float(enc["wall_ms"]), float(enc["user_ms"]), float(enc["sys_ms"]), int(enc["bytes"])))
                    if dec.get("peak_kb"):
                        rows.append((cls, d, al, n, px, e, "decode", int(dec["peak_kb"]), int(dec["delta_kb"]),
                                     float(dec["wall_ms"]), float(dec["user_ms"]), float(dec["sys_ms"]), int(enc["bytes"])))
                    print(f"[{i}/{total}] {cls} {n}^2 {al} d{d} e{e} -> "
                          f"enc {int(enc['delta_kb'])//1024}MB {float(enc['wall_ms']):.0f}ms "
                          f"({float(enc['wall_ms'])*1e3/px:.2f}us/px) | dec {float(dec.get('wall_ms',0)):.0f}ms", flush=True)

    out.parent.mkdir(exist_ok=True)
    with open(out, "w") as f:
        f.write("content\tdepth\talpha\tsize\tpixels\teffort\top\tpeak_kb\tdelta_kb\twall_ms\tuser_ms\tsys_ms\tbytes\n")
        for r in rows:
            f.write("\t".join(str(x) for x in r) + "\n")
    with open(str(out) + ".meta", "w") as f:
        f.write(f"# png_resource_calibrate\ncommit: {commit}\nhost: {socket.gethostname()}\ndate: {date}\n"
                f"bin: {a.bin}\nsizes: {sizes}\nefforts: {efforts}\nalphas: {alphas}\ndepths: {depths}\n"
                f"content_classes: {sorted(set(c[1] for c in content))}\n"
                f"measure: png_probe VmHWM delta + wall (Instant) + user/sys (/proc/self/stat), "
                f"with_parallel(false), one process per op.\n")
    print(f"\nwrote {out} ({len(rows)} rows)")

    # ---- fit ----
    enc = [r for r in rows if r[6] == "encode" and r[4] >= 512 * 512]
    dec = [r for r in rows if r[6] == "decode" and r[4] >= 512 * 512]
    med = lambda v: sorted(v)[len(v) // 2]
    from collections import defaultdict
    print("\n=== ENCODE mem B/px + wall us/px, per effort (px>=512^2) ===")
    print(f"{'eff':>3} {'n':>3} {'mem p50':>8} {'mem p100':>9} {'wall us/px p50':>15} {'user us/px p50':>15}")
    by = defaultdict(list)
    for r in enc:
        by[r[5]].append(r)
    for e in sorted(by):
        v = by[e]
        mem = [r[8] * 1024.0 / r[4] for r in v]
        wpp = [r[9] * 1e3 / r[4] for r in v]
        upp = [r[10] * 1e3 / r[4] for r in v]
        print(f"{e:>3} {len(v):>3} {med(mem):>8.1f} {max(mem):>9.1f} {med(wpp):>15.2f} {med(upp):>15.2f}")
    if dec:
        mem = [r[8] * 1024.0 / r[4] for r in dec]
        wpp = [r[9] * 1e3 / r[4] for r in dec]
        print(f"\n=== DECODE (px>=512^2): mem p50={med(mem):.1f} p100={max(mem):.1f} B/px | "
              f"wall p50={med(wpp):.3f} us/px (n={len(dec)}) ===")
    print("\n=== alpha/depth deltas (encode e13, p50) ===")
    for axis, idx in (("alpha", 2), ("depth", 1)):
        g = defaultdict(list)
        for r in [x for x in rows if x[6] == "encode" and x[5] == 13 and x[4] >= 512 * 512]:
            g[r[idx]].append((r[8] * 1024.0 / r[4], r[9] * 1e3 / r[4]))
        for k in sorted(g, key=str):
            print(f"  {axis}={k}: mem p50={med([x[0] for x in g[k]]):.1f} B/px, "
                  f"wall p50={med([x[1] for x in g[k]]):.2f} us/px (n={len(g[k])})")


if __name__ == "__main__":
    main()
