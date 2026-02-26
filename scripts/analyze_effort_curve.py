#!/usr/bin/env python3
"""Analyze effort curve data: compression vs time, monotonicity, strategy usefulness.

Usage: python3 scripts/analyze_effort_curve.py
"""

import csv
import numpy as np
from pathlib import Path
from collections import defaultdict

def main():
    csv_path = Path('/mnt/v/output/zenpng/effort_curve/effort_curve.csv')
    if not csv_path.exists():
        print(f"Not found: {csv_path}")
        return

    # Parse CSV
    images = []
    with open(csv_path) as f:
        reader = csv.DictReader(f)
        headers = reader.fieldnames

        # Find effort columns
        efforts = []
        for h in headers:
            if h.endswith('_size') and h.startswith('e'):
                e = int(h[1:].replace('_size', ''))
                efforts.append(e)
        efforts.sort()

        for row in reader:
            fname = row['filename']
            raw = int(row['raw_bytes'])
            if raw == 0:
                continue

            sizes = {}
            times = {}
            valid = True
            for e in efforts:
                try:
                    s = int(row.get(f'e{e}_size') or 0)
                    t = int(row.get(f'e{e}_ms') or 0)
                except (ValueError, TypeError):
                    valid = False
                    break
                if s == 0:
                    valid = False
                    break
                sizes[e] = s
                times[e] = t

            if not valid:
                continue

            images.append({
                'filename': fname,
                'raw_bytes': raw,
                'width': int(row['width']),
                'height': int(row['height']),
                'color_type': row['color_type'],
                'bpp': int(row['bpp']),
                'sizes': sizes,
                'times': times,
            })

    n = len(images)
    print(f"Loaded {n} images, {len(efforts)} effort levels: {efforts}")

    # ============================================================
    # 1. AGGREGATE EFFORT CURVE
    # ============================================================
    print("\n" + "=" * 80)
    print("AGGREGATE EFFORT CURVE")
    print("=" * 80)

    # Strategy boundaries from zenflate
    strategy_names = {
        0: 'Store',
        1: 'Turbo', 2: 'Turbo', 3: 'Turbo', 4: 'Turbo',
        5: 'FastHt', 6: 'FastHt', 7: 'FastHt', 8: 'FastHt', 9: 'FastHt',
        10: 'Greedy',
        11: 'Lazy', 12: 'Lazy', 13: 'Lazy', 14: 'Lazy', 15: 'Lazy', 16: 'Lazy', 17: 'Lazy',
        18: 'Lazy2', 19: 'Lazy2', 20: 'Lazy2', 21: 'Lazy2', 22: 'Lazy2',
        23: 'NearOpt', 24: 'NearOpt', 25: 'NearOpt', 26: 'NearOpt', 27: 'NearOpt',
        28: 'NearOpt', 29: 'NearOpt', 30: 'NearOpt',
        31: 'NrOp+FO',
    }

    # Pipeline phases from compress.rs
    pipeline_notes = {
        0: 'store',
        1: '1 strat, screen-only',
        2: '3 strat, screen-only',
        3: '3 strat, screen-only',
        4: '5 strat, screen-only',
        5: '5 strat, screen-only',
        6: '5 strat, screen-only',
        7: '5 strat, screen-only',
        8: '5 strat, screen+refine[8]',
        9: '5 strat, screen+refine[10]',
        10: '9 strat, screen+refine[12]',
        11: '9 strat, screen+refine[14]',
        12: '9 strat, screen+refine[15]',
        13: '9 strat, screen+refine[17]',
        14: '9 strat, screen+refine[18]',
        15: '9 strat, screen+refine[20]',
        16: '9 strat, screen+refine[20,22]',
        17: '9 strat, screen+refine[20,22] top_k=4',
        18: '9 strat, screen+refine[22,24]',
        19: '9 strat, screen+refine[22,24] top_k=4',
        20: '9 strat, screen+refine[24,26]',
        21: '9 strat, screen+refine[26,28]',
        22: '9 strat, screen+refine[26,28] top_k=4',
        23: '9 strat, screen+refine[28,30]',
        24: '9 strat, screen+refine[28,30]+BF(5,1)',
        25: '9 strat, screen+refine[28,30]+BF(5,1)+BFF[10]',
        26: '9 strat, refine[30]+BF+BFF[10]+AF(15,2)',
        27: '9 strat, refine[30]+BF+BFF[10,15]+AF(15,2)(22,2)',
        28: '9 strat, refine[30]+fullBF+BFF[10,15]+AF+recompress',
        29: '9 strat, refine[30]+fullBF+BFF[10,15]+AF+beam(10,3)+recompress',
        30: '9 strat, refine[30]+fullBF+BFF[10,15]+AF+beam+recompress',
        31: 'full e30+FullOpt(15i)',
    }

    print(f"\n{'Effort':>6} {'Strategy':>8} {'Agg Size':>12} {'vs e0':>8} {'vs prev':>8} {'Med ms':>8} {'Pipeline'}")
    print("-" * 110)

    prev_total = None
    e0_total = None
    for e in efforts:
        total = sum(img['sizes'][e] for img in images)
        med_ms = np.median([img['times'][e] for img in images])
        strat = strategy_names.get(e, '?')
        pipeline = pipeline_notes.get(e, '')

        if e0_total is None:
            e0_total = total

        vs_e0 = (total / e0_total - 1) * 100
        if prev_total is not None:
            vs_prev = (total / prev_total - 1) * 100
            vs_prev_str = f"{vs_prev:+.3f}%"
        else:
            vs_prev_str = "—"

        print(f"{e:>6} {strat:>8} {total:>12,} {vs_e0:>+7.2f}% {vs_prev_str:>8} {med_ms:>7.0f}ms  {pipeline}")
        prev_total = total

    # ============================================================
    # 2. MONOTONICITY ANALYSIS
    # ============================================================
    print("\n" + "=" * 80)
    print("MONOTONICITY ANALYSIS")
    print("=" * 80)

    violation_count = 0
    significant_violations = []  # > 0.1% regression
    running_min_violations = 0

    for img in images:
        running_min = float('inf')
        for e in efforts:
            s = img['sizes'][e]
            if e > 0 and s > running_min:
                violation_count += 1
                delta = s - running_min
                pct = delta / running_min * 100
                if pct > 0.1:
                    significant_violations.append({
                        'filename': img['filename'],
                        'effort': e,
                        'size': s,
                        'min_so_far': running_min,
                        'delta': delta,
                        'pct': pct,
                    })
            running_min = min(running_min, s)

    total_checks = n * (len(efforts) - 1)
    print(f"\nTotal effort transitions checked: {total_checks}")
    print(f"Monotonicity violations (vs running min): {violation_count} ({violation_count/total_checks*100:.2f}%)")
    print(f"Significant violations (>0.1%): {len(significant_violations)}")

    if significant_violations:
        # Group by effort transition
        by_effort = defaultdict(list)
        for v in significant_violations:
            by_effort[v['effort']].append(v)

        print("\nViolations by effort level:")
        for e in sorted(by_effort):
            vs = by_effort[e]
            print(f"\n  Effort {e} ({strategy_names.get(e, '?')}):")
            for v in sorted(vs, key=lambda x: -x['pct'])[:5]:  # top 5
                print(f"    {v['filename']:<55s} {v['min_so_far']:>8,} -> {v['size']:>8,} (+{v['delta']:>5,}, +{v['pct']:.2f}%)")

    # ============================================================
    # 3. STEP-OVER-STEP ANALYSIS — where do we actually improve?
    # ============================================================
    print("\n" + "=" * 80)
    print("STEP-OVER-STEP IMPROVEMENT")
    print("=" * 80)

    print(f"\n{'Transition':>12} {'Improved':>8} {'Same':>6} {'Worse':>6} {'Med Δ':>8} {'Med Δ%':>8} {'Strategy change'}")
    print("-" * 80)

    for i in range(1, len(efforts)):
        e_prev = efforts[i - 1]
        e_curr = efforts[i]

        improved = 0
        same = 0
        worse = 0
        deltas = []
        pct_deltas = []

        for img in images:
            s_prev = img['sizes'][e_prev]
            s_curr = img['sizes'][e_curr]

            if s_curr < s_prev:
                improved += 1
                deltas.append(s_prev - s_curr)
                pct_deltas.append((s_prev - s_curr) / s_prev * 100)
            elif s_curr > s_prev:
                worse += 1
                deltas.append(-(s_curr - s_prev))
                pct_deltas.append(-(s_curr - s_prev) / s_prev * 100)
            else:
                same += 1

        med_delta = np.median(deltas) if deltas else 0
        med_pct = np.median(pct_deltas) if pct_deltas else 0

        strat_prev = strategy_names.get(e_prev, '?')
        strat_curr = strategy_names.get(e_curr, '?')
        change = f"{strat_prev}->{strat_curr}" if strat_prev != strat_curr else ""

        print(f"  e{e_prev:>2}->e{e_curr:<2} {improved:>8} {same:>6} {worse:>6} {med_delta:>+7.0f}B {med_pct:>+7.3f}% {change}")

    # ============================================================
    # 4. STRATEGY BOUNDARY ANALYSIS — do strategy changes help?
    # ============================================================
    print("\n" + "=" * 80)
    print("STRATEGY BOUNDARY ANALYSIS")
    print("=" * 80)

    # Compare last effort of one strategy vs first of next
    boundaries = [
        (4, 5, 'Turbo->FastHt'),
        (9, 10, 'FastHt->Greedy (also 5->9 strats)'),
        (10, 11, 'Greedy->Lazy'),
        (17, 18, 'Lazy->Lazy2'),
        (22, 23, 'Lazy2->NearOpt'),
        (30, 31, 'NearOpt->FullOpt (also no screen)'),
    ]

    for e_before, e_after, label in boundaries:
        if e_before not in efforts or e_after not in efforts:
            continue

        improved = 0
        same = 0
        worse = 0
        deltas_pct = []

        for img in images:
            s_before = img['sizes'][e_before]
            s_after = img['sizes'][e_after]
            pct = (s_after - s_before) / s_before * 100
            deltas_pct.append(pct)

            if s_after < s_before:
                improved += 1
            elif s_after > s_before:
                worse += 1
            else:
                same += 1

        med_pct = np.median(deltas_pct)
        p5 = np.percentile(deltas_pct, 5)
        p95 = np.percentile(deltas_pct, 95)
        print(f"\n  {label}:")
        print(f"    Improved: {improved}/{n} ({improved/n*100:.0f}%), Same: {same}, Worse: {worse}")
        print(f"    Median Δ: {med_pct:+.3f}%, Range: [{p5:+.3f}%, {p95:+.3f}%]")

    # ============================================================
    # 5. NEVER-HELPFUL IDENTIFICATION
    # ============================================================
    print("\n" + "=" * 80)
    print("EFFORT LEVELS THAT NEVER WIN (vs running min from all lower efforts)")
    print("=" * 80)

    for e in efforts[1:]:
        # For how many images does this effort produce the best-so-far result?
        wins = 0
        for img in images:
            # Running min from efforts 0..e-1
            prev_min = min(img['sizes'][ep] for ep in efforts if ep < e)
            if img['sizes'][e] < prev_min:
                wins += 1

        win_pct = wins / n * 100
        strat = strategy_names.get(e, '?')
        marker = " <-- NEVER WINS" if wins == 0 else (" <-- RARELY WINS" if win_pct < 5 else "")
        print(f"  e{e:>2} ({strat:>8}): wins on {wins:>3}/{n} images ({win_pct:>5.1f}%){marker}")

    # ============================================================
    # 6. TIME-EFFECTIVENESS ANALYSIS
    # ============================================================
    print("\n" + "=" * 80)
    print("TIME-EFFECTIVENESS (marginal bytes saved per marginal ms)")
    print("=" * 80)

    print(f"\n{'Effort':>6} {'Strategy':>8} {'Agg Size':>12} {'Agg ms':>10} {'Δ bytes':>10} {'Δ ms':>10} {'bytes/ms':>10}")
    print("-" * 80)

    prev_total_size = None
    prev_total_ms = None
    for e in efforts:
        total_size = sum(img['sizes'][e] for img in images)
        total_ms = sum(img['times'][e] for img in images)
        strat = strategy_names.get(e, '?')

        if prev_total_size is not None:
            d_size = total_size - prev_total_size
            d_ms = total_ms - prev_total_ms
            if d_ms > 0:
                effectiveness = d_size / d_ms  # negative = good (saving bytes)
                print(f"{e:>6} {strat:>8} {total_size:>12,} {total_ms:>10,} {d_size:>+10,} {d_ms:>+10,} {effectiveness:>+9.1f}")
            else:
                print(f"{e:>6} {strat:>8} {total_size:>12,} {total_ms:>10,} {d_size:>+10,} {d_ms:>+10,}         —")
        else:
            print(f"{e:>6} {strat:>8} {total_size:>12,} {total_ms:>10,}          —          —         —")

        prev_total_size = total_size
        prev_total_ms = total_ms

    # ============================================================
    # 7. PER-IMAGE BEST EFFORT (what effort is "enough"?)
    # ============================================================
    print("\n" + "=" * 80)
    print("DIMINISHING RETURNS: effort at which 95%/99%/100% of max savings achieved")
    print("=" * 80)

    thresholds = [0.95, 0.99, 1.00]
    effort_at_threshold = {t: [] for t in thresholds}

    for img in images:
        s0 = img['sizes'][efforts[0]]
        s_min = min(img['sizes'][e] for e in efforts)
        total_savings = s0 - s_min
        if total_savings <= 0:
            for t in thresholds:
                effort_at_threshold[t].append(0)
            continue

        for t in thresholds:
            target = s0 - total_savings * t
            found = False
            for e in efforts:
                if img['sizes'][e] <= target:
                    effort_at_threshold[t].append(e)
                    found = True
                    break
            if not found:
                effort_at_threshold[t].append(efforts[-1])

    for t in thresholds:
        vals = effort_at_threshold[t]
        med = np.median(vals)
        p75 = np.percentile(vals, 75)
        p95 = np.percentile(vals, 95)
        print(f"  {t*100:.0f}% of savings: median e{med:.0f}, 75th e{p75:.0f}, 95th e{p95:.0f}")


if __name__ == '__main__':
    main()
