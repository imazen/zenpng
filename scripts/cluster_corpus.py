#!/usr/bin/env python3
"""Cluster corpus images by compression behavior to select representative subset.

Reads CSV files from corpus_profiler, normalizes compression ratios,
clusters with KMeans, and selects representative images from each cluster.

Usage: python3 scripts/cluster_corpus.py [--target N]
"""

import sys
import os
import csv
import numpy as np
from pathlib import Path

def load_csv(path):
    """Load profiler CSV, return (filenames, features, metadata)."""
    rows = []
    with open(path) as f:
        reader = csv.DictReader(f)
        headers = reader.fieldnames
        # Find effort columns (e*_size)
        effort_cols = [h for h in headers if h.endswith('_size')]

        for row in reader:
            # Skip rows with zero sizes (failed encoding)
            sizes = []
            valid = True
            for col in effort_cols:
                s = int(row[col])
                if s == 0:
                    valid = False
                    break
                sizes.append(s)
            if not valid:
                continue

            raw = int(row['raw_bytes'])
            if raw == 0:
                continue

            fname = row['filename']
            w = int(row['width'])
            h = int(row['height'])
            ct = row['color_type']
            bpp = int(row['bpp'])
            fsize = int(row['filesize'])

            # Features: compression ratios at each effort level
            ratios = [s / raw for s in sizes]

            # Additional feature: ratio of improvement from fastest to slowest
            if ratios[0] > 0:
                improvement_range = (ratios[0] - ratios[-1]) / ratios[0]
            else:
                improvement_range = 0

            # Feature: how much each step improves over the previous
            step_improvements = []
            for j in range(1, len(ratios)):
                if ratios[j-1] > 0:
                    step_improvements.append((ratios[j-1] - ratios[j]) / ratios[j-1])
                else:
                    step_improvements.append(0)

            rows.append({
                'filename': fname,
                'filepath': path.parent.parent.parent / 'corpus-builder' / path.stem / fname,
                'source_dir': path.stem,
                'width': w,
                'height': h,
                'color_type': ct,
                'bpp': bpp,
                'filesize': fsize,
                'raw_bytes': raw,
                'ratios': ratios,
                'improvement_range': improvement_range,
                'step_improvements': step_improvements,
            })

    return rows, effort_cols

def main():
    target = 100
    for i, arg in enumerate(sys.argv[1:]):
        if arg == '--target' and i + 2 < len(sys.argv):
            target = int(sys.argv[i + 2])

    profile_dir = Path(os.environ.get('ZENPNG_OUTPUT_DIR', '/mnt/v/output/zenpng')) / 'corpus_profile'

    # Load all CSVs
    all_rows = []
    for csv_path in sorted(profile_dir.glob('*.csv')):
        rows, effort_cols = load_csv(csv_path)
        print(f"Loaded {len(rows)} valid images from {csv_path.name}")
        all_rows.extend(rows)

    if not all_rows:
        print("No data found!")
        sys.exit(1)

    print(f"\nTotal: {len(all_rows)} valid images")

    # Build feature matrix for clustering
    # Features: compression ratios + step improvements + log(raw_bytes)
    features = []
    for r in all_rows:
        feat = list(r['ratios'])  # absolute compression ratios
        feat.extend(r['step_improvements'])  # per-step improvement rates
        feat.append(r['improvement_range'])  # total improvement range
        feat.append(np.log10(r['raw_bytes']))  # image size (log scale)
        feat.append(r['bpp'] / 8.0)  # color depth normalized
        features.append(feat)

    X = np.array(features)

    # Normalize features to zero mean, unit variance
    from sklearn.preprocessing import StandardScaler
    scaler = StandardScaler()
    X_scaled = scaler.fit_transform(X)

    # Determine cluster count per source directory
    source_counts = {}
    for r in all_rows:
        src = r['source_dir']
        source_counts[src] = source_counts.get(src, 0) + 1

    total_images = len(all_rows)
    print(f"\nSource distribution:")
    for src, cnt in sorted(source_counts.items()):
        pct = cnt / total_images * 100
        print(f"  {src}: {cnt} images ({pct:.1f}%)")

    # Allocate cluster budget proportional to source count, minimum 5 per source
    n_sources = len(source_counts)
    budget = {}
    remaining = target
    for src in source_counts:
        budget[src] = max(5, int(target * source_counts[src] / total_images))
        remaining -= budget[src]
    # Distribute any remaining budget to largest sources
    if remaining > 0:
        sorted_sources = sorted(source_counts.items(), key=lambda x: -x[1])
        for src, _ in sorted_sources:
            if remaining <= 0:
                break
            budget[src] += 1
            remaining -= 1
    # If over budget, trim from largest allocations
    while sum(budget.values()) > target:
        sorted_sources = sorted(budget.items(), key=lambda x: -x[1])
        for src, cnt in sorted_sources:
            if sum(budget.values()) <= target:
                break
            if cnt > 5:
                budget[src] -= 1

    print(f"\nCluster budget (target={target}):")
    for src in sorted(budget):
        print(f"  {src}: {budget[src]} images")

    # Cluster each source separately for better within-source diversity
    from sklearn.cluster import KMeans

    selected = []

    for src in sorted(source_counts):
        src_indices = [i for i, r in enumerate(all_rows) if r['source_dir'] == src]
        n_clusters = min(budget[src], len(src_indices))

        if n_clusters <= 0:
            continue

        X_src = X_scaled[src_indices]

        if len(src_indices) <= n_clusters:
            # Fewer images than budget — take all
            for idx in src_indices:
                selected.append(all_rows[idx])
            continue

        # KMeans clustering
        kmeans = KMeans(n_clusters=n_clusters, random_state=42, n_init=10)
        labels = kmeans.fit_predict(X_src)

        # From each cluster, pick the image closest to centroid (most representative)
        for c in range(n_clusters):
            cluster_mask = labels == c
            cluster_indices = np.array(src_indices)[cluster_mask]
            cluster_features = X_src[cluster_mask]

            # Distance to centroid
            centroid = kmeans.cluster_centers_[c]
            dists = np.linalg.norm(cluster_features - centroid, axis=1)
            best_local = np.argmin(dists)
            best_global = cluster_indices[best_local]

            selected.append(all_rows[best_global])

    print(f"\nSelected {len(selected)} images:")

    # Group by source for display
    by_source = {}
    for r in selected:
        src = r['source_dir']
        by_source.setdefault(src, []).append(r)

    for src in sorted(by_source):
        imgs = by_source[src]
        print(f"\n  === {src} ({len(imgs)} images) ===")
        for r in sorted(imgs, key=lambda x: x['filename']):
            ratios_str = ' '.join(f'{x:.3f}' for x in r['ratios'])
            print(f"    {r['filename']:<60s} {r['width']:>5d}x{r['height']:<5d} {r['color_type']:>6s} ratios=[{ratios_str}] imp={r['improvement_range']:.3f}")

    # Write selection list
    out_path = profile_dir / 'selected_corpus.txt'
    with open(out_path, 'w') as f:
        f.write(f"# Representative corpus subset ({len(selected)} images)\n")
        f.write(f"# Selected via KMeans clustering on compression ratio profiles\n")
        f.write(f"# Format: source_dir/filename\n\n")
        for r in sorted(selected, key=lambda x: (x['source_dir'], x['filename'])):
            f.write(f"{r['source_dir']}/{r['filename']}\n")
    print(f"\nSelection list written to {out_path}")

    # Write detailed CSV for analysis
    detail_path = profile_dir / 'selected_corpus_detail.csv'
    with open(detail_path, 'w') as f:
        f.write("source_dir,filename,width,height,color_type,bpp,filesize,raw_bytes")
        for i in range(len(selected[0]['ratios'])):
            f.write(f",ratio_{i}")
        f.write(",improvement_range\n")
        for r in sorted(selected, key=lambda x: (x['source_dir'], x['filename'])):
            f.write(f"{r['source_dir']},{r['filename']},{r['width']},{r['height']},{r['color_type']},{r['bpp']},{r['filesize']},{r['raw_bytes']}")
            for ratio in r['ratios']:
                f.write(f",{ratio:.6f}")
            f.write(f",{r['improvement_range']:.6f}\n")
    print(f"Detail CSV written to {detail_path}")

    # Print summary statistics
    print(f"\n=== Selection Statistics ===")
    all_improvements = [r['improvement_range'] for r in selected]
    all_ratios_fast = [r['ratios'][0] for r in selected]
    all_ratios_slow = [r['ratios'][-1] for r in selected]
    all_sizes = [r['raw_bytes'] for r in selected]

    print(f"  Compression ratio (fastest): {np.min(all_ratios_fast):.3f} - {np.max(all_ratios_fast):.3f} (median {np.median(all_ratios_fast):.3f})")
    print(f"  Compression ratio (slowest): {np.min(all_ratios_slow):.3f} - {np.max(all_ratios_slow):.3f} (median {np.median(all_ratios_slow):.3f})")
    print(f"  Improvement range: {np.min(all_improvements):.3f} - {np.max(all_improvements):.3f} (median {np.median(all_improvements):.3f})")
    print(f"  Image size (raw bytes): {np.min(all_sizes):,} - {np.max(all_sizes):,} (median {int(np.median(all_sizes)):,})")

    # Color type distribution
    ct_counts = {}
    for r in selected:
        ct = r['color_type']
        ct_counts[ct] = ct_counts.get(ct, 0) + 1
    print(f"  Color types: {dict(sorted(ct_counts.items()))}")

    # Copy selected files to output directory
    corpus_dir = Path(os.environ.get('ZENPNG_OUTPUT_DIR', '/mnt/v/output/zenpng')) / 'test_corpus'
    corpus_dir.mkdir(parents=True, exist_ok=True)

    copied = 0
    for r in selected:
        # Try to find the source file
        src_dir_name = r['source_dir']
        fname = r['filename']
        cb_base = os.environ.get('CORPUS_BUILDER_OUTPUT_DIR', '/mnt/v/output/corpus-builder')
        src_path = Path(f'{cb_base}/{src_dir_name}/{fname}')
        if src_path.exists():
            dst = corpus_dir / f"{src_dir_name}__{fname}"
            if not dst.exists():
                import shutil
                shutil.copy2(src_path, dst)
            copied += 1

    print(f"\nCopied {copied}/{len(selected)} files to {corpus_dir}")

if __name__ == '__main__':
    main()
