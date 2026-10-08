# Decode pipeline crossover (2026-10-08)

When does the two-thread decode pipeline (inflate on a second thread, unfilter
and expand on the caller's) reach the bar of **1.3x single-thread throughput**,
i.e. decode time <= 0.77x of one thread? Sets `pipeline_min_bytes_for` in
`src/decoder/pipeline.rs`.

- Bench: `benches/pareto.rs --group=pdec` (bench code of 37c942e) with
  `ZENPNG_PIPELINE_MIN_BYTES=0` (every image pipelines in `decode_mt`) and
  `ZENPNG_PARETO_PDEC_DECODE_ONLY=1`; arms `decode_st` (`max_threads(1)`) and
  `decode_mt` (`max_threads(0)`), `zenpng::decode`.
- Inputs: 320 PNGs from imazen-26 sources 1207 1407 2007 2207 2407 3307 5207
  5307 6607 6807 at long edges 512-3072 px (`scripts/vs_png_inputs.sh`),
  plus renditions at some sizes of 1609 3007 5007 6007 7007 8007 8107 9007
  9097 9227 and the 1024/4096 px pdec set (n per row below).
- x86: i265 (Core Ultra 7 265K), P-cores 0-7, `nice -n19`.
- ARM: arm-big (Neoverse-N1, 8 vCPU), all cores.
- Table: `scripts/pipeline_crossover.py <inputs> x86=<zenbench.txt> arm=<zenbench.txt>`;
  per-image ratios (median mt / median st) in
  `decode_pipeline_crossover_2026-10-08.tsv`.

Median of per-image mt/st by format and long edge (filtered MiB = median
`height * (row bytes + 1)`):

| format | px | n | filtered MiB | x86 | ARM |
|---|---|---|---|---|---|
| gray8 | 512 | 10 | 0.19 | 1.561 | 1.438 |
| gray8 | 768 | 10 | 0.42 | 1.036 | 1.168 |
| gray8 | 1024 | 7 | 0.75 | 0.832 | 1.114 |
| gray8 | 1536 | 10 | 1.69 | **0.728** | 0.953 |
| gray8 | 2048 | 10 | 3.00 | 0.687 | 0.846 |
| gray8 | 2560 | 10 | 4.69 | 0.677 | 0.797 |
| gray8 | 3072 | 10 | 6.75 | 0.681 | 0.795 |
| gray8 | 4096 | 3 | 12.00 | 0.659 | **0.746** |
| rgb8 | 512 | 20 | 0.56 | 1.065 | 1.423 |
| rgb8 | 768 | 20 | 1.27 | 0.862 | 1.274 |
| rgb8 | 1024 | 7 | 2.25 | **0.690** | 1.045 |
| rgb8 | 1536 | 17 | 5.06 | 0.687 | 0.857 |
| rgb8 | 2048 | 15 | 9.00 | 0.686 | 0.779 |
| rgb8 | 2560 | 14 | 14.06 | 0.664 | **0.708** |
| rgb8 | 3072 | 13 | 20.25 | 0.674 | 0.707 |
| rgb8 | 4096 | 3 | 36.00 | 0.586 | 0.603 |
| rgba8 | 512 | 10 | 0.75 | 0.931 | 1.240 |
| rgba8 | 768 | 10 | 1.69 | 0.775 | 1.083 |
| rgba8 | 1024 | 7 | 3.00 | **0.713** | 0.879 |
| rgba8 | 1536 | 10 | 6.75 | 0.679 | **0.727** |
| rgba8 | 2048 | 10 | 12.00 | 0.666 | 0.677 |
| rgba8 | 2560 | 10 | 18.75 | 0.660 | 0.673 |
| rgba8 | 3072 | 10 | 27.00 | 0.647 | 0.663 |
| rgba8 | 4096 | 3 | 48.00 | 0.571 | 0.609 |
| rgb16 | 512 | 10 | 1.13 | 0.966 | 1.304 |
| rgb16 | 768 | 10 | 2.53 | 0.806 | 1.135 |
| rgb16 | 1024 | 7 | 4.50 | 0.791 | 0.937 |
| rgb16 | 1536 | 10 | 10.13 | **0.733** | 0.801 |
| rgb16 | 2048 | 10 | 18.00 | 0.709 | 0.772 |
| rgb16 | 2560 | 10 | 28.13 | 0.708 | **0.661** |
| rgb16 | 3072 | 10 | 40.50 | 0.752 | 0.756 |
| rgb16 | 4096 | 3 | 72.00 | 0.697 | 0.675 |

Bold: the first size from which that machine passes at every larger size.
Thresholds set from it (filtered bytes; x86_64 uses the x86 column, every
other target the ARM column; gray+alpha, gray16 and RGBA16 were not measured
and take the RGB16 value):

| format | x86_64 | other targets |
|---|---|---|
| gray8 | 1.69 MiB | 12 MiB |
| RGB8 | 2.25 MiB | 14.06 MiB |
| RGBA8 | 3 MiB | 6.75 MiB |
| RGB16 (and the rest) | 10.13 MiB | 28.13 MiB |

The previous rule pipelined everything from 0.5 MiB, which on ARM made
768 px RGB8 1.27x slower and 1024 px gray8 1.11x slower. Palette and sub-byte
gray never pipeline (unchanged). `iDOT` parallel decode keeps its own
threshold (`idot::workers_for_bytes`).
