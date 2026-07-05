# zenpng picker code-heuristics

Interpretable, zero-load CART pickers for the zenpng lossless config space, depth-refined from the 2026-06-28 dual-model fan-out. png has no MLP peer in this fan-out (see "No MLP yet").

## What's here

- `zenpng_lossless_cart_zensim_2026-06-28.rs` — picker fit against zensim quality.
- `zenpng_lossless_cart_ssim2_2026-06-28.rs` — picker fit against SSIMULACRA2.

Both `.rs` files are >30 KB, so per house rule 7b they are **not committed
in-repo** — each is relocated to `/mnt/v/zen/picker-training/zenpng-2026-06-28/`
with a tracked `<name>.pointer.md` (path + sha256 + provenance) in this
directory. Fetch from block storage to regenerate/inspect; this README
documents their shape without needing the bytes in-tree.

Each file is a standalone, auto-generated decision tree: `pick_zenpng_lossless_heuristic(feats: &[f32], zq: f32) -> u16` returns a sweep-cell id (an index into the 17-cell list below) from the 469 zenanalyze image features and a target quality `zq` in 0..100. No model file, no runtime, no dependencies — just nested f64 comparisons that match sklearn exactly. Each file also carries a `main()` that byte-exact-verifies the tree against a `*_cases.bin` fixture. These are reference artifacts; they are not wired into the crate build.

Unlike webp lossless, the zenpng metric is not degenerate: several cells are lossy palette-quantize variants (imagequant `iq*`, zenquant `zq*`), so zensim and ssim2 produce different trees.

## Depth refinement

The fan-out's default depth-6 CART was too shallow for png's 17 cells (val mean overhead 20-27%). The depth curve below (full train-mode, held-out val) shows the knee at depth 8 — deeper barely helps and balloons the generated file roughly 7x (depth-12 is ~340 KB for a 0.5pp gain on zensim / 2.2pp on ssim2). These pickers are codegen'd at **depth 8, full train-mode**.

| depth | zensim ov_mean | ssim2 ov_mean | leaves (zensim/ssim2) |
|---|---|---|---|
| 6  | 20.2% | 21.4% | 59 / 63 |
| 8  | 16.4% | 20.2% | 200 / 207 |
| 10 | 16.1% | 19.4% | 595 / 627 |
| 12 | 15.9% | 18.0% | 1419 / 1551 |
| 16 | 16.3% | 17.9% | 3107 / 3985 |

## The CART can't match the GBDT here

Even refined, the CART stays around 13x the per-cell GBDT teacher (1.2% for both metrics). png's 17 cells mix true-lossless effort levels (identical pixels, differing only in bytes) with lossy palette-quantize cells (fewer colors, smaller files), so a hard-tree mispick can be catastrophic — p99 overhead ~300%, worst ~500%. The GBDT predicts bytes per cell and takes the argmin over reachable cells, which sidesteps that tail; a single decision tree cannot. Treat the CART as a no-load fallback, not a replacement for the GBDT.

## No MLP yet

png has no MLP bake in this fan-out: there is no `zenpng_picker` config, so the `train_hybrid` stage produced nothing (both `train_hybrid/` prefixes on R2 are empty). A trained MLP/ZNPR picker for png is a follow-up, blocked on adding that config. None of the picker `--allow-unsafe` bake issues seen on the other codecs apply here, because nothing was baked.

## Cells

17 zenpng lossless configs, in `cell_labels` order (the u16 return value indexes this list):

```
0  png-aggressive       6  png-balanced-zq128   12 png-high
1  png-balanced         7  png-balanced-zq256   13 png-intense
2  png-balanced-iq128   8  png-balanced-zq32    14 png-none
3  png-balanced-iq256   9  png-balanced-zq64    15 png-thorough
4  png-balanced-iq32    10 png-fast             16 png-turbo
5  png-balanced-iq64    11 png-fastest
```

## Provenance

- Source dumps: `s3://zentrain/dualmodel-2026-06-28/zenpng_lossless/picker_tree_ab/dump_zenpng_lossless_val.tar` (zensim) and `.../zenpng_lossless__ssim2/...` (ssim2).
- Generator: `zenmetrics/scripts/train/cart_analysis.py`, `--train-mode full --codegen-depth 8`. The fan-out's committed depth-6 default was regenerated here at depth 8.
- Split: imazen-26 origin even/odd (`zenmetrics/scripts/picker/origin_split.py`) — 59340 train / 35610 val / 20880 test rows; no rendition leakage.
- Dominant split features (GBDT permutation importance): pixel count, padded-pixel size buckets, patch fraction.

## Regenerating

```
# fetch and extract a dump tar from R2, then:
python3 zenmetrics/scripts/train/cart_analysis.py \
  --dump-dir <extracted>/val --codec-tag zenpng_lossless --eval-split val \
  --train-mode full --depths 6,8,10,12,16 \
  --codegen-depth 8 --codegen-out zenpng_lossless_cart.rs
```
