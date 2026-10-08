# Strip layout at every thread count: size cost (2026-10-08)

Efforts 1-15 now compress every image of two or more strips (about 512 KiB
of filtered rows each) strip by strip, with or without threads, so the output
no longer depends on the thread limit (`tests/thread_determinism.rs`). Strips
are compressed without history from the strips before them. Before, a
single-threaded encode compressed the whole image as one stream.

Filter choice: every strip is screened under every strategy, the
strategies with the smallest total over all strips (`top_k`) are refined in
every strip together with the strip's own screen winner, and the screen-level
output stays a candidate. Strip streaming (`push_rows`) can't see the strips
still to come and chooses per strip.

- Inputs: the 51 images of the Pareto set that span two or more strips
  (46 at 1024 px: 20 RGB8, 5 each RGBA8, gray8, RGB16, palette and
  interlaced, 1 gray1; 5 RGB8 at 4096 px). 64/256 px images are one strip and unchanged.
- Sizes: `examples/roundtrip_sweep.rs` (`ROUNDTRIP_SIZES=1`), e1-15, zenflate
  f041b61. Baseline: the whole-image encode (`ZENPNG_STRIP_BYTES=1e12` for
  4096 px; zenpng 37c942e single-threaded for 1024 px). Per-strip:
  `ZENPNG_STRIP_SELECT=strip`. "Top-k only": an intermediate version without
  each strip's own winner and screen-level output.
- Ratios are new size / whole-image size; below 1 is smaller.

| effort | image-wide (shipped) geomean | worst | per-strip (streaming) geomean | worst | image-wide top-k only, worst |
|---|---|---|---|---|---|
| e1 | 0.9925 | 1.0449 (8007_rgb8_1024) | 0.9925 | 1.0449 (8007_rgb8_1024) | 1.0449 (8007_rgb8_1024) |
| e2 | 0.9994 | 1.0013 (8007_rgb8_1024) | 0.9995 | 1.0015 (1407_rgb8_1024) | 1.0124 (9007_gray8_1024) |
| e3 | 1.0001 | 1.0070 (7007_rgb8_1024) | 1.0003 | 1.0070 (7007_rgb8_1024) | 1.0124 (9007_gray8_1024) |
| e4 | 1.0002 | 1.0078 (7007_rgb8_1024) | 1.0004 | 1.0078 (7007_rgb8_1024) | 1.0124 (9007_gray8_1024) |
| e5 | 1.0005 | 1.0097 (7007_rgb8_1024) | 1.0007 | 1.0097 (7007_rgb8_1024) | 1.0124 (9007_gray8_1024) |
| e6 | 0.9996 | 1.0100 (7007_rgb8_1024) | 0.9998 | 1.0100 (7007_rgb8_1024) | 1.0121 (9007_gray8_1024) |
| e7 | 1.0003 | 1.0108 (7007_rgb8_1024) | 1.0005 | 1.0108 (7007_rgb8_1024) | 1.0121 (9007_gray8_1024) |
| e8 | 1.0011 | 1.0251 (8007_rgb8_1024) | 1.0025 | 1.0686 (7007_rgb8_1024) | 1.0251 (8007_rgb8_1024) |
| e9 | 1.0011 | 1.0252 (8007_rgb8_1024) | 1.0026 | 1.0727 (7007_rgb8_1024) | 1.0252 (8007_rgb8_1024) |
| e10 | 1.0012 | 1.0263 (8007_rgb8_1024) | 1.0029 | 1.0822 (7007_rgb8_1024) | 1.0263 (8007_rgb8_1024) |
| e11 | 1.0012 | 1.0263 (8007_rgb8_1024) | 1.0018 | 1.0263 (8007_rgb8_1024) | 1.0263 (8007_rgb8_1024) |
| e12 | 1.0012 | 1.0136 (7007_rgb8_1024) | 1.0020 | 1.0225 (8107_rgba8_1024) | 1.0136 (7007_rgb8_1024) |
| e13 | 1.0017 | 1.0155 (7007_rgb8_1024) | 1.0026 | 1.0323 (8107_rgba8_1024) | 1.0155 (7007_rgb8_1024) |
| e14 | 1.0019 | 1.0160 (7007_rgb8_1024) | 1.0028 | 1.0345 (8107_rgba8_1024) | 1.0160 (7007_rgb8_1024) |
| e15 | 1.0014 | 1.0152 (7007_rgb8_1024) | 1.0022 | 1.0352 (8107_rgba8_1024) | 1.0152 (7007_rgb8_1024) |

The shipped choice is never larger than either alternative on any image and
effort. The remaining loss is strip independence (each strip starts with an
empty LZ77 window): e1 has one strategy, so it measures that alone (+4.5% on
8007 line art, -0.75% geomean).
zenflate's dictionary-primed strips are the planned fix.

Monotonicity (shipped choice, 51 images): inversions e7->e8 (6 images, max
+1.22% 9007_gray8, the same as the whole-image encode: e8's screen switch),
e9->e10, e10->e11, e12->e13, e13->e14 (max +0.03%), e3->e4 (one palette
image, +0.00%).

Time: refining each strip's own winner and keeping the screen output cost
1.03-1.15x the top-k-only version (median per effort, one run each on i265
E-cores, 4 threads; e2-e5 +11-15%, e8+ +3-13%).
