#!/usr/bin/env python3
"""Analyze strategy explorer data with zopfli results across 5 images."""

import csv
import os
from collections import defaultdict

CSV_DIR = "/mnt/v/output/zenpng/strategy_zopfli"
FILES = ["02809272.csv", "0369d229.csv", "07b9f93f.csv", "097cb426.csv", "0c49a5cc.csv"]


def load_data():
    """Load all CSV files, skipping non-CSV lines."""
    all_rows = []
    for fname in FILES:
        path = os.path.join(CSV_DIR, fname)
        image_id = fname.replace(".csv", "")
        with open(path) as f:
            lines = f.readlines()
        # Find the CSV header line
        header_idx = None
        for i, line in enumerate(lines):
            if line.startswith("strategy,context,eval,block,zf_level,"):
                header_idx = i
                break
        if header_idx is None:
            print(f"WARNING: No CSV header found in {fname}")
            continue
        # Parse CSV rows after header
        csv_lines = [lines[header_idx]]
        for line in lines[header_idx + 1:]:
            stripped = line.strip()
            if stripped and stripped[0].isalpha() and "," in stripped:
                parts = stripped.split(",")
                if len(parts) == 10:
                    csv_lines.append(line)
        reader = csv.DictReader(csv_lines)
        for row in reader:
            try:
                row["image"] = image_id
                row["zf_level"] = int(row["zf_level"])
                row["size"] = int(row["size"])
                row["filter_ms"] = float(row["filter_ms"])
                row["compress_ms"] = float(row["compress_ms"])
                row["total_ms"] = float(row["total_ms"])
                row["vs_baseline_pct"] = float(row["vs_baseline_pct"])
                all_rows.append(row)
            except (ValueError, KeyError):
                continue
    return all_rows


def get_base_strategy(strategy):
    """Strip '+zopfli' suffix to get base strategy name."""
    return strategy.replace("+zopfli", "")


def q1_zopfli_vs_zenflate(rows):
    """Compare zopfli-50 vs zenflate L12 for matching strategies."""
    print("=" * 80)
    print("Q1: Zopfli-50 vs Zenflate L12 -- How much does zopfli compress beyond L12?")
    print("=" * 80)

    # Group by (image, base_strategy)
    l12_data = {}
    zopfli50_data = {}

    for r in rows:
        base = get_base_strategy(r["strategy"])
        key = (r["image"], base)
        if r["zf_level"] == 12 and "+zopfli" not in r["strategy"]:
            l12_data[key] = r
        elif r["zf_level"] == 150 and "+zopfli" in r["strategy"]:
            zopfli50_data[key] = r

    diffs_bytes = []
    diffs_pct = []

    strat_accum = defaultdict(lambda: {"l12_sizes": [], "z50_sizes": [], "diffs_b": [], "diffs_p": []})

    for key in sorted(l12_data.keys()):
        if key in zopfli50_data:
            l12 = l12_data[key]
            z50 = zopfli50_data[key]
            diff_b = z50["size"] - l12["size"]
            diff_p = z50["vs_baseline_pct"] - l12["vs_baseline_pct"]
            diffs_bytes.append(diff_b)
            diffs_pct.append(diff_p)
            base = key[1]
            strat_accum[base]["l12_sizes"].append(l12["size"])
            strat_accum[base]["z50_sizes"].append(z50["size"])
            strat_accum[base]["diffs_b"].append(diff_b)
            strat_accum[base]["diffs_p"].append(diff_p)

    print(f"\n{'Strategy':<25} {'Images':>6} {'Avg L12 Size':>12} {'Avg Z50 Size':>12} {'Diff (bytes)':>13} {'Diff (pct pts)':>14}")
    print("-" * 90)

    for strat in sorted(strat_accum.keys()):
        d = strat_accum[strat]
        n = len(d["diffs_b"])
        avg_l12 = sum(d["l12_sizes"]) / n
        avg_z50 = sum(d["z50_sizes"]) / n
        avg_db = sum(d["diffs_b"]) / n
        avg_dp = sum(d["diffs_p"]) / n
        print(f"{strat:<25} {n:>6} {avg_l12:>12.0f} {avg_z50:>12.0f} {avg_db:>+13.0f} {avg_dp:>+14.2f}")

    if diffs_bytes:
        avg_b = sum(diffs_bytes) / len(diffs_bytes)
        avg_p = sum(diffs_pct) / len(diffs_pct)
        print("-" * 90)
        print(f"{'OVERALL AVERAGE':<25} {len(diffs_bytes):>6} {'':>12} {'':>12} {avg_b:>+13.0f} {avg_p:>+14.2f}")
        print(f"\nZopfli-50 saves an average of {-avg_b:.0f} bytes ({-avg_p:.2f} pct pts) beyond zenflate L12.")


def q2_iteration_scaling(rows):
    """Show compression improvement from 5->15->50 iterations."""
    print("\n" + "=" * 80)
    print("Q2: Zopfli Iteration Scaling -- Compression from 5 -> 15 -> 50 iterations")
    print("=" * 80)

    tier_sizes = defaultdict(list)
    tier_pcts = defaultdict(list)
    for r in rows:
        if "+zopfli" in r["strategy"]:
            tier_sizes[r["zf_level"]].append(r["size"])
            tier_pcts[r["zf_level"]].append(r["vs_baseline_pct"])

    tiers = sorted(tier_sizes.keys())
    print(f"\n{'Zopfli Tier':<15} {'Iterations':>10} {'Avg Size':>12} {'Avg vs_base%':>14} {'Count':>6}")
    print("-" * 60)
    tier_avgs = {}
    for zf in tiers:
        iters = zf - 100
        avg_size = sum(tier_sizes[zf]) / len(tier_sizes[zf])
        avg_pct = sum(tier_pcts[zf]) / len(tier_pcts[zf])
        tier_avgs[zf] = avg_size
        print(f"{'zopfli-' + str(iters):<15} {iters:>10} {avg_size:>12.0f} {avg_pct:>+14.2f} {len(tier_sizes[zf]):>6}")

    print(f"\n{'Step':<20} {'Size Reduction':>15} {'Reduction %':>12}")
    print("-" * 50)
    prev_zf = None
    for zf in tiers:
        if prev_zf is not None:
            iters_prev = prev_zf - 100
            iters_cur = zf - 100
            diff = tier_avgs[prev_zf] - tier_avgs[zf]
            pct = diff / tier_avgs[prev_zf] * 100
            print(f"{iters_prev} -> {iters_cur} iters{'':<8} {diff:>+15.0f} {pct:>+12.3f}%")
        prev_zf = zf


def q3_best_strategy(rows):
    """Does the best filter strategy change with zopfli vs zenflate L12?"""
    print("\n" + "=" * 80)
    print("Q3: Best Strategy -- Does the winner change with zopfli vs zenflate L12?")
    print("=" * 80)

    l12_best = {}
    z50_best = {}

    for r in rows:
        if r["zf_level"] == 12 and "+zopfli" not in r["strategy"]:
            img = r["image"]
            if img not in l12_best or r["size"] < l12_best[img]["size"]:
                l12_best[img] = r
        elif r["zf_level"] == 150 and "+zopfli" in r["strategy"]:
            img = r["image"]
            if img not in z50_best or r["size"] < z50_best[img]["size"]:
                z50_best[img] = r

    print(f"\n{'Image':<12} {'Best at L12':<30} {'Size L12':>10} {'Best at Zopfli-50':<30} {'Size Z50':>10} {'Same?':>6}")
    print("-" * 105)

    same_count = 0
    for img in sorted(l12_best.keys()):
        l12 = l12_best.get(img)
        z50 = z50_best.get(img)
        if l12 and z50:
            l12_base = get_base_strategy(l12["strategy"])
            z50_base = get_base_strategy(z50["strategy"])
            same = "YES" if l12_base == z50_base else "NO"
            if same == "YES":
                same_count += 1
            print(f"{img:<12} {l12['strategy']:<30} {l12['size']:>10,} {z50['strategy']:<30} {z50['size']:>10,} {same:>6}")

    total = len(l12_best)
    print(f"\nSame winner in {same_count}/{total} images.")


def q4_time_size_tradeoff(rows):
    """Time-size tradeoff for zopfli iterations."""
    print("\n" + "=" * 80)
    print("Q4: Time-Size Tradeoff -- compress_ms and bytes saved per zopfli tier")
    print("=" * 80)

    tier_data = defaultdict(lambda: {"compress_ms": [], "size": []})
    for r in rows:
        if "+zopfli" in r["strategy"]:
            tier_data[r["zf_level"]]["compress_ms"].append(r["compress_ms"])
            tier_data[r["zf_level"]]["size"].append(r["size"])

    tiers = sorted(tier_data.keys())
    print(f"\n{'Tier':<15} {'Avg compress_ms':>16} {'Avg Size':>12} {'Count':>6}")
    print("-" * 55)

    tier_avgs = {}
    for zf in tiers:
        iters = zf - 100
        avg_ms = sum(tier_data[zf]["compress_ms"]) / len(tier_data[zf]["compress_ms"])
        avg_sz = sum(tier_data[zf]["size"]) / len(tier_data[zf]["size"])
        tier_avgs[zf] = (avg_ms, avg_sz)
        print(f"{'zopfli-' + str(iters):<15} {avg_ms:>16.1f} {avg_sz:>12.0f} {len(tier_data[zf]['compress_ms']):>6}")

    print(f"\n{'Step':<20} {'Extra ms':>12} {'Bytes Saved':>12} {'ms/byte':>10}")
    print("-" * 58)
    prev_zf = None
    for zf in tiers:
        if prev_zf is not None:
            ip = prev_zf - 100
            ic = zf - 100
            delta_ms = tier_avgs[zf][0] - tier_avgs[prev_zf][0]
            delta_sz = tier_avgs[prev_zf][1] - tier_avgs[zf][1]
            ms_per_byte = delta_ms / delta_sz if delta_sz > 0 else float("inf")
            print(f"{ip} -> {ic} iters{'':<8} {delta_ms:>+12.1f} {delta_sz:>+12.0f} {ms_per_byte:>10.2f}")
        prev_zf = zf


def q5_pareto_frontier(rows):
    """Combined Pareto frontier across all strategies and compression levels."""
    print("\n" + "=" * 80)
    print("Q5: Combined Pareto Frontier -- All strategies + zenflate levels + zopfli")
    print("=" * 80)

    combo_data = defaultdict(lambda: {"total_ms": [], "size": [], "images": set()})

    for r in rows:
        key = (r["strategy"], r["zf_level"])
        combo_data[key]["total_ms"].append(r["total_ms"])
        combo_data[key]["size"].append(r["size"])
        combo_data[key]["images"].add(r["image"])

    # Only keep combos present in all 5 images
    combos = []
    for (strat, zf), d in combo_data.items():
        if len(d["images"]) == 5:
            avg_ms = sum(d["total_ms"]) / len(d["total_ms"])
            avg_sz = sum(d["size"]) / len(d["size"])
            combos.append((strat, zf, avg_ms, avg_sz, len(d["images"])))

    combos.sort(key=lambda x: x[2])

    # Find Pareto optimal: no other combo is both faster AND smaller
    pareto = []
    for c in combos:
        dominated = False
        for other in combos:
            if other is c:
                continue
            if other[2] <= c[2] and other[3] <= c[3] and (other[2] < c[2] or other[3] < c[3]):
                dominated = True
                break
        if not dominated:
            pareto.append(c)

    pareto.sort(key=lambda x: x[2])

    def zf_label(zf):
        if zf <= 12:
            return f"L{zf}"
        else:
            return f"zopfli-{zf - 100}"

    print(f"\nPareto-optimal set ({len(pareto)} combos out of {len(combos)} total):\n")
    print(f"{'Strategy':<30} {'Level':<12} {'Avg ms':>10} {'Avg Size':>12}")
    print("-" * 68)

    for strat, zf, avg_ms, avg_sz, n_img in pareto:
        print(f"{strat:<30} {zf_label(zf):<12} {avg_ms:>10.1f} {avg_sz:>12.0f}")

    # Full table
    print(f"\nFull table (all {len(combos)} combos, sorted by avg_ms):\n")
    print(f"{'Strategy':<30} {'Level':<12} {'Avg ms':>10} {'Avg Size':>12} {'Pareto?':>8}")
    print("-" * 76)
    pareto_set = {(c[0], c[1]) for c in pareto}
    for strat, zf, avg_ms, avg_sz, n_img in combos:
        is_pareto = "***" if (strat, zf) in pareto_set else ""
        print(f"{strat:<30} {zf_label(zf):<12} {avg_ms:>10.1f} {avg_sz:>12.0f} {is_pareto:>8}")


def q6_filter_sensitivity(rows):
    """At zopfli-50, how much does filter strategy matter?"""
    print("\n" + "=" * 80)
    print("Q6: Zopfli Filter Sensitivity -- Size range across strategies at zopfli-50")
    print("=" * 80)

    z50_by_image = defaultdict(list)
    for r in rows:
        if r["zf_level"] == 150 and "+zopfli" in r["strategy"]:
            z50_by_image[r["image"]].append(r)

    print(f"\n{'Image':<12} {'Best Strategy':<30} {'Best Size':>10} {'Worst Strategy':<30} {'Worst Size':>10} {'Range':>8} {'Range%':>8}")
    print("-" * 115)

    total_ranges = []
    total_range_pcts = []

    for img in sorted(z50_by_image.keys()):
        entries = sorted(z50_by_image[img], key=lambda x: x["size"])
        best = entries[0]
        worst = entries[-1]
        rng = worst["size"] - best["size"]
        rng_pct = rng / best["size"] * 100
        total_ranges.append(rng)
        total_range_pcts.append(rng_pct)
        print(f"{img:<12} {get_base_strategy(best['strategy']):<30} {best['size']:>10,} {get_base_strategy(worst['strategy']):<30} {worst['size']:>10,} {rng:>8,} {rng_pct:>7.2f}%")

    if total_ranges:
        print("-" * 115)
        avg_rng = sum(total_ranges) / len(total_ranges)
        avg_pct = sum(total_range_pcts) / len(total_range_pcts)
        print(f"{'AVERAGE':<12} {'':<30} {'':>10} {'':<30} {'':>10} {avg_rng:>8,.0f} {avg_pct:>7.2f}%")

    # Detailed per-image ranking
    print("\nDetailed per-image ranking at zopfli-50:")
    for img in sorted(z50_by_image.keys()):
        entries = sorted(z50_by_image[img], key=lambda x: x["size"])
        print(f"\n  {img}:")
        for e in entries:
            base = get_base_strategy(e["strategy"])
            print(f"    {base:<30} {e['size']:>10,}  ({e['vs_baseline_pct']:>+.2f}%)")


def main():
    rows = load_data()
    print(f"Loaded {len(rows)} data rows from {len(FILES)} images.\n")

    strategies = sorted(set(r["strategy"] for r in rows))
    zf_levels = sorted(set(r["zf_level"] for r in rows))
    print(f"Strategies ({len(strategies)}): {', '.join(strategies)}")
    print(f"ZF levels: {zf_levels}")
    print()

    q1_zopfli_vs_zenflate(rows)
    q2_iteration_scaling(rows)
    q3_best_strategy(rows)
    q4_time_size_tradeoff(rows)
    q5_pareto_frontier(rows)
    q6_filter_sensitivity(rows)


if __name__ == "__main__":
    main()
