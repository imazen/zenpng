# benches/pareto.rs --group=dec, x86 i265 (Core Ultra 7 265K) P-cores (taskset -c 0-7), nice 19, zenbench lock. 2026-10-07.
# zenpng 0f75a1b0bf2ca1bfed8bd92c11e534e4294e42b0 (before f9ab5a1's decoder box/slack change). zenflate png-mode 66ead6f739cb3be4ba27ad41230c9da5938ec719 (clean; 32-byte match copy, v3 streaming loop, v4 one-shot).
# 97 inputs from scripts/vs_png_inputs.sh SIZES="64 256 1024 4096" RGBA_SIZES="256 1024" (~/tmp/pareto_in, same set as pareto_x86_decode_2026-10-06.md). zenpng_mt = max_threads(0) (pipeline / iDOT); others single-threaded.
# Since 2026-10-06 (zenflate 8f886ab): zenpng_st median vs image-png went tiny 1.11 -> 1.063, overall 1.07 -> ~1.0 (large 1.017, medium 1.004, small 0.996).
# Command: ZENPNG_PARETO_DIR=~/tmp/pareto_in nice -n19 taskset -c 0-7 <pareto bench> --bench --group=dec; report scripts/pareto_report.py.

## dec

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 96 | 0.0128 + 0.942·MP |  |
| png | 96 | 0.0095 + 1.302·MP |  |
| zenpng_mt | 96 | 0.0128 + 0.330·MP |  |
| zenpng_st | 96 | 0.0129 + 0.312·MP |  |
| zune | 96 | 0.0135 + 0.634·MP |  |

### dec by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.623 |
| png | 1.000 |
| zenpng_st | 1.017 |
| lodepng | 1.224 |
| zune | 1.554 |

**medium** (n=46)

| arm | time |
|---|---|
| zenpng_mt | 0.913 |
| png | 1.000 |
| zenpng_st | 1.004 |
| lodepng | 1.221 |
| zune | 1.238 |

**small** (n=25)

| arm | time |
|---|---|
| zenpng_st | 0.996 |
| zenpng_mt | 0.997 |
| png | 1.000 |
| lodepng | 1.209 |
| zune | 1.227 |

**tiny** (n=20)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.061 |
| zenpng_st | 1.063 |
| zune | 1.216 |
| lodepng | 1.225 |

### dec by content: median time / png time

**document** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.983 |
| png | 1.000 |
| zenpng_st | 1.011 |
| lodepng | 1.166 |
| zune | 1.208 |

**lineart** (n=16)

| arm | time |
|---|---|
| zenpng_mt | 0.937 |
| zenpng_st | 0.976 |
| png | 1.000 |
| lodepng | 1.117 |
| zune | 1.127 |

**mixed** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.026 |
| zenpng_st | 1.050 |
| lodepng | 1.333 |
| zune | 1.396 |

**photo** (n=32)

| arm | time |
|---|---|
| zenpng_mt | 0.986 |
| png | 1.000 |
| zenpng_st | 1.004 |
| lodepng | 1.309 |
| zune | 1.314 |

**screen** (n=12)

| arm | time |
|---|---|
| zenpng_mt | 0.986 |
| zenpng_st | 0.993 |
| png | 1.000 |
| lodepng | 1.114 |
| zune | 1.163 |

## dec_idot

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 75 | 0.0024 + 3.779·MP |  |
| png | 75 | 0.0020 + 3.509·MP |  |
| zenpng_mt | 75 | 0.0061 + 2.175·MP |  |
| zenpng_st | 75 | 0.0021 + 3.493·MP |  |
| zune | 75 | 0.0018 + 4.236·MP |  |

### dec_idot by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.205 |
| zenpng_st | 0.991 |
| png | 1.000 |
| lodepng | 1.152 |
| zune | 1.280 |

**medium** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.618 |
| zenpng_st | 0.993 |
| png | 1.000 |
| lodepng | 1.121 |
| zune | 1.157 |

**small** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.968 |
| zenpng_st | 0.970 |
| png | 1.000 |
| zune | 1.100 |
| lodepng | 1.124 |

**tiny** (n=20)

| arm | time |
|---|---|
| zenpng_mt | 0.928 |
| zenpng_st | 0.930 |
| png | 1.000 |
| lodepng | 1.058 |
| zune | 1.152 |

### dec_idot by content: median time / png time

**document** (n=17)

| arm | time |
|---|---|
| zenpng_mt | 0.873 |
| zenpng_st | 0.961 |
| png | 1.000 |
| lodepng | 1.051 |
| zune | 1.084 |

**lineart** (n=11)

| arm | time |
|---|---|
| zenpng_mt | 0.963 |
| zenpng_st | 0.997 |
| png | 1.000 |
| lodepng | 1.101 |
| zune | 1.111 |

**mixed** (n=11)

| arm | time |
|---|---|
| zenpng_mt | 0.850 |
| png | 1.000 |
| zenpng_st | 1.009 |
| lodepng | 1.172 |
| zune | 1.215 |

**photo** (n=28)

| arm | time |
|---|---|
| zenpng_mt | 0.924 |
| zenpng_st | 0.973 |
| png | 1.000 |
| lodepng | 1.160 |
| zune | 1.177 |

**screen** (n=8)

| arm | time |
|---|---|
| zenpng_mt | 0.931 |
| zenpng_st | 0.993 |
| png | 1.000 |
| lodepng | 1.040 |
| zune | 1.119 |


---

# Rerun on zenflate png-mode a26c4b97 (inflate asks A/B/D/F) and zenpng 093fd1c (decoder box/slack f9ab5a1, push/stream pipelines). Same host, inputs and command.
# zenpng_st vs image-png median: tiny 1.063 -> 1.042, small 0.996 -> 0.980, medium 1.004 -> 0.994, large 1.017 -> 0.993. pal8 pipelined ran 1.32x slower than single-threaded: fixed in the next commit (no pipeline for palette / sub-byte gray).

## dec

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 96 | 0.0129 + 0.948·MP |  |
| png | 96 | 0.0094 + 1.304·MP |  |
| zenpng_mt | 96 | 0.0123 + 0.326·MP |  |
| zenpng_st | 96 | 0.0124 + 0.313·MP |  |
| zune | 96 | 0.0136 + 0.648·MP |  |

### dec by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.634 |
| zenpng_st | 0.993 |
| png | 1.000 |
| lodepng | 1.223 |
| zune | 1.529 |

**medium** (n=46)

| arm | time |
|---|---|
| zenpng_mt | 0.919 |
| zenpng_st | 0.994 |
| png | 1.000 |
| lodepng | 1.219 |
| zune | 1.222 |

**small** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.980 |
| zenpng_st | 0.980 |
| png | 1.000 |
| lodepng | 1.216 |
| zune | 1.225 |

**tiny** (n=20)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_st | 1.042 |
| zenpng_mt | 1.046 |
| lodepng | 1.245 |
| zune | 1.251 |

### dec by content: median time / png time

**document** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.962 |
| zenpng_st | 0.994 |
| png | 1.000 |
| lodepng | 1.158 |
| zune | 1.211 |

**lineart** (n=16)

| arm | time |
|---|---|
| zenpng_mt | 0.941 |
| zenpng_st | 0.959 |
| png | 1.000 |
| lodepng | 1.127 |
| zune | 1.138 |

**mixed** (n=11)

| arm | time |
|---|---|
| png | 1.000 |
| zenpng_mt | 1.009 |
| zenpng_st | 1.028 |
| lodepng | 1.354 |
| zune | 1.412 |

**photo** (n=32)

| arm | time |
|---|---|
| zenpng_mt | 0.972 |
| zenpng_st | 0.996 |
| png | 1.000 |
| lodepng | 1.311 |
| zune | 1.315 |

**screen** (n=12)

| arm | time |
|---|---|
| zenpng_mt | 0.977 |
| zenpng_st | 0.977 |
| png | 1.000 |
| lodepng | 1.120 |
| zune | 1.158 |

## dec_idot

| arm | n | ms = a + b·MP (a ms, b ms/MP; relative-error fit) | bytes = a + b·MP (enc) |
|---|---|---|---|
| lodepng | 75 | 0.0023 + 3.820·MP |  |
| png | 75 | 0.0019 + 3.527·MP |  |
| zenpng_mt | 75 | 0.0054 + 2.191·MP |  |
| zenpng_st | 75 | 0.0015 + 3.459·MP |  |
| zune | 75 | 0.0018 + 4.278·MP |  |

### dec_idot by size_class: median time / png time

**large** (n=5)

| arm | time |
|---|---|
| zenpng_mt | 0.199 |
| zenpng_st | 0.955 |
| png | 1.000 |
| lodepng | 1.157 |
| zune | 1.271 |

**medium** (n=25)

| arm | time |
|---|---|
| zenpng_mt | 0.615 |
| zenpng_st | 0.989 |
| png | 1.000 |
| lodepng | 1.130 |
| zune | 1.160 |

**small** (n=25)

| arm | time |
|---|---|
| zenpng_st | 0.949 |
| zenpng_mt | 0.950 |
| png | 1.000 |
| zune | 1.091 |
| lodepng | 1.134 |

**tiny** (n=20)

| arm | time |
|---|---|
| zenpng_mt | 0.905 |
| zenpng_st | 0.907 |
| png | 1.000 |
| lodepng | 1.067 |
| zune | 1.173 |

### dec_idot by content: median time / png time

**document** (n=17)

| arm | time |
|---|---|
| zenpng_mt | 0.841 |
| zenpng_st | 0.949 |
| png | 1.000 |
| lodepng | 1.066 |
| zune | 1.091 |

**lineart** (n=11)

| arm | time |
|---|---|
| zenpng_mt | 0.952 |
| zenpng_st | 0.977 |
| png | 1.000 |
| lodepng | 1.111 |
| zune | 1.116 |

**mixed** (n=11)

| arm | time |
|---|---|
| zenpng_mt | 0.825 |
| png | 1.000 |
| zenpng_st | 1.000 |
| lodepng | 1.163 |
| zune | 1.220 |

**photo** (n=28)

| arm | time |
|---|---|
| zenpng_mt | 0.895 |
| zenpng_st | 0.948 |
| png | 1.000 |
| lodepng | 1.166 |
| zune | 1.202 |

**screen** (n=8)

| arm | time |
|---|---|
| zenpng_mt | 0.927 |
| zenpng_st | 0.957 |
| png | 1.000 |
| lodepng | 1.054 |
| zune | 1.123 |


## Streaming vs whole-image decode (benches/pareto.rs --group=sdec, same run family; whole_mt/push/stream pipeline from 512 KiB with threads; pal8 before the palette rule)

| format | edge | n | whole_mt/whole_st | push/whole_st | stream/whole_st | whole_st/png_rows | stream/png_rows |
|---|---|---|---|---|---|---|---|
| gray1 | 1024 | 1 | 0.989 | 0.990 | 1.476 | 0.204 | 0.300 |
| gray8 | 1024 | 5 | 0.994 | 0.987 | 1.026 | 1.026 | 1.006 |
| pal8 | 1024 | 5 | 1.198 | 1.207 | 1.189 | 0.964 | 1.122 |
| rgb16 | 1024 | 5 | 0.859 | 0.783 | 0.796 | nan | nan |
| rgb8 | 64 | 20 | 1.000 | 1.016 | 1.156 | 0.963 | 1.127 |
| rgb8 | 256 | 20 | 0.993 | 0.998 | 1.052 | 0.919 | 0.976 |
| rgb8 | 1024 | 20 | 0.821 | 0.820 | 0.826 | 0.983 | 0.824 |
| rgb8 | 4096 | 5 | 0.640 | 0.641 | 0.675 | 0.986 | 0.630 |
| rgba8 | 256 | 5 | 0.994 | 0.999 | 1.029 | 0.974 | 1.000 |
| rgba8 | 1024 | 5 | 0.766 | 0.771 | 0.769 | 1.007 | 0.742 |
