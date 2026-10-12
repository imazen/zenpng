# Encode thread scaling (2026-10-08)

Sets the encoder's default thread use (`strip_threads`, and the effort 16+
rules in `compress_filtered`, `src/encoder/compress.rs`). Bar, from the
brief: threads on by default where they make an encode at least 1.5x faster
at half the ideal efficiency or better (speedup / threads >= 0.5). The
output is identical at every thread count (`tests/thread_determinism.rs`),
so only speed and memory change.

- Bench: `benches/pareto.rs --group=enc` with `ZENPNG_PARETO_MT_THREADS=2,4,8`
  (arms `zenpng_e<E>_st` = `max_threads 1`, `zenpng_e<E>_t<N>` = parallel,
  `max_threads N`), `ZENPNG_PARETO_NO_OTHERS=1`, `ZENPNG_PARETO_NO_IDOT=1`.
  Table: `scripts/thread_scaling.py <zenbench results>`: median over images
  of (ST median time / N-thread median time), its range, and speedup / N.
  Arms with fewer than 3 rounds are dropped (zenbench's 120 s per-group wall
  clock; `ZENPNG_PARETO_MAX_WALL` raises it).
- Inputs: 20 imazen-26 renditions: RGB8 at 1024 px (8: 1207 1407 2207 5207
  6807 7007 8107 9097; 3-6 strips), RGBA8 at 1024 px (4: 1207 6807 8107
  9007; 5-8 strips), RGB8 at 2048 px (4: 1207 1407 5207 6807; 18 strips),
  RGB8 at 4096 px (4: 1407 2207 6607 6807; 65-75 strips). Strips are
  512 KiB of filtered rows.
- Efforts 1-15 code: strip layout with image-wide filter choice but before
  each strip also refined its own winner (+3-15% work per strip, all of it
  inside the strips, so it parallelises the same way).

## Efforts 1-15, x86 (i265 Core Ultra 7 265K, P-cores 0-7, `nice -n19`)

The 4096 px rows at e15 are missing (0 rounds, wall clock). At 1024 px the
8-thread arm runs one thread per strip at most (3-8 strips), so its
efficiency column understates the threads it used.

| effort | input | n | t2 | eff | t4 | eff | t8 | eff |
|---|---|---|---|---|---|---|---|---|
| e1 | rgb8_1024 | 8 | 1.13 (0.86-1.21) | 0.56 | 1.19 (0.91-1.44) | 0.30 | 1.23 (0.88-1.30) | 0.15 |
| e1 | rgb8_2048 | 4 | 1.51 (1.35-1.58) | 0.76 | 2.05 (1.59-2.24) | 0.51 | 2.24 (1.74-2.62) | 0.28 |
| e1 | rgb8_4096 | 4 | 1.80 (1.75-1.91) | 0.90 | 2.84 (2.66-3.00) | 0.71 | 3.78 (3.05-3.97) | 0.47 |
| e1 | rgba8_1024 | 4 | 1.24 (1.11-1.52) | 0.62 | 1.29 (1.26-1.75) | 0.32 | 1.30 (1.12-2.01) | 0.16 |
| e2 | rgb8_1024 | 8 | 1.51 (1.22-1.58) | 0.75 | 2.04 (1.68-2.36) | 0.51 | 2.21 (1.63-2.44) | 0.28 |
| e2 | rgb8_2048 | 4 | 1.79 (1.69-1.84) | 0.89 | 2.84 (2.55-2.97) | 0.71 | 4.07 (3.52-4.50) | 0.51 |
| e2 | rgb8_4096 | 4 | 1.96 (1.90-1.97) | 0.98 | 3.47 (3.43-3.59) | 0.87 | 5.58 (5.10-5.92) | 0.70 |
| e2 | rgba8_1024 | 4 | 1.52 (1.35-1.68) | 0.76 | 1.90 (1.75-2.64) | 0.47 | 2.41 (2.17-3.80) | 0.30 |
| e5 | rgb8_1024 | 8 | 1.61 (1.40-1.80) | 0.80 | 2.40 (1.62-2.99) | 0.60 | 2.60 (1.64-3.05) | 0.32 |
| e5 | rgb8_2048 | 4 | 1.85 (1.73-1.90) | 0.93 | 3.19 (2.75-3.22) | 0.80 | 5.10 (3.82-5.16) | 0.64 |
| e5 | rgb8_4096 | 4 | 1.96 (1.95-1.99) | 0.98 | 3.66 (3.61-3.68) | 0.92 | 6.67 (6.28-6.82) | 0.83 |
| e5 | rgba8_1024 | 4 | 1.75 (1.48-1.77) | 0.87 | 2.50 (2.20-2.74) | 0.63 | 3.85 (2.59-4.02) | 0.48 |
| e7 | rgb8_1024 | 8 | 1.79 (1.59-1.89) | 0.89 | 2.96 (1.70-3.45) | 0.74 | 3.06 (1.68-3.46) | 0.38 |
| e7 | rgb8_2048 | 4 | 1.89 (1.85-1.91) | 0.94 | 3.27 (3.18-3.34) | 0.82 | 5.35 (4.63-5.65) | 0.67 |
| e7 | rgb8_4096 | 4 | 1.95 (1.95-1.98) | 0.98 | 3.69 (3.66-3.74) | 0.92 | 6.96 (6.77-7.27) | 0.87 |
| e7 | rgba8_1024 | 4 | 1.83 (1.61-1.92) | 0.92 | 2.71 (2.39-3.42) | 0.68 | 4.50 (3.64-6.10) | 0.56 |
| e10 | rgb8_1024 | 8 | 1.77 (1.69-1.91) | 0.88 | 2.99 (1.69-3.44) | 0.75 | 3.01 (1.70-3.53) | 0.38 |
| e10 | rgb8_2048 | 4 | 1.88 (1.85-1.91) | 0.94 | 3.34 (3.19-3.48) | 0.83 | 5.34 (5.04-6.11) | 0.67 |
| e10 | rgb8_4096 | 4 | 1.96 (1.92-1.97) | 0.98 | 3.67 (3.59-3.73) | 0.92 | 6.91 (6.29-7.29) | 0.86 |
| e10 | rgba8_1024 | 4 | 1.82 (1.69-1.96) | 0.91 | 2.80 (2.46-3.58) | 0.70 | 4.34 (4.12-6.69) | 0.54 |
| e13 | rgb8_1024 | 8 | 1.81 (1.72-1.92) | 0.91 | 2.96 (1.94-3.44) | 0.74 | 2.97 (1.94-4.21) | 0.37 |
| e13 | rgb8_2048 | 4 | 1.92 (1.89-1.96) | 0.96 | 3.47 (3.27-3.62) | 0.87 | 5.69 (5.17-6.38) | 0.71 |
| e13 | rgb8_4096 | 4 | 1.95 (1.93-1.96) | 0.97 | 3.70 (3.57-3.73) | 0.93 | 7.02 (6.63-7.27) | 0.88 |
| e13 | rgba8_1024 | 4 | 1.82 (1.67-1.93) | 0.91 | 2.84 (2.47-3.60) | 0.71 | 4.38 (4.19-6.82) | 0.55 |
| e15 | rgb8_1024 | 8 | 1.80 (1.72-1.91) | 0.90 | 2.97 (2.02-3.42) | 0.74 | 2.97 (2.02-4.50) | 0.37 |
| e15 | rgb8_2048 | 4 | 1.91 (1.85-1.94) | 0.96 | 3.47 (3.22-3.60) | 0.87 | 5.68 (5.09-6.42) | 0.71 |
| e15 | rgba8_1024 | 4 | 1.81 (1.67-1.93) | 0.91 | 2.83 (2.44-3.66) | 0.71 | 4.42 (4.25-7.01) | 0.55 |

## Efforts 1-15, ARM (arm-big, Neoverse-N1, 8 vCPU)

`ZENPNG_STRIP_THREADS=all` (a thread per strip up to N; the switch is now
`ZENPNG_THREAD_RULES=off`). 4096 px from a second run with
`ZENPNG_PARETO_MAX_WALL=1500`.

| effort | input | n | t2 | eff | t4 | eff | t8 | eff |
|---|---|---|---|---|---|---|---|---|
| e1 | rgb8_1024 | 8 | 1.03 (0.65-1.35) | 0.51 | 1.35 (0.78-1.55) | 0.34 | 1.36 (0.77-1.65) | 0.17 |
| e1 | rgb8_2048 | 4 | 1.39 (1.10-1.52) | 0.69 | 1.95 (1.20-2.08) | 0.49 | 2.49 (1.58-2.59) | 0.31 |
| e1 | rgba8_1024 | 4 | 1.16 (0.95-1.27) | 0.58 | 1.44 (1.15-1.49) | 0.36 | 1.59 (1.32-1.92) | 0.20 |
| e2 | rgb8_1024 | 8 | 1.40 (1.12-1.54) | 0.70 | 1.98 (1.35-2.24) | 0.49 | 1.97 (1.32-2.67) | 0.25 |
| e2 | rgb8_2048 | 4 | 1.80 (1.65-1.81) | 0.90 | 3.10 (2.65-3.23) | 0.78 | 4.81 (3.98-5.04) | 0.60 |
| e2 | rgba8_1024 | 4 | 1.44 (1.26-1.58) | 0.72 | 1.96 (1.68-2.23) | 0.49 | 2.11 (1.93-3.14) | 0.26 |
| e5 | rgb8_1024 | 8 | 1.49 (1.16-1.76) | 0.75 | 2.12 (1.29-2.55) | 0.53 | 2.11 (1.34-2.56) | 0.26 |
| e5 | rgb8_2048 | 4 | 1.86 (1.71-1.96) | 0.93 | 3.32 (2.72-3.66) | 0.83 | 5.30 (4.02-6.64) | 0.66 |
| e5 | rgba8_1024 | 4 | 1.56 (1.35-1.63) | 0.78 | 2.22 (2.04-2.73) | 0.55 | 2.77 (2.31-3.29) | 0.35 |
| e7 | rgb8_1024 | 8 | 1.69 (1.35-1.88) | 0.85 | 2.71 (1.43-3.38) | 0.68 | 2.74 (1.41-3.36) | 0.34 |
| e7 | rgb8_2048 | 4 | 1.89 (1.85-1.96) | 0.95 | 3.55 (3.41-3.60) | 0.89 | 6.04 (4.62-6.55) | 0.75 |
| e7 | rgba8_1024 | 4 | 1.66 (1.58-1.73) | 0.83 | 2.68 (2.25-3.06) | 0.67 | 3.34 (3.11-4.30) | 0.42 |
| e13 | rgb8_1024 | 8 | 1.83 (1.69-1.97) | 0.92 | 3.23 (2.23-3.70) | 0.81 | 3.21 (2.19-4.47) | 0.40 |
| e13 | rgb8_2048 | 4 | 1.93 (1.90-1.97) | 0.97 | 3.65 (3.42-3.66) | 0.91 | 6.35 (5.47-6.68) | 0.79 |
| e13 | rgba8_1024 | 4 | 1.89 (1.71-1.97) | 0.94 | 3.26 (2.97-3.83) | 0.81 | 3.71 (3.57-7.33) | 0.46 |
| e1 | rgb8_4096 | 4 | 1.68 (1.64-1.78) | 0.84 | 2.60 (2.39-2.83) | 0.65 | 3.86 (3.19-4.23) | 0.48 |
| e2 | rgb8_4096 | 4 | 1.90 (1.86-1.93) | 0.95 | 3.61 (3.52-3.65) | 0.90 | 6.50 (6.08-6.55) | 0.81 |
| e5 | rgb8_4096 | 4 | 1.93 (1.92-1.96) | 0.97 | 3.70 (3.63-3.82) | 0.92 | 6.52 (6.20-6.89) | 0.82 |
| e13 | rgb8_4096 | 4 | 1.99 (1.97-2.00) | 0.99 | 3.88 (3.77-3.93) | 0.97 | 7.15 (6.75-7.58) | 0.89 |

## Efforts 16+, x86

Before this change, `max_threads` above 1 was not a cap here: screening spawned
a thread per strategy (9 from e20), refinement and Phase 4 recompression one
per candidate. They now run on a work queue of at most `max_threads` threads
(`par_map`). Measured with `ZENPNG_THREAD_RULES=off`, 3 RGB8 images at
1024 px (1207 5207 7007), `ZENPNG_PARETO_MAX_WALL=2400`, 5 rounds each:

| effort | input | n | t2 | eff | t4 | eff | t8 | eff |
|---|---|---|---|---|---|---|---|---|
| e16 | rgb8_1024 | 3 | 1.01 (1.01-1.02) | 0.51 | 1.02 (1.02-1.04) | 0.26 | 1.02 (1.02-1.03) | 0.13 |
| e19 | rgb8_1024 | 3 | 1.01 (1.01-1.01) | 0.50 | 1.01 (1.01-1.01) | 0.25 | 1.01 (1.01-1.02) | 0.13 |
| e20 | rgb8_1024 | 3 | 1.38 (1.34-1.38) | 0.69 | 2.02 (1.96-2.16) | 0.51 | 2.03 (1.98-2.18) | 0.25 |
| e21 | rgb8_1024 | 3 | 1.37 (1.34-1.38) | 0.68 | 2.08 (2.01-2.23) | 0.52 | 2.10 (2.02-2.24) | 0.26 |
| e24 | rgb8_1024 | 3 | 1.19 (1.17-1.22) | 0.59 | 1.45 (1.40-1.55) | 0.36 | 1.46 (1.40-1.55) | 0.18 |

Single runs (`examples/roundtrip_sweep.rs`, `ZENPNG_THREAD_RULES=off`,
`ROUNDTRIP_THREADS=N`, P-cores, same 3 images 1207, 5207, 7007), speedup
over one thread per image:

| effort | 2 threads | 3 threads | 4 threads |
|---|---|---|---|
| e20 | 1.36, 1.38, 1.39 | 1.98, 2.16, 2.05 | 1.98, 2.16, 2.04 |
| e21 | 1.35, 1.39, 1.33 | 2.05, 2.21, 2.01 | 2.07, 2.22, 2.01 |
| e22 | 1.32, 1.35, 1.33 | 1.90, 2.07, 1.89 | 1.90, 2.07, 1.89 |
| e23 | 1.27, 1.31, 1.29 | 1.74, 1.87, 1.75 | 1.74, 1.86, 1.75 |

| effort | 1 thread (s) | 4 threads (s) | speedup |
|---|---|---|---|
| e22 | 8.2, 5.3, 10.3 | 4.2, 2.6, 5.5 | 1.93, 2.08, 1.88 |
| e23 | 9.0, 6.1, 11.4 | 5.2, 3.2, 6.5 | 1.74, 1.88, 1.75 |
| e25 | 15.5, 8.9, 17.5 | 11.5, 6.1, 12.6 | 1.35, 1.45, 1.38 |
| e26 | 17.3, 9.6, 18.7 | 13.6, 7.0, 13.7 | 1.28, 1.37, 1.36 |
| e28 | 29.9, 14.5, 28.3 | 24.7, 10.4, 20.4 | 1.21, 1.41, 1.39 |
| e30 | 44.6, 20.9, 41.8 | 34.8, 14.0, 27.3 | 1.28, 1.49, 1.53 |

Efforts 16-19 spend their time in single-threaded brute force. Efforts 20-21
reach the bar at 4 threads; 8 add nothing (their parallel work is 9
screening jobs and 3 recompress jobs). 3 threads
give everything 4 do at e20-e23 (3 recompress jobs). From effort 24 the
single-threaded searches dominate: 1.21-1.53x with 4 threads.

## Rule (shipped)

| effort | threads (at most `max_threads`) |
|---|---|
| any, under 512 KiB of filtered rows | 1 |
| 1 | 1 below 12 strips, else a thread per 6 strips, at most 4 |
| 2-4 | 1 below 6 strips, else a thread per 2 strips |
| 5-15 | a thread per strip |
| 16-19 | 1 |
| 20-23 | at most 3 |
| 24+ (incl. 31+, not measured) | 1 |

Checks against the data: e1 at 1024 px (3-8 strips) stays at 1 (1.0-1.36x
measured); e2 at 1024 px RGB8 (4 strips) stays at 1 (1.40x ARM, 1.51x x86
with 2); e5+ at 1024 px RGB8 runs 4 threads (2.1-3.2x); e1 at 4096 px runs
4 (2.6-2.8x, 0.65-0.71).

## Memory

Peak RSS (`/usr/bin/time -v`, `examples/roundtrip_sweep.rs`, which also holds
the decoded source as RGBA16: about 430 MB of the totals) on 1407_rgb8_4096,
i265 E-cores 16-19 (the default resolves to 4 threads there):

| effort | 1 thread | default | wall 1 thread / default |
|---|---|---|---|
| e2 | 426 MB | 464 MB | 0.63 s / 0.39 s |
| e7 | 446 MB | 489 MB | 2.97 s / 1.01 s |
| e13 | 454 MB | 528 MB | 14.37 s / 3.87 s |

e21 on 1207_rgb8_1024, P-cores, before the cap (all threads): 71 MB at one
thread, 120 MB with threads, 7.46 s / 3.58 s.
