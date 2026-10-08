# Streaming and decode-threading timing, x86 (2026-10-07/08)

- Host i265 (Core Ultra 7 265K), P-cores 0-7, nice 19, zenbench lock; benches/pareto.rs groups below; zenflate f041b61 (= df005e3 src).
- Ratios are medians of per-image ratios; "ms" is the median per-image mean time of the reference arm.
- "mt" = threads allowed (zenpng::decode max_threads 0; zencodec default Parallel threading policy, 8 cores); "st" = max_threads 1 / Sequential policy.

## 1. Decode: two-thread pipeline and iDOT, per API (`--group=pdec`, zenpng 6501bbd + 92e2797's bench fix)

Inputs ~/tmp/pdec_in: 7 imazen-26 sources (1207 1407 2207 5207 6807 8107 9007) as gray8 / pal8 / RGB8 / RGBA8 / RGB16 at 256 / 1024 / 4096 px
(scripts/vs_png_inputs.sh SIZES="256 1024 4096" RGBA_SIZES="256 1024 4096" FORMAT_SIZES="256 1024 4096" FORMAT_SOURCES=...; 3 sources reach 4096).
Every API's output is checked against zenpng::decode before timing. iDOT columns: each input re-encoded by zenpng at effort 7 with
`with_decode_segments(8)` (default downcasts), decoded serially (`idot_st`) and in parallel (`idot_mt`); "iDOT written" counts the inputs whose
re-encode got an iDOT chunk. The RGB16 inputs hold 8-bit values (x257), so their re-encodes are 8-bit RGB: their iDOT columns compare an 8-bit
file. pal8 re-encodes stay palette.

| format | px | n | decode_st ms | decode mt/st | push mt/st | stream mt/st | push_st/decode_st | stream_st/decode_st | iDOT written | idot_st ms | idot mt/st | idot_mt/decode_mt |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| gray8 | 256 | 7 | 0.098 | 1.000 | 1.000 | 1.001 | 1.009 | 1.012 | 0/7 | — | — | — |
| gray8 | 1024 | 7 | 1.55 | 0.852 | 0.859 | 0.871 | 1.006 | 1.005 | 0/7 | — | — | — |
| gray8 | 4096 | 3 | 31.2 | 0.652 | 0.654 | 0.672 | 1.006 | 0.996 | 3/3 | 30.7 | 0.287 | 0.453 |
| pal8 | 256 | 7 | 0.0673 | 1.003 | 0.999 | 0.999 | 1.001 | 1.025 | 0/7 | — | — | — |
| pal8 | 1024 | 7 | 1.06 | 1.000 | 1.000 | 1.000 | 1.000 | 0.999 | 0/7 | — | — | — |
| pal8 | 4096 | 3 | 27.6 | 1.000 | 0.994 | 1.000 | 1.028 | 1.108 | 3/3 | 44.2 | 0.316 | 0.493 |
| rgb8 | 256 | 7 | 0.216 | 0.999 | 1.000 | 0.999 | 0.999 | 1.001 | 0/7 | — | — | — |
| rgb8 | 1024 | 7 | 3.63 | 0.704 | 0.684 | 0.693 | 0.994 | 0.993 | 6/7 | 4.11 | 0.597 | 1.006 |
| rgb8 | 4096 | 3 | 76.7 | 0.596 | 0.642 | 0.609 | 0.974 | 0.985 | 3/3 | 88.8 | 0.178 | 0.324 |
| rgba8 | 256 | 7 | 0.286 | 1.000 | 0.999 | 0.999 | 1.000 | 1.001 | 0/7 | — | — | — |
| rgba8 | 1024 | 7 | 4.79 | 0.712 | 0.706 | 0.707 | 0.992 | 0.987 | 7/7 | 5.1 | 0.575 | 0.893 |
| rgba8 | 4096 | 3 | 96 | 0.585 | 0.677 | 0.592 | 0.975 | 0.997 | 3/3 | 110 | 0.180 | 0.320 |
| rgb16 | 256 | 7 | 0.315 | 0.999 | 1.001 | 0.999 | 0.988 | 0.997 | 0/7 | — | — | — |
| rgb16 | 1024 | 7 | 4.86 | 0.819 | 0.812 | 0.810 | 0.951 | 0.955 | 6/7 | 4.12 | 0.596 | 0.730 |
| rgb16 | 4096 | 3 | 128 | 0.703 | 0.643 | 0.655 | 0.859 | 0.865 | 3/3 | 88.3 | 0.177 | 0.178 |

Reading it: the pipeline (inflate on a second thread) pays from about 1 MiB of filtered rows, the same for decode, push_decoder and
streaming_decoder: 0.70-0.85x at 1024 px, 0.59-0.70x at 4096 px; nothing at 256 px (below its 512 KiB threshold). Palette never pipelines
(row expansion is the bottleneck; 1.32x slower before ab39e02). An iDOT file decodes its strips in parallel in zenpng::decode only (not
push_decoder / streaming_decoder): 0.18-0.32x of its serial time at 4096 px with 8 segments, palette included; at 1024 px a 2-segment file
is 0.58-0.60x serial: equal to the pipeline on RGB8 (1.006x), 11% faster on RGBA8.

## When zenpng writes iDOT

Only when asked: `EncodeConfig::with_decode_segments(n)` with n >= 2 (default 0; no zencodec builder sets it, so zencodec callers can't).
Segments = min(n, height, 16, decoder workers for the size): 2 from 2 MiB of filtered rows, +1 per further 4 MiB; under 2 MiB no table is
written (all 256 px inputs, gray8 / pal8 at 1024 px above). Never for 1/2/4-bit gray (Apple ImageIO mis-decodes those in parallel). Costs
-0.09% to +0.40% size for n <= 8 (CHANGELOG). With `with_parallel(true)` the segments come straight from the strip encoder.

## 2. Decode: streaming vs whole-image (`--group=sdec`, zenpng PR #25 build = main de659b8 + builders; inputs ~/tmp/pareto_in, 97 files)

| format | px | n | whole_st ms | whole_mt/whole_st | push/whole_st | stream/whole_st | stream/png_rows |
|---|---|---|---|---|---|---|---|
| gray1 | 1024 | 1 | 0.0668 | 0.991 | 0.999 | 1.082 | 0.228 |
| gray8 | 1024 | 5 | 1.53 | 0.961 | 0.980 | 0.987 | 0.974 |
| pal8 | 1024 | 5 | 0.55 | 1.000 | 0.998 | 0.991 | 0.990 |
| rgb16 | 1024 | 5 | 3.79 | 0.860 | 0.776 | 0.776 | nan |
| rgb8 | 64 | 20 | 0.0154 | 1.000 | 1.017 | 1.035 | 0.964 |
| rgb8 | 256 | 20 | 0.173 | 0.993 | 0.999 | 1.003 | 0.948 |
| rgb8 | 1024 | 20 | 2.94 | 0.815 | 0.820 | 0.808 | 0.820 |
| rgb8 | 4096 | 5 | 73.8 | 0.624 | 0.634 | 0.635 | 0.627 |
| rgba8 | 256 | 5 | 0.258 | 0.992 | 0.995 | 0.998 | 0.982 |
| rgba8 | 1024 | 5 | 4.45 | 0.760 | 0.762 | 0.748 | 0.727 |


## 3. Encode: push_rows vs one-shot (`--group=senc`, same build; efforts 1/2/7 at 64-4096 px, effort 13 up to 1024 px)


Default = default downcasts (push_rows buffers at e≥2, pre-filters at e1); v = `DowncastFlags::none()` (strip streaming at e1-15); vmt = v + `with_parallel(true)` (8 threads). e13 only up to 1024 px.

| effort | px | n | whole ms (default) | push/whole default | vpush/vwhole | vmtpush/vmtwhole | vmtwhole/vwhole |
|---|---|---|---|---|---|---|---|
| 1 | 64 | 20 | 0.0166 | 0.998 | 1.120 | 1.069 | 1.580 |
| 1 | 256 | 25 | 0.12 | 0.932 | 1.017 | 0.997 | 1.102 |
| 1 | 1024 | 25 | 1.72 | 0.896 | 1.051 | 0.908 | 0.986 |
| 1 | 4096 | 5 | 37.8 | 0.870 | 0.823 | 1.040 | 0.336 |
| 2 | 64 | 20 | 0.0469 | 1.018 | 1.013 | 1.023 | 5.707 |
| 2 | 256 | 25 | 0.483 | 1.012 | 1.004 | 1.006 | 1.769 |
| 2 | 1024 | 25 | 7.73 | 1.025 | 0.957 | 0.959 | 0.440 |
| 2 | 4096 | 5 | 157 | 1.085 | 0.807 | 1.029 | 0.178 |
| 7 | 64 | 20 | 0.198 | 1.005 | 1.011 | 1.012 | 2.269 |
| 7 | 256 | 25 | 3.8 | 1.000 | 1.003 | 1.002 | 1.119 |
| 7 | 1024 | 25 | 61.2 | 1.003 | 0.994 | 0.995 | 0.304 |
| 7 | 4096 | 5 | 1.22e+03 | 1.008 | 0.967 | 0.999 | 0.139 |
| 13 | 64 | 20 | 1.3 | 1.000 | 0.993 | 1.008 | 1.304 |
| 13 | 256 | 25 | 17.7 | 1.000 | 1.000 | 1.002 | 0.938 |
| 13 | 1024 | 25 | 354 | 1.000 | 0.993 | 1.002 | 0.314 |

The vmtwhole/vwhole column above 1 at 64/256 px (threads slower than one thread on small images) is fixed in 92e2797: images under 512 KiB
of filtered rows now encode single-threaded (MT/ST 0.98-1.00 on 45 images at 64/256 px, efforts 1/2/7, identical bytes).
